//! `moba_room` — 5v5 custom WoW MOBA room coordinator, 10-seat roster (`0..10`),
//! host-grace handoff, disconnection toast actions (`"play_turn"` / `"turn_ai_on"`),
//! immediate human rejoin AI revocation, and `/ws/moba/{room}` live match broadcast.

use std::collections::HashMap;
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::{broadcast, RwLock};

/// Total seats in a 5v5 MOBA match (`0..5` Alliance, `5..10` Horde).
pub const NUM_SEATS: usize = 10;

/// 30-second grace window before temporary host authority transfers to the next connected human.
pub const HOST_GRACE_MS: u64 = 30_000;

/// 3-minute (180,000 ms) central objective spawn cycle.
pub const OBJECTIVE_CYCLE_MS: u64 = 180_000;

/// Seat badge state when controlled by a connected human player.
pub const BADGE_EMERALD_HUMAN: &str = "emerald_human";
/// Seat badge state when a human player disconnected and is awaiting rejoin or AI takeover.
pub const BADGE_AMBER_DISCONNECTED: &str = "amber_disconnected";
/// Seat badge state when controlled by an AI Hero Bot or AI takeover.
pub const BADGE_BLUE_AI: &str = "blue_ai";

/// State of a single hero seat (`0..10`) in a 5v5 MOBA room.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobaSeat {
    pub seat_index: usize,
    pub team: String,
    pub role: String,
    pub class_id: u8,
    pub player_id: Option<String>,
    pub player_name: String,
    pub connected: bool,
    pub is_ai_controlled: bool,
    pub badge_state: String,
    pub pending_ai_step: u64,
    pub show_play_turn: bool,
    pub show_turn_ai_on: bool,
}

/// Active disconnection toast actions surfaced to the room host / team for a disconnected human seat.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DisconnectToast {
    pub seat_index: usize,
    pub player_id: String,
    pub player_name: String,
    pub show_play_turn: bool,
    pub show_turn_ai_on: bool,
    pub is_ai_controlled: bool,
}

/// Shared team XP, level (`1..=20`), kills, and gold pool.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TeamProgressState {
    pub team: String,
    pub level: u8,
    pub shared_xp: u32,
    pub xp_to_next_level: u32,
    pub kills: u32,
    pub gold_pool: u32,
}

/// State of a lane tower or faction Core (`Tier 1`, `Tier 2`, `Core`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TowerState {
    pub id: String,
    pub team: String,
    pub tier: String,
    pub hp: u32,
    pub max_hp: u32,
    pub destroyed: bool,
}

/// Complete snapshot of a 5v5 MOBA room returned by REST endpoints.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobaRoomSnapshot {
    pub room_id: String,
    pub map_index: usize,
    pub draft_mode: String,
    pub primary_host_id: String,
    pub acting_host_id: String,
    pub host_disconnected_at_ms: Option<u64>,
    pub host_grace_elapsed_ms: u64,
    pub host_grace_remaining_ms: u64,
    pub host_handoff_active: bool,
    pub seats: Vec<MobaSeat>,
    pub active_toasts: Vec<DisconnectToast>,
    pub alliance_team_state: TeamProgressState,
    pub horde_team_state: TeamProgressState,
    pub towers: Vec<TowerState>,
    pub event_clock_ms: u64,
    pub next_objective_spawn_ms: u64,
}

/// Summary metadata for an active MOBA room returned by `GET /api/moba/rooms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MobaRoomSummary {
    pub room_id: String,
    pub map_index: usize,
    pub draft_mode: String,
    pub primary_host_id: String,
    pub acting_host_id: String,
    pub connected_humans: usize,
    pub ai_seats: usize,
    pub disconnected_humans: usize,
}

/// Response body for `GET /api/moba/rooms`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RoomListResponse {
    pub rooms: Vec<MobaRoomSummary>,
}

/// Request body for `POST /api/moba/rooms`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateRoomRequest {
    pub room_id: String,
    #[serde(default)]
    pub map_index: usize,
    #[serde(default = "default_draft_mode")]
    pub draft_mode: String,
    #[serde(default = "default_host_id")]
    pub host_id: String,
    #[serde(default = "default_host_name")]
    pub host_name: String,
}

fn default_draft_mode() -> String {
    "all_pick".to_string()
}

fn default_host_id() -> String {
    "host-1".to_string()
}

fn default_host_name() -> String {
    "Host".to_string()
}

/// Request body for `POST /api/moba/rooms/{room}/join`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JoinRoomRequest {
    pub player_id: String,
    #[serde(default)]
    pub player_name: Option<String>,
    #[serde(default)]
    pub preferred_seat: Option<usize>,
    #[serde(default)]
    pub preferred_team: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub class_id: Option<u8>,
}

/// Request body for `POST /api/moba/rooms/{room}/disconnect`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisconnectRequest {
    #[serde(default)]
    pub player_id: Option<String>,
    #[serde(default)]
    pub seat_index: Option<usize>,
    #[serde(default)]
    pub now_ms: Option<u64>,
}

/// Request body for `POST /api/moba/rooms/{room}/ai-action`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AiActionRequest {
    pub seat_index: usize,
    pub action: String,
    #[serde(default)]
    pub now_ms: Option<u64>,
}

/// Request body for `POST /api/moba/rooms/{room}/advance-grace`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvanceGraceRequest {
    #[serde(default)]
    pub elapsed_ms: Option<u64>,
    #[serde(default)]
    pub now_ms: Option<u64>,
}

/// Structured WebSocket event payloads broadcast across `/ws/moba/{room}`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum MobaWsEvent {
    HeroMove {
        seat_index: usize,
        x: f32,
        y: f32,
        z: f32,
        facing: f32,
    },
    AbilityCast {
        seat_index: usize,
        slot: u8,
        spell_id: u32,
        target_seat: Option<usize>,
    },
    CardPick {
        seat_index: usize,
        level: u8,
        card_id: String,
    },
    ShopPurchase {
        seat_index: usize,
        item_id: u32,
        gold_spent: u32,
    },
    CourierLaunch {
        seat_index: usize,
        item_ids: Vec<u32>,
    },
    ObjectiveCapture {
        team: String,
        xp_awarded: u32,
        event_clock_ms: u64,
    },
    SeatUpdated {
        seat_index: usize,
        badge_state: String,
        is_ai_controlled: bool,
        connected: bool,
        pending_ai_step: u64,
        acting_host_id: String,
    },
}

/// Mutable state + broadcast channel for a single 5v5 MOBA room.
pub struct MobaRoom {
    pub room_id: String,
    pub map_index: usize,
    pub draft_mode: String,
    pub primary_host_id: String,
    pub acting_host_id: String,
    pub host_disconnected_at_ms: Option<u64>,
    pub host_grace_elapsed_ms: u64,
    pub host_handoff_active: bool,
    pub seats: Vec<MobaSeat>,
    pub alliance_team_state: TeamProgressState,
    pub horde_team_state: TeamProgressState,
    pub towers: Vec<TowerState>,
    pub event_clock_ms: u64,
    pub next_objective_spawn_ms: u64,
    pub tx: broadcast::Sender<String>,
}

impl MobaRoom {
    /// Creates a new 5v5 room pre-populated with 10 balanced Alliance (`0..5`) and Horde (`5..10`)
    /// seats. Seat 0 is claimed by the room host; Seats `1..10` start with Role-Aware AI Hero Bots.
    pub fn new(
        room_id: String,
        map_index: usize,
        draft_mode: String,
        host_id: String,
        host_name: String,
    ) -> Self {
        let preset_roster: [(usize, &str, &str, u8, &str); NUM_SEATS] = [
            // Alliance (0..5): 1 Tank, 1 Healer, 1 Melee DPS, 2 Ranged DPS
            (0, "alliance", "tank", 1, "Bot Lothar"),
            (1, "alliance", "healer", 2, "Bot Uther"),
            (2, "alliance", "melee_dps", 4, "Bot Valeera"),
            (3, "alliance", "ranged_dps", 8, "Bot Jaina"),
            (4, "alliance", "ranged_dps", 3, "Bot Alleria"),
            // Horde (5..10): 1 Tank, 1 Healer, 1 Melee DPS, 2 Ranged DPS
            (5, "horde", "tank", 1, "Bot Garrosh"),
            (6, "horde", "healer", 7, "Bot Thrall"),
            (7, "horde", "melee_dps", 4, "Bot Garona"),
            (8, "horde", "ranged_dps", 9, "Bot Gul'dan"),
            (9, "horde", "ranged_dps", 3, "Bot Rexxar"),
        ];

        let seats = preset_roster
            .into_iter()
            .map(|(seat_index, team, role, class_id, bot_name)| {
                if seat_index == 0 {
                    MobaSeat {
                        seat_index,
                        team: team.to_string(),
                        role: role.to_string(),
                        class_id,
                        player_id: Some(host_id.clone()),
                        player_name: host_name.clone(),
                        connected: true,
                        is_ai_controlled: false,
                        badge_state: BADGE_EMERALD_HUMAN.to_string(),
                        pending_ai_step: 0,
                        show_play_turn: false,
                        show_turn_ai_on: false,
                    }
                } else {
                    MobaSeat {
                        seat_index,
                        team: team.to_string(),
                        role: role.to_string(),
                        class_id,
                        player_id: None,
                        player_name: bot_name.to_string(),
                        connected: false,
                        is_ai_controlled: true,
                        badge_state: BADGE_BLUE_AI.to_string(),
                        pending_ai_step: 0,
                        show_play_turn: false,
                        show_turn_ai_on: false,
                    }
                }
            })
            .collect();

        let towers = vec![
            TowerState {
                id: "alliance_t1".to_string(),
                team: "alliance".to_string(),
                tier: "tier_1".to_string(),
                hp: 3500,
                max_hp: 3500,
                destroyed: false,
            },
            TowerState {
                id: "alliance_t2".to_string(),
                team: "alliance".to_string(),
                tier: "tier_2".to_string(),
                hp: 4500,
                max_hp: 4500,
                destroyed: false,
            },
            TowerState {
                id: "alliance_core".to_string(),
                team: "alliance".to_string(),
                tier: "core".to_string(),
                hp: 8000,
                max_hp: 8000,
                destroyed: false,
            },
            TowerState {
                id: "horde_t1".to_string(),
                team: "horde".to_string(),
                tier: "tier_1".to_string(),
                hp: 3500,
                max_hp: 3500,
                destroyed: false,
            },
            TowerState {
                id: "horde_t2".to_string(),
                team: "horde".to_string(),
                tier: "tier_2".to_string(),
                hp: 4500,
                max_hp: 4500,
                destroyed: false,
            },
            TowerState {
                id: "horde_core".to_string(),
                team: "horde".to_string(),
                tier: "core".to_string(),
                hp: 8000,
                max_hp: 8000,
                destroyed: false,
            },
        ];

        let (tx, _) = broadcast::channel(256);

        Self {
            room_id,
            map_index: map_index % 10,
            draft_mode,
            primary_host_id: host_id.clone(),
            acting_host_id: host_id,
            host_disconnected_at_ms: None,
            host_grace_elapsed_ms: 0,
            host_handoff_active: false,
            seats,
            alliance_team_state: TeamProgressState {
                team: "alliance".to_string(),
                level: 1,
                shared_xp: 0,
                xp_to_next_level: 1000,
                kills: 0,
                gold_pool: 500,
            },
            horde_team_state: TeamProgressState {
                team: "horde".to_string(),
                level: 1,
                shared_xp: 0,
                xp_to_next_level: 1000,
                kills: 0,
                gold_pool: 500,
            },
            towers,
            event_clock_ms: 0,
            next_objective_spawn_ms: OBJECTIVE_CYCLE_MS,
            tx,
        }
    }

    /// Evaluates the 30-second host handoff grace timer against a current timestamp `now_ms`.
    pub fn check_host_grace(&mut self, now_ms: u64) {
        if let Some(disc_at) = self.host_disconnected_at_ms {
            let elapsed = now_ms.saturating_sub(disc_at);
            self.evaluate_grace_elapsed(elapsed);
        }
    }

    /// Evaluates the 30-second host handoff grace timer given `elapsed_ms` since host disconnect.
    /// If `elapsed_ms >= HOST_GRACE_MS` (`30_000`), promotes the next connected human player
    /// to `acting_host_id`.
    pub fn evaluate_grace_elapsed(&mut self, elapsed_ms: u64) {
        self.host_grace_elapsed_ms = elapsed_ms;
        if self.host_disconnected_at_ms.is_some() && elapsed_ms >= HOST_GRACE_MS {
            if let Some(next_human_id) = self
                .seats
                .iter()
                .find(|s| {
                    s.connected
                        && s.player_id.is_some()
                        && s.player_id.as_deref() != Some(self.primary_host_id.as_str())
                })
                .and_then(|s| s.player_id.clone())
            {
                self.acting_host_id = next_human_id;
                self.host_handoff_active = true;
            }
        }
    }

    /// Builds a serializable snapshot of the room state.
    pub fn snapshot(&self) -> MobaRoomSnapshot {
        let active_toasts = self
            .seats
            .iter()
            .filter_map(|s| {
                let pid = s.player_id.as_ref()?;
                if !s.connected {
                    Some(DisconnectToast {
                        seat_index: s.seat_index,
                        player_id: pid.clone(),
                        player_name: s.player_name.clone(),
                        show_play_turn: s.show_play_turn,
                        show_turn_ai_on: s.show_turn_ai_on,
                        is_ai_controlled: s.is_ai_controlled,
                    })
                } else {
                    None
                }
            })
            .collect();

        let host_grace_remaining_ms = if self.host_disconnected_at_ms.is_some() {
            HOST_GRACE_MS.saturating_sub(self.host_grace_elapsed_ms)
        } else {
            HOST_GRACE_MS
        };

        MobaRoomSnapshot {
            room_id: self.room_id.clone(),
            map_index: self.map_index,
            draft_mode: self.draft_mode.clone(),
            primary_host_id: self.primary_host_id.clone(),
            acting_host_id: self.acting_host_id.clone(),
            host_disconnected_at_ms: self.host_disconnected_at_ms,
            host_grace_elapsed_ms: self.host_grace_elapsed_ms,
            host_grace_remaining_ms,
            host_handoff_active: self.host_handoff_active,
            seats: self.seats.clone(),
            active_toasts,
            alliance_team_state: self.alliance_team_state.clone(),
            horde_team_state: self.horde_team_state.clone(),
            towers: self.towers.clone(),
            event_clock_ms: self.event_clock_ms,
            next_objective_spawn_ms: self.next_objective_spawn_ms,
        }
    }

    /// Builds a summary entry for `GET /api/moba/rooms`.
    pub fn summary(&self) -> MobaRoomSummary {
        let connected_humans = self
            .seats
            .iter()
            .filter(|s| s.player_id.is_some() && s.connected)
            .count();
        let disconnected_humans = self
            .seats
            .iter()
            .filter(|s| s.player_id.is_some() && !s.connected)
            .count();
        let ai_seats = self.seats.iter().filter(|s| s.is_ai_controlled).count();

        MobaRoomSummary {
            room_id: self.room_id.clone(),
            map_index: self.map_index,
            draft_mode: self.draft_mode.clone(),
            primary_host_id: self.primary_host_id.clone(),
            acting_host_id: self.acting_host_id.clone(),
            connected_humans,
            ai_seats,
            disconnected_humans,
        }
    }

    fn broadcast_seat_update(&self, seat_index: usize) {
        if let Some(seat) = self.seats.get(seat_index) {
            let evt = MobaWsEvent::SeatUpdated {
                seat_index: seat.seat_index,
                badge_state: seat.badge_state.clone(),
                is_ai_controlled: seat.is_ai_controlled,
                connected: seat.connected,
                pending_ai_step: seat.pending_ai_step,
                acting_host_id: self.acting_host_id.clone(),
            };
            if let Ok(json) = serde_json::to_string(&evt) {
                let _ = self.tx.send(json);
            }
        }
    }

    fn apply_ws_event(&mut self, event: &MobaWsEvent) {
        if let MobaWsEvent::ObjectiveCapture {
            team,
            xp_awarded,
            event_clock_ms,
        } = event
        {
            self.event_clock_ms = *event_clock_ms;
            self.next_objective_spawn_ms = event_clock_ms.saturating_add(OBJECTIVE_CYCLE_MS);
            let progress = if team.eq_ignore_ascii_case("horde") {
                &mut self.horde_team_state
            } else {
                &mut self.alliance_team_state
            };
            progress.shared_xp = progress.shared_xp.saturating_add(*xp_awarded);
            while progress.level < 20 && progress.shared_xp >= progress.xp_to_next_level {
                progress.shared_xp -= progress.xp_to_next_level;
                progress.level += 1;
                progress.xp_to_next_level = u32::from(progress.level) * 1000;
            }
        }
    }
}

/// Shared multi-room registry mounted by [`router`].
#[derive(Clone, Default)]
pub struct MobaRoomsState {
    rooms: Arc<RwLock<HashMap<String, Arc<RwLock<MobaRoom>>>>>,
}

/// Builds the MOBA REST + WebSocket router (`/api/moba/rooms*` and `/ws/moba/{room}`).
pub fn router() -> Router {
    Router::new()
        .route("/api/moba/rooms", get(list_rooms).post(create_room))
        .route("/api/moba/rooms/{room}", get(get_room))
        .route("/api/moba/rooms/{room}/join", post(join_room))
        .route("/api/moba/rooms/{room}/disconnect", post(disconnect_room))
        .route("/api/moba/rooms/{room}/ai-action", post(ai_action_room))
        .route(
            "/api/moba/rooms/{room}/advance-grace",
            post(advance_grace_room),
        )
        .route("/ws/moba/{room}", get(ws_upgrade))
        .with_state(MobaRoomsState::default())
}

async fn list_rooms(State(state): State<MobaRoomsState>) -> Json<RoomListResponse> {
    let map = state.rooms.read().await;
    let mut rooms = Vec::with_capacity(map.len());
    for room_lock in map.values() {
        let room = room_lock.read().await;
        rooms.push(room.summary());
    }
    rooms.sort_by(|a, b| a.room_id.cmp(&b.room_id));
    Json(RoomListResponse { rooms })
}

async fn create_room(
    State(state): State<MobaRoomsState>,
    Json(req): Json<CreateRoomRequest>,
) -> (StatusCode, Json<MobaRoomSnapshot>) {
    let room = MobaRoom::new(
        req.room_id.clone(),
        req.map_index,
        req.draft_mode,
        req.host_id,
        req.host_name,
    );
    let snapshot = room.snapshot();
    let mut map = state.rooms.write().await;
    map.insert(req.room_id, Arc::new(RwLock::new(room)));
    (StatusCode::CREATED, Json(snapshot))
}

async fn get_room(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
) -> Result<Json<MobaRoomSnapshot>, StatusCode> {
    let room_lock = {
        let map = state.rooms.read().await;
        map.get(&room_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let room = room_lock.read().await;
    Ok(Json(room.snapshot()))
}

async fn join_room(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
    Json(req): Json<JoinRoomRequest>,
) -> Result<Json<MobaRoomSnapshot>, StatusCode> {
    let room_lock = {
        let map = state.rooms.read().await;
        map.get(&room_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let mut room = room_lock.write().await;

    // 1. Check if `player_id` already owns a seat in this room (rejoin / reconnect).
    if let Some(existing_idx) = room
        .seats
        .iter()
        .position(|s| s.player_id.as_deref() == Some(req.player_id.as_str()))
    {
        let seat = &mut room.seats[existing_idx];
        seat.connected = true;
        // Crucial: Immediately revoke AI control and clear pending AI actions on human rejoin!
        seat.is_ai_controlled = false;
        seat.badge_state = BADGE_EMERALD_HUMAN.to_string();
        seat.pending_ai_step = 0;
        seat.show_play_turn = false;
        seat.show_turn_ai_on = false;
        if let Some(name) = req.player_name.filter(|n| !n.is_empty()) {
            seat.player_name = name;
        }
        if let Some(role) = req.role {
            seat.role = role;
        }
        if let Some(class_id) = req.class_id {
            seat.class_id = class_id;
        }

        // If the primary host reconnected, immediately reclaim host authority and clear grace timer.
        if req.player_id == room.primary_host_id {
            room.acting_host_id = room.primary_host_id.clone();
            room.host_disconnected_at_ms = None;
            room.host_grace_elapsed_ms = 0;
            room.host_handoff_active = false;
        }

        room.broadcast_seat_update(existing_idx);
        return Ok(Json(room.snapshot()));
    }

    // 2. Otherwise, assign an unoccupied bot seat (`player_id.is_none()`).
    let target_idx = if let Some(pref_seat) = req.preferred_seat {
        if pref_seat < NUM_SEATS && room.seats[pref_seat].player_id.is_none() {
            Some(pref_seat)
        } else {
            None
        }
    } else if let Some(ref pref_team) = req.preferred_team {
        room.seats
            .iter()
            .position(|s| s.player_id.is_none() && s.team.eq_ignore_ascii_case(pref_team))
    } else {
        room.seats.iter().position(|s| s.player_id.is_none())
    };

    let Some(idx) = target_idx else {
        return Err(StatusCode::CONFLICT);
    };

    let seat = &mut room.seats[idx];
    seat.player_id = Some(req.player_id.clone());
    seat.player_name = req.player_name.unwrap_or_else(|| req.player_id.clone());
    if let Some(role) = req.role {
        seat.role = role;
    }
    if let Some(class_id) = req.class_id {
        seat.class_id = class_id;
    }
    seat.connected = true;
    seat.is_ai_controlled = false;
    seat.badge_state = BADGE_EMERALD_HUMAN.to_string();
    seat.pending_ai_step = 0;
    seat.show_play_turn = false;
    seat.show_turn_ai_on = false;

    room.broadcast_seat_update(idx);
    Ok(Json(room.snapshot()))
}

async fn disconnect_room(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
    Json(req): Json<DisconnectRequest>,
) -> Result<Json<MobaRoomSnapshot>, StatusCode> {
    let room_lock = {
        let map = state.rooms.read().await;
        map.get(&room_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let mut room = room_lock.write().await;

    let target_idx = if let Some(ref pid) = req.player_id {
        room.seats
            .iter()
            .position(|s| s.player_id.as_deref() == Some(pid.as_str()))
    } else if let Some(idx) = req.seat_index {
        (idx < NUM_SEATS && room.seats[idx].player_id.is_some()).then_some(idx)
    } else {
        None
    };

    let Some(idx) = target_idx else {
        return Err(StatusCode::NOT_FOUND);
    };

    let is_primary_host =
        room.seats[idx].player_id.as_deref() == Some(room.primary_host_id.as_str());

    let seat = &mut room.seats[idx];
    seat.connected = false;
    seat.is_ai_controlled = false;
    seat.badge_state = BADGE_AMBER_DISCONNECTED.to_string();
    seat.show_play_turn = true;
    seat.show_turn_ai_on = true;

    if is_primary_host {
        room.host_disconnected_at_ms = Some(req.now_ms.unwrap_or(0));
        room.host_grace_elapsed_ms = 0;
    }

    room.broadcast_seat_update(idx);
    Ok(Json(room.snapshot()))
}

async fn ai_action_room(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
    Json(req): Json<AiActionRequest>,
) -> Result<Json<MobaRoomSnapshot>, StatusCode> {
    let room_lock = {
        let map = state.rooms.read().await;
        map.get(&room_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let mut room = room_lock.write().await;

    if req.seat_index >= NUM_SEATS {
        return Err(StatusCode::BAD_REQUEST);
    }

    if let Some(now_ms) = req.now_ms {
        room.check_host_grace(now_ms);
    }

    let seat = &mut room.seats[req.seat_index];
    match req.action.as_str() {
        "play_turn" => {
            // Executes a single legal tactical AI step for a disconnected human seat without
            // permanently flipping `is_ai_controlled`.
            if seat.connected {
                return Err(StatusCode::CONFLICT);
            }
            seat.pending_ai_step = seat.pending_ai_step.saturating_add(1);
        }
        "turn_ai_on" => {
            // Activates continuous AI bot takeover until the human returns.
            if seat.connected {
                return Err(StatusCode::CONFLICT);
            }
            seat.is_ai_controlled = true;
            seat.badge_state = BADGE_BLUE_AI.to_string();
            seat.show_turn_ai_on = false;
        }
        "revoke_ai" => {
            seat.is_ai_controlled = false;
            seat.pending_ai_step = 0;
            if seat.connected {
                seat.badge_state = BADGE_EMERALD_HUMAN.to_string();
                seat.show_play_turn = false;
                seat.show_turn_ai_on = false;
            } else {
                seat.badge_state = BADGE_AMBER_DISCONNECTED.to_string();
                seat.show_play_turn = true;
                seat.show_turn_ai_on = true;
            }
        }
        _ => return Err(StatusCode::BAD_REQUEST),
    }

    room.broadcast_seat_update(req.seat_index);
    Ok(Json(room.snapshot()))
}

async fn advance_grace_room(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
    Json(req): Json<AdvanceGraceRequest>,
) -> Result<Json<MobaRoomSnapshot>, StatusCode> {
    let room_lock = {
        let map = state.rooms.read().await;
        map.get(&room_id).cloned().ok_or(StatusCode::NOT_FOUND)?
    };
    let mut room = room_lock.write().await;

    if let Some(elapsed_ms) = req.elapsed_ms {
        if room.host_disconnected_at_ms.is_none() {
            room.host_disconnected_at_ms = Some(0);
        }
        room.evaluate_grace_elapsed(elapsed_ms);
    } else if let Some(now_ms) = req.now_ms {
        room.check_host_grace(now_ms);
    }

    Ok(Json(room.snapshot()))
}

async fn ws_upgrade(
    State(state): State<MobaRoomsState>,
    Path(room_id): Path<String>,
    ws: WebSocketUpgrade,
) -> Response {
    let room_lock = {
        let mut map = state.rooms.write().await;
        Arc::clone(map.entry(room_id.clone()).or_insert_with(|| {
            Arc::new(RwLock::new(MobaRoom::new(
                room_id,
                0,
                default_draft_mode(),
                default_host_id(),
                default_host_name(),
            )))
        }))
    };

    // Subscribe before upgrading so that once the client receives 101 Switching Protocols,
    // its broadcast receiver is already active and cannot miss immediate broadcasts.
    let (tx, rx) = {
        let room = room_lock.read().await;
        (room.tx.clone(), room.tx.subscribe())
    };

    ws.on_upgrade(move |socket| handle_ws_peer(socket, room_lock, tx, rx))
        .into_response()
}

async fn handle_ws_peer(
    ws: WebSocket,
    room_lock: Arc<RwLock<MobaRoom>>,
    tx: broadcast::Sender<String>,
    mut rx: broadcast::Receiver<String>,
) {
    let (mut ws_tx, mut ws_rx) = ws.split();

    let broadcast_to_ws = async move {
        while let Ok(msg) = rx.recv().await {
            if ws_tx.send(Message::Text(msg.into())).await.is_err() {
                break;
            }
        }
    };

    let ws_to_broadcast = async move {
        while let Some(Ok(frame)) = ws_rx.next().await {
            match frame {
                Message::Text(text) => {
                    let text_str = text.to_string();
                    if let Ok(evt) = serde_json::from_str::<MobaWsEvent>(&text_str) {
                        let mut room = room_lock.write().await;
                        room.apply_ws_event(&evt);
                    }
                    let _ = tx.send(text_str);
                }
                Message::Binary(bytes) => {
                    if let Ok(text_str) = String::from_utf8(bytes.to_vec()) {
                        if let Ok(evt) = serde_json::from_str::<MobaWsEvent>(&text_str) {
                            let mut room = room_lock.write().await;
                            room.apply_ws_event(&evt);
                        }
                        let _ = tx.send(text_str);
                    }
                }
                Message::Ping(_) | Message::Pong(_) => {}
                Message::Close(_) => break,
            }
        }
    };

    tokio::select! {
        _ = broadcast_to_ws => {}
        _ = ws_to_broadcast => {}
    }
}
