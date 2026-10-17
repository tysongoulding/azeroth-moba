//! Unified Dual Progression:
//! - HoTS Shared 5-Man Team XP Curve (`Level 1..=20`) with Underdog Comeback XP Multiplier (`+25%` per level behind when down >= 2 levels)
//! - Non-Pausing 3-Card Milestone Drafts (`Levels 1, 4, 7, 10, 13, 16, 20`)
//! - Guaranteed Dual Heroic Ultimate (`R`) choice at Team Level 10

use bevy::prelude::*;

use super::data::{MobaAbilitySlot, MobaClassDef};
use crate::survivors::data::Stat;
use crate::survivors::Rng;

pub const MAX_TEAM_LEVEL: u32 = 20;
pub const MILESTONE_LEVELS: [u32; 7] = [1, 4, 7, 10, 13, 16, 20];

/// Rarity color tiers matching Vanilla WoW `ITEM_QUALITY_COLORS`.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum CardRarity {
    Common,
    Uncommon,
    Rare,
    Epic,
    Legendary,
}

impl CardRarity {
    pub const fn color(self) -> Color {
        match self {
            Self::Common => Color::srgb(1.0, 1.0, 1.0),
            Self::Uncommon => Color::srgb(0.118, 1.0, 0.0),
            Self::Rare => Color::srgb(0.0, 0.439, 0.867),
            Self::Epic => Color::srgb(0.639, 0.208, 0.933),
            Self::Legendary => Color::srgb(1.0, 0.502, 0.0),
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Uncommon => "Uncommon",
            Self::Rare => "Rare",
            Self::Epic => "Epic",
            Self::Legendary => "Legendary",
        }
    }
}

/// Payload of a non-pausing 3-card milestone upgrade card.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum MilestoneCardKind {
    /// Mutates one of the hero's class abilities (`AutoPrimary`, `Q`, `W`, `E`, `MobilityF`).
    AbilityMutator {
        slot: MobaAbilitySlot,
        bonus_dmg_pct: f32,
        cdr_pct: f32,
        bonus_radius: f32,
    },
    /// Unlocks one of the class's two Heroic Ultimates (`0` or `1`) on `R` at Team Level 10!
    HeroicUltimate { ult_index: usize },
    /// Raises a core hero stat (`Might`, `MaxHp`, `Speed`, `Haste`, `Armor`, `Crit`, `Regen`).
    StatBoost { stat: Stat, amount: f32 },
    /// High-impact Legendary mutator granted at late milestones or Raid Boss kills.
    LegendaryMutator {
        might_bonus: f32,
        haste_bonus: f32,
        hp_bonus: f32,
    },
}

/// A single card offered in the non-pausing 3-card milestone tray.
#[derive(Clone, Debug)]
pub struct MilestoneCard {
    pub title: String,
    pub subtitle: String,
    pub desc: String,
    pub spell_id: u32,
    pub rarity: CardRarity,
    pub kind: MilestoneCardKind,
}

/// Experience required to advance the Shared 5-Man Team Level from `level` to `level + 1`.
pub fn team_xp_to_next(level: u32) -> f32 {
    if level >= MAX_TEAM_LEVEL {
        return f32::INFINITY;
    }
    180.0 + (level.saturating_sub(1) as f32) * 85.0
}

/// Underdog Comeback XP Multiplier: when `my_level + 2 <= enemy_level`, grants `+25%` bonus XP
/// for every level the team is behind (`1.0 + 0.25 * (enemy_level - my_level)`).
pub fn comeback_xp_multiplier(my_level: u32, enemy_level: u32) -> f32 {
    if enemy_level >= my_level + 2 {
        1.0 + 0.25 * (enemy_level - my_level) as f32
    } else {
        1.0
    }
}

/// Returns `true` if `level` is one of the 7 HoTS/Survivors talent card milestone levels.
pub fn is_milestone_level(level: u32) -> bool {
    MILESTONE_LEVELS.contains(&level)
}

/// Shared 5-Man Team XP & Level state for one faction (`Alliance` or `Horde`).
#[derive(Clone, Debug)]
pub struct TeamProgression {
    pub level: u32,
    pub xp: f32,
    pub total_xp: f32,
    pub milestones_triggered: Vec<u32>,
}

impl Default for TeamProgression {
    fn default() -> Self {
        Self {
            level: 1,
            xp: 0.0,
            total_xp: 0.0,
            milestones_triggered: vec![1],
        }
    }
}

impl TeamProgression {
    /// Adds raw XP (applying the Underdog Comeback multiplier when behind `enemy_level` by >= 2 levels)
    /// and returns any newly reached milestone levels (`[4, 7, 10, 13, 16, 20]`).
    pub fn add_xp(&mut self, raw_xp: f32, enemy_level: u32) -> Vec<u32> {
        if raw_xp <= 0.0 || self.level >= MAX_TEAM_LEVEL {
            return Vec::new();
        }
        let mult = comeback_xp_multiplier(self.level, enemy_level);
        let gained = raw_xp * mult;
        self.xp += gained;
        self.total_xp += gained;

        let mut new_milestones = Vec::new();
        while self.level < MAX_TEAM_LEVEL {
            let need = team_xp_to_next(self.level);
            if self.xp < need {
                break;
            }
            self.xp -= need;
            self.level += 1;
            if is_milestone_level(self.level) && !self.milestones_triggered.contains(&self.level) {
                self.milestones_triggered.push(self.level);
                new_milestones.push(self.level);
            }
        }
        if self.level >= MAX_TEAM_LEVEL {
            self.xp = 0.0;
        }
        new_milestones
    }

    pub fn xp_fraction(&self) -> f32 {
        if self.level >= MAX_TEAM_LEVEL {
            1.0
        } else {
            (self.xp / team_xp_to_next(self.level)).clamp(0.0, 1.0)
        }
    }
}

/// Deals 3 Rarity-Colored Milestone Cards for `class` at `milestone_level`.
/// - At **Team Level 10**, slots 0 and 1 are GUARANTEED to be the hero's 2 Heroic Ultimates (`R`),
///   and slot 2 is a `CardRarity::Legendary` passive mutator!
pub fn deal_milestone_cards(
    class: &MobaClassDef,
    milestone_level: u32,
    rng: &mut Rng,
) -> [MilestoneCard; 3] {
    if milestone_level == 10 {
        let u0 = &class.ultimates[0];
        let u1 = &class.ultimates[1];
        return [
            MilestoneCard {
                title: u0.name.to_string(),
                subtitle: "Heroic Ultimate [R]".to_string(),
                desc: u0.desc.to_string(),
                spell_id: u0.spell_id,
                rarity: CardRarity::Legendary,
                kind: MilestoneCardKind::HeroicUltimate { ult_index: 0 },
            },
            MilestoneCard {
                title: u1.name.to_string(),
                subtitle: "Heroic Ultimate [R]".to_string(),
                desc: u1.desc.to_string(),
                spell_id: u1.spell_id,
                rarity: CardRarity::Legendary,
                kind: MilestoneCardKind::HeroicUltimate { ult_index: 1 },
            },
            MilestoneCard {
                title: "Blessing of the Aspect".to_string(),
                subtitle: "Legendary Mutator".to_string(),
                desc: "+22% Might, +12% Haste, +160 Max HP.".to_string(),
                spell_id: 20217,
                rarity: CardRarity::Legendary,
                kind: MilestoneCardKind::LegendaryMutator {
                    might_bonus: 0.22,
                    haste_bonus: 0.12,
                    hp_bonus: 160.0,
                },
            },
        ];
    }

    let base_rarity = match milestone_level {
        1 => CardRarity::Uncommon,
        4 | 7 => CardRarity::Rare,
        13 | 16 => CardRarity::Epic,
        _ => CardRarity::Legendary,
    };

    let scale = match base_rarity {
        CardRarity::Common => 1.0,
        CardRarity::Uncommon => 1.15,
        CardRarity::Rare => 1.35,
        CardRarity::Epic => 1.65,
        CardRarity::Legendary => 2.0,
    };

    // Card 0: Class Ability Mutator (Q or W)
    let (slot0, ab0) = if rng.next_u64() & 1 == 0 {
        (MobaAbilitySlot::Q, &class.q_skill)
    } else {
        (MobaAbilitySlot::W, &class.w_skill)
    };
    let dmg_pct = (0.18_f32 * scale * 100.0).round() / 100.0;
    let cdr_pct = (0.10_f32 * scale * 100.0).round() / 100.0;
    let card0 = MilestoneCard {
        title: format!("Empowered {}", ab0.name),
        subtitle: format!("[{}] Ability Mutator", slot0.hotkey_label()),
        desc: format!(
            "+{}% {} damage/healing and -{}% cooldown.",
            (dmg_pct * 100.0).round() as u32,
            ab0.name,
            (cdr_pct * 100.0).round() as u32
        ),
        spell_id: ab0.spell_id,
        rarity: base_rarity,
        kind: MilestoneCardKind::AbilityMutator {
            slot: slot0,
            bonus_dmg_pct: dmg_pct,
            cdr_pct,
            bonus_radius: 1.0,
        },
    };

    // Card 1: E or AutoPrimary Mutator
    let (slot1, ab1) = if milestone_level >= 7 {
        (MobaAbilitySlot::E, &class.e_skill)
    } else {
        (MobaAbilitySlot::AutoPrimary, &class.auto_primary)
    };
    let card1 = MilestoneCard {
        title: format!("Mastered {}", ab1.name),
        subtitle: format!("[{}] Tactical Mutator", slot1.hotkey_label()),
        desc: format!(
            "+{}% {} power, -{}% cooldown, +1.5 yd radius.",
            (dmg_pct * 110.0).round() as u32,
            ab1.name,
            (cdr_pct * 100.0).round() as u32
        ),
        spell_id: ab1.spell_id,
        rarity: base_rarity,
        kind: MilestoneCardKind::AbilityMutator {
            slot: slot1,
            bonus_dmg_pct: dmg_pct * 1.1,
            cdr_pct,
            bonus_radius: 1.5,
        },
    };

    // Card 2: Role-tuned Stat / Legendary Mutator
    let card2 = if milestone_level >= 16 {
        MilestoneCard {
            title: "High Warlord's Surge".to_string(),
            subtitle: "Legendary Mutator".to_string(),
            desc: "+25% Might, +15% Haste, +200 Max HP.".to_string(),
            spell_id: 19740,
            rarity: CardRarity::Legendary,
            kind: MilestoneCardKind::LegendaryMutator {
                might_bonus: 0.25,
                haste_bonus: 0.15,
                hp_bonus: 200.0,
            },
        }
    } else {
        let (stat, amount, name, spell_id, desc) = match rng.index(4) {
            0 => (
                Stat::Might,
                0.15 * scale,
                "Blessing of Might",
                19740,
                format!(
                    "+{}% all Damage and Healing.",
                    (15.0_f32 * scale).round() as u32
                ),
            ),
            1 => (
                Stat::MaxHp,
                0.18 * scale,
                "Power Word: Fortitude",
                1243,
                format!("+{}% Maximum Health.", (18.0_f32 * scale).round() as u32),
            ),
            2 => (
                Stat::Haste,
                0.10 * scale,
                "Arcane Brilliance",
                1459,
                format!("+{}% Ability Haste.", (10.0_f32 * scale).round() as u32),
            ),
            _ => (
                Stat::Speed,
                0.10 * scale,
                "Aspect of the Pack",
                13159,
                format!("+{}% Movement Speed.", (10.0_f32 * scale).round() as u32),
            ),
        };
        MilestoneCard {
            title: name.to_string(),
            subtitle: "Talent Blessing".to_string(),
            desc,
            spell_id,
            rarity: base_rarity,
            kind: MilestoneCardKind::StatBoost { stat, amount },
        }
    };

    [card0, card1, card2]
}

/// Deals 3 Legendary cards to every member of the team that slays the Center Raid Boss.
pub fn deal_legendary_boss_cards(class: &MobaClassDef) -> [MilestoneCard; 3] {
    [
        MilestoneCard {
            title: "Might of the Scourge".to_string(),
            subtitle: "Raid Boss Legendary".to_string(),
            desc: "+28% Might, +15% Haste, +220 Max HP.".to_string(),
            spell_id: 24275,
            rarity: CardRarity::Legendary,
            kind: MilestoneCardKind::LegendaryMutator {
                might_bonus: 0.28,
                haste_bonus: 0.15,
                hp_bonus: 220.0,
            },
        },
        MilestoneCard {
            title: format!("Overcharged {}", class.q_skill.name),
            subtitle: "Raid Boss Legendary".to_string(),
            desc: format!(
                "+40% {} damage/healing and -20% cooldown.",
                class.q_skill.name
            ),
            spell_id: class.q_skill.spell_id,
            rarity: CardRarity::Legendary,
            kind: MilestoneCardKind::AbilityMutator {
                slot: MobaAbilitySlot::Q,
                bonus_dmg_pct: 0.40,
                cdr_pct: 0.20,
                bonus_radius: 2.5,
            },
        },
        MilestoneCard {
            title: "Titan's Bulwark".to_string(),
            subtitle: "Raid Boss Legendary".to_string(),
            desc: "+18% Might, +20% Haste, +320 Max HP.".to_string(),
            spell_id: 20217,
            rarity: CardRarity::Legendary,
            kind: MilestoneCardKind::LegendaryMutator {
                might_bonus: 0.18,
                haste_bonus: 0.20,
                hp_bonus: 320.0,
            },
        },
    ]
}
