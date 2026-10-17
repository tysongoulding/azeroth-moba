//! 4-Game Neutral Event Clock:
//! 1. `90s` River Power Runes (*Berserking*, *Haste*, *Arcane*, *Shield*)
//! 2. `150s` Pushing Mercenary Camps (*Defias Siegebreakers* & *Boulderfist Ogre Bruisers*)
//! 3. `7:00` (`420s`) & `13:00` (`780s`) Center Raid Boss Pit (*Patchwerk* & *Ragnaros / Kel'Thuzad*)
//!    granting Aegis Soulstone in-place revival + Team Legendary Card Offer + Pushing Boss Avatar
//! 4. `18:00` (`1080s`) Sudden Death — Faction Generals march to center bridge (`x = 0`)

use bevy::prelude::*;

use super::combat::MobaHero;
use super::data::Team;

pub const RUNE_INTERVAL: f32 = 90.0;
pub const RUNE_DURATION: f32 = 30.0;
pub const MERC_SPAWN_TIME: f32 = 150.0;
pub const MERC_RESPAWN_SECS: f32 = 150.0;
pub const FIRST_BOSS_TIME: f32 = 420.0;
pub const SECOND_BOSS_TIME: f32 = 780.0;
pub const SUDDEN_DEATH_TIME: f32 = 1080.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RuneKind {
    Berserking,
    Haste,
    Arcane,
    Shield,
}

impl RuneKind {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Berserking => "Berserking Rune (+35% Damage)",
            Self::Haste => "Haste Rune (+50% Speed)",
            Self::Arcane => "Arcane Rune (-40% Cooldowns)",
            Self::Shield => "Shield Rune (35% Max HP Barrier)",
        }
    }

    pub const fn from_index(i: usize) -> Self {
        match i % 4 {
            0 => Self::Berserking,
            1 => Self::Haste,
            2 => Self::Arcane,
            _ => Self::Shield,
        }
    }
}

/// Active River Power Rune at North (`(0.0, -18.0)`) or South (`(0.0, 18.0)`) alcove.
#[derive(Clone, Debug)]
pub struct RiverRune {
    pub id: u32,
    pub pos: Vec2,
    pub kind: RuneKind,
    pub active: bool,
}

impl RiverRune {
    pub fn apply_to_hero(&self, hero: &mut MobaHero, team_level: u32) {
        if !self.active || hero.dead {
            return;
        }
        let st = hero.stats(team_level);
        match self.kind {
            RuneKind::Berserking => {
                hero.might_buff_bonus = hero.might_buff_bonus.max(0.35);
                hero.might_buff_t = hero.might_buff_t.max(RUNE_DURATION);
            }
            RuneKind::Haste => {
                hero.haste_buff_mult = hero.haste_buff_mult.max(1.50);
                hero.haste_buff_t = hero.haste_buff_t.max(RUNE_DURATION);
            }
            RuneKind::Arcane => {
                hero.rune_cdr_t = hero.rune_cdr_t.max(RUNE_DURATION);
                hero.power = st.max_power;
            }
            RuneKind::Shield => {
                hero.shield = hero.shield.max(st.max_hp * 0.35);
                hero.shield_t = hero.shield_t.max(RUNE_DURATION);
            }
        }
    }
}

/// Neutral Pushing Mercenary Camp (North: *Defias Siegebreakers*, South: *Boulderfist Ogre Bruisers*).
#[derive(Clone, Debug)]
pub struct MercCamp {
    pub id: u32,
    pub name: &'static str,
    pub display_id: u32,
    pub pos: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub damage: f32,
    pub active: bool,
    pub respawn_timer: f32,
    pub last_captured_by: Option<Team>,
}

impl MercCamp {
    pub fn damage_camp(&mut self, amount: f32, attacker_team: Team) -> bool {
        if !self.active || amount <= 0.0 {
            return false;
        }
        self.hp = (self.hp - amount).max(0.0);
        if self.hp <= 0.0 {
            self.active = false;
            self.respawn_timer = MERC_RESPAWN_SECS;
            self.last_captured_by = Some(attacker_team);
            return true;
        }
        false
    }
}

/// Center Raid Boss Pit (*Patchwerk* at 7:00, *Kel'Thuzad / Ragnaros* at 13:00).
#[derive(Clone, Debug)]
pub struct RaidBossPit {
    pub name: &'static str,
    pub display_id: u32,
    pub pos: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub damage: f32,
    pub attack_cd: f32,
    pub active: bool,
    pub tier: u8,
    pub first_spawned: bool,
    pub second_spawned: bool,
    pub slain_count: u32,
}

impl Default for RaidBossPit {
    fn default() -> Self {
        Self {
            name: "Patchwerk, Avatar of War",
            display_id: 16174,
            pos: Vec2::new(0.0, -4.0),
            hp: 3600.0,
            max_hp: 3600.0,
            damage: 95.0,
            attack_cd: 0.0,
            active: false,
            tier: 0,
            first_spawned: false,
            second_spawned: false,
            slain_count: 0,
        }
    }
}

impl RaidBossPit {
    pub fn spawn_tier(&mut self, tier: u8) {
        self.tier = tier;
        self.active = true;
        self.attack_cd = 0.0;
        if tier <= 1 {
            self.name = "Patchwerk, Avatar of War";
            self.display_id = 16174;
            self.max_hp = 3600.0;
            self.hp = 3600.0;
            self.damage = 95.0;
            self.first_spawned = true;
        } else {
            self.name = "Kel'Thuzad, Archlich of Naxxramas";
            self.display_id = 15945;
            self.max_hp = 5200.0;
            self.hp = 5200.0;
            self.damage = 130.0;
            self.second_spawned = true;
        }
    }
}

/// Aggregated 4-Game Neutral Event Clock state.
#[derive(Clone, Debug)]
pub struct ObjectiveClock {
    pub runes: [RiverRune; 2],
    pub rune_spawns: u32,
    pub merc_camps: [MercCamp; 2],
    pub raid_boss: RaidBossPit,
    pub sudden_death_active: bool,
}

impl Default for ObjectiveClock {
    fn default() -> Self {
        Self {
            runes: [
                RiverRune {
                    id: 1,
                    pos: Vec2::new(0.0, -18.0),
                    kind: RuneKind::Berserking,
                    active: false,
                },
                RiverRune {
                    id: 2,
                    pos: Vec2::new(0.0, 18.0),
                    kind: RuneKind::Haste,
                    active: false,
                },
            ],
            rune_spawns: 0,
            merc_camps: [
                MercCamp {
                    id: 1,
                    name: "Defias Siegebreaker Camp",
                    display_id: 5035,
                    pos: Vec2::new(-10.0, -20.0),
                    hp: 1100.0,
                    max_hp: 1100.0,
                    damage: 38.0,
                    active: false,
                    respawn_timer: MERC_SPAWN_TIME,
                    last_captured_by: None,
                },
                MercCamp {
                    id: 2,
                    name: "Boulderfist Ogre Camp",
                    display_id: 14401,
                    pos: Vec2::new(10.0, 20.0),
                    hp: 1100.0,
                    max_hp: 1100.0,
                    damage: 38.0,
                    active: false,
                    respawn_timer: MERC_SPAWN_TIME,
                    last_captured_by: None,
                },
            ],
            raid_boss: RaidBossPit::default(),
            sudden_death_active: false,
        }
    }
}

impl ObjectiveClock {
    /// Ticks the 4-Game Neutral Event Clock (`90s` Runes, `150s` Mercs, `420s`/`780s` Raid Boss,
    /// `1080s` Sudden Death) and returns any announcement banner messages triggered this frame.
    pub fn tick(&mut self, elapsed: f32, dt: f32) -> Vec<String> {
        let mut announcements = Vec::new();

        // 1. 90s River Power Runes
        let due_runes = (elapsed / RUNE_INTERVAL) as u32;
        if due_runes > self.rune_spawns {
            self.rune_spawns = due_runes;
            let k0 = RuneKind::from_index((due_runes as usize) * 2);
            let k1 = RuneKind::from_index((due_runes as usize) * 2 + 1);
            self.runes[0].kind = k0;
            self.runes[0].active = true;
            self.runes[1].kind = k1;
            self.runes[1].active = true;
            announcements.push(format!("River Power Runes spawned: {}!", k0.name()));
        }

        // 2. 150s Pushing Mercenary Camps
        for camp in &mut self.merc_camps {
            if !camp.active {
                camp.respawn_timer = (camp.respawn_timer - dt).max(0.0);
                if camp.respawn_timer <= 0.0 && elapsed >= MERC_SPAWN_TIME - 0.01 {
                    camp.active = true;
                    camp.hp = camp.max_hp;
                    announcements.push(format!("{} is ready to capture!", camp.name));
                }
            }
        }

        // 3. 7:00 (420s) & 13:00 (780s) Center Raid Boss Pit
        if elapsed >= FIRST_BOSS_TIME && !self.raid_boss.first_spawned {
            self.raid_boss.spawn_tier(1);
            announcements.push("RAID BOSS: Patchwerk has entered the Center Pit!".to_string());
        } else if elapsed >= SECOND_BOSS_TIME && !self.raid_boss.second_spawned {
            self.raid_boss.spawn_tier(2);
            announcements.push("RAID BOSS: Kel'Thuzad has risen in the Center Pit!".to_string());
        }

        // 4. 18:00 (1080s) Sudden Death — Clash of the Generals
        if elapsed >= SUDDEN_DEATH_TIME && !self.sudden_death_active {
            self.sudden_death_active = true;
            announcements.push(
                "SUDDEN DEATH! Highlord Bolvar & Overlord Saurfang march to the Center Bridge!"
                    .to_string(),
            );
        }

        announcements
    }

    /// Formats a concise HUD status string for the next upcoming or active objective.
    pub fn hud_status(&self, elapsed: f32) -> String {
        if self.sudden_death_active {
            return "SUDDEN DEATH: Generals Clash at Mid!".to_string();
        }
        if self.raid_boss.active {
            return format!(
                "RAID BOSS: {} ({:.0}% HP)",
                self.raid_boss.name,
                (self.raid_boss.hp / self.raid_boss.max_hp * 100.0).clamp(0.0, 100.0)
            );
        }
        if self.runes.iter().any(|r| r.active) {
            return "River Power Runes ACTIVE".to_string();
        }
        if self.merc_camps.iter().any(|c| c.active) {
            return "Mercenary Camps ACTIVE".to_string();
        }
        let next_rune = ((self.rune_spawns + 1) as f32 * RUNE_INTERVAL - elapsed).max(0.0);
        format!("Next River Rune in {:02.0}s", next_rune)
    }
}
