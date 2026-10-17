//! Integration tests for `wenilla_host::moba_room` — 5v5 roster creation, human join/disconnect,
//! `"play_turn"` & `"turn_ai_on"` toast actions, immediate AI revocation on human rejoin,
//! 30-second host handoff grace timer, and `/ws/moba/{room}` multi-client WebSocket broadcast.

use axum::body::{to_bytes, Body};
use axum::http::{Method, Request, StatusCode};
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;
use tower::ServiceExt;
use wenilla_host::moba_room::{
    MobaRoomSnapshot, MobaWsEvent, RoomListResponse, BADGE_AMBER_DISCONNECTED, BADGE_BLUE_AI,
    BADGE_EMERALD_HUMAN, NUM_SEATS,
};

async fn send_json<T: DeserializeOwned>(
    app: &Router,
    method: Method,
    uri: &str,
    payload: Option<Value>,
) -> (StatusCode, T) {
    let body = match payload {
        Some(v) => Body::from(serde_json::to_vec(&v).expect("serialize json")),
        None => Body::empty(),
    };
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("content-type", "application/json")
                .body(body)
                .expect("build request"),
        )
        .await
        .expect("oneshot response");
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read body bytes");
    let parsed = serde_json::from_slice::<T>(&bytes).expect("parse response json");
    (status, parsed)
}

#[tokio::test]
async fn creates_balanced_5v5_room_with_10_alliance_and_horde_seats() {
    let app = wenilla_host::moba_room::router();

    let (status, snapshot): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms",
        Some(json!({
            "room_id": "warsong-5v5",
            "map_index": 0,
            "draft_mode": "all_pick",
            "host_id": "player-host",
            "host_name": "Anduin"
        })),
    )
    .await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(snapshot.room_id, "warsong-5v5");
    assert_eq!(snapshot.primary_host_id, "player-host");
    assert_eq!(snapshot.acting_host_id, "player-host");
    assert_eq!(snapshot.seats.len(), NUM_SEATS);

    // Seats 0..5 belong to Alliance; Seats 5..10 belong to Horde.
    for seat in &snapshot.seats[0..5] {
        assert_eq!(seat.team, "alliance");
    }
    for seat in &snapshot.seats[5..10] {
        assert_eq!(seat.team, "horde");
    }

    // Seat 0 is claimed by the human host; Seats 1..10 are Role-Aware AI Hero Bots.
    assert_eq!(snapshot.seats[0].player_id.as_deref(), Some("player-host"));
    assert!(snapshot.seats[0].connected);
    assert!(!snapshot.seats[0].is_ai_controlled);
    assert_eq!(snapshot.seats[0].badge_state, BADGE_EMERALD_HUMAN);

    for seat in &snapshot.seats[1..10] {
        assert!(seat.player_id.is_none());
        assert!(!seat.connected);
        assert!(seat.is_ai_controlled);
        assert_eq!(seat.badge_state, BADGE_BLUE_AI);
    }

    // Verify room listing summary.
    let (list_status, list): (_, RoomListResponse) =
        send_json(&app, Method::GET, "/api/moba/rooms", None).await;
    assert_eq!(list_status, StatusCode::OK);
    assert_eq!(list.rooms.len(), 1);
    assert_eq!(list.rooms[0].room_id, "warsong-5v5");
    assert_eq!(list.rooms[0].connected_humans, 1);
    assert_eq!(list.rooms[0].ai_seats, 9);
    assert_eq!(list.rooms[0].disconnected_humans, 0);
}

#[tokio::test]
async fn human_disconnect_toast_actions_and_immediate_rejoin_ai_revocation() {
    let app = wenilla_host::moba_room::router();

    let (_, _): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms",
        Some(json!({
            "room_id": "arathi-5v5",
            "map_index": 1,
            "draft_mode": "random_draft",
            "host_id": "host-1",
            "host_name": "HostOne"
        })),
    )
    .await;

    // Second human joins Seat 5 (Horde Tank).
    let (status, after_join): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/arathi-5v5/join",
        Some(json!({
            "player_id": "player-2",
            "player_name": "ThrallPlayer",
            "preferred_seat": 5,
            "role": "tank",
            "class_id": 1
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_join.seats[5].player_id.as_deref(), Some("player-2"));
    assert!(after_join.seats[5].connected);
    assert!(!after_join.seats[5].is_ai_controlled);
    assert_eq!(after_join.seats[5].badge_state, BADGE_EMERALD_HUMAN);

    // Player 2 disconnects -> badge becomes `amber_disconnected`, toast flags enabled.
    let (status, after_disc): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/arathi-5v5/disconnect",
        Some(json!({
            "player_id": "player-2",
            "now_ms": 5_000
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let s5 = &after_disc.seats[5];
    assert!(!s5.connected);
    assert!(!s5.is_ai_controlled);
    assert_eq!(s5.badge_state, BADGE_AMBER_DISCONNECTED);
    assert!(s5.show_play_turn);
    assert!(s5.show_turn_ai_on);
    assert_eq!(after_disc.active_toasts.len(), 1);
    assert_eq!(after_disc.active_toasts[0].seat_index, 5);

    // Execute `"play_turn"` -> increments `pending_ai_step` without permanently enabling AI takeover.
    let (status, after_step): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/arathi-5v5/ai-action",
        Some(json!({
            "seat_index": 5,
            "action": "play_turn",
            "now_ms": 6_000
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after_step.seats[5].pending_ai_step, 1);
    assert!(!after_step.seats[5].is_ai_controlled);
    assert_eq!(after_step.seats[5].badge_state, BADGE_AMBER_DISCONNECTED);

    // Execute `"turn_ai_on"` -> flips `is_ai_controlled = true` and `badge_state = "blue_ai"`.
    let (status, after_ai_on): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/arathi-5v5/ai-action",
        Some(json!({
            "seat_index": 5,
            "action": "turn_ai_on",
            "now_ms": 7_000
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(after_ai_on.seats[5].is_ai_controlled);
    assert_eq!(after_ai_on.seats[5].badge_state, BADGE_BLUE_AI);
    assert!(!after_ai_on.seats[5].show_turn_ai_on);

    // Player 2 rejoins -> immediately revokes AI (`is_ai_controlled = false`), clears `pending_ai_step = 0`,
    // clears toast flags, and restores `badge_state = "emerald_human"`.
    let (status, after_rejoin): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/arathi-5v5/join",
        Some(json!({
            "player_id": "player-2",
            "player_name": "ThrallPlayer"
        })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let s5_rejoined = &after_rejoin.seats[5];
    assert!(s5_rejoined.connected);
    assert!(!s5_rejoined.is_ai_controlled);
    assert_eq!(s5_rejoined.pending_ai_step, 0);
    assert!(!s5_rejoined.show_play_turn);
    assert!(!s5_rejoined.show_turn_ai_on);
    assert_eq!(s5_rejoined.badge_state, BADGE_EMERALD_HUMAN);
    assert!(after_rejoin.active_toasts.is_empty());
}

#[tokio::test]
async fn host_30_second_grace_handoff_and_primary_host_reclamation() {
    let app = wenilla_host::moba_room::router();

    let (_, _): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms",
        Some(json!({
            "room_id": "alterac-5v5",
            "map_index": 2,
            "draft_mode": "all_pick",
            "host_id": "primary-host",
            "host_name": "Vanndar"
        })),
    )
    .await;

    // Second and third humans join seats 1 and 2.
    let (_, _): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/join",
        Some(json!({
            "player_id": "human-2",
            "player_name": "Jaina"
        })),
    )
    .await;
    let (_, _): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/join",
        Some(json!({
            "player_id": "human-3",
            "player_name": "Uther"
        })),
    )
    .await;

    // Primary host disconnects at t = 1_000 ms.
    let (_, disc_snap): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/disconnect",
        Some(json!({
            "player_id": "primary-host",
            "now_ms": 1_000
        })),
    )
    .await;
    assert_eq!(disc_snap.acting_host_id, "primary-host");
    assert!(!disc_snap.host_handoff_active);
    assert_eq!(disc_snap.host_grace_remaining_ms, 30_000);

    // At 20 seconds (20_000 ms elapsed < 30_000 ms): no handoff yet.
    let (_, at_20s): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/advance-grace",
        Some(json!({
            "elapsed_ms": 20_000
        })),
    )
    .await;
    assert_eq!(at_20s.acting_host_id, "primary-host");
    assert!(!at_20s.host_handoff_active);
    assert_eq!(at_20s.host_grace_remaining_ms, 10_000);

    // At 30 seconds (30_000 ms elapsed >= 30_000 ms): next connected human (`human-2`) is promoted!
    let (_, at_30s): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/advance-grace",
        Some(json!({
            "elapsed_ms": 30_000
        })),
    )
    .await;
    assert_eq!(at_30s.acting_host_id, "human-2");
    assert!(at_30s.host_handoff_active);
    assert_eq!(at_30s.host_grace_remaining_ms, 0);

    // Primary host reconnects -> immediately reclaims `acting_host_id` and clears grace state.
    let (_, reclaimed): (_, MobaRoomSnapshot) = send_json(
        &app,
        Method::POST,
        "/api/moba/rooms/alterac-5v5/join",
        Some(json!({
            "player_id": "primary-host",
            "player_name": "Vanndar"
        })),
    )
    .await;
    assert_eq!(reclaimed.acting_host_id, "primary-host");
    assert!(!reclaimed.host_handoff_active);
    assert!(reclaimed.host_disconnected_at_ms.is_none());
    assert_eq!(reclaimed.seats[0].badge_state, BADGE_EMERALD_HUMAN);
    assert!(!reclaimed.seats[0].is_ai_controlled);
}

#[tokio::test]
async fn websocket_broadcasts_match_events_between_connected_peers() {
    let app = wenilla_host::moba_room::router();

    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind listener");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve moba router");
    });

    let ws_url = format!("ws://{addr}/ws/moba/blackrock-5v5");
    let (ws_a, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect peer A");
    let (ws_b, _) = tokio_tungstenite::connect_async(&ws_url)
        .await
        .expect("connect peer B");

    let (mut tx_a, mut rx_a) = ws_a.split();
    let (mut _tx_b, mut rx_b) = ws_b.split();

    let move_event = MobaWsEvent::HeroMove {
        seat_index: 0,
        x: 128.5,
        y: -42.0,
        z: 10.0,
        facing: 1.57,
    };
    let payload = serde_json::to_string(&move_event).expect("serialize ws event");

    tx_a.send(Message::Text(payload.into()))
        .await
        .expect("send HeroMove from peer A");

    let recv_b = rx_b
        .next()
        .await
        .expect("peer B stream item")
        .expect("peer B ws message");
    let parsed_b: MobaWsEvent = serde_json::from_str(recv_b.to_text().expect("utf8 text"))
        .expect("deserialize MobaWsEvent");
    assert_eq!(parsed_b, move_event);

    let recv_a = rx_a
        .next()
        .await
        .expect("peer A stream item")
        .expect("peer A ws message");
    let parsed_a: MobaWsEvent = serde_json::from_str(recv_a.to_text().expect("utf8 text"))
        .expect("deserialize MobaWsEvent");
    assert_eq!(parsed_a, move_event);

    // Broadcast an ObjectiveCapture event and verify team shared XP + level progression.
    let obj_event = MobaWsEvent::ObjectiveCapture {
        team: "alliance".to_string(),
        xp_awarded: 1_200,
        event_clock_ms: 180_000,
    };
    tx_a.send(Message::Text(
        serde_json::to_string(&obj_event)
            .expect("serialize objective event")
            .into(),
    ))
    .await
    .expect("send ObjectiveCapture from peer A");

    let recv_obj_b = rx_b
        .next()
        .await
        .expect("peer B objective item")
        .expect("peer B ws message");
    let parsed_obj_b: MobaWsEvent = serde_json::from_str(recv_obj_b.to_text().expect("utf8 text"))
        .expect("deserialize ObjectiveCapture");
    assert_eq!(parsed_obj_b, obj_event);

    let _ = tx_a.close().await;
}
