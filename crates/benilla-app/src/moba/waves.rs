//! Synchronized 30s Faction Minion Waves, Siege Engines, Super Minions, XP Wisps & Regen Globes.

use bevy::prelude::*;

use super::data::Team;

pub const WAVE_INTERVAL: f32 = 30.0;
pub const LAST_HIT_GOLD: u32 = 22;
pub const AMBIENT_MINION_GOLD: u32 = 8;
pub const AMBIENT_GOLD_RADIUS: f32 = 24.0;
pub const REGEN_GLOBE_RESTORE_PCT: f32 = 0.15;
pub const REGEN_GLOBE_AOE_RADIUS: f32 = 16.0;
pub const WISP_MAGNET_RADIUS: f32 = 12.0;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum MinionKind {
    Melee,
    LeadCaster,
    Ranged,
    SiegeEngine,
    SuperMinion,
    MercenaryBruiser,
    RaidBossAvatar,
}

impl MinionKind {
    pub const fn is_siege(self) -> bool {
        matches!(self, Self::SiegeEngine | Self::RaidBossAvatar)
    }

    pub const fn drops_regen_globe(self) -> bool {
        matches!(self, Self::LeadCaster | Self::MercenaryBruiser)
    }
}

/// A lane minion, mercenary bruiser, or pushing Raid Boss Avatar marching along the mid corridor.
#[derive(Clone, Debug)]
pub struct LaneMinion {
    pub id: u32,
    pub name: &'static str,
    pub team: Team,
    pub kind: MinionKind,
    pub display_id: u32,
    pub scale: f32,
    pub pos: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub speed: f32,
    pub damage: f32,
    pub attack_range: f32,
    pub attack_cd: f32,
    pub xp_value: f32,
    pub drops_regen_globe: bool,
    pub dead: bool,
}

/// Ground pickups dropped by slain minions:
/// - `XpWisp`: magnetizes to nearby heroes and feeds the **Shared 5-Man Team XP Bar**.
/// - `RegenGlobe`: dropped by the Lead Caster minion; restores `15%` Max HP & Mana to nearby heroes.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MobaPickupKind {
    XpWisp {
        xp: f32,
        for_team: Team,
    },
    RegenGlobe {
        /// Initially owned by the killing team for 4s, then neutral/contested.
        owner_team: Option<Team>,
    },
}

#[derive(Clone, Debug)]
pub struct MobaPickup {
    pub id: u32,
    pub pos: Vec2,
    pub kind: MobaPickupKind,
    pub age: f32,
    pub collected: bool,
}

/// Spawns one synchronized minion wave for `team` (`wave_number` is 1-indexed: `1, 2, 3, ...`).
/// - 3 Melee (*Stormwind Footman* vs *Orgrimmar Grunt*)
/// - 1 Lead Caster (*Dwarven Priest* vs *Darkspear Witch Doctor*) carrying a Regen Globe
/// - 3 Ranged (*Ironforge Rifleman* vs *Jungle Headhunter*)
/// - Every 3rd wave (`wave_number % 3 == 0`): +1 Siege Engine (*Dwarven Mortar* vs *Forsaken Catapult*)
/// - If `spawn_super_minion` is true (enemy Barracks destroyed): +1 Super Minion (*Mountain Giant* vs *Abomination*)
pub fn spawn_wave_for_team(
    team: Team,
    wave_number: u32,
    spawn_super_minion: bool,
    next_id: &mut u32,
) -> Vec<LaneMinion> {
    let mut out = Vec::with_capacity(9);
    let base_x = match team {
        Team::Alliance => -92.0,
        Team::Horde => 92.0,
    };
    let dir = team.dir_sign();
    let scale_mult = 1.0 + (wave_number.saturating_sub(1) as f32) * 0.04;

    // 1. 3 Melee Minions at the front of the formation
    let (melee_name, melee_display) = match team {
        Team::Alliance => ("Stormwind Footman", 1574),
        Team::Horde => ("Orgrimmar Grunt", 4556),
    };
    for i in 0..3 {
        let z_off = (i as f32 - 1.0) * 2.6;
        *next_id += 1;
        out.push(LaneMinion {
            id: *next_id,
            name: melee_name,
            team,
            kind: MinionKind::Melee,
            display_id: melee_display,
            scale: 1.0,
            pos: Vec2::new(base_x + dir * 4.0, z_off),
            hp: 290.0 * scale_mult,
            max_hp: 290.0 * scale_mult,
            speed: 5.8,
            damage: 18.0 * scale_mult,
            attack_range: 4.5,
            attack_cd: 0.0,
            xp_value: 28.0,
            drops_regen_globe: false,
            dead: false,
        });
    }

    // 2. 1 Lead Caster Minion in the center (drops the contested Regen Globe!)
    let (caster_name, caster_display) = match team {
        Team::Alliance => ("Dwarven Priest", 2137),
        Team::Horde => ("Darkspear Witch Doctor", 3816),
    };
    *next_id += 1;
    out.push(LaneMinion {
        id: *next_id,
        name: caster_name,
        team,
        kind: MinionKind::LeadCaster,
        display_id: caster_display,
        scale: 1.05,
        pos: Vec2::new(base_x + dir * 1.5, 0.0),
        hp: 230.0 * scale_mult,
        max_hp: 230.0 * scale_mult,
        speed: 5.8,
        damage: 22.0 * scale_mult,
        attack_range: 14.0,
        attack_cd: 0.0,
        xp_value: 34.0,
        drops_regen_globe: true,
        dead: false,
    });

    // 3. 3 Ranged Minions behind the Lead Caster
    let (ranged_name, ranged_display) = match team {
        Team::Alliance => ("Ironforge Rifleman", 1703),
        Team::Horde => ("Jungle Headhunter", 3810),
    };
    for i in 0..3 {
        let z_off = (i as f32 - 1.0) * 2.8;
        *next_id += 1;
        out.push(LaneMinion {
            id: *next_id,
            name: ranged_name,
            team,
            kind: MinionKind::Ranged,
            display_id: ranged_display,
            scale: 1.0,
            pos: Vec2::new(base_x - dir * 1.5, z_off),
            hp: 195.0 * scale_mult,
            max_hp: 195.0 * scale_mult,
            speed: 5.8,
            damage: 25.0 * scale_mult,
            attack_range: 16.0,
            attack_cd: 0.0,
            xp_value: 26.0,
            drops_regen_globe: false,
            dead: false,
        });
    }

    // 4. Every 3rd wave: 1 Siege Engine
    if wave_number.is_multiple_of(3) {
        let (siege_name, siege_display) = match team {
            Team::Alliance => ("Dwarven Siege Mortar", 4372),
            Team::Horde => ("Forsaken Plague Catapult", 10906),
        };
        *next_id += 1;
        out.push(LaneMinion {
            id: *next_id,
            name: siege_name,
            team,
            kind: MinionKind::SiegeEngine,
            display_id: siege_display,
            scale: 1.35,
            pos: Vec2::new(base_x - dir * 4.5, 0.0),
            hp: 580.0 * scale_mult,
            max_hp: 580.0 * scale_mult,
            speed: 5.4,
            damage: 52.0 * scale_mult,
            attack_range: 21.0,
            attack_cd: 0.0,
            xp_value: 60.0,
            drops_regen_globe: false,
            dead: false,
        });
    }

    // 5. Super Minion when the enemy team's Barracks is destroyed!
    if spawn_super_minion {
        let (super_name, super_display) = match team {
            Team::Alliance => ("Mountain Giant Vanguard", 8395),
            Team::Horde => ("Stitched Abomination", 1693),
        };
        *next_id += 1;
        out.push(LaneMinion {
            id: *next_id,
            name: super_name,
            team,
            kind: MinionKind::SuperMinion,
            display_id: super_display,
            scale: 1.55,
            pos: Vec2::new(base_x + dir * 5.5, 0.0),
            hp: 980.0 * scale_mult,
            max_hp: 980.0 * scale_mult,
            speed: 6.0,
            damage: 74.0 * scale_mult,
            attack_range: 5.5,
            attack_cd: 0.0,
            xp_value: 75.0,
            drops_regen_globe: true,
            dead: false,
        });
    }

    out
}

/// Spawns 2 pushing Mercenary Bruisers/Siegebreakers for `capturing_team` when a Merc Camp is captured.
pub fn spawn_mercenary_push(
    capturing_team: Team,
    camp_pos: Vec2,
    next_id: &mut u32,
) -> Vec<LaneMinion> {
    let (name, display) = match capturing_team {
        Team::Alliance => ("Defias Siegebreaker", 5035),
        Team::Horde => ("Boulderfist Ogre Bruiser", 14401),
    };
    let mut out = Vec::with_capacity(2);
    for i in 0..2 {
        *next_id += 1;
        let z_off = if i == 0 { -2.0 } else { 2.0 };
        out.push(LaneMinion {
            id: *next_id,
            name,
            team: capturing_team,
            kind: MinionKind::MercenaryBruiser,
            display_id: display,
            scale: 1.35,
            pos: Vec2::new(camp_pos.x, z_off),
            hp: 680.0,
            max_hp: 680.0,
            speed: 6.0,
            damage: 48.0,
            attack_range: 6.0,
            attack_cd: 0.0,
            xp_value: 50.0,
            drops_regen_globe: i == 0,
            dead: false,
        });
    }
    out
}

/// Spawns a pushing Raid Boss Avatar for `slaying_team` when the Center Raid Boss is defeated.
pub fn spawn_raid_boss_avatar(
    slaying_team: Team,
    boss_name: &'static str,
    display_id: u32,
    next_id: &mut u32,
) -> LaneMinion {
    *next_id += 1;
    LaneMinion {
        id: *next_id,
        name: boss_name,
        team: slaying_team,
        kind: MinionKind::RaidBossAvatar,
        display_id,
        scale: 1.85,
        pos: Vec2::ZERO,
        hp: 2400.0,
        max_hp: 2400.0,
        speed: 5.5,
        damage: 135.0,
        attack_range: 8.0,
        attack_cd: 0.0,
        xp_value: 150.0,
        drops_regen_globe: true,
        dead: false,
    }
}
