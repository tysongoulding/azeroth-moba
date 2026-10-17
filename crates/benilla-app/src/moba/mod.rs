//! Azeroth 5v5 Mid-Only Custom WoW MOBA (`warcraft-moba` / `run_moba`).
//!
//! Combines:
//! - Single-lane 5v5 Mid Brawl on iconic Vanilla WoW battlegrounds (`data::MOBA_MAPS`)
//! - Dual-Mode Draft (`draft::TavernDraftState`: Tavern Bench Shuffle + Shared Reroll Bench vs All Pick)
//!   with automatic 5-man role balancing (`1 Tank + 1 Healer + 3 DPS`)
//! - Hybrid Action-MOBA class kits (`Auto-Primary` + manual `Q/W/E` + Lv 10 Dual `R` Heroic Ultimates + `F` Mobility)
//! - HoTS Shared 5-Man Team XP (`Lv 1..=20`) with Underdog Comeback XP Multiplier (`+25%` per level behind)
//!   and non-pausing 3-Card Milestone Drafts (`Lv 1, 4, 7, 10, 13, 16, 20`)
//! - LoL ARAM Fountain Shop Lock + Dota 2 Goblin Courier (`90s` CD) + Soulstone Buyback (`360s` CD)
//! - Classic WoW 15-Second PvP Diminishing Returns (`100% -> 50% -> 25% -> Immune`) + BKB Insignia Cleanse
//! - 3-Tier Lane Structures (5 Tower Gold Plates before 8:00, True Sight, Deterministic Aggro,
//!   Moonwells, Barracks -> Super Minions, Backdoor Protection, High-Ground 25% Uphill Miss, Glyph)
//! - 4-Game Neutral Event Clock (`90s` River Runes, `150s` Pushing Mercs, `7:00/13:00` Raid Boss Pit
//!   with Aegis Soulstone in-place revival, `18:00` Sudden Death General march)

pub mod bots;
pub mod combat;
pub mod data;
pub mod draft;
pub mod heroes;
pub mod hud;
pub mod lane;
pub mod objectives;
pub mod progression;
pub mod waves;

use std::collections::HashMap;

use benilla_world::schedule::WorldStage;
use benilla_world::world_point::WorldPoint;
use bevy::prelude::*;

use bots::{
    bot_choose_movement_target, bot_economy_and_items_step, recommended_item_order_for_hero,
};
use combat::{MobaHero, MAX_INVENTORY_SLOTS};
use data::{MobaAbilitySlot, MobaEffect, MobaMapDef, Team, MOBA_MAPS};
use draft::TavernDraftState;
use lane::{
    build_initial_moonwells, build_initial_structures, elevation_at, is_uphill_attack,
    select_tower_target, GlyphState, Moonwell, Structure, StructureKind, TowerTargetCandidate,
    BACKDOOR_RADIUS, LANE_HALF_LEN, LANE_HALF_WIDTH, MOONWELL_INTERACT_RADIUS, PLATE_XP_REWARD,
    UPHILL_MISS_CHANCE,
};
use objectives::ObjectiveClock;
use progression::{
    deal_legendary_boss_cards_for_hero, deal_milestone_cards_for_hero, TeamProgression,
};
use waves::{
    spawn_mercenary_push, spawn_raid_boss_avatar, spawn_wave_for_team, LaneMinion, MobaPickup,
    MobaPickupKind, AMBIENT_GOLD_RADIUS, AMBIENT_MINION_GOLD, LAST_HIT_GOLD,
    REGEN_GLOBE_AOE_RADIUS, REGEN_GLOBE_RESTORE_PCT, WAVE_INTERVAL, WISP_MAGNET_RADIUS,
};

use crate::creature_anim::{move_flags, MovementState};
use crate::net::ObjectStore;
pub use crate::survivors::{data::Stat, Rng};
use crate::survivors::{Phase, Run, RunEntity, Selection};

/// Set by [`crate::run_moba`] before the app builds, or enabled via `WOW_MOBA` / `WOW_MOBA_AUTO`.
pub static ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn active() -> bool {
    ACTIVE.load(std::sync::atomic::Ordering::Relaxed)
        || crate::webenv::var("WOW_MOBA").is_some()
        || crate::webenv::var("WOW_MOBA_AUTO").is_some()
}

/// Shared mode selector resource toggled at the menu (`Mode: 5v5 Mid Brawl` vs `Mode: Survivors Solo`).
#[derive(Resource, Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub enum GameMode {
    #[default]
    SurvivorsSolo,
    Moba5v5,
}

impl GameMode {
    pub const fn toggle(self) -> Self {
        match self {
            Self::SurvivorsSolo => Self::Moba5v5,
            Self::Moba5v5 => Self::SurvivorsSolo,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Moba5v5 => "Mode: 5v5 Mid Brawl",
            Self::SurvivorsSolo => "Mode: Survivors Solo",
        }
    }
}

/// Complete live state of an Azeroth 5v5 Mid Brawl match.
#[derive(Resource, Clone, Debug)]
pub struct MobaMatch {
    pub map_idx: usize,
    pub player_team: Team,
    pub player_hero_id: u32,
    pub elapsed: f32,
    pub gold_tick_accum: f32,
    pub wave_timer: f32,
    pub wave_count: u32,
    pub next_entity_id: u32,
    pub heroes: Vec<MobaHero>,
    pub structures: Vec<Structure>,
    pub moonwells: Vec<Moonwell>,
    pub minions: Vec<LaneMinion>,
    pub pickups: Vec<MobaPickup>,
    pub alliance_prog: TeamProgression,
    pub horde_prog: TeamProgression,
    pub alliance_glyph: GlyphState,
    pub horde_glyph: GlyphState,
    pub alliance_super_minions: bool,
    pub horde_super_minions: bool,
    pub objectives: ObjectiveClock,
    pub alliance_kills: u32,
    pub horde_kills: u32,
    pub winner: Option<Team>,
    pub banner: Option<(String, f32)>,
}

impl Default for MobaMatch {
    fn default() -> Self {
        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        Self::new_from_draft(&draft, 0, false, &mut rng)
    }
}

impl MobaMatch {
    /// Initializes a fresh 5v5 Mid Brawl match from `draft` on `MOBA_MAPS[map_idx]`.
    pub fn new_from_draft(
        draft: &TavernDraftState,
        map_idx: usize,
        all_ai: bool,
        rng: &mut Rng,
    ) -> Self {
        let mut heroes = Vec::with_capacity(10);
        let mut next_entity_id = 100u32;
        let player_team = draft.player_team;
        let mut player_hero_id = 1u32;

        for (slot, &card) in draft.alliance_roster.iter().enumerate() {
            let id = (slot + 1) as u32;
            let is_player = !all_ai && player_team == Team::Alliance && slot == 0;
            if is_player {
                player_hero_id = id;
            }
            let hero_name = card.hero_def().name;
            let mut h = MobaHero::new(id, hero_name, Team::Alliance, card, !is_player, slot);
            h.pending_card_offers
                .push(deal_milestone_cards_for_hero(h.hero_def(), 1, rng));
            heroes.push(h);
        }

        for (slot, &card) in draft.horde_roster.iter().enumerate() {
            let id = (slot + 6) as u32;
            let is_player = !all_ai && player_team == Team::Horde && slot == 0;
            if is_player {
                player_hero_id = id;
            }
            let hero_name = card.hero_def().name;
            let mut h = MobaHero::new(id, hero_name, Team::Horde, card, !is_player, slot);
            h.pending_card_offers
                .push(deal_milestone_cards_for_hero(h.hero_def(), 1, rng));
            heroes.push(h);
        }

        let mut minions = spawn_wave_for_team(Team::Alliance, 1, false, &mut next_entity_id);
        minions.extend(spawn_wave_for_team(
            Team::Horde,
            1,
            false,
            &mut next_entity_id,
        ));

        Self {
            map_idx: map_idx.min(MOBA_MAPS.len() - 1),
            player_team,
            player_hero_id,
            elapsed: 0.0,
            gold_tick_accum: 0.0,
            wave_timer: WAVE_INTERVAL,
            wave_count: 1,
            next_entity_id,
            heroes,
            structures: build_initial_structures(),
            moonwells: build_initial_moonwells(),
            minions,
            pickups: Vec::new(),
            alliance_prog: TeamProgression::default(),
            horde_prog: TeamProgression::default(),
            alliance_glyph: GlyphState::default(),
            horde_glyph: GlyphState::default(),
            alliance_super_minions: false,
            horde_super_minions: false,
            objectives: ObjectiveClock::default(),
            alliance_kills: 0,
            horde_kills: 0,
            winner: None,
            banner: Some((
                "Welcome to the 5v5 Mid Brawl! First wave is on the bridge!".to_string(),
                5.0,
            )),
        }
    }

    pub fn map_def(&self) -> &'static MobaMapDef {
        &MOBA_MAPS[self.map_idx.min(MOBA_MAPS.len() - 1)]
    }

    pub fn player_hero(&self) -> Option<&MobaHero> {
        self.heroes.iter().find(|h| h.id == self.player_hero_id)
    }

    pub fn player_hero_mut(&mut self) -> Option<&mut MobaHero> {
        let id = self.player_hero_id;
        self.heroes.iter_mut().find(|h| h.id == id)
    }

    pub fn team_level(&self, team: Team) -> u32 {
        match team {
            Team::Alliance => self.alliance_prog.level,
            Team::Horde => self.horde_prog.level,
        }
    }

    pub fn glyph_for(&self, team: Team) -> &GlyphState {
        match team {
            Team::Alliance => &self.alliance_glyph,
            Team::Horde => &self.horde_glyph,
        }
    }

    pub fn glyph_for_mut(&mut self, team: Team) -> &mut GlyphState {
        match team {
            Team::Alliance => &mut self.alliance_glyph,
            Team::Horde => &mut self.horde_glyph,
        }
    }

    /// Awards Shared 5-Man Team XP to `team` (applying the Underdog Comeback XP Multiplier when
    /// behind by >= 2 levels) and deals non-pausing 3-card milestone offers to all 5 teammates
    /// whenever a milestone level (`4, 7, 10, 13, 16, 20`) is reached!
    pub fn add_team_xp(&mut self, team: Team, raw_xp: f32, rng: &mut Rng) {
        let enemy_level = self.team_level(team.opposite());
        let milestones = match team {
            Team::Alliance => self.alliance_prog.add_xp(raw_xp, enemy_level),
            Team::Horde => self.horde_prog.add_xp(raw_xp, enemy_level),
        };
        if !milestones.is_empty() {
            for &ms_lvl in &milestones {
                for h in &mut self.heroes {
                    if h.team == team {
                        let cards = deal_milestone_cards_for_hero(h.hero_def(), ms_lvl, rng);
                        h.pending_card_offers.push(cards);
                    }
                }
                self.banner = Some((
                    format!(
                        "{} reached Team Level {}! Milestone Cards Dealt!",
                        team.name(),
                        ms_lvl
                    ),
                    4.0,
                ));
            }
        }
    }

    /// Player picks card `choice_idx` (`0..3`) from their non-pausing milestone card tray.
    pub fn pick_player_card(&mut self, choice_idx: usize) -> bool {
        let team_lvl = self.team_level(self.player_team);
        if let Some(h) = self.player_hero_mut() {
            h.pick_milestone_card(choice_idx, team_lvl)
        } else {
            false
        }
    }

    /// Player uses active inventory item `slot_idx` (`0..3`).
    pub fn use_player_item(&mut self, slot_idx: usize) -> bool {
        let team_lvl = self.team_level(self.player_team);
        let Some(h) = self.player_hero_mut() else {
            return false;
        };
        h.use_active_item(slot_idx, team_lvl).is_some()
    }

    /// Player quick-buys next recommended item or calls the Goblin Courier if items are in `stash`.
    pub fn player_quick_buy_and_courier(&mut self) {
        let team_lvl = self.team_level(self.player_team);
        let mut new_banner = None;
        if let Some(h) = self.player_hero_mut() {
            let total = h.inventory.len() + h.stash.len();
            if total < MAX_INVENTORY_SLOTS {
                let order = recommended_item_order_for_hero(h);
                for &key in order {
                    let has = h.inventory.iter().any(|i| i.key == key)
                        || h.stash.iter().any(|i| i.key == key);
                    if !has {
                        if let Ok(outcome) = h.buy_item(key, team_lvl) {
                            new_banner = Some((format!("Purchased {key} ({outcome:?})"), 2.5));
                        }
                        break;
                    }
                }
            }
            if !h.stash.is_empty() && h.call_goblin_courier() {
                new_banner = Some(("Goblin Courier dispatched from Fountain!".to_string(), 3.0));
            }
        }
        if let Some(b) = new_banner {
            self.banner = Some(b);
        }
    }

    /// Player presses `G`: activates Team Glyph of Fortification (or Soulstone Buyback if dead).
    pub fn player_glyph_or_buyback(&mut self) {
        let team = self.player_team;
        let team_lvl = self.team_level(team);
        let is_dead = self.player_hero().is_some_and(|h| h.dead);
        if is_dead {
            if let Some(h) = self.player_hero_mut() {
                if h.soulstone_buyback(team_lvl, 0) {
                    self.banner =
                        Some(("Soulstone Buyback! Respawned at Fountain!".to_string(), 3.5));
                }
            }
        } else if self.glyph_for_mut(team).activate() {
            self.banner = Some((
                format!(
                    "{} activated Glyph of Fortification (5s Invulnerable Structures)!",
                    team.name()
                ),
                4.0,
            ));
        }
    }

    /// Player casts manual ability slot (`Q`, `W`, `E`, `R`, `F`).
    pub fn cast_player_ability(&mut self, slot: MobaAbilitySlot) -> bool {
        let Some(idx) = self.heroes.iter().position(|h| h.id == self.player_hero_id) else {
            return false;
        };
        let mut rng = Rng::default();
        self.try_cast_hero_ability(idx, slot, None, &mut rng)
    }

    /// Checks whether `target_kind` on `team` has its prerequisite outer tier destroyed.
    pub fn is_structure_prereq_destroyed(&self, team: Team, kind: StructureKind) -> bool {
        match kind {
            StructureKind::OuterTower => true,
            StructureKind::InnerKeep | StructureKind::Barracks => self
                .structures
                .iter()
                .any(|s| s.team == team && s.kind == StructureKind::OuterTower && s.destroyed),
            StructureKind::FactionGeneral => {
                self.objectives.sudden_death_active
                    || self.structures.iter().any(|s| {
                        s.team == team && s.kind == StructureKind::InnerKeep && s.destroyed
                    })
            }
        }
    }

    /// Attempts to cast `slot` for `heroes[hero_idx]`.
    pub fn try_cast_hero_ability(
        &mut self,
        hero_idx: usize,
        slot: MobaAbilitySlot,
        aim_override: Option<Vec2>,
        rng: &mut Rng,
    ) -> bool {
        let hero = &self.heroes[hero_idx];
        if hero.dead || hero.aegis_revive_t.is_some() || hero.stun_t > 0.0 || hero.fear_t > 0.0 {
            return false;
        }
        if slot != MobaAbilitySlot::AutoPrimary && hero.silence_t > 0.0 {
            return false;
        }
        let team = hero.team;
        let team_lvl = self.team_level(team);
        if slot == MobaAbilitySlot::UltimateR && team_lvl < 10 && hero.chosen_ultimate.is_none() {
            return false;
        }
        if hero.slot_cd(slot) > 0.0 {
            return false;
        }
        let Some(def) = hero.ability_def(slot) else {
            return false;
        };
        if def.power_cost > 0.0 && hero.power < def.power_cost {
            return false;
        }

        let school = def.damage_school(hero.damage_profile());
        let st = hero.stats(team_lvl);
        let mutator = hero.slot_mutators[slot as usize];
        let range = (def.range + mutator.bonus_radius).max(5.5);
        let radius = def.radius + mutator.bonus_radius;
        let hero_pos = hero.pos;
        let dir_sign = team.dir_sign();

        // Find primary enemy target (hero, minion, raid boss, merc camp, or structure) within range
        let nearest_enemy_hero = self
            .heroes
            .iter()
            .enumerate()
            .filter(|(_, eh)| !eh.dead && eh.team != team && eh.pos.distance(hero_pos) <= range)
            .min_by(|(_, a), (_, b)| {
                a.pos
                    .distance(hero_pos)
                    .total_cmp(&b.pos.distance(hero_pos))
            })
            .map(|(i, eh)| (i, eh.pos));

        let nearest_enemy_minion = self
            .minions
            .iter()
            .enumerate()
            .filter(|(_, m)| !m.dead && m.team != team && m.pos.distance(hero_pos) <= range)
            .min_by(|(_, a), (_, b)| {
                a.pos
                    .distance(hero_pos)
                    .total_cmp(&b.pos.distance(hero_pos))
            })
            .map(|(i, m)| (i, m.pos));

        let nearest_enemy_struct = self
            .structures
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                !s.destroyed
                    && s.team != team
                    && s.pos.distance(hero_pos) <= range
                    && self.is_structure_prereq_destroyed(s.team, s.kind)
            })
            .min_by(|(_, a), (_, b)| {
                a.pos
                    .distance(hero_pos)
                    .total_cmp(&b.pos.distance(hero_pos))
            })
            .map(|(i, s)| (i, s.pos));

        let boss_in_range = (self.objectives.raid_boss.active
            && self.objectives.raid_boss.pos.distance(hero_pos) <= range)
            .then_some(self.objectives.raid_boss.pos);

        let camp_in_range = self
            .objectives
            .merc_camps
            .iter()
            .enumerate()
            .find(|(_, c)| c.active && c.pos.distance(hero_pos) <= range)
            .map(|(i, c)| (i, c.pos));

        let has_combat_target = nearest_enemy_hero.is_some()
            || nearest_enemy_minion.is_some()
            || nearest_enemy_struct.is_some()
            || boss_in_range.is_some()
            || camp_in_range.is_some();

        let is_pure_heal_or_mobility =
            (def.damage <= 0.0 && def.heal > 0.0) || slot == MobaAbilitySlot::MobilityF;
        if !has_combat_target && !is_pure_heal_or_mobility && aim_override.is_none() {
            return false;
        }

        let target_pos = aim_override
            .or_else(|| nearest_enemy_hero.map(|(_, p)| p))
            .or_else(|| nearest_enemy_minion.map(|(_, p)| p))
            .or_else(|| nearest_enemy_struct.map(|(_, p)| p))
            .or(boss_in_range)
            .or_else(|| camp_in_range.map(|(_, p)| p))
            .unwrap_or_else(|| Vec2::new(hero_pos.x + dir_sign * 10.0, hero_pos.y));

        // Spend/generate power and start cooldown
        {
            let h = &mut self.heroes[hero_idx];
            if def.power_cost > 0.0 {
                h.power = (h.power - def.power_cost).max(0.0);
            } else if def.power_cost < 0.0 {
                h.power = (h.power - def.power_cost).min(st.max_power);
            }
            let cdr = (st.haste + mutator.cdr_pct).clamp(0.0, 0.70);
            let cd = (def.cooldown * (1.0 - cdr)).max(0.25);
            h.set_slot_cd(slot, cd);
            // Offensive actions end spawn mount launch boost
            if def.damage > 0.0 {
                h.mount_launch_t = 0.0;
            }
        }

        let crit_mult = if rng.f32() < st.crit { 1.75 } else { 1.0 };
        let scaled_dmg = def.damage * st.might * (1.0 + mutator.bonus_dmg_pct) * crit_mult;
        let scaled_heal = def.heal * st.might * (1.0 + mutator.bonus_dmg_pct);

        // Apply self/ally effects (Mobility, Shields, Anti-Magic Shells, Heals, Bloodlust, Immunity)
        match def.effect {
            MobaEffect::Blink { distance } | MobaEffect::Dash { distance, .. } => {
                if self.heroes[hero_idx].can_move() {
                    let delta = target_pos - hero_pos;
                    let step = if delta.length() > 0.5 {
                        delta.normalize() * distance.min(delta.length().max(8.0))
                    } else {
                        Vec2::new(dir_sign * distance, 0.0)
                    };
                    let new_x =
                        (hero_pos.x + step.x).clamp(-LANE_HALF_LEN + 6.0, LANE_HALF_LEN - 6.0);
                    let new_y =
                        (hero_pos.y + step.y).clamp(-LANE_HALF_WIDTH + 2.0, LANE_HALF_WIDTH - 2.0);
                    self.heroes[hero_idx].pos = Vec2::new(new_x, new_y);
                }
                if let MobaEffect::Dash { stun_secs, .. } = def.effect {
                    if stun_secs > 0.0 {
                        if let Some((eh_idx, _)) = nearest_enemy_hero {
                            self.heroes[eh_idx].apply_cc(data::CcCategory::Stun, stun_secs);
                        }
                    }
                }
            }
            MobaEffect::HasteSelf {
                speed_mult,
                duration,
            } => {
                let h = &mut self.heroes[hero_idx];
                h.haste_buff_mult = h.haste_buff_mult.max(speed_mult);
                h.haste_buff_t = h.haste_buff_t.max(duration);
            }
            MobaEffect::StealthAmbush {
                speed_mult,
                duration,
            } => {
                let h = &mut self.heroes[hero_idx];
                h.haste_buff_mult = h.haste_buff_mult.max(speed_mult);
                h.haste_buff_t = h.haste_buff_t.max(duration);
                h.stealth_t = h.stealth_t.max(duration);
            }
            MobaEffect::ShieldAlly { amount, duration } => {
                let shield_val = amount * st.might;
                if let Some(ally) = self
                    .heroes
                    .iter_mut()
                    .filter(|a| !a.dead && a.team == team && a.pos.distance(hero_pos) <= 24.0)
                    .min_by(|a, b| a.hp.total_cmp(&b.hp))
                {
                    ally.shield = ally.shield.max(shield_val);
                    ally.shield_t = ally.shield_t.max(duration);
                }
            }
            MobaEffect::AntiMagicShell {
                magic_absorb,
                duration,
                cleanse_magic,
            } => {
                let shell_val = magic_absorb * st.might;
                if let Some(ally) = self
                    .heroes
                    .iter_mut()
                    .filter(|a| !a.dead && a.team == team && a.pos.distance(hero_pos) <= 24.0)
                    .min_by(|a, b| a.hp.total_cmp(&b.hp))
                {
                    if cleanse_magic {
                        ally.purge_and_grant_unstoppable(0.5);
                    }
                    ally.grant_anti_magic_shell(shell_val, duration);
                }
            }
            MobaEffect::ImmunityAndHeal { duration, heal } => {
                self.heroes[hero_idx].purge_and_grant_unstoppable(duration);
                let bonus_heal = heal * st.might;
                for ally in &mut self.heroes {
                    if !ally.dead && ally.team == team && ally.pos.distance(hero_pos) <= 20.0 {
                        ally.receive_healing(bonus_heal, team_lvl);
                    }
                }
            }
            MobaEffect::BloodlustTeam {
                haste_bonus,
                speed_bonus,
                duration,
            } => {
                for ally in &mut self.heroes {
                    if !ally.dead && ally.team == team {
                        ally.haste_buff_mult = ally.haste_buff_mult.max(1.0 + speed_bonus);
                        ally.haste_buff_t = ally.haste_buff_t.max(duration);
                        ally.might_buff_bonus = ally.might_buff_bonus.max(haste_bonus);
                        ally.might_buff_t = ally.might_buff_t.max(duration);
                    }
                }
            }
            _ => {}
        }

        // Apply direct healing to most injured ally in range
        if scaled_heal > 0.0 {
            let mut actual_healed = 0.0;
            if let Some(ally) = self
                .heroes
                .iter_mut()
                .filter(|a| !a.dead && a.team == team && a.pos.distance(hero_pos) <= 25.0)
                .min_by(|a, b| a.hp.total_cmp(&b.hp))
            {
                actual_healed = ally.receive_healing(scaled_heal, team_lvl);
            }
            self.heroes[hero_idx].healing_done += actual_healed;
        }

        if scaled_dmg <= 0.0 {
            return true;
        }

        let aoe_r = radius.max(3.0);
        let mut total_dealt = 0.0f32;
        let mut hero_takedowns = Vec::new();
        let enemy_lvl = self.team_level(team.opposite());

        // 1. Hit enemy heroes in AoE/target
        for eh_idx in 0..self.heroes.len() {
            if self.heroes[eh_idx].dead || self.heroes[eh_idx].team == team {
                continue;
            }
            let eh_pos = self.heroes[eh_idx].pos;
            let hit = eh_pos.distance(target_pos) <= aoe_r
                || nearest_enemy_hero.is_some_and(|(idx, _)| idx == eh_idx);
            if !hit {
                continue;
            }
            // Uphill Miss Chance (25%) when attacking from low ground onto high ground!
            if is_uphill_attack(hero_pos, eh_pos) && rng.f32() < UPHILL_MISS_CHANCE {
                continue;
            }
            // Apply passive item/theme on-hit -Armor and -Magic shreds
            if st.armor_shred_on_hit > 0.0 {
                self.heroes[eh_idx].apply_armor_shred(st.armor_shred_on_hit, 4.5);
            }
            if st.magic_shred_on_hit > 0.0 {
                self.heroes[eh_idx].apply_magic_shred(st.magic_shred_on_hit, 4.5);
            }
            // Apply CC / Slow / Theme Shred / Webs / PoisonAffliction effects
            match def.effect {
                MobaEffect::Cc { category, duration } => {
                    self.heroes[eh_idx].apply_cc(category, duration);
                }
                MobaEffect::Slow { factor, duration }
                    if self.heroes[eh_idx].unstoppable_t <= 0.0 =>
                {
                    self.heroes[eh_idx].slow_factor = factor;
                    self.heroes[eh_idx].slow_t = self.heroes[eh_idx].slow_t.max(duration);
                }
                MobaEffect::ArmorShred { amount, duration } => {
                    self.heroes[eh_idx].apply_armor_shred(amount, duration);
                }
                MobaEffect::MagicShred { amount, duration } => {
                    self.heroes[eh_idx].apply_magic_shred(amount, duration);
                }
                MobaEffect::DualShred {
                    armor,
                    magic,
                    duration,
                } => {
                    self.heroes[eh_idx].apply_armor_shred(armor, duration);
                    self.heroes[eh_idx].apply_magic_shred(magic, duration);
                }
                MobaEffect::WebSnare {
                    root_secs,
                    slow_factor,
                    slow_secs,
                } => {
                    self.heroes[eh_idx].apply_web_snare(root_secs, slow_factor, slow_secs);
                }
                MobaEffect::PoisonAffliction {
                    dps,
                    duration,
                    heal_cut,
                } => {
                    self.heroes[eh_idx].apply_poison_affliction(dps * st.might, duration, heal_cut);
                }
                _ => {}
            }
            let (dealt, died) =
                self.heroes[eh_idx].take_typed_damage(scaled_dmg, school, enemy_lvl);
            total_dealt += dealt;
            if died {
                hero_takedowns.push(eh_idx);
            }
        }

        if total_dealt > 0.0 {
            self.heroes[hero_idx].attacked_enemy_hero_t = 3.5;
        }

        // Award hero takedown Kills, Assists, Gold, and Shared Team XP!
        let caster_id = self.heroes[hero_idx].id;
        for _dead_idx in hero_takedowns {
            self.heroes[hero_idx].kills += 1;
            self.heroes[hero_idx].gold += 180;
            for ally in &mut self.heroes {
                if ally.team == team && ally.id != caster_id && !ally.dead {
                    ally.assists += 1;
                    ally.gold += 65;
                }
            }
            match team {
                Team::Alliance => self.alliance_kills += 1,
                Team::Horde => self.horde_kills += 1,
            }
            self.add_team_xp(team, 145.0, rng);
        }

        // 2. Hit enemy minions in AoE/target
        let mut slain_minions = Vec::new();
        for (m_idx, m) in self.minions.iter_mut().enumerate() {
            if m.dead || m.team == team {
                continue;
            }
            let hit = m.pos.distance(target_pos) <= aoe_r
                || nearest_enemy_minion.is_some_and(|(idx, _)| idx == m_idx);
            if !hit {
                continue;
            }
            m.hp -= scaled_dmg;
            total_dealt += scaled_dmg;
            if m.hp <= 0.0 {
                m.hp = 0.0;
                m.dead = true;
                slain_minions.push((m.pos, m.xp_value, m.drops_regen_globe));
            }
        }

        for (m_pos, xp_val, drops_globe) in slain_minions {
            self.heroes[hero_idx].creep_score += 1;
            self.heroes[hero_idx].gold += LAST_HIT_GOLD;
            for ally in &mut self.heroes {
                if !ally.dead
                    && ally.team == team
                    && ally.id != caster_id
                    && ally.pos.distance(m_pos) <= AMBIENT_GOLD_RADIUS
                {
                    ally.gold += AMBIENT_MINION_GOLD;
                }
            }
            self.spawn_minion_pickups(m_pos, xp_val, drops_globe, team);
        }

        // 3. Hit enemy structure if targeted
        if let Some((s_idx, s_pos)) = nearest_enemy_struct {
            if !is_uphill_attack(hero_pos, s_pos) || rng.f32() >= UPHILL_MISS_CHANCE {
                let s_team = self.structures[s_idx].team;
                let s_kind = self.structures[s_idx].kind;
                let prereq = self.is_structure_prereq_destroyed(s_team, s_kind);
                let glyph_invuln = self.glyph_for(s_team).is_invulnerable();
                let enemy_minion_nearby = self
                    .minions
                    .iter()
                    .any(|m| !m.dead && m.team == team && m.pos.distance(s_pos) <= BACKDOOR_RADIUS);
                let out = self.structures[s_idx].apply_damage(
                    scaled_dmg * 0.65,
                    self.elapsed,
                    enemy_minion_nearby,
                    glyph_invuln,
                    prereq,
                );
                total_dealt += out.actual_damage;
                if out.gold_awarded > 0 {
                    self.award_structure_rewards(team, out);
                }
            }
        }

        // 4. Hit active Center Raid Boss or Mercenary Camp
        if self.objectives.raid_boss.active
            && self.objectives.raid_boss.pos.distance(target_pos) <= aoe_r + 3.0
        {
            self.objectives.raid_boss.hp = (self.objectives.raid_boss.hp - scaled_dmg).max(0.0);
            total_dealt += scaled_dmg;
            if self.objectives.raid_boss.hp <= 0.0 {
                self.on_raid_boss_slain(hero_idx, team);
            }
        }
        for c_idx in 0..self.objectives.merc_camps.len() {
            if self.objectives.merc_camps[c_idx].active
                && self.objectives.merc_camps[c_idx].pos.distance(target_pos) <= aoe_r + 3.0
                && self.objectives.merc_camps[c_idx].damage_camp(scaled_dmg, team)
            {
                let camp_pos = self.objectives.merc_camps[c_idx].pos;
                let pushed = spawn_mercenary_push(team, camp_pos, &mut self.next_entity_id);
                self.minions.extend(pushed);
                self.add_team_xp(team, 110.0, rng);
            }
        }

        // LifeDrain secondary effect
        if let MobaEffect::LifeDrain { fraction } = def.effect {
            let max_hp = self.heroes[hero_idx].stats(team_lvl).max_hp;
            self.heroes[hero_idx].hp =
                (self.heroes[hero_idx].hp + total_dealt * fraction).min(max_hp);
        }
        self.heroes[hero_idx].damage_dealt += total_dealt;
        true
    }

    fn spawn_minion_pickups(
        &mut self,
        pos: Vec2,
        xp: f32,
        drops_globe: bool,
        beneficiary_team: Team,
    ) {
        self.next_entity_id += 1;
        self.pickups.push(MobaPickup {
            id: self.next_entity_id,
            pos,
            kind: MobaPickupKind::XpWisp {
                xp,
                for_team: beneficiary_team,
            },
            age: 0.0,
            collected: false,
        });
        if drops_globe {
            self.next_entity_id += 1;
            self.pickups.push(MobaPickup {
                id: self.next_entity_id,
                pos: pos + Vec2::new(0.0, 1.2),
                kind: MobaPickupKind::RegenGlobe {
                    owner_team: Some(beneficiary_team),
                },
                age: 0.0,
                collected: false,
            });
        }
    }

    fn award_structure_rewards(&mut self, attacking_team: Team, out: lane::StructureDamageOutcome) {
        for ally in &mut self.heroes {
            if ally.team == attacking_team {
                ally.gold += out.gold_awarded / 5 + 25;
            }
        }
        let mut rng = Rng::default();
        if out.plates_broken > 0 {
            self.add_team_xp(
                attacking_team,
                f32::from(out.plates_broken) * PLATE_XP_REWARD,
                &mut rng,
            );
        }
        if out.just_destroyed {
            self.add_team_xp(attacking_team, 180.0, &mut rng);
        }
        // Check Barracks destruction -> unlock Super Minions, and Faction General destruction -> Victory!
        for s in &self.structures {
            if s.destroyed {
                match (s.team, s.kind) {
                    (Team::Horde, StructureKind::Barracks) => self.alliance_super_minions = true,
                    (Team::Alliance, StructureKind::Barracks) => self.horde_super_minions = true,
                    (Team::Horde, StructureKind::FactionGeneral) => {
                        self.winner = Some(Team::Alliance);
                    }
                    (Team::Alliance, StructureKind::FactionGeneral) => {
                        self.winner = Some(Team::Horde);
                    }
                    _ => {}
                }
            }
        }
    }

    /// Triggers the 3 simultaneous rewards when `heroes[killer_idx]` slays the Center Raid Boss:
    /// 1. **Aegis Soulstone** on the killer (`has_aegis_soulstone = true`)
    /// 2. **Team-Wide Bonus Legendary Card Offer** for all 5 teammates
    /// 3. **Pushing Raid Boss Avatar** marching down mid lane against enemy structures!
    pub fn on_raid_boss_slain(&mut self, killer_idx: usize, slaying_team: Team) {
        self.objectives.raid_boss.active = false;
        self.objectives.raid_boss.slain_count += 1;
        let boss_name = self.objectives.raid_boss.name;
        let boss_display = self.objectives.raid_boss.display_id;

        if let Some(killer) = self.heroes.get_mut(killer_idx) {
            killer.has_aegis_soulstone = true;
        }
        for h in &mut self.heroes {
            if h.team == slaying_team {
                h.gold += 250;
                h.pending_card_offers
                    .push(deal_legendary_boss_cards_for_hero(h.hero_def()));
            }
        }
        let avatar = spawn_raid_boss_avatar(
            slaying_team,
            boss_name,
            boss_display,
            &mut self.next_entity_id,
        );
        self.minions.push(avatar);
        self.banner = Some((
            format!(
                "{} slayed {}! Aegis Soulstone + Team Legendary Cards + Boss Push!",
                slaying_team.name(),
                boss_name
            ),
            5.0,
        ));
    }

    /// Advances the entire 5v5 Mid Brawl simulation by `dt` seconds.
    pub fn step(&mut self, dt: f32, rng: &mut Rng) {
        if self.winner.is_some() || dt <= 0.0 {
            return;
        }
        self.elapsed += dt;

        // Tick announcement banner
        if let Some((_, t)) = self.banner.as_mut() {
            *t -= dt;
            if *t <= 0.0 {
                self.banner = None;
            }
        }

        // Passive gold trickle (+4g/sec to all heroes)
        self.gold_tick_accum += dt;
        while self.gold_tick_accum >= 1.0 {
            self.gold_tick_accum -= 1.0;
            for h in &mut self.heroes {
                h.gold += 4;
            }
        }

        // Tick Glyphs, Moonwells, and 4-Game Neutral Event Clock
        self.alliance_glyph.tick(dt);
        self.horde_glyph.tick(dt);
        for well in &mut self.moonwells {
            well.tick(dt);
        }
        let announcements = self.objectives.tick(self.elapsed, dt);
        if let Some(msg) = announcements.into_iter().last() {
            self.banner = Some((msg, 4.5));
        }

        // Expire Tier 1 Tower Gold Plates at 8:00 (480.0s)
        if self.elapsed >= lane::PLATE_FALLOFF_TIME {
            for s in &mut self.structures {
                if s.kind == StructureKind::OuterTower {
                    s.plates_remaining = 0;
                }
            }
        }

        // 18:00 Sudden Death: both Faction Generals leave their keeps and march to x = 0!
        if self.objectives.sudden_death_active {
            for s in &mut self.structures {
                if !s.destroyed && s.kind == StructureKind::FactionGeneral {
                    s.marching_sudden_death = true;
                    let target_x = match s.team {
                        Team::Alliance => -4.0,
                        Team::Horde => 4.0,
                    };
                    let dx = target_x - s.pos.x;
                    if dx.abs() > 0.5 {
                        s.pos.x += dx.signum() * (6.0 * dt).min(dx.abs());
                    }
                }
            }
        }

        // Spawn synchronized 30s Faction Minion Waves
        self.wave_timer -= dt;
        if self.wave_timer <= 0.0 {
            self.wave_timer += WAVE_INTERVAL;
            self.wave_count += 1;
            let a_wave = spawn_wave_for_team(
                Team::Alliance,
                self.wave_count,
                self.alliance_super_minions,
                &mut self.next_entity_id,
            );
            let h_wave = spawn_wave_for_team(
                Team::Horde,
                self.wave_count,
                self.horde_super_minions,
                &mut self.next_entity_id,
            );
            self.minions.extend(a_wave);
            self.minions.extend(h_wave);
        }

        // Tick all 10 Heroes, AI Bot decisions, Moonwells, River Runes, and Pickups
        let a_lvl = self.alliance_prog.level;
        let h_lvl = self.horde_prog.level;
        let snapshot_heroes = self.heroes.clone();

        for i in 0..self.heroes.len() {
            let team = self.heroes[i].team;
            let lvl = if team == Team::Alliance { a_lvl } else { h_lvl };
            let slot_in_team = i % 5;
            self.heroes[i].tick(dt, lvl, slot_in_team);

            if self.heroes[i].is_ai_controlled {
                bot_economy_and_items_step(&mut self.heroes[i], lvl, slot_in_team, self.elapsed);
                if self.heroes[i].can_move() {
                    let move_target = bot_choose_movement_target(
                        &self.heroes[i],
                        lvl,
                        slot_in_team,
                        &snapshot_heroes,
                        &self.minions,
                        &self.structures,
                        &self.moonwells,
                        &self.pickups,
                        &self.objectives,
                    );
                    let delta = move_target - self.heroes[i].pos;
                    if delta.length() > 1.2 {
                        let speed = self.heroes[i].stats(lvl).speed;
                        let step = delta.normalize() * (speed * dt).min(delta.length());
                        self.heroes[i].pos += step;
                        self.heroes[i].facing_yaw = (-delta.x).atan2(-delta.y);
                    }
                }
            }

            if self.heroes[i].dead {
                continue;
            }

            // Moonwell interaction when HP < 75%
            let st = self.heroes[i].stats(lvl);
            if self.heroes[i].hp < st.max_hp * 0.75 {
                let h_pos = self.heroes[i].pos;
                for well in &mut self.moonwells {
                    if well.team == team
                        && well.is_ready()
                        && well.pos.distance(h_pos) <= MOONWELL_INTERACT_RADIUS
                    {
                        if let Some((hp_add, pow_add)) = well.try_drink(st.max_hp, st.max_power) {
                            self.heroes[i].hp = (self.heroes[i].hp + hp_add).min(st.max_hp);
                            self.heroes[i].power =
                                (self.heroes[i].power + pow_add).min(st.max_power);
                        }
                    }
                }
            }

            // River Power Rune pickup
            let h_pos = self.heroes[i].pos;
            for rune in &mut self.objectives.runes {
                if rune.active && rune.pos.distance(h_pos) <= 4.5 {
                    rune.apply_to_hero(&mut self.heroes[i], lvl);
                    rune.active = false;
                }
            }
        }

        // Collect XP Wisps and Regen Globes
        let mut xp_gains = Vec::new();
        let mut globe_heals = Vec::new();
        for p in &mut self.pickups {
            if p.collected {
                continue;
            }
            p.age += dt;
            for h in &self.heroes {
                if h.dead {
                    continue;
                }
                match p.kind {
                    MobaPickupKind::XpWisp { xp, for_team } => {
                        if h.team == for_team && h.pos.distance(p.pos) <= WISP_MAGNET_RADIUS {
                            p.collected = true;
                            xp_gains.push((for_team, xp));
                            break;
                        }
                    }
                    MobaPickupKind::RegenGlobe { owner_team } => {
                        let allowed = owner_team.is_none_or(|t| t == h.team || p.age >= 4.0);
                        if allowed && h.pos.distance(p.pos) <= 4.5 {
                            p.collected = true;
                            globe_heals.push((h.team, p.pos));
                            break;
                        }
                    }
                }
            }
        }
        self.pickups.retain(|p| !p.collected && p.age < 30.0);

        for (team, xp) in xp_gains {
            self.add_team_xp(team, xp, rng);
        }
        for (team, g_pos) in globe_heals {
            let lvl = self.team_level(team);
            for h in &mut self.heroes {
                if !h.dead && h.team == team && h.pos.distance(g_pos) <= REGEN_GLOBE_AOE_RADIUS {
                    let st = h.stats(lvl);
                    h.hp = (h.hp + st.max_hp * REGEN_GLOBE_RESTORE_PCT).min(st.max_hp);
                    h.power = (h.power + st.max_power * REGEN_GLOBE_RESTORE_PCT).min(st.max_power);
                }
            }
        }

        // Cast Hero Abilities: Auto-Primary fires automatically for all heroes; AI bots also fire Q/W/E/R/F!
        for i in 0..self.heroes.len() {
            let _ = self.try_cast_hero_ability(i, MobaAbilitySlot::AutoPrimary, None, rng);
            if self.heroes[i].is_ai_controlled {
                for slot in [
                    MobaAbilitySlot::UltimateR,
                    MobaAbilitySlot::Q,
                    MobaAbilitySlot::W,
                    MobaAbilitySlot::E,
                    MobaAbilitySlot::MobilityF,
                ] {
                    let _ = self.try_cast_hero_ability(i, slot, None, rng);
                }
            }
        }

        // Step Lane Minions (movement + attacks on enemy minions, structures, and heroes)
        self.step_minions(dt, rng);

        // Step Structures (Deterministic Tower Aggro Priority + Generals duel in Sudden Death)
        self.step_structures(dt, rng);
    }

    fn step_minions(&mut self, dt: f32, rng: &mut Rng) {
        let mut minion_deaths = Vec::new();
        let minion_snapshot: Vec<(usize, Team, Vec2, f32, bool)> = self
            .minions
            .iter()
            .enumerate()
            .map(|(idx, m)| (idx, m.team, m.pos, m.attack_range, m.dead))
            .collect();

        for idx in 0..self.minions.len() {
            if self.minions[idx].dead {
                continue;
            }
            self.minions[idx].attack_cd = (self.minions[idx].attack_cd - dt).max(0.0);
            let team = self.minions[idx].team;
            let pos = self.minions[idx].pos;
            let range = self.minions[idx].attack_range;
            let dmg = self.minions[idx].damage;

            // Find closest enemy minion, structure, or hero in range
            let target_minion = minion_snapshot
                .iter()
                .filter(|&&(o_idx, o_team, o_pos, _, o_dead)| {
                    !o_dead && o_idx != idx && o_team != team && o_pos.distance(pos) <= range
                })
                .min_by(|a, b| a.2.distance(pos).total_cmp(&b.2.distance(pos)))
                .map(|&(o_idx, ..)| o_idx);

            let target_struct = self
                .structures
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    !s.destroyed
                        && s.team != team
                        && s.pos.distance(pos) <= range
                        && self.is_structure_prereq_destroyed(s.team, s.kind)
                })
                .min_by(|(_, a), (_, b)| a.pos.distance(pos).total_cmp(&b.pos.distance(pos)))
                .map(|(s_idx, _)| s_idx);

            let target_hero = self
                .heroes
                .iter()
                .enumerate()
                .filter(|(_, h)| !h.dead && h.team != team && h.pos.distance(pos) <= range)
                .min_by(|(_, a), (_, b)| a.pos.distance(pos).total_cmp(&b.pos.distance(pos)))
                .map(|(h_idx, _)| h_idx);

            if let Some(m_target) = target_minion {
                if self.minions[idx].attack_cd <= 0.0 {
                    self.minions[idx].attack_cd = 1.1;
                    self.minions[m_target].hp -= dmg;
                    if self.minions[m_target].hp <= 0.0 && !self.minions[m_target].dead {
                        self.minions[m_target].dead = true;
                        minion_deaths.push((
                            self.minions[m_target].pos,
                            self.minions[m_target].xp_value,
                            self.minions[m_target].drops_regen_globe,
                            team,
                        ));
                    }
                }
            } else if let Some(s_target) = target_struct {
                if self.minions[idx].attack_cd <= 0.0 {
                    self.minions[idx].attack_cd = 1.1;
                    let s_team = self.structures[s_target].team;
                    let s_kind = self.structures[s_target].kind;
                    let prereq = self.is_structure_prereq_destroyed(s_team, s_kind);
                    let glyph_invuln = self.glyph_for(s_team).is_invulnerable();
                    let out = self.structures[s_target].apply_damage(
                        dmg,
                        self.elapsed,
                        true,
                        glyph_invuln,
                        prereq,
                    );
                    if out.gold_awarded > 0 {
                        self.award_structure_rewards(team, out);
                    }
                }
            } else if let Some(h_target) = target_hero {
                if self.minions[idx].attack_cd <= 0.0 {
                    self.minions[idx].attack_cd = 1.2;
                    let enemy_lvl = self.team_level(team.opposite());
                    let (_, died) = self.heroes[h_target].take_damage(dmg * 0.7, enemy_lvl);
                    if died {
                        match team {
                            Team::Alliance => self.alliance_kills += 1,
                            Team::Horde => self.horde_kills += 1,
                        }
                        self.add_team_xp(team, 120.0, rng);
                    }
                }
            } else {
                // March forward along the mid lane toward enemy General
                let dir = team.dir_sign();
                let speed = self.minions[idx].speed;
                self.minions[idx].pos.x =
                    (self.minions[idx].pos.x + dir * speed * dt).clamp(-106.0, 106.0);
            }
        }

        for (m_pos, xp_val, drops_globe, killer_team) in minion_deaths {
            for ally in &mut self.heroes {
                if !ally.dead
                    && ally.team == killer_team
                    && ally.pos.distance(m_pos) <= AMBIENT_GOLD_RADIUS
                {
                    ally.gold += AMBIENT_MINION_GOLD;
                }
            }
            self.spawn_minion_pickups(m_pos, xp_val, drops_globe, killer_team);
        }

        self.minions.retain(|m| !m.dead);
    }

    fn step_structures(&mut self, dt: f32, rng: &mut Rng) {
        for s_idx in 0..self.structures.len() {
            if self.structures[s_idx].destroyed || self.structures[s_idx].attack_damage <= 0.0 {
                continue;
            }
            self.structures[s_idx].attack_cd = (self.structures[s_idx].attack_cd - dt).max(0.0);
            if self.structures[s_idx].attack_cd > 0.0 {
                continue;
            }

            let s_team = self.structures[s_idx].team;
            let s_pos = self.structures[s_idx].pos;
            let s_range = self.structures[s_idx].attack_range;
            let cur_target = self.structures[s_idx].current_target;

            // Sudden Death: if opposing Faction General is in range, clash directly!
            if self.objectives.sudden_death_active
                && self.structures[s_idx].kind == StructureKind::FactionGeneral
            {
                if let Some(enemy_gen_idx) = self.structures.iter().position(|o| {
                    !o.destroyed
                        && o.team != s_team
                        && o.kind == StructureKind::FactionGeneral
                        && o.pos.distance(s_pos) <= s_range + 4.0
                }) {
                    self.structures[s_idx].attack_cd = self.structures[s_idx].attack_interval;
                    let dmg = self.structures[s_idx].attack_damage * 1.5;
                    let out = self.structures[enemy_gen_idx].apply_damage(
                        dmg,
                        self.elapsed,
                        true,
                        false,
                        true,
                    );
                    if out.just_destroyed {
                        self.winner = Some(s_team);
                    }
                    continue;
                }
            }

            // Build candidate list for deterministic MOBA Tower Aggro Priority
            let mut candidates = Vec::new();
            for m in &self.minions {
                if !m.dead && m.team != s_team && m.pos.distance(s_pos) <= s_range {
                    candidates.push(TowerTargetCandidate {
                        id: m.id,
                        pos: m.pos,
                        is_hero: false,
                        is_siege_minion: m.kind.is_siege(),
                        is_minion: !m.kind.is_siege(),
                        attacking_allied_hero: false,
                    });
                }
            }
            for h in &self.heroes {
                if !h.dead && h.team != s_team && h.pos.distance(s_pos) <= s_range {
                    candidates.push(TowerTargetCandidate {
                        id: h.id,
                        pos: h.pos,
                        is_hero: true,
                        is_siege_minion: false,
                        is_minion: false,
                        attacking_allied_hero: h.attacked_enemy_hero_t > 0.0,
                    });
                }
            }

            let Some(chosen_id) = select_tower_target(s_pos, s_range, cur_target, &candidates)
            else {
                self.structures[s_idx].current_target = None;
                self.structures[s_idx].consecutive_hero_hits = 0;
                continue;
            };

            self.structures[s_idx].attack_cd = self.structures[s_idx].attack_interval;
            let base_dmg = self.structures[s_idx].attack_damage;

            if let Some(h_idx) = self.heroes.iter().position(|h| h.id == chosen_id) {
                if self.structures[s_idx].current_target == Some(chosen_id) {
                    self.structures[s_idx].consecutive_hero_hits =
                        (self.structures[s_idx].consecutive_hero_hits + 1).min(5);
                } else {
                    self.structures[s_idx].current_target = Some(chosen_id);
                    self.structures[s_idx].consecutive_hero_hits = 1;
                }
                let ramp = 1.0 + f32::from(self.structures[s_idx].consecutive_hero_hits - 1) * 0.20;
                let enemy_lvl = self.team_level(s_team.opposite());
                let (_, died) = self.heroes[h_idx].take_damage(base_dmg * ramp, enemy_lvl);
                if died {
                    match s_team {
                        Team::Alliance => self.alliance_kills += 1,
                        Team::Horde => self.horde_kills += 1,
                    }
                    self.add_team_xp(s_team, 125.0, rng);
                }
            } else if let Some(m_idx) = self.minions.iter().position(|m| m.id == chosen_id) {
                self.structures[s_idx].current_target = Some(chosen_id);
                self.structures[s_idx].consecutive_hero_hits = 0;
                self.minions[m_idx].hp -= base_dmg;
                if self.minions[m_idx].hp <= 0.0 && !self.minions[m_idx].dead {
                    self.minions[m_idx].dead = true;
                    let m_pos = self.minions[m_idx].pos;
                    let xp = self.minions[m_idx].xp_value;
                    let globe = self.minions[m_idx].drops_regen_globe;
                    self.spawn_minion_pickups(m_pos, xp, globe, s_team);
                }
            }
        }
    }
}

/// Starts a new 5v5 Mid Brawl match from the menu, syncing `Selection` and `Run` and resetting `MobaMatch`.
pub(crate) fn start_moba_match(
    moba: &mut MobaMatch,
    draft: &TavernDraftState,
    selection: &mut Selection,
    run: &mut Run,
    rng: &mut Rng,
) -> bool {
    let map_idx = selection.map.min(MOBA_MAPS.len() - 1);
    selection.stage = map_idx;
    let all_ai = bots::auto_class_from_env().is_some();
    *moba = MobaMatch::new_from_draft(draft, map_idx, all_ai, rng);

    let card = draft.active_player_card();
    let surv_class = &crate::survivors::data::CLASSES[card.class_def().survivors_idx];
    let next_guid = run.next_guid;
    *run = Run::default();
    run.class = surv_class;
    run.race = card.race;
    run.next_guid = next_guid;
    run.hp = moba.player_hero().map_or(600.0, |h| h.hp);
    run.version += 1;
    true
}

/// Maps a 2D mid-lane coordinate (`x` in `[-120, 120]`, `y` in `[-24, 24]`) onto 3D Bevy world space.
pub(crate) fn lane_to_world(map: &MobaMapDef, lane_pos: Vec2, world: Option<&WorldPoint>) -> Vec3 {
    let center = Vec3::new(-map.center[1], map.center[2], -map.center[0]);
    let mut p = center + Vec3::new(lane_pos.x, elevation_at(lane_pos), lane_pos.y);
    if let Some(w) = world {
        if let Some(h) = w.terrain_height_under(p) {
            p.y = h + elevation_at(lane_pos);
        }
    }
    p
}

#[derive(Component)]
pub(crate) struct MobaVisualUnit(#[allow(dead_code)] pub u32);

#[derive(Resource, Default)]
struct MobaEntityMap(HashMap<u32, Entity>);

pub struct MobaPlugin;

impl Plugin for MobaPlugin {
    fn build(&self, app: &mut App) {
        let initial_mode = if active() {
            GameMode::Moba5v5
        } else {
            GameMode::SurvivorsSolo
        };
        app.insert_resource(initial_mode)
            .init_resource::<TavernDraftState>()
            .init_resource::<MobaMatch>()
            .init_resource::<MobaEntityMap>()
            .add_systems(Startup, hud::setup_moba_hud)
            .add_systems(
                Update,
                (
                    drive_moba_player_input,
                    tick_moba_simulation,
                    sync_moba_3d_entities,
                    hud::moba_hud_inputs,
                    hud::refresh_moba_hud,
                )
                    .chain()
                    .after(WorldStage::Input),
            );
    }
}

/// Applies WASD player steering to `moba.player_hero_mut()` when in `GameMode::Moba5v5`.
fn drive_moba_player_input(
    time: Res<Time>,
    keys: Res<ButtonInput<KeyCode>>,
    phase: Res<State<Phase>>,
    mode: Res<GameMode>,
    mut moba: ResMut<MobaMatch>,
) {
    if *mode != GameMode::Moba5v5 || *phase.get() != Phase::Playing {
        return;
    }
    let dt = time.delta_secs();
    let team_lvl = moba.team_level(moba.player_team);
    let Some(hero) = moba.player_hero_mut() else {
        return;
    };
    if hero.is_ai_controlled || !hero.can_move() {
        return;
    }
    let mut dir = Vec2::ZERO;
    if keys.any_pressed([KeyCode::KeyW, KeyCode::ArrowUp]) {
        dir.y -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyS, KeyCode::ArrowDown]) {
        dir.y += 1.0;
    }
    if keys.any_pressed([KeyCode::KeyA, KeyCode::ArrowLeft]) {
        dir.x -= 1.0;
    }
    if keys.any_pressed([KeyCode::KeyD, KeyCode::ArrowRight]) {
        dir.x += 1.0;
    }
    if dir != Vec2::ZERO && dt > 0.0 {
        let dir = dir.normalize();
        let speed = hero.stats(team_lvl).speed;
        hero.pos.x =
            (hero.pos.x + dir.x * speed * dt).clamp(-LANE_HALF_LEN + 4.0, LANE_HALF_LEN - 4.0);
        hero.pos.y =
            (hero.pos.y + dir.y * speed * dt).clamp(-LANE_HALF_WIDTH + 2.0, LANE_HALF_WIDTH - 2.0);
        hero.facing_yaw = (-dir.x).atan2(-dir.y);
    }
}

/// Advances `MobaMatch::step` each frame during `Phase::Playing` in `GameMode::Moba5v5`.
fn tick_moba_simulation(
    time: Res<Time>,
    phase: Res<State<Phase>>,
    mode: Res<GameMode>,
    loading: Res<crate::loading_screen::LoadingScreen>,
    mut moba: ResMut<MobaMatch>,
    mut run: ResMut<Run>,
    mut rng: ResMut<Rng>,
    mut next: ResMut<NextState<Phase>>,
) {
    if *mode != GameMode::Moba5v5 || *phase.get() != Phase::Playing || loading.covering() {
        return;
    }
    let dt = time.delta_secs().min(0.1);
    moba.step(dt, &mut rng);

    // Sync `Run` stats so the shared GameOver summary screen shows accurate MOBA stats!
    run.elapsed = moba.elapsed;
    run.level = moba.team_level(moba.player_team);
    if let Some(p) = moba.player_hero() {
        run.kills = p.kills;
        run.damage_done = f64::from(p.damage_dealt);
        run.hp = p.hp.max(1.0);
    }
    run.bosses = moba.objectives.raid_boss.slain_count;

    if let Some(winner) = moba.winner {
        run.won = winner == moba.player_team;
        next.set(Phase::GameOver);
    }
}

/// Spawns and updates 3D `NetEntity` / `ObjectStore` representations of the 10 heroes,
/// 8 structures, active Raid Boss, and lane minions in the live Bevy world.
#[allow(clippy::too_many_arguments)]
fn sync_moba_3d_entities(
    mut commands: Commands,
    phase: Res<State<Phase>>,
    mode: Res<GameMode>,
    moba: Res<MobaMatch>,
    mut run: ResMut<Run>,
    mut map: ResMut<MobaEntityMap>,
    world: WorldPoint,
    mut player: ResMut<crate::player::Player>,
    mut transforms: Query<(
        &mut Transform,
        Option<&mut MovementState>,
        Option<&mut ObjectStore>,
    )>,
) {
    if *mode != GameMode::Moba5v5 || *phase.get() != Phase::Playing {
        if !map.0.is_empty() {
            for (_, e) in map.0.drain() {
                commands.entity(e).despawn();
            }
        }
        return;
    }

    let map_def = moba.map_def();
    let mut active_ids = Vec::new();

    // 1. Sync 10 Heroes
    for h in &moba.heroes {
        active_ids.push(h.id);
        let world_pos = lane_to_world(map_def, h.pos, Some(&world));
        if h.id == moba.player_hero_id {
            player.active = true;
            player.detached = false;
            player.settling = false;
            player.world_stale = false;
            player.pos = world_pos;
        }
        if let Some(&e) = map.0.get(&h.id) {
            if let Ok((mut tf, mv, _)) = transforms.get_mut(e) {
                let moved = (tf.translation - world_pos).length() > 0.05;
                tf.translation = world_pos;
                tf.rotation = Quat::from_rotation_y(h.facing_yaw);
                if let Some(mut mv) = mv {
                    mv.speed = if moved && !h.dead { 7.0 } else { 0.0 };
                    mv.flags = if moved && !h.dead {
                        move_flags::FORWARD
                    } else {
                        0
                    };
                }
            }
        } else {
            let surv_class = &crate::survivors::data::CLASSES[h.card.class_def().survivors_idx];
            let h_def = h.hero_def();
            let display = h_def
                .display_override
                .unwrap_or_else(|| surv_class.looks.first().copied().unwrap_or(1300));
            let _ = h_def.weapons;
            let serial = run.guid();
            let e = crate::survivors::units::spawn_unit(
                &mut commands,
                &crate::survivors::units::UnitSpec {
                    serial,
                    display,
                    scale: h_def.scale,
                    level: 60,
                    faction: h.team.faction_template(),
                    bytes0: u32::from(h.card.race)
                        | (u32::from(h.card.class_def().id) << 8)
                        | (u32::from(h.card.sex) << 16),
                    weapons: surv_class.weapons,
                    sheath: surv_class.sheath,
                },
                world_pos,
                h.facing_yaw,
            );
            commands.entity(e).insert((MobaVisualUnit(h.id), RunEntity));
            if h.id == moba.player_hero_id {
                commands
                    .entity(e)
                    .insert(crate::survivors::hero::Hero { vy: 0.0, air: 0.0 });
            }
            map.0.insert(h.id, e);
        }
    }

    // 2. Sync Structures
    for s in &moba.structures {
        if s.destroyed {
            continue;
        }
        let key_id = 10_000 + s.id;
        active_ids.push(key_id);
        let world_pos = lane_to_world(map_def, s.pos, Some(&world));
        if let Some(&e) = map.0.get(&key_id) {
            if let Ok((mut tf, _, _)) = transforms.get_mut(e) {
                tf.translation = world_pos;
            }
        } else {
            let serial = run.guid();
            let e = crate::survivors::units::spawn_unit(
                &mut commands,
                &crate::survivors::units::UnitSpec {
                    serial,
                    display: s.display_id,
                    scale: s.scale,
                    level: 63,
                    faction: s.team.faction_template(),
                    bytes0: 1 << 8,
                    weapons: &[],
                    sheath: 1,
                },
                world_pos,
                if s.team == Team::Alliance {
                    0.0
                } else {
                    std::f32::consts::PI
                },
            );
            commands
                .entity(e)
                .insert((MobaVisualUnit(key_id), RunEntity));
            map.0.insert(key_id, e);
        }
    }

    // 3. Sync Lane Minions
    for m in &moba.minions {
        if m.dead {
            continue;
        }
        let key_id = 20_000 + m.id;
        active_ids.push(key_id);
        let world_pos = lane_to_world(map_def, m.pos, Some(&world));
        if let Some(&e) = map.0.get(&key_id) {
            if let Ok((mut tf, mv, _)) = transforms.get_mut(e) {
                tf.translation = world_pos;
                if let Some(mut mv) = mv {
                    mv.speed = m.speed;
                    mv.flags = move_flags::FORWARD;
                }
            }
        } else {
            let serial = run.guid();
            let e = crate::survivors::units::spawn_unit(
                &mut commands,
                &crate::survivors::units::UnitSpec {
                    serial,
                    display: m.display_id,
                    scale: m.scale,
                    level: 60,
                    faction: m.team.faction_template(),
                    bytes0: 1 << 8,
                    weapons: &[],
                    sheath: 1,
                },
                world_pos,
                if m.team == Team::Alliance {
                    0.0
                } else {
                    std::f32::consts::PI
                },
            );
            commands
                .entity(e)
                .insert((MobaVisualUnit(key_id), RunEntity));
            map.0.insert(key_id, e);
        }
    }

    // Despawn any stale visual units
    let stale: Vec<u32> = map
        .0
        .keys()
        .copied()
        .filter(|id| !active_ids.contains(id))
        .collect();
    for id in stale {
        if let Some(e) = map.0.remove(&id) {
            commands.entity(e).despawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::moba::bots::recommended_item_order_for_role;
    use crate::moba::combat::{BuyOutcome, DrTracker};
    use crate::moba::data::{
        heroes_by_theme, heroes_by_type, moba_hero_by_key, CcCategory, DamageSchool, HeroRole,
        HeroType, ThemeTag, MOBA_CLASSES, MOBA_HEROES, MOBA_MAPS, SHOP_ITEMS,
    };
    use crate::moba::draft::{
        auto_balance_team_roster, card_for_hero_index, is_roster_role_balanced,
        is_roster_type_balanced, TavernDraftState,
    };
    use crate::moba::lane::{
        brush_at, is_uphill_attack, is_visible_to_team, select_tower_target, PLATE_FALLOFF_TIME,
        PLATE_GOLD_REWARD,
    };
    use crate::moba::objectives::{
        FIRST_BOSS_TIME, MERC_SPAWN_TIME, RUNE_INTERVAL, SECOND_BOSS_TIME, SUDDEN_DEATH_TIME,
    };
    use crate::moba::progression::{
        comeback_xp_multiplier, deal_legendary_boss_cards, deal_milestone_cards,
        deal_milestone_cards_for_hero, MilestoneCardKind,
    };

    #[test]
    fn all_nine_wow_classes_maps_and_shop_items_are_valid() {
        assert_eq!(MOBA_CLASSES.len(), 9);
        for c in &MOBA_CLASSES {
            assert!(!c.name.is_empty());
            assert!(c.base_hp >= 500.0);
            assert!(c.auto_primary.spell_id > 0);
            assert!(c.q_skill.spell_id > 0);
            assert!(c.w_skill.spell_id > 0);
            assert!(c.e_skill.spell_id > 0);
            assert!(c.ultimates[0].spell_id > 0);
            assert!(c.ultimates[1].spell_id > 0);
            assert_ne!(c.ultimates[0].key, c.ultimates[1].key);
            assert!(c.mobility.spell_id > 0);
        }
        assert_eq!(MOBA_MAPS.len(), 4);
        assert!(SHOP_ITEMS.len() >= 25);
        assert!(data::shop_item("insignia_of_the_alliance").is_some());
        assert!(data::shop_item("spellbreaker_crest").is_some());
        assert!(data::shop_item("sunder_cleaver").is_some());
        assert!(data::shop_item("curse_of_elements_staff").is_some());
        assert!(data::shop_item("annihilator_axe").is_some());
        assert!(data::shop_item("nightfall_spellblade").is_some());
        assert!(data::shop_item("thunderfury").is_some());
        assert!(data::shop_item("sulfuras_hand_of_ragnaros").is_some());
        let _ = (deal_legendary_boss_cards, recommended_item_order_for_role);
    }

    #[test]
    fn all_36_theme_heroes_and_35_themes_and_three_combat_types_are_valid() {
        assert_eq!(MOBA_HEROES.len(), 36);
        assert_eq!(heroes_by_type(HeroType::Brawler).len(), 14);
        assert_eq!(heroes_by_type(HeroType::Damage).len(), 12);
        assert_eq!(heroes_by_type(HeroType::Support).len(), 10);

        for (idx, h) in MOBA_HEROES.iter().enumerate() {
            assert_eq!(h.idx, idx);
            assert!(!h.key.is_empty());
            assert!(!h.name.is_empty());
            assert!(!h.title.is_empty());
            assert!(h.themes.len() >= 2 && h.themes.len() <= 4);
            assert!(h.base_hp >= 500.0);
            assert!(h.base_armor > 0.0);
            assert!(h.base_magic_resist > 0.0);
            assert!(h.auto_primary.spell_id > 0);
            assert!(h.q_skill.spell_id > 0);
            assert!(h.w_skill.spell_id > 0);
            assert!(h.e_skill.spell_id > 0);
            assert!(h.ultimates[0].spell_id > 0);
            assert!(h.ultimates[1].spell_id > 0);
            assert_ne!(h.ultimates[0].key, h.ultimates[1].key);
            assert!(h.mobility.spell_id > 0);
            assert!(moba_hero_by_key(h.key).is_some());

            // Verify Brawler is melee-range auto, Damage & Support are ranged auto
            match h.hero_type {
                HeroType::Brawler => assert!(h.auto_primary.range <= 9.0),
                HeroType::Damage | HeroType::Support => assert!(h.auto_primary.range >= 20.0),
            }
        }

        // Every single one of the 35 requested themes in ThemeTag::ALL is represented!
        assert_eq!(ThemeTag::ALL.len(), 35);
        for &theme in &ThemeTag::ALL {
            let matching = heroes_by_theme(theme);
            assert!(
                !matching.is_empty(),
                "ThemeTag::{theme:?} ({}) must be represented by at least 1 hero in MOBA_HEROES",
                theme.label()
            );
        }
    }

    #[test]
    fn dual_mitigation_armor_vs_magic_resist_and_shred_amplification() {
        // Pick Brawler 0 (Thaelin: high Armor) and Support 26 (Alonsus: high Magic Resist)
        let mut h = MobaHero::new(
            1,
            "Target",
            Team::Alliance,
            card_for_hero_index(0),
            false,
            0,
        );
        h.hp = 5000.0;
        h.card_hp_flat = 4000.0;

        // 1. Positive Armor & Magic Resist mitigate Physical & Magic damage compared to Pure damage
        let (pure_dmg, _) = h.take_typed_damage(200.0, DamageSchool::Pure, 1);
        assert!((pure_dmg - 200.0).abs() < 1e-3);

        let (phys_base, _) = h.take_typed_damage(200.0, DamageSchool::Physical, 1);
        let (mag_base, _) = h.take_typed_damage(200.0, DamageSchool::Magic, 1);
        assert!(phys_base < pure_dmg);
        assert!(mag_base < pure_dmg);

        // 2. Apply -Armor shred (`armor_shred`) -> increases Physical damage taken, leaves Magic damage unchanged!
        h.apply_armor_shred(35.0, 5.0);
        assert!(h.effective_armor(1) < 0.0);
        let (phys_shredded, _) = h.take_typed_damage(200.0, DamageSchool::Physical, 1);
        let (mag_unshredded, _) = h.take_typed_damage(200.0, DamageSchool::Magic, 1);
        assert!(
            phys_shredded > pure_dmg,
            "Negative effective armor should amplify physical damage above raw (got {phys_shredded})"
        );
        assert!((mag_unshredded - mag_base).abs() < 1e-3);

        // 3. Apply -Magic shred (`magic_shred`) -> drives effective Magic Resist negative and amplifies Magic damage!
        h.apply_magic_shred(35.0, 5.0);
        assert!(h.effective_magic_resist(1) < 0.0);
        let (mag_shredded, _) = h.take_typed_damage(200.0, DamageSchool::Magic, 1);
        assert!(
            mag_shredded > pure_dmg,
            "Negative effective magic resist should amplify magic damage above raw (got {mag_shredded})"
        );

        // 4. Grant Anti-Magic Shell -> purges active magic_shred and absorbs Magic damage while Physical bypasses it!
        h.grant_anti_magic_shell(150.0, 4.0);
        assert_eq!(h.magic_shred, 0.0);
        let hp_before_magic = h.hp;
        let _ = h.take_typed_damage(100.0, DamageSchool::Magic, 1);
        assert!(
            (h.hp - hp_before_magic).abs() < 1e-3,
            "Anti-Magic Shell should absorb 100% of Magic damage while shell remains"
        );
        assert!(h.anti_magic_shell < 150.0);

        let hp_before_phys = h.hp;
        let _ = h.take_typed_damage(100.0, DamageSchool::Physical, 1);
        assert!(
            h.hp < hp_before_phys,
            "Physical damage should bypass Anti-Magic Shell and reduce HP"
        );

        // 5. Poison/Affliction DoT + Healing Cut (`heal_cut_pct`)
        h.hp = 500.0;
        h.apply_poison_affliction(40.0, 4.0, 0.40);
        let healed = h.receive_healing(100.0, 1);
        assert!((healed - 60.0).abs() < 1e-3);
    }

    #[test]
    fn tavern_bench_shuffle_reroll_swap_and_role_balancing() {
        let mut rng = Rng::default();
        let mut draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        assert_eq!(draft.player_hand.len(), 3);
        assert!(!draft.bench.is_empty());
        assert!(is_roster_role_balanced(&draft.alliance_roster));
        assert!(is_roster_role_balanced(&draft.horde_roster));
        assert!(is_roster_type_balanced(&draft.alliance_roster));
        assert!(is_roster_type_balanced(&draft.horde_roster));

        let bench_before = draft.bench.len();
        assert!(draft.reroll_hand(&mut rng));
        assert_eq!(
            draft.bench.len(),
            (bench_before + 1).min(draft::MAX_BENCH_CARDS)
        );
        assert!(is_roster_role_balanced(&draft.alliance_roster));

        let bench0 = draft.bench[0];
        assert!(draft.swap_with_bench(0, &mut rng));
        assert_eq!(draft.active_player_card(), bench0);
        assert!(is_roster_role_balanced(&draft.alliance_roster));

        // Verify cycling through all 36 heroes in AllPick / CycleHero
        let start_idx = draft.active_player_card().hero_idx;
        let next_card = draft.cycle_hero(&mut rng);
        assert_eq!(next_card.hero_idx, (start_idx + 1) % MOBA_HEROES.len());
        assert!(is_roster_role_balanced(&draft.alliance_roster));

        // Verify role & type balancing across every possible lead role
        for role in [
            HeroRole::Tank,
            HeroRole::Healer,
            HeroRole::MeleeDps,
            HeroRole::RangedDps,
        ] {
            let lead = draft::random_card_for_role(role, Team::Alliance, &mut rng);
            let roster = auto_balance_team_roster(lead, &mut rng, Team::Alliance);
            assert!(is_roster_role_balanced(&roster));
            assert!(is_roster_type_balanced(&roster));
        }
    }

    #[test]
    fn classic_wow_15s_diminishing_returns_and_bkb_cleanse() {
        let mut dr = DrTracker::default();
        // 1st Stun: 100% (4.0s)
        assert!((dr.apply_cc(CcCategory::Stun, 4.0, false) - 4.0).abs() < 1e-4);
        // 2nd Stun within 15s: 50% (2.0s)
        assert!((dr.apply_cc(CcCategory::Stun, 4.0, false) - 2.0).abs() < 1e-4);
        // 3rd Stun within 15s: 25% (1.0s)
        assert!((dr.apply_cc(CcCategory::Stun, 4.0, false) - 1.0).abs() < 1e-4);
        // 4th Stun within 15s: 0% (Immune!)
        assert_eq!(dr.apply_cc(CcCategory::Stun, 4.0, false), 0.0);

        // Independent category (Fear) still lands at 100%!
        assert!((dr.apply_cc(CcCategory::Fear, 3.0, false) - 3.0).abs() < 1e-4);

        // Tick 15.1s -> resets Stun DR back to 100%!
        dr.tick(15.1);
        assert!((dr.apply_cc(CcCategory::Stun, 4.0, false) - 4.0).abs() < 1e-4);

        // Verify Hero BKB Insignia Trinket purges active CC and grants 3.0s Unstoppable immunity
        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        let mut moba = MobaMatch::new_from_draft(&draft, 0, false, &mut rng);
        let h = &mut moba.heroes[0];
        h.gold = 3000;
        assert_eq!(
            h.buy_item("insignia_of_the_alliance", 1),
            Ok(BuyOutcome::EquippedDirectly)
        );
        h.apply_cc(CcCategory::Stun, 3.5);
        assert!(h.stun_t > 0.0);
        assert!(h.use_active_item(0, 1).is_some());
        assert_eq!(h.stun_t, 0.0);
        assert!((h.unstoppable_t - 3.0).abs() < 1e-4);
        // While Unstoppable, new CC returns 0.0s!
        assert_eq!(h.apply_cc(CcCategory::Fear, 3.0), 0.0);
    }

    #[test]
    fn aram_fountain_lock_goblin_courier_moonwell_and_buyback() {
        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        let mut moba = MobaMatch::new_from_draft(&draft, 0, false, &mut rng);
        let h = &mut moba.heroes[0];
        h.gold = 5000;

        // On Fountain before leaving spawn: equips directly!
        assert!(h.can_shop_directly());
        assert_eq!(
            h.buy_item("alterac_manna_biscuit", 1),
            Ok(BuyOutcome::EquippedDirectly)
        );
        assert_eq!(h.inventory.len(), 1);

        // Move out into mid lane -> locks direct Fountain shopping!
        h.pos = Vec2::new(0.0, 0.0);
        h.tick(0.1, 1, 0);
        assert!(h.has_left_fountain);
        assert!(!h.can_shop_directly());

        // Buying while in lane places item into Stash!
        assert_eq!(
            h.buy_item("highlanders_greaves", 1),
            Ok(BuyOutcome::PlacedInStash)
        );
        assert_eq!(h.stash.len(), 1);
        assert_eq!(h.inventory.len(), 1);

        // Call Goblin Courier (90s CD) -> delivers after flight!
        assert!(h.call_goblin_courier());
        assert!((h.courier_cd - combat::COURIER_COOLDOWN).abs() < 1e-4);
        h.tick(2.5, 1, 0);
        assert_eq!(h.stash.len(), 0);
        assert_eq!(h.inventory.len(), 2);

        // Moonwell restores 40% HP & Power and enters 75s cooldown
        let mut well = moba.moonwells[0].clone();
        assert!(well.is_ready());
        let (hp_gain, _) = well.try_drink(1000.0, 500.0).unwrap();
        assert!((hp_gain - 400.0).abs() < 1e-4);
        assert!(!well.is_ready());
        assert!((well.cooldown - lane::MOONWELL_COOLDOWN).abs() < 1e-4);

        // Soulstone Buyback while dead respawns immediately at Fountain with 360s CD
        let h = &mut moba.heroes[0];
        h.dead = true;
        h.respawn_timer = 25.0;
        assert!(h.soulstone_buyback(5, 0));
        assert!(!h.dead);
        assert!((h.buyback_cd - combat::BUYBACK_COOLDOWN).abs() < 1e-4);
        assert!((h.mount_launch_t - combat::MOUNT_LAUNCH_SECS).abs() < 1e-4);
    }

    #[test]
    fn shared_team_xp_comeback_multiplier_and_level_10_dual_ultimates() {
        assert_eq!(comeback_xp_multiplier(5, 5), 1.0);
        assert_eq!(comeback_xp_multiplier(5, 6), 1.0);
        // 2 levels behind -> +50% XP (1.5x); 4 levels behind -> +100% XP (2.0x)
        assert!((comeback_xp_multiplier(4, 6) - 1.50).abs() < 1e-4);
        assert!((comeback_xp_multiplier(4, 8) - 2.00).abs() < 1e-4);

        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        let mut moba = MobaMatch::new_from_draft(&draft, 0, false, &mut rng);

        // Advance Alliance Team XP all the way to Level 10 and verify non-pausing milestone deals
        moba.add_team_xp(Team::Alliance, 6000.0, &mut rng);
        assert!(moba.alliance_prog.level >= 10);

        // Verify Level 10 cards guarantee slots 0 and 1 are the 2 Heroic Ultimates for all 36 heroes!
        for hero in &MOBA_HEROES {
            let cards = deal_milestone_cards_for_hero(hero, 10, &mut rng);
            assert_eq!(
                cards[0].kind,
                MilestoneCardKind::HeroicUltimate { ult_index: 0 }
            );
            assert_eq!(
                cards[1].kind,
                MilestoneCardKind::HeroicUltimate { ult_index: 1 }
            );
            assert_eq!(cards[0].rarity, progression::CardRarity::Legendary);
            assert_eq!(cards[1].rarity, progression::CardRarity::Legendary);
        }
        let _ = deal_milestone_cards;
    }

    #[test]
    fn lane_fortifications_plates_aggro_backdoor_highground_and_glyph() {
        let mut structures = build_initial_structures();
        // Tier 1 Outer Tower has 5 Gold Plates before 8:00 (480s)
        let t1 = &mut structures[0];
        assert_eq!(t1.kind, StructureKind::OuterTower);
        assert_eq!(t1.plates_remaining, 5);
        let out = t1.apply_damage(500.0, 120.0, true, false, true);
        assert!(out.plates_broken >= 1);
        assert_eq!(
            out.gold_awarded,
            u32::from(out.plates_broken) * PLATE_GOLD_REWARD
        );

        // After 8:00 (480s), remaining plates fall off
        let out_late = t1.apply_damage(500.0, PLATE_FALLOFF_TIME + 10.0, true, false, true);
        assert_eq!(out_late.plates_broken, 0);
        assert_eq!(t1.plates_remaining, 0);

        // Backdoor Protection reduces Tier 2/3 damage by 60% when no enemy minions are nearby
        let keep = &mut structures[1];
        let full = keep.apply_damage(200.0, 200.0, true, false, true);
        let backdoor = keep.apply_damage(200.0, 200.0, false, false, true);
        assert!(backdoor.backdoor_protected);
        assert!((backdoor.actual_damage - full.actual_damage * 0.40).abs() < 1e-3);

        // Glyph of Fortification grants 5.0s structure invulnerability on 180s CD
        let mut glyph = GlyphState::default();
        assert!(glyph.activate());
        assert!(glyph.is_invulnerable());
        let invuln_out = keep.apply_damage(500.0, 200.0, true, glyph.is_invulnerable(), true);
        assert_eq!(invuln_out.actual_damage, 0.0);

        // Deterministic Tower Aggro Priority: Hero attacking allied hero > Siege > Minion > Hero
        let tower_pos = Vec2::new(-48.0, 0.0);
        let cands = vec![
            TowerTargetCandidate {
                id: 10,
                pos: Vec2::new(-42.0, 0.0),
                is_hero: true,
                is_siege_minion: false,
                is_minion: false,
                attacking_allied_hero: false,
            },
            TowerTargetCandidate {
                id: 20,
                pos: Vec2::new(-40.0, 0.0),
                is_hero: false,
                is_siege_minion: false,
                is_minion: true,
                attacking_allied_hero: false,
            },
            TowerTargetCandidate {
                id: 30,
                pos: Vec2::new(-38.0, 0.0),
                is_hero: false,
                is_siege_minion: true,
                is_minion: false,
                attacking_allied_hero: false,
            },
        ];
        // Picks Siege minion (30) over regular minion (20) and non-aggressor hero (10)
        assert_eq!(select_tower_target(tower_pos, 20.0, None, &cands), Some(30));

        // When Hero (10) attacks an allied hero, Tower immediately switches aggro to Hero (10)!
        let mut cands_aggro = cands.clone();
        cands_aggro[0].attacking_allied_hero = true;
        assert_eq!(
            select_tower_target(tower_pos, 20.0, Some(30), &cands_aggro),
            Some(10)
        );

        // High-Ground Ramp uphill miss check & Brush stealth + True Sight reveal
        assert!(is_uphill_attack(
            Vec2::new(-65.0, 0.0),
            Vec2::new(-85.0, 0.0)
        ));
        assert!(!is_uphill_attack(
            Vec2::new(-85.0, 0.0),
            Vec2::new(-65.0, 0.0)
        ));

        let brush_pos = lane::BRUSH_ZONES[0].center;
        assert_eq!(brush_at(brush_pos), Some(0));
        assert!(!is_visible_to_team(
            Team::Horde,
            brush_pos,
            false,
            &structures,
            &[Vec2::new(10.0, 0.0)]
        ));
    }

    #[test]
    fn four_game_neutral_event_clock_and_raid_boss_aegis_soulstone() {
        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        let mut moba = MobaMatch::new_from_draft(&draft, 0, true, &mut rng);

        // 1. 90s River Power Runes
        moba.objectives.tick(RUNE_INTERVAL + 1.0, 1.0);
        assert!(moba.objectives.runes[0].active);
        assert!(moba.objectives.runes[1].active);

        // 2. 150s Pushing Mercenary Camps
        moba.objectives
            .tick(MERC_SPAWN_TIME + 1.0, MERC_SPAWN_TIME + 1.0);
        assert!(moba.objectives.merc_camps[0].active);
        let minions_before = moba.minions.len();
        assert!(moba.objectives.merc_camps[0].damage_camp(2000.0, Team::Alliance));
        let pushed = spawn_mercenary_push(
            Team::Alliance,
            moba.objectives.merc_camps[0].pos,
            &mut moba.next_entity_id,
        );
        moba.minions.extend(pushed);
        assert_eq!(moba.minions.len(), minions_before + 2);

        // 3. 7:00 (420s) Center Raid Boss Pit (Patchwerk) + Aegis Soulstone in-place revival
        moba.objectives.tick(FIRST_BOSS_TIME + 1.0, 1.0);
        assert!(moba.objectives.raid_boss.active);
        assert_eq!(moba.objectives.raid_boss.tier, 1);

        moba.on_raid_boss_slain(0, Team::Alliance);
        assert!(moba.heroes[0].has_aegis_soulstone);
        // Deal fatal damage to hero 0 -> triggers Aegis Soulstone 3.0s in-place revival instead of death!
        let (_, died_for_real) = moba.heroes[0].take_damage(99999.0, 10);
        assert!(!died_for_real);
        assert!(!moba.heroes[0].dead);
        assert!(moba.heroes[0].aegis_revive_t.is_some());
        moba.heroes[0].tick(3.5, 10, 0);
        assert!(moba.heroes[0].aegis_revive_t.is_none());
        assert!(moba.heroes[0].hp > 500.0);

        // 4. 13:00 (780s) Second Raid Boss & 18:00 (1080s) Sudden Death General March
        moba.objectives.tick(SECOND_BOSS_TIME + 1.0, 1.0);
        assert!(moba.objectives.raid_boss.active);
        assert_eq!(moba.objectives.raid_boss.tier, 2);

        moba.elapsed = SUDDEN_DEATH_TIME + 1.0;
        moba.step(1.0, &mut rng);
        assert!(moba.objectives.sudden_death_active);
        assert!(moba
            .structures
            .iter()
            .filter(|s| s.kind == StructureKind::FactionGeneral)
            .all(|s| s.marching_sudden_death));
    }

    #[test]
    fn full_5v5_ai_match_simulation_runs_to_general_victory() {
        let mut rng = Rng::default();
        let draft = TavernDraftState::new_shuffle(&mut rng, Team::Alliance);
        let mut moba = MobaMatch::new_from_draft(&draft, 0, true, &mut rng);

        // Simulate up to 22 minutes of 5v5 AI vs AI combat in 0.25s steps
        let dt = 0.25;
        let max_steps = (1320.0 / dt) as usize;
        for _ in 0..max_steps {
            moba.step(dt, &mut rng);
            if moba.winner.is_some() {
                break;
            }
        }

        assert!(
            moba.winner.is_some(),
            "5v5 match should conclude with a Faction General victory (elapsed={:.1}s)",
            moba.elapsed
        );
        assert!(moba.alliance_prog.level >= 5 || moba.horde_prog.level >= 5);
        assert!(moba.alliance_kills + moba.horde_kills > 0);
        assert!(moba.heroes.iter().any(|h| !h.inventory.is_empty()));
        assert!(moba.heroes.iter().any(|h| h.cards_picked >= 1));
    }
}
