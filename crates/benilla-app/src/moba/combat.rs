//! Hybrid Action-MOBA Combat, Classic WoW 15s PvP Diminishing Returns (`DrTracker`),
//! LoL ARAM Fountain Shop Lock, Dota 2 Goblin Courier, Gryphon/Wyvern Fast-Launch,
//! Soulstone Buyback, Aegis Soulstone In-Place Revival, and BKB Medallion Cleanse.

use bevy::prelude::*;

use super::data::{
    moba_class_by_index, shop_item, ActiveItemEffect, CcCategory, HeroRole, MobaAbilityDef,
    MobaAbilitySlot, MobaClassDef, PowerType, ShopItemDef, Team,
};
use super::draft::HeroCard;
use super::lane::FOUNTAIN_X;
use super::progression::{MilestoneCard, MilestoneCardKind};
use crate::survivors::data::Stat;

pub const DR_WINDOW_SECS: f32 = 15.0;
pub const FOUNTAIN_SHOP_RADIUS: f32 = 18.0;
pub const COURIER_COOLDOWN: f32 = 90.0;
pub const COURIER_FLIGHT_SECS: f32 = 2.0;
pub const MOUNT_LAUNCH_SECS: f32 = 5.0;
pub const MOUNT_LAUNCH_SPEED_MULT: f32 = 3.0; // +200% speed boost!
pub const BUYBACK_COOLDOWN: f32 = 360.0;
pub const AEGIS_REVIVE_SECS: f32 = 3.0;
pub const MAX_INVENTORY_SLOTS: usize = 6;

/// One CC category's 15-second Diminishing Returns state.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrEntry {
    /// Number of CCs applied in this category within the active 15s window (`0..=4`).
    pub hits: u8,
    /// Seconds remaining until this category's DR resets to `100%`.
    pub reset_timer: f32,
}

/// Classic WoW 1.12 15-Second PvP Diminishing Returns state machine per unit:
/// - 1st CC in category: `100%` duration (`1.0`)
/// - 2nd CC within `15.0s`: `50%` duration (`0.5`)
/// - 3rd CC within `15.0s`: `25%` duration (`0.25`)
/// - 4th+ CC within `15.0s`: `0.0s` (**Immune!**)
/// - Resets cleanly after `15.0s` without a hit in that category.
#[derive(Clone, Copy, Debug, Default)]
pub struct DrTracker {
    pub categories: [DrEntry; 4],
}

impl DrTracker {
    /// Applies a CC of `category` with `base_duration`, returning the effective duration after DR
    /// (or `0.0` if `unstoppable` or at 4th+ stack).
    pub fn apply_cc(&mut self, category: CcCategory, base_duration: f32, unstoppable: bool) -> f32 {
        if unstoppable || base_duration <= 0.0 {
            return 0.0;
        }
        let entry = &mut self.categories[category.index()];
        if entry.reset_timer <= 0.0 {
            entry.hits = 0;
        }
        let mult = match entry.hits {
            0 => 1.0,
            1 => 0.5,
            2 => 0.25,
            _ => 0.0,
        };
        entry.hits = entry.hits.saturating_add(1);
        if mult > 0.0 {
            entry.reset_timer = DR_WINDOW_SECS;
        }
        base_duration * mult
    }

    pub fn tick(&mut self, dt: f32) {
        for entry in &mut self.categories {
            if entry.reset_timer > 0.0 {
                entry.reset_timer = (entry.reset_timer - dt).max(0.0);
                if entry.reset_timer <= 0.0 {
                    entry.hits = 0;
                }
            }
        }
    }

    pub fn hits_in(&self, category: CcCategory) -> u8 {
        self.categories[category.index()].hits
    }
}

/// Outcome of purchasing an item from the MOBA shop.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BuyOutcome {
    /// Purchased while at Fountain (before leaving spawn) or while dead: equipped immediately.
    EquippedDirectly,
    /// Purchased while alive out in the lane: placed in `stash` for Goblin Courier delivery.
    PlacedInStash,
}

/// Ability mutator bonuses accumulated from non-pausing milestone card picks.
#[derive(Clone, Copy, Debug, Default)]
pub struct SlotMutator {
    pub bonus_dmg_pct: f32,
    pub cdr_pct: f32,
    pub bonus_radius: f32,
}

/// Aggregated derived stats for a `MobaHero` (from base class + team level + cards + inventory items + runes).
#[derive(Clone, Copy, Debug)]
pub struct HeroDerivedStats {
    pub max_hp: f32,
    pub max_power: f32,
    pub might: f32,
    pub armor: f32,
    pub speed: f32,
    pub haste: f32,
    pub crit: f32,
    pub regen: f32,
    pub area: f32,
}

/// One of the 10 heroes (5 Alliance, 5 Horde) in the 5v5 Mid Brawl.
#[derive(Clone, Debug)]
pub struct MobaHero {
    pub id: u32,
    pub name: String,
    pub team: Team,
    pub card: HeroCard,
    pub is_ai_controlled: bool,
    pub pos: Vec2,
    pub facing_yaw: f32,
    pub hp: f32,
    pub power: f32,
    pub shield: f32,
    pub shield_t: f32,

    // Ability cooldowns & Level 10 Ultimate choice
    pub auto_cd: f32,
    pub q_cd: f32,
    pub w_cd: f32,
    pub e_cd: f32,
    pub r_cd: f32,
    pub f_cd: f32,
    pub chosen_ultimate: Option<usize>,

    // Non-pausing milestone card queue & mutators
    pub pending_card_offers: Vec<[MilestoneCard; 3]>,
    pub cards_picked: usize,
    pub slot_mutators: [SlotMutator; 6],
    pub card_might_bonus: f32,
    pub card_hp_bonus_pct: f32,
    pub card_hp_flat: f32,
    pub card_speed_bonus_pct: f32,
    pub card_haste_bonus: f32,
    pub card_armor_bonus: f32,
    pub card_crit_bonus: f32,
    pub card_regen_bonus: f32,

    // Crowd Control & Classic WoW 15s Diminishing Returns
    pub dr_tracker: DrTracker,
    pub stun_t: f32,
    pub root_t: f32,
    pub fear_t: f32,
    pub silence_t: f32,
    pub slow_t: f32,
    pub slow_factor: f32,
    pub unstoppable_t: f32,
    pub stealth_t: f32,

    // Temporary combat buffs (Bloodlust, Haste, Zandalarian Charm, River Runes)
    pub haste_buff_t: f32,
    pub haste_buff_mult: f32,
    pub might_buff_t: f32,
    pub might_buff_bonus: f32,
    pub rune_cdr_t: f32,

    // Economy, ARAM Fountain Lock, Inventory & Goblin Courier
    pub gold: u32,
    pub has_left_fountain: bool,
    pub inventory: Vec<&'static ShopItemDef>,
    pub item_cooldowns: [f32; MAX_INVENTORY_SLOTS],
    pub stash: Vec<&'static ShopItemDef>,
    pub courier_cd: f32,
    pub courier_flight_t: Option<f32>,

    // Respawn, Gryphon/Wyvern Fast-Launch, Soulstone Buyback & Aegis Soulstone
    pub dead: bool,
    pub respawn_timer: f32,
    pub mount_launch_t: f32,
    pub buyback_cd: f32,
    pub has_aegis_soulstone: bool,
    pub aegis_revive_t: Option<f32>,

    // Match statistics & Tower Aggro tracking
    pub kills: u32,
    pub deaths: u32,
    pub assists: u32,
    pub creep_score: u32,
    pub damage_dealt: f32,
    pub healing_done: f32,
    /// Seconds left on the "attacked an enemy hero" flag used by deterministic Tower Aggro Priority 1.
    pub attacked_enemy_hero_t: f32,
}

impl MobaHero {
    pub fn new(
        id: u32,
        name: impl Into<String>,
        team: Team,
        card: HeroCard,
        is_ai_controlled: bool,
        slot_in_team: usize,
    ) -> Self {
        let class = card.class_def();
        let spawn_pos = Self::fountain_pos_for(team, slot_in_team);
        let init_power = match class.power_type {
            PowerType::Mana | PowerType::Energy => class.base_power,
            PowerType::Rage => 0.0,
        };
        Self {
            id,
            name: name.into(),
            team,
            card,
            is_ai_controlled,
            pos: spawn_pos,
            facing_yaw: if team == Team::Alliance {
                0.0
            } else {
                std::f32::consts::PI
            },
            hp: class.base_hp,
            power: init_power,
            shield: 0.0,
            shield_t: 0.0,
            auto_cd: 0.0,
            q_cd: 0.0,
            w_cd: 0.0,
            e_cd: 0.0,
            r_cd: 0.0,
            f_cd: 0.0,
            chosen_ultimate: None,
            pending_card_offers: Vec::new(),
            cards_picked: 0,
            slot_mutators: [SlotMutator::default(); 6],
            card_might_bonus: 0.0,
            card_hp_bonus_pct: 0.0,
            card_hp_flat: 0.0,
            card_speed_bonus_pct: 0.0,
            card_haste_bonus: 0.0,
            card_armor_bonus: 0.0,
            card_crit_bonus: 0.0,
            card_regen_bonus: 0.0,
            dr_tracker: DrTracker::default(),
            stun_t: 0.0,
            root_t: 0.0,
            fear_t: 0.0,
            silence_t: 0.0,
            slow_t: 0.0,
            slow_factor: 1.0,
            unstoppable_t: 0.0,
            stealth_t: 0.0,
            haste_buff_t: 0.0,
            haste_buff_mult: 1.0,
            might_buff_t: 0.0,
            might_buff_bonus: 0.0,
            rune_cdr_t: 0.0,
            gold: 600,
            has_left_fountain: false,
            inventory: Vec::with_capacity(MAX_INVENTORY_SLOTS),
            item_cooldowns: [0.0; MAX_INVENTORY_SLOTS],
            stash: Vec::new(),
            courier_cd: 0.0,
            courier_flight_t: None,
            dead: false,
            respawn_timer: 0.0,
            mount_launch_t: MOUNT_LAUNCH_SECS,
            buyback_cd: 0.0,
            has_aegis_soulstone: false,
            aegis_revive_t: None,
            kills: 0,
            deaths: 0,
            assists: 0,
            creep_score: 0,
            damage_dealt: 0.0,
            healing_done: 0.0,
            attacked_enemy_hero_t: 0.0,
        }
    }

    pub fn fountain_center(team: Team) -> Vec2 {
        match team {
            Team::Alliance => Vec2::new(-FOUNTAIN_X, 0.0),
            Team::Horde => Vec2::new(FOUNTAIN_X, 0.0),
        }
    }

    pub fn fountain_pos_for(team: Team, slot_in_team: usize) -> Vec2 {
        let base = Self::fountain_center(team);
        let z_off = (slot_in_team as f32 - 2.0) * 2.5;
        Vec2::new(base.x, z_off)
    }

    pub fn class_def(&self) -> &'static MobaClassDef {
        moba_class_by_index(self.card.class_idx)
    }

    pub fn role(&self) -> HeroRole {
        self.card.role
    }

    /// Computes current aggregated hero stats at `team_level`.
    pub fn stats(&self, team_level: u32) -> HeroDerivedStats {
        let c = self.class_def();
        let lvl = team_level.saturating_sub(1) as f32;
        let mut max_hp =
            (c.base_hp + lvl * 52.0 + self.card_hp_flat) * (1.0 + self.card_hp_bonus_pct);
        let max_power = match c.power_type {
            PowerType::Mana => c.base_power + lvl * 28.0,
            PowerType::Rage | PowerType::Energy => 100.0,
        };
        let mut might = 1.0 + lvl * 0.04 + self.card_might_bonus;
        let mut armor = c.base_armor + lvl * 0.9 + self.card_armor_bonus;
        let mut speed_pct = self.card_speed_bonus_pct;
        let mut haste = self.card_haste_bonus;
        let mut crit = 0.06 + self.card_crit_bonus;
        let mut regen = 3.0 + lvl * 0.4 + self.card_regen_bonus;
        let mut area = 1.0;

        for item in &self.inventory {
            max_hp += item.bonus_hp;
            might += item.bonus_might;
            armor += item.bonus_armor;
            speed_pct += item.bonus_speed_pct;
            haste += item.bonus_haste;
            crit += item.bonus_crit;
            regen += item.bonus_regen;
            area += item.bonus_area;
        }

        if self.might_buff_t > 0.0 {
            might += self.might_buff_bonus;
        }
        if self.rune_cdr_t > 0.0 {
            haste += 0.40;
        }
        if self.haste_buff_t > 0.0 && self.haste_buff_mult > 1.0 {
            haste += (self.haste_buff_mult - 1.0) * 0.5;
        }

        let mut speed = c.base_speed * (1.0 + speed_pct);
        if self.mount_launch_t > 0.0 {
            speed *= MOUNT_LAUNCH_SPEED_MULT;
        } else if self.haste_buff_t > 0.0 {
            speed *= self.haste_buff_mult;
        }
        if self.slow_t > 0.0 && self.unstoppable_t <= 0.0 {
            speed *= self.slow_factor.clamp(0.25, 1.0);
        }

        HeroDerivedStats {
            max_hp,
            max_power,
            might,
            armor,
            speed,
            haste: haste.min(0.70),
            crit: crit.min(0.75),
            regen,
            area,
        }
    }

    pub fn is_incapacitated(&self) -> bool {
        self.dead || self.aegis_revive_t.is_some() || self.stun_t > 0.0 || self.fear_t > 0.0
    }

    pub fn can_move(&self) -> bool {
        !self.is_incapacitated() && self.root_t <= 0.0
    }

    pub fn can_cast_spells(&self) -> bool {
        !self.is_incapacitated() && self.silence_t <= 0.0
    }

    /// Applies a crowd-control effect through the Hero's Classic WoW 15s Diminishing Returns tracker.
    /// Returns the actual applied duration (`0.0` if Unstoppable or 4th-stack DR Immune!).
    pub fn apply_cc(&mut self, category: CcCategory, base_duration: f32) -> f32 {
        let unstoppable = self.unstoppable_t > 0.0;
        let effective = self
            .dr_tracker
            .apply_cc(category, base_duration, unstoppable);
        if effective > 0.0 {
            match category {
                CcCategory::Stun => self.stun_t = self.stun_t.max(effective),
                CcCategory::Root => self.root_t = self.root_t.max(effective),
                CcCategory::Fear => self.fear_t = self.fear_t.max(effective),
                CcCategory::Silence => self.silence_t = self.silence_t.max(effective),
            }
        }
        effective
    }

    /// Purges all active CC (Stun, Root, Fear, Silence, Slow) and grants `immunity_secs` of
    /// Unstoppable debuff immunity (*Insignia of the Alliance/Horde* / Divine Shield).
    pub fn purge_and_grant_unstoppable(&mut self, immunity_secs: f32) {
        self.stun_t = 0.0;
        self.root_t = 0.0;
        self.fear_t = 0.0;
        self.silence_t = 0.0;
        self.slow_t = 0.0;
        self.slow_factor = 1.0;
        self.unstoppable_t = self.unstoppable_t.max(immunity_secs);
    }

    /// LoL ARAM Fountain Lock rule: direct shopping into inventory is only permitted while dead
    /// OR while standing on the Fountain before leaving spawn (`!has_left_fountain`).
    pub fn can_shop_directly(&self) -> bool {
        let at_fountain =
            self.pos.distance(Self::fountain_center(self.team)) <= FOUNTAIN_SHOP_RADIUS;
        self.dead || (!self.has_left_fountain && at_fountain)
    }

    /// Purchases `item_key` with gold:
    /// - If `can_shop_directly()` is true and inventory has room, equips directly (`BuyOutcome::EquippedDirectly`).
    /// - Otherwise places the item in `stash` (`BuyOutcome::PlacedInStash`) for Goblin Courier delivery!
    pub fn buy_item(
        &mut self,
        item_key: &str,
        team_level: u32,
    ) -> Result<BuyOutcome, &'static str> {
        let Some(item) = shop_item(item_key) else {
            return Err("Unknown shop item");
        };
        if self.gold < item.cost {
            return Err("Insufficient gold");
        }
        if self.inventory.len() + self.stash.len() >= MAX_INVENTORY_SLOTS {
            return Err("Inventory and stash are full");
        }
        self.gold -= item.cost;
        if self.can_shop_directly() && self.inventory.len() < MAX_INVENTORY_SLOTS {
            self.inventory.push(item);
            if !self.dead {
                self.hp = (self.hp + item.bonus_hp).min(self.stats(team_level).max_hp);
            }
            Ok(BuyOutcome::EquippedDirectly)
        } else {
            self.stash.push(item);
            Ok(BuyOutcome::PlacedInStash)
        }
    }

    /// Calls the Dota 2 / WoW Goblin Zephyrus Courier (`90.0s` cooldown) to deliver all items
    /// waiting in `stash` to the hero in lane!
    pub fn call_goblin_courier(&mut self) -> bool {
        if self.courier_cd > 0.0 || self.stash.is_empty() || self.dead {
            return false;
        }
        self.courier_cd = COURIER_COOLDOWN;
        self.courier_flight_t = Some(COURIER_FLIGHT_SECS);
        true
    }

    /// Delivers items from `stash` into `inventory` when the Goblin Courier arrives (or on respawn).
    pub fn deliver_stash(&mut self, team_level: u32) -> usize {
        let mut delivered = 0;
        while !self.stash.is_empty() && self.inventory.len() < MAX_INVENTORY_SLOTS {
            let item = self.stash.remove(0);
            self.inventory.push(item);
            self.hp += item.bonus_hp;
            delivered += 1;
        }
        let max_hp = self.stats(team_level).max_hp;
        if delivered > 0 && !self.dead {
            self.hp = (self.hp + max_hp * 0.10).min(max_hp);
        }
        self.courier_flight_t = None;
        delivered
    }

    /// Cost of Soulstone Buyback at `team_level`.
    pub fn buyback_cost(team_level: u32) -> u32 {
        150 + team_level * 45
    }

    /// Executes **Soulstone Buyback** while dead (`360.0s` cooldown), immediately respawning the
    /// hero at their team's Fountain with full HP/Power and Gryphon/Wyvern Fast-Launch!
    pub fn soulstone_buyback(&mut self, team_level: u32, slot_in_team: usize) -> bool {
        let cost = Self::buyback_cost(team_level);
        if !self.dead || self.buyback_cd > 0.0 || self.gold < cost {
            return false;
        }
        self.gold -= cost;
        self.buyback_cd = BUYBACK_COOLDOWN;
        self.respawn_at_fountain(team_level, slot_in_team);
        true
    }

    pub fn respawn_at_fountain(&mut self, team_level: u32, slot_in_team: usize) {
        self.dead = false;
        self.respawn_timer = 0.0;
        self.aegis_revive_t = None;
        self.pos = Self::fountain_pos_for(self.team, slot_in_team);
        self.has_left_fountain = false;
        self.mount_launch_t = MOUNT_LAUNCH_SECS;
        self.purge_and_grant_unstoppable(1.5);
        self.deliver_stash(team_level);
        let st = self.stats(team_level);
        self.hp = st.max_hp;
        self.power = match self.class_def().power_type {
            PowerType::Mana | PowerType::Energy => st.max_power,
            PowerType::Rage => 25.0,
        };
    }

    /// Picks card `choice_idx` (`0..3`) from the front of `pending_card_offers` without pausing the game.
    pub fn pick_milestone_card(&mut self, choice_idx: usize, team_level: u32) -> bool {
        if self.pending_card_offers.is_empty() || choice_idx >= 3 {
            return false;
        }
        let offer = self.pending_card_offers.remove(0);
        let chosen = &offer[choice_idx];
        match chosen.kind {
            MilestoneCardKind::AbilityMutator {
                slot,
                bonus_dmg_pct,
                cdr_pct,
                bonus_radius,
            } => {
                let idx = slot as usize;
                self.slot_mutators[idx].bonus_dmg_pct += bonus_dmg_pct;
                self.slot_mutators[idx].cdr_pct += cdr_pct;
                self.slot_mutators[idx].bonus_radius += bonus_radius;
            }
            MilestoneCardKind::HeroicUltimate { ult_index } => {
                self.chosen_ultimate = Some(ult_index.min(1));
            }
            MilestoneCardKind::StatBoost { stat, amount } => match stat {
                Stat::Might => self.card_might_bonus += amount,
                Stat::MaxHp => self.card_hp_bonus_pct += amount,
                Stat::Speed => self.card_speed_bonus_pct += amount,
                Stat::Haste => self.card_haste_bonus += amount,
                Stat::Armor => self.card_armor_bonus += amount,
                Stat::Crit => self.card_crit_bonus += amount,
                Stat::Regen => self.card_regen_bonus += amount,
                _ => self.card_might_bonus += amount,
            },
            MilestoneCardKind::LegendaryMutator {
                might_bonus,
                haste_bonus,
                hp_bonus,
            } => {
                self.card_might_bonus += might_bonus;
                self.card_haste_bonus += haste_bonus;
                self.card_hp_flat += hp_bonus;
            }
        }
        self.cards_picked += 1;
        let max_hp = self.stats(team_level).max_hp;
        if !self.dead {
            self.hp = (self.hp + max_hp * 0.08).min(max_hp);
        }
        true
    }

    /// Uses an active item in inventory slot `inv_idx` (`0..6`).
    /// Note that *Insignia of the Alliance/Horde* (`BkbCleanse`) can be activated even while stunned/feared/silenced!
    pub fn use_active_item(&mut self, inv_idx: usize, team_level: u32) -> Option<ActiveItemEffect> {
        if self.dead || inv_idx >= self.inventory.len() {
            return None;
        }
        if self.item_cooldowns[inv_idx] > 0.0 {
            return None;
        }
        let item = self.inventory[inv_idx];
        if !item.is_active() {
            return None;
        }
        let is_bkb = matches!(item.active, ActiveItemEffect::BkbCleanse { .. });
        if !is_bkb && self.is_incapacitated() {
            return None;
        }
        self.item_cooldowns[inv_idx] = item.active_cd;
        let st = self.stats(team_level);

        match item.active {
            ActiveItemEffect::BkbCleanse { immunity_secs } => {
                self.purge_and_grant_unstoppable(immunity_secs);
            }
            ActiveItemEffect::HealthstoneHeal { heal_amount } => {
                self.hp = (self.hp + heal_amount).min(st.max_hp);
            }
            ActiveItemEffect::AegisBarrier {
                shield_amount,
                duration,
            } => {
                self.shield = self.shield.max(shield_amount);
                self.shield_t = self.shield_t.max(duration);
            }
            ActiveItemEffect::RocketBootsDash {
                speed_mult,
                duration,
            } => {
                self.haste_buff_mult = self.haste_buff_mult.max(speed_mult);
                self.haste_buff_t = self.haste_buff_t.max(duration);
            }
            ActiveItemEffect::ZandalarianBurst {
                might_bonus,
                duration,
            } => {
                self.might_buff_bonus = self.might_buff_bonus.max(might_bonus);
                self.might_buff_t = self.might_buff_t.max(duration);
            }
            ActiveItemEffect::BlinkStrike { .. } | ActiveItemEffect::TidalCharmStun { .. } => {}
            ActiveItemEffect::None => return None,
        }
        Some(item.active)
    }

    pub fn ability_def(&self, slot: MobaAbilitySlot) -> Option<&'static MobaAbilityDef> {
        let c = self.class_def();
        match slot {
            MobaAbilitySlot::AutoPrimary => Some(&c.auto_primary),
            MobaAbilitySlot::Q => Some(&c.q_skill),
            MobaAbilitySlot::W => Some(&c.w_skill),
            MobaAbilitySlot::E => Some(&c.e_skill),
            MobaAbilitySlot::UltimateR => {
                let idx = self.chosen_ultimate.unwrap_or(0).min(1);
                Some(&c.ultimates[idx])
            }
            MobaAbilitySlot::MobilityF => Some(&c.mobility),
        }
    }

    pub fn slot_cd(&self, slot: MobaAbilitySlot) -> f32 {
        match slot {
            MobaAbilitySlot::AutoPrimary => self.auto_cd,
            MobaAbilitySlot::Q => self.q_cd,
            MobaAbilitySlot::W => self.w_cd,
            MobaAbilitySlot::E => self.e_cd,
            MobaAbilitySlot::UltimateR => self.r_cd,
            MobaAbilitySlot::MobilityF => self.f_cd,
        }
    }

    pub fn set_slot_cd(&mut self, slot: MobaAbilitySlot, cd: f32) {
        match slot {
            MobaAbilitySlot::AutoPrimary => self.auto_cd = cd,
            MobaAbilitySlot::Q => self.q_cd = cd,
            MobaAbilitySlot::W => self.w_cd = cd,
            MobaAbilitySlot::E => self.e_cd = cd,
            MobaAbilitySlot::UltimateR => self.r_cd = cd,
            MobaAbilitySlot::MobilityF => self.f_cd = cd,
        }
    }

    /// Applies incoming damage to this hero, respecting `unstoppable_t` / Divine Shield, Armor,
    /// Shield barriers, and triggering **Aegis Soulstone** in-place revival if fatal!
    /// Returns `(actual_damage_taken, died_for_real)`.
    pub fn take_damage(&mut self, raw_damage: f32, team_level: u32) -> (f32, bool) {
        if self.dead || self.aegis_revive_t.is_some() || raw_damage <= 0.0 {
            return (0.0, false);
        }
        let st = self.stats(team_level);
        let mut dmg = (raw_damage - st.armor).max(1.0);
        if self.shield_t > 0.0 && self.shield > 0.0 {
            let absorbed = dmg.min(self.shield);
            self.shield -= absorbed;
            dmg -= absorbed;
        }
        self.hp -= dmg;
        if self.hp <= 0.0 {
            self.hp = 0.0;
            if self.has_aegis_soulstone {
                // Aegis Soulstone triggers! Revives in-place after 3.0s at 100% HP & Mana!
                self.has_aegis_soulstone = false;
                self.aegis_revive_t = Some(AEGIS_REVIVE_SECS);
                return (dmg, false);
            }
            self.dead = true;
            self.deaths += 1;
            self.respawn_timer = 8.0 + (team_level as f32) * 1.4;
            return (dmg, true);
        }
        (dmg, false)
    }

    /// Ticks timers, regeneration, Goblin Courier flight, Aegis Soulstone revival, and respawn.
    pub fn tick(&mut self, dt: f32, team_level: u32, slot_in_team: usize) {
        self.dr_tracker.tick(dt);
        for cd in [
            &mut self.auto_cd,
            &mut self.q_cd,
            &mut self.w_cd,
            &mut self.e_cd,
            &mut self.r_cd,
            &mut self.f_cd,
            &mut self.courier_cd,
            &mut self.buyback_cd,
            &mut self.attacked_enemy_hero_t,
        ] {
            *cd = (*cd - dt).max(0.0);
        }
        for icd in &mut self.item_cooldowns {
            *icd = (*icd - dt).max(0.0);
        }
        for buff in [
            &mut self.shield_t,
            &mut self.stun_t,
            &mut self.root_t,
            &mut self.fear_t,
            &mut self.silence_t,
            &mut self.slow_t,
            &mut self.unstoppable_t,
            &mut self.stealth_t,
            &mut self.haste_buff_t,
            &mut self.might_buff_t,
            &mut self.rune_cdr_t,
            &mut self.mount_launch_t,
        ] {
            *buff = (*buff - dt).max(0.0);
        }

        // Check if hero has walked away from their Fountain (locks direct Fountain shopping!).
        if !self.dead
            && !self.has_left_fountain
            && self.pos.distance(Self::fountain_center(self.team)) > FOUNTAIN_SHOP_RADIUS
        {
            self.has_left_fountain = true;
        }

        // Goblin Courier flight delivery
        if let Some(flight) = self.courier_flight_t.as_mut() {
            *flight -= dt;
            if *flight <= 0.0 {
                self.deliver_stash(team_level);
            }
        }

        // Aegis Soulstone in-place revival
        if let Some(rev) = self.aegis_revive_t.as_mut() {
            *rev -= dt;
            if *rev <= 0.0 {
                self.aegis_revive_t = None;
                let st = self.stats(team_level);
                self.hp = st.max_hp;
                self.power = st.max_power;
                self.purge_and_grant_unstoppable(2.0);
            }
            return;
        }

        // Respawn countdown
        if self.dead {
            self.respawn_timer -= dt;
            if self.respawn_timer <= 0.0 {
                self.respawn_at_fountain(team_level, slot_in_team);
            }
            return;
        }

        // Alive HP & Power regeneration (plus Fountain rapid regeneration when on spawn pad)
        let st = self.stats(team_level);
        let on_fountain =
            self.pos.distance(Self::fountain_center(self.team)) <= FOUNTAIN_SHOP_RADIUS;
        let hp_regen = if on_fountain {
            st.regen + st.max_hp * 0.08
        } else {
            st.regen
        };
        self.hp = (self.hp + hp_regen * dt).min(st.max_hp);

        match self.class_def().power_type {
            PowerType::Mana => {
                let mana_regen = if on_fountain { 45.0 } else { 14.0 };
                self.power = (self.power + mana_regen * dt).min(st.max_power);
            }
            PowerType::Energy => {
                self.power = (self.power + 20.0 * dt).min(st.max_power);
            }
            PowerType::Rage => {
                self.power = (self.power - 1.5 * dt).clamp(0.0, st.max_power);
            }
        }
    }
}
