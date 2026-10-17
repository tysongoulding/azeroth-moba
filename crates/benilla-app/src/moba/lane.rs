//! Single-Lane 3-Tier Fortifications, High-Ground Base Ramps, Stealth Brushes, Moonwells,
//! Tower Gold Plates, True Sight, Deterministic Tower Aggro, Backdoor Protection, and Glyph.

use bevy::prelude::*;

use super::data::Team;

pub const LANE_HALF_LEN: f32 = 120.0;
pub const LANE_HALF_WIDTH: f32 = 24.0;
pub const FOUNTAIN_X: f32 = 112.0;
pub const HIGH_GROUND_RAMP_START: f32 = 72.0;
pub const HIGH_GROUND_PLATEAU_X: f32 = 82.0;
pub const HIGH_GROUND_HEIGHT: f32 = 6.0;
pub const UPHILL_MISS_CHANCE: f32 = 0.25;

pub const PLATE_FALLOFF_TIME: f32 = 480.0;
pub const PLATE_GOLD_REWARD: u32 = 120;
pub const PLATE_XP_REWARD: f32 = 55.0;
pub const TRUE_SIGHT_RADIUS: f32 = 20.0;
pub const BACKDOOR_RADIUS: f32 = 22.0;
pub const BACKDOOR_DAMAGE_MULT: f32 = 0.40; // 60% damage reduction!

pub const GLYPH_DURATION: f32 = 5.0;
pub const GLYPH_COOLDOWN: f32 = 180.0;

pub const MOONWELL_RESTORE_PCT: f32 = 0.40;
pub const MOONWELL_COOLDOWN: f32 = 75.0;
pub const MOONWELL_INTERACT_RADIUS: f32 = 5.5;

/// Line-of-Sight Smoke Vent / Stealth Brush along the mid bridge.
#[derive(Clone, Copy, Debug)]
pub struct BrushZone {
    pub name: &'static str,
    pub center: Vec2,
    pub radius: f32,
}

pub static BRUSH_ZONES: [BrushZone; 4] = [
    BrushZone {
        name: "Alliance North Smoke Vent",
        center: Vec2::new(-32.0, -14.0),
        radius: 5.5,
    },
    BrushZone {
        name: "Alliance South Brush",
        center: Vec2::new(-32.0, 14.0),
        radius: 5.5,
    },
    BrushZone {
        name: "Horde North Smoke Vent",
        center: Vec2::new(32.0, -14.0),
        radius: 5.5,
    },
    BrushZone {
        name: "Horde South Brush",
        center: Vec2::new(32.0, 14.0),
        radius: 5.5,
    },
];

/// Returns `Some(brush_index)` if `pos` is inside one of the 4 mid-lane stealth brushes.
pub fn brush_at(pos: Vec2) -> Option<usize> {
    BRUSH_ZONES
        .iter()
        .position(|b| pos.distance(b.center) <= b.radius)
}

/// Elevation above the mid bridge floor at `pos` (`x` along corridor, `y` across corridor).
/// High-ground base ramps rise from `|x| = 72.0` to `|x| = 82.0`.
pub fn elevation_at(pos: Vec2) -> f32 {
    let ax = pos.x.abs();
    if ax >= HIGH_GROUND_PLATEAU_X {
        HIGH_GROUND_HEIGHT
    } else if ax > HIGH_GROUND_RAMP_START {
        let t = (ax - HIGH_GROUND_RAMP_START) / (HIGH_GROUND_PLATEAU_X - HIGH_GROUND_RAMP_START);
        t * HIGH_GROUND_HEIGHT
    } else {
        0.0
    }
}

/// Returns `true` when `attacker_pos` is on low ground attacking a target on high ground,
/// incurring a `25%` (`UPHILL_MISS_CHANCE`) ranged/melee attack miss chance.
pub fn is_uphill_attack(attacker_pos: Vec2, target_pos: Vec2) -> bool {
    elevation_at(target_pos) - elevation_at(attacker_pos) >= 2.5
}

/// High-ground fog-of-war rule: a target on high ground (`elevation >= 2.5`) is only visible if
/// at least one allied unit is also on high ground within `32.0` yards, or within `10.0` yards.
pub fn has_high_ground_vision(target_pos: Vec2, ally_positions: &[Vec2]) -> bool {
    if elevation_at(target_pos) < 2.5 {
        return true;
    }
    ally_positions.iter().any(|&ally| {
        (elevation_at(ally) >= 2.5 && ally.distance(target_pos) <= 32.0)
            || ally.distance(target_pos) <= 10.0
    })
}

/// Checks whether a target at `target_pos` (optionally stealthed) is visible to `observer_team`.
/// Accounts for:
/// 1. Tower True Sight (`20.0` yards from any standing structure of `observer_team`).
/// 2. Brush stealth (hidden inside a `BrushZone` unless an observer ally is in the same brush or has True Sight).
/// 3. High-Ground Base Ramp fog of war.
pub fn is_visible_to_team(
    observer_team: Team,
    target_pos: Vec2,
    target_stealthed: bool,
    structures: &[Structure],
    ally_positions: &[Vec2],
) -> bool {
    let in_true_sight = structures.iter().any(|s| {
        !s.destroyed && s.team == observer_team && s.pos.distance(target_pos) <= TRUE_SIGHT_RADIUS
    });
    if in_true_sight {
        return true;
    }
    if target_stealthed {
        return false;
    }
    if let Some(brush_idx) = brush_at(target_pos) {
        let observer_shares_brush = ally_positions
            .iter()
            .any(|&ally| brush_at(ally) == Some(brush_idx));
        if !observer_shares_brush {
            return false;
        }
    }
    has_high_ground_vision(target_pos, ally_positions)
}

/// The 4 fortification types per team along the mid lane.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum StructureKind {
    /// Tier 1 Outer Guard Tower (`5` Gold Plates before 8:00, True Sight `20.0` yd).
    OuterTower,
    /// Tier 2 Inner Keep Tower (Backdoor Protection, True Sight `20.0` yd).
    InnerKeep,
    /// Tier 2 Faction Barracks beside Inner Keep (destroying unlocks enemy Super Minions).
    Barracks,
    /// Tier 3 Faction General Core (*Highlord Bolvar Fordragon* vs *Overlord Saurfang*).
    FactionGeneral,
}

/// Candidate target evaluated by deterministic MOBA tower aggro priority.
#[derive(Clone, Copy, Debug)]
pub struct TowerTargetCandidate {
    pub id: u32,
    pub pos: Vec2,
    pub is_hero: bool,
    pub is_siege_minion: bool,
    pub is_minion: bool,
    /// True if this enemy hero recently dealt damage to an allied hero.
    pub attacking_allied_hero: bool,
}

/// Deterministic MOBA Tower Aggro Priority selector:
/// 1. Enemy hero attacking an allied hero (`is_hero && attacking_allied_hero`)
/// 2. Enemy Siege minion (`is_siege_minion`)
/// 3. Enemy Melee / Caster / Super minion (`is_minion`)
/// 4. Closest enemy hero (`is_hero`)
pub fn select_tower_target(
    tower_pos: Vec2,
    range: f32,
    current_target: Option<u32>,
    candidates: &[TowerTargetCandidate],
) -> Option<u32> {
    let in_range: Vec<&TowerTargetCandidate> = candidates
        .iter()
        .filter(|c| c.pos.distance(tower_pos) <= range)
        .collect();
    if in_range.is_empty() {
        return None;
    }

    // Priority 1: Enemy hero attacking an allied hero immediately draws tower aggro!
    if let Some(aggressor) = in_range
        .iter()
        .filter(|c| c.is_hero && c.attacking_allied_hero)
        .min_by(|a, b| {
            a.pos
                .distance(tower_pos)
                .total_cmp(&b.pos.distance(tower_pos))
        })
    {
        return Some(aggressor.id);
    }

    // If current target is a minion still in range, maintain lock.
    if let Some(cur) = current_target {
        if let Some(still) = in_range.iter().find(|c| c.id == cur && !c.is_hero) {
            return Some(still.id);
        }
    }

    let priority_rank = |c: &TowerTargetCandidate| -> u8 {
        if c.is_hero && c.attacking_allied_hero {
            0
        } else if c.is_siege_minion {
            1
        } else if c.is_minion {
            2
        } else {
            3
        }
    };

    in_range
        .into_iter()
        .min_by(|a, b| {
            priority_rank(a).cmp(&priority_rank(b)).then_with(|| {
                a.pos
                    .distance(tower_pos)
                    .total_cmp(&b.pos.distance(tower_pos))
            })
        })
        .map(|c| c.id)
}

/// A lane structure or Faction General Core.
#[derive(Clone, Debug)]
pub struct Structure {
    pub id: u32,
    pub name: &'static str,
    pub team: Team,
    pub kind: StructureKind,
    pub display_id: u32,
    pub scale: f32,
    pub pos: Vec2,
    pub hp: f32,
    pub max_hp: f32,
    pub armor: f32,
    pub attack_damage: f32,
    pub attack_range: f32,
    pub attack_cd: f32,
    pub attack_interval: f32,
    pub current_target: Option<u32>,
    /// Remaining Gold Plates (`0..=5`) on Tier 1 `OuterTower` before `8:00` (`480.0s`).
    pub plates_remaining: u8,
    /// Consecutive hits on the same hero (ramps tower shot damage by `+20%` per hit!).
    pub consecutive_hero_hits: u8,
    pub destroyed: bool,
    /// For Tier 3 Faction Generals during `18:00` Sudden Death march.
    pub marching_sudden_death: bool,
}

/// Outcome of applying damage to a `Structure`.
#[derive(Clone, Copy, Debug, Default)]
pub struct StructureDamageOutcome {
    pub actual_damage: f32,
    pub plates_broken: u8,
    pub gold_awarded: u32,
    pub just_destroyed: bool,
    pub backdoor_protected: bool,
}

impl Structure {
    /// Applies raw damage to this structure, respecting:
    /// - Glyph of Fortification (`glyph_invulnerable`)
    /// - Tier ordering prerequisites (`prereq_destroyed`)
    /// - Backdoor Protection (`60%` damage reduction on Tier 2/3 when no enemy minion is within `22.0` yd)
    /// - Tier 1 Gold Plates (`+120g` per 20% HP chunk removed before `480.0s`)
    pub fn apply_damage(
        &mut self,
        raw_damage: f32,
        elapsed: f32,
        enemy_minion_nearby: bool,
        glyph_invulnerable: bool,
        prereq_destroyed: bool,
    ) -> StructureDamageOutcome {
        if self.destroyed || glyph_invulnerable || !prereq_destroyed || raw_damage <= 0.0 {
            return StructureDamageOutcome::default();
        }

        // Expire remaining plates at 8:00 (480s).
        if elapsed >= PLATE_FALLOFF_TIME {
            self.plates_remaining = 0;
        }

        let needs_minions = matches!(
            self.kind,
            StructureKind::InnerKeep | StructureKind::Barracks | StructureKind::FactionGeneral
        ) && !self.marching_sudden_death;
        let backdoor_protected = needs_minions && !enemy_minion_nearby;

        let mitigated = (raw_damage - self.armor).max(1.0);
        let actual_damage = if backdoor_protected {
            mitigated * BACKDOOR_DAMAGE_MULT
        } else {
            mitigated
        };

        self.hp = (self.hp - actual_damage).max(0.0);

        let mut plates_broken = 0u8;
        if self.kind == StructureKind::OuterTower && elapsed < PLATE_FALLOFF_TIME {
            let hp_ratio = (self.hp / self.max_hp).clamp(0.0, 1.0);
            // At 100% -> 5 plates left; <80% -> 4 left; <60% -> 3 left; <40% -> 2 left; <20% -> 1 left; 0% -> 0 left.
            let should_remain = if self.hp <= 0.0 {
                0
            } else {
                ((hp_ratio * 5.0).ceil() as u8).clamp(1, 5)
            };
            if should_remain < self.plates_remaining {
                plates_broken = self.plates_remaining - should_remain;
                self.plates_remaining = should_remain;
            }
        }

        let just_destroyed = self.hp <= 0.0;
        if just_destroyed {
            self.destroyed = true;
        }

        let mut gold_awarded = u32::from(plates_broken) * PLATE_GOLD_REWARD;
        if just_destroyed {
            gold_awarded += match self.kind {
                StructureKind::OuterTower => 200,
                StructureKind::InnerKeep => 250,
                StructureKind::Barracks => 200,
                StructureKind::FactionGeneral => 500,
            };
        }

        StructureDamageOutcome {
            actual_damage,
            plates_broken,
            gold_awarded,
            just_destroyed,
            backdoor_protected,
        }
    }
}

/// Faction Moonwell (Alliance) or Bloodwell (Horde) behind Tier 1 and Tier 2 towers.
#[derive(Clone, Debug)]
pub struct Moonwell {
    pub id: u32,
    pub name: &'static str,
    pub team: Team,
    pub pos: Vec2,
    pub cooldown: f32,
}

impl Moonwell {
    pub fn is_ready(&self) -> bool {
        self.cooldown <= 0.0
    }

    /// Consumes the Moonwell if ready, starting its `75.0s` cooldown and returning `(hp_restored, power_restored)`.
    pub fn try_drink(&mut self, max_hp: f32, max_power: f32) -> Option<(f32, f32)> {
        if !self.is_ready() {
            return None;
        }
        self.cooldown = MOONWELL_COOLDOWN;
        Some((
            max_hp * MOONWELL_RESTORE_PCT,
            max_power * MOONWELL_RESTORE_PCT,
        ))
    }

    pub fn tick(&mut self, dt: f32) {
        self.cooldown = (self.cooldown - dt).max(0.0);
    }
}

/// Per-team Glyph of Fortification (`5.0s` structure invulnerability on a `180.0s` cooldown).
#[derive(Clone, Copy, Debug, Default)]
pub struct GlyphState {
    pub active_timer: f32,
    pub cooldown: f32,
}

impl GlyphState {
    pub fn is_invulnerable(&self) -> bool {
        self.active_timer > 0.0
    }

    pub fn can_activate(&self) -> bool {
        self.cooldown <= 0.0 && self.active_timer <= 0.0
    }

    pub fn activate(&mut self) -> bool {
        if !self.can_activate() {
            return false;
        }
        self.active_timer = GLYPH_DURATION;
        self.cooldown = GLYPH_COOLDOWN;
        true
    }

    pub fn tick(&mut self, dt: f32) {
        self.active_timer = (self.active_timer - dt).max(0.0);
        self.cooldown = (self.cooldown - dt).max(0.0);
    }
}

/// Builds the initial 8 fortifications (4 Alliance, 4 Horde) for a 5v5 Mid Brawl match.
pub fn build_initial_structures() -> Vec<Structure> {
    vec![
        // --- Alliance Fortifications (West / -X) ---
        Structure {
            id: 1,
            name: "Alliance Outer Guard Tower",
            team: Team::Alliance,
            kind: StructureKind::OuterTower,
            display_id: 8395,
            scale: 1.5,
            pos: Vec2::new(-48.0, 0.0),
            hp: 2200.0,
            max_hp: 2200.0,
            armor: 10.0,
            attack_damage: 115.0,
            attack_range: 20.0,
            attack_cd: 0.0,
            attack_interval: 1.1,
            current_target: None,
            plates_remaining: 5,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 2,
            name: "Alliance Inner Keep",
            team: Team::Alliance,
            kind: StructureKind::InnerKeep,
            display_id: 8395,
            scale: 1.75,
            pos: Vec2::new(-78.0, 0.0),
            hp: 2800.0,
            max_hp: 2800.0,
            armor: 14.0,
            attack_damage: 140.0,
            attack_range: 21.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 3,
            name: "Alliance Lion's Pride Barracks",
            team: Team::Alliance,
            kind: StructureKind::Barracks,
            display_id: 378,
            scale: 1.6,
            pos: Vec2::new(-84.0, 8.0),
            hp: 1900.0,
            max_hp: 1900.0,
            armor: 12.0,
            attack_damage: 0.0,
            attack_range: 0.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 4,
            name: "Highlord Bolvar Fordragon",
            team: Team::Alliance,
            kind: StructureKind::FactionGeneral,
            display_id: 5566,
            scale: 2.1,
            pos: Vec2::new(-104.0, 0.0),
            hp: 4200.0,
            max_hp: 4200.0,
            armor: 16.0,
            attack_damage: 165.0,
            attack_range: 16.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        // --- Horde Fortifications (East / +X) ---
        Structure {
            id: 5,
            name: "Horde Outer Watchtower",
            team: Team::Horde,
            kind: StructureKind::OuterTower,
            display_id: 15343,
            scale: 1.5,
            pos: Vec2::new(48.0, 0.0),
            hp: 2200.0,
            max_hp: 2200.0,
            armor: 10.0,
            attack_damage: 115.0,
            attack_range: 20.0,
            attack_cd: 0.0,
            attack_interval: 1.1,
            current_target: None,
            plates_remaining: 5,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 6,
            name: "Horde Inner Stronghold",
            team: Team::Horde,
            kind: StructureKind::InnerKeep,
            display_id: 15343,
            scale: 1.75,
            pos: Vec2::new(78.0, 0.0),
            hp: 2800.0,
            max_hp: 2800.0,
            armor: 14.0,
            attack_damage: 140.0,
            attack_range: 21.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 7,
            name: "Horde Warsong Barracks",
            team: Team::Horde,
            kind: StructureKind::Barracks,
            display_id: 1693,
            scale: 1.6,
            pos: Vec2::new(84.0, -8.0),
            hp: 1900.0,
            max_hp: 1900.0,
            armor: 12.0,
            attack_damage: 0.0,
            attack_range: 0.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
        Structure {
            id: 8,
            name: "Overlord Saurfang",
            team: Team::Horde,
            kind: StructureKind::FactionGeneral,
            display_id: 4556,
            scale: 2.1,
            pos: Vec2::new(104.0, 0.0),
            hp: 4200.0,
            max_hp: 4200.0,
            armor: 16.0,
            attack_damage: 165.0,
            attack_range: 16.0,
            attack_cd: 0.0,
            attack_interval: 1.0,
            current_target: None,
            plates_remaining: 0,
            consecutive_hero_hits: 0,
            destroyed: false,
            marching_sudden_death: false,
        },
    ]
}

/// Builds the 4 Faction Moonwells / Bloodwells (behind Tier 1 & Tier 2 for each team).
pub fn build_initial_moonwells() -> Vec<Moonwell> {
    vec![
        Moonwell {
            id: 1,
            name: "Alliance Outer Moonwell",
            team: Team::Alliance,
            pos: Vec2::new(-58.0, -6.0),
            cooldown: 0.0,
        },
        Moonwell {
            id: 2,
            name: "Alliance Keep Moonwell",
            team: Team::Alliance,
            pos: Vec2::new(-90.0, -6.0),
            cooldown: 0.0,
        },
        Moonwell {
            id: 3,
            name: "Horde Outer Bloodwell",
            team: Team::Horde,
            pos: Vec2::new(58.0, 6.0),
            cooldown: 0.0,
        },
        Moonwell {
            id: 4,
            name: "Horde Keep Bloodwell",
            team: Team::Horde,
            pos: Vec2::new(90.0, 6.0),
            cooldown: 0.0,
        },
    ]
}
