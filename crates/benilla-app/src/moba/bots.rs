//! Role-Aware 5v5 AI Hero Bot Decision Engine (`Tank`, `Healer`, `MeleeDps`, `RangedDps`)
//! and `WOW_MOBA_AUTO` hands-off automation support.

use bevy::prelude::*;

use super::combat::{MobaHero, MAX_INVENTORY_SLOTS};
use super::data::{ActiveItemEffect, HeroRole, Team, MOBA_CLASSES};
use super::lane::{Moonwell, Structure, LANE_HALF_LEN, LANE_HALF_WIDTH};
use super::objectives::ObjectiveClock;
use super::progression::{MilestoneCard, MilestoneCardKind};
use super::waves::{LaneMinion, MobaPickup};
use crate::survivors::data::Stat;

/// Selects the highest-synergy card (`0..3`) from a 3-card milestone offer for `role`.
pub fn pick_best_card_for_role(role: HeroRole, offer: &[MilestoneCard; 3]) -> usize {
    // At Team Level 10, slots 0 and 1 are the hero's 2 Heroic Ultimates: always pick an Ultimate!
    if matches!(offer[0].kind, MilestoneCardKind::HeroicUltimate { .. }) {
        return match role {
            HeroRole::Tank | HeroRole::Healer => 0,
            HeroRole::MeleeDps | HeroRole::RangedDps => 0,
        };
    }

    let score_card = |c: &MilestoneCard| -> f32 {
        match c.kind {
            MilestoneCardKind::HeroicUltimate { .. } => 100.0,
            MilestoneCardKind::LegendaryMutator {
                might_bonus,
                haste_bonus,
                hp_bonus,
            } => might_bonus * 100.0 + haste_bonus * 80.0 + hp_bonus * 0.15,
            MilestoneCardKind::AbilityMutator {
                bonus_dmg_pct,
                cdr_pct,
                ..
            } => bonus_dmg_pct * 90.0 + cdr_pct * 75.0,
            MilestoneCardKind::StatBoost { stat, amount } => match (role, stat) {
                (HeroRole::Tank, Stat::MaxHp | Stat::Armor) => amount * 120.0,
                (HeroRole::Healer, Stat::Haste | Stat::Might) => amount * 115.0,
                (HeroRole::MeleeDps | HeroRole::RangedDps, Stat::Might | Stat::Speed) => {
                    amount * 120.0
                }
                _ => amount * 80.0,
            },
        }
    };

    (0..3)
        .max_by(|&a, &b| score_card(&offer[a]).total_cmp(&score_card(&offer[b])))
        .unwrap_or(0)
}

/// Role-tailored 6-slot item build path from `SHOP_ITEMS`.
pub fn recommended_item_order_for_role(role: HeroRole) -> &'static [&'static str; 6] {
    match role {
        HeroRole::Tank => &[
            "major_healthstone",
            "highlanders_greaves",
            "insignia_of_the_alliance",
            "aegis_of_the_blood_god",
            "quel_serrar",
            "bulwark_of_azzinoth_relic",
        ],
        HeroRole::Healer => &[
            "alterac_manna_biscuit",
            "boots_of_the_shadow_flame",
            "insignia_of_the_alliance",
            "tidal_charm",
            "staff_of_dominance",
            "flask_of_supreme_power",
        ],
        HeroRole::MeleeDps => &[
            "battleground_signet",
            "goblin_rocket_boots",
            "insignia_of_the_alliance",
            "blink_strike_dagger",
            "arcanite_reaper",
            "thunderfury",
        ],
        HeroRole::RangedDps => &[
            "battleground_signet",
            "boots_of_the_shadow_flame",
            "insignia_of_the_alliance",
            "zandalarian_hero_charm",
            "neltharions_tear",
            "sulfuras_hand_of_ragnaros",
        ],
    }
}

/// Executes non-pausing milestone card picks, item shopping, Goblin Courier calls, Active Trinket
/// usage (including BKB *Insignia* when stunned/feared/rooted!), and late-game Soulstone Buyback.
pub fn bot_economy_and_items_step(
    hero: &mut MobaHero,
    team_level: u32,
    slot_in_team: usize,
    elapsed: f32,
) {
    // 1. Immediately pick any pending milestone card offers without pausing!
    while let Some(offer) = hero.pending_card_offers.first() {
        let pick_idx = pick_best_card_for_role(hero.role(), offer);
        hero.pick_milestone_card(pick_idx, team_level);
    }

    // 2. If dead in late-game (elapsed >= 480s) and Buyback is ready, use Soulstone Buyback!
    if hero.dead && elapsed >= 480.0 {
        let _ = hero.soulstone_buyback(team_level, slot_in_team);
    }

    // 3. Purchase next recommended item if affordable
    let total_owned = hero.inventory.len() + hero.stash.len();
    if total_owned < MAX_INVENTORY_SLOTS {
        let build = recommended_item_order_for_role(hero.role());
        for &item_key in build {
            let already_has = hero.inventory.iter().any(|i| i.key == item_key)
                || hero.stash.iter().any(|i| i.key == item_key);
            if !already_has {
                let _ = hero.buy_item(item_key, team_level);
                break;
            }
        }
    }

    // 4. Call Goblin Courier if items are waiting in stash while alive in lane!
    if !hero.dead && !hero.stash.is_empty() && hero.courier_cd <= 0.0 {
        hero.call_goblin_courier();
    }

    // 5. Trigger active items (especially BKB Insignia when stunned/feared/rooted/silenced!)
    if !hero.dead {
        let cc_afflicted =
            hero.stun_t > 0.2 || hero.fear_t > 0.2 || hero.root_t > 0.4 || hero.silence_t > 0.4;
        let hp_pct = hero.hp / hero.stats(team_level).max_hp.max(1.0);
        for slot in 0..hero.inventory.len() {
            let effect = hero.inventory[slot].active;
            let should_use = match effect {
                ActiveItemEffect::BkbCleanse { .. } => cc_afflicted,
                ActiveItemEffect::HealthstoneHeal { .. }
                | ActiveItemEffect::AegisBarrier { .. } => hp_pct < 0.55,
                ActiveItemEffect::ZandalarianBurst { .. }
                | ActiveItemEffect::RocketBootsDash { .. } => {
                    hero.attacked_enemy_hero_t > 0.0 || hp_pct < 0.75
                }
                ActiveItemEffect::BlinkStrike { .. } | ActiveItemEffect::TidalCharmStun { .. } => {
                    hero.attacked_enemy_hero_t > 0.0
                }
                ActiveItemEffect::None => false,
            };
            if should_use {
                hero.use_active_item(slot, team_level);
            }
        }
    }
}

/// Computes the tactical movement target for an AI-controlled hero based on:
/// - Moonwell survival threshold (`HP < 45%`)
/// - Nearby XP Wisps & Regen Globes
/// - 4-Game Event Clock objectives (River Runes, Mercenary Camps, Center Raid Boss)
/// - Role-specific frontline offset (`Tank` ahead, `Healer` & `RangedDps` behind)
#[allow(clippy::too_many_arguments)]
pub fn bot_choose_movement_target(
    hero: &MobaHero,
    team_level: u32,
    slot_in_team: usize,
    all_heroes: &[MobaHero],
    minions: &[LaneMinion],
    structures: &[Structure],
    moonwells: &[Moonwell],
    pickups: &[MobaPickup],
    objectives: &ObjectiveClock,
) -> Vec2 {
    let st = hero.stats(team_level);
    let hp_pct = hero.hp / st.max_hp.max(1.0);
    let dir = hero.team.dir_sign();

    // 1. Low HP (< 45%): drink from nearest ready allied Moonwell, or retreat behind allied tower!
    if hp_pct < 0.45 {
        if let Some(well) = moonwells
            .iter()
            .filter(|w| w.team == hero.team && w.is_ready())
            .min_by(|a, b| {
                a.pos
                    .distance(hero.pos)
                    .total_cmp(&b.pos.distance(hero.pos))
            })
        {
            return well.pos;
        }
        if let Some(tower) = structures
            .iter()
            .filter(|s| !s.destroyed && s.team == hero.team)
            .max_by(|a, b| (a.pos.x * dir).total_cmp(&(b.pos.x * dir)))
        {
            return Vec2::new(tower.pos.x - dir * 8.0, 0.0);
        }
        return MobaHero::fountain_center(hero.team);
    }

    // 2. Collect nearby Regen Globe or XP Wisp within 14 yards
    if let Some(p) = pickups
        .iter()
        .filter(|p| !p.collected && p.pos.distance(hero.pos) <= 14.0)
        .min_by(|a, b| {
            a.pos
                .distance(hero.pos)
                .total_cmp(&b.pos.distance(hero.pos))
        })
    {
        return p.pos;
    }

    // 3. Contest active River Power Rune if close
    if let Some(rune) = objectives
        .runes
        .iter()
        .filter(|r| r.active && r.pos.distance(hero.pos) <= 34.0)
        .min_by(|a, b| {
            a.pos
                .distance(hero.pos)
                .total_cmp(&b.pos.distance(hero.pos))
        })
    {
        return rune.pos;
    }

    // 4. Rally to Center Raid Boss Pit when spawned and healthy!
    if objectives.raid_boss.active
        && hp_pct > 0.55
        && hero.pos.distance(objectives.raid_boss.pos) <= 45.0
    {
        let spread_z = (slot_in_team as f32 - 2.0) * 2.2;
        return Vec2::new(
            objectives.raid_boss.pos.x - dir * 4.5,
            objectives.raid_boss.pos.y + spread_z,
        );
    }

    // 5. Capture allied-side Mercenary Camp when healthy and camp is active nearby
    if hp_pct > 0.62 && matches!(hero.role(), HeroRole::Tank | HeroRole::MeleeDps) {
        if let Some(camp) = objectives
            .merc_camps
            .iter()
            .find(|c| c.active && c.pos.distance(hero.pos) <= 28.0)
        {
            return camp.pos;
        }
    }

    // 6. Standard Mid-Lane Frontline advancement based on Allied Wave + Enemy Frontline
    let allied_wave_x = minions
        .iter()
        .filter(|m| !m.dead && m.team == hero.team)
        .map(|m| m.pos.x * dir)
        .reduce(f32::max)
        .map(|max_proj| max_proj * dir)
        .unwrap_or(0.0);

    let enemy_front_x = structures
        .iter()
        .filter(|s| !s.destroyed && s.team != hero.team)
        .map(|s| s.pos.x)
        .min_by(|a, b| (a * dir).total_cmp(&(b * dir)))
        .unwrap_or(dir * 95.0);

    // Never dive past an enemy standing tower without allied minions near it!
    let allied_minion_near_tower = minions
        .iter()
        .any(|m| !m.dead && m.team == hero.team && (m.pos.x - enemy_front_x).abs() <= 18.0);

    let safe_limit_x = if allied_minion_near_tower {
        enemy_front_x - dir * 5.0
    } else {
        enemy_front_x - dir * 19.0
    };

    let desired_front_x = if (allied_wave_x * dir) < (safe_limit_x * dir) {
        allied_wave_x + dir * 4.0
    } else {
        safe_limit_x
    };

    // Role offset from the battle line:
    // Tank initiates at the front; MeleeDps flanks; RangedDps and Healer stay safely behind!
    let role_offset = match hero.role() {
        HeroRole::Tank => 2.0,
        HeroRole::MeleeDps => 0.0,
        HeroRole::RangedDps => -11.0,
        HeroRole::Healer => -13.5,
    };

    // If Healer, stay near the most injured allied hero in the lane
    if hero.role() == HeroRole::Healer {
        if let Some(ally) = all_heroes
            .iter()
            .filter(|a| !a.dead && a.team == hero.team && a.id != hero.id)
            .min_by(|a, b| a.hp.total_cmp(&b.hp))
        {
            let target_x =
                (ally.pos.x - dir * 6.0).clamp(-LANE_HALF_LEN + 8.0, LANE_HALF_LEN - 8.0);
            return Vec2::new(target_x, ally.pos.y * 0.7);
        }
    }

    let lane_z = (slot_in_team as f32 - 2.0) * 3.2;
    Vec2::new(
        (desired_front_x + dir * role_offset).clamp(-LANE_HALF_LEN + 8.0, LANE_HALF_LEN - 8.0),
        lane_z.clamp(-LANE_HALF_WIDTH + 3.0, LANE_HALF_WIDTH - 3.0),
    )
}

/// Optional hands-off environment driver (`WOW_MOBA_AUTO=<class>`).
pub fn auto_class_from_env() -> Option<(usize, Team)> {
    let val = crate::webenv::var("WOW_MOBA_AUTO")?;
    let trimmed = val.trim();
    let idx = MOBA_CLASSES
        .iter()
        .position(|c| c.name.eq_ignore_ascii_case(trimmed))
        .unwrap_or(0);
    Some((idx, Team::Alliance))
}
