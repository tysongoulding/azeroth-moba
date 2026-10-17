//! Dual-Mode Hero Draft & Team Composition Balancing (`TavernBenchShuffle` vs `AllPick`)
//! across the 36-Hero Theme Roster (`MOBA_HEROES`).
//!
//! Every hero belongs to one of the 3 Core Combat Types (`HeroType::Brawler`, `HeroType::Damage`,
//! `HeroType::Support`), a `DamageProfile` (`Physical`, `Magic`, `Hybrid`), and 2–4 `ThemeTag`s.
//!
//! In `TavernBenchShuffle` (HoTS ARAM Shuffle Pick + LoL ARAM Shared Reroll Bench):
//! - Deals 3 distinct Theme-Hero Cards from `MOBA_HEROES` (`0..36`) to the player.
//! - Maintains a Shared Team Bench (`bench`) holding up to 6 unpicked/rerolled cards.
//! - Supports `reroll_hand()`, `swap_with_bench(bench_idx)`, and `cycle_hero()` (All Pick).
//! - Automatically balances every 5-man team (`auto_balance_team_roster`) so every team of 5
//!   fields at least **1 Brawler (Tank), 1 Support (Healer), and 3 DPS (including Ranged Damage)**.

use bevy::prelude::*;

use super::data::{
    moba_class_by_index, moba_hero_by_index, DamageProfile, HeroRole, HeroType, MobaHeroDef, Team,
    ThemeTag, MOBA_CLASSES, MOBA_HEROES,
};
use crate::survivors::Rng;

pub const MAX_BENCH_CARDS: usize = 6;
pub const HAND_SIZE: usize = 3;
pub const INITIAL_REROLLS: u8 = 2;

/// Hero selection draft mode toggled at the menu.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum DraftMode {
    #[default]
    TavernBenchShuffle,
    AllPick,
}

impl DraftMode {
    pub const fn toggle(self) -> Self {
        match self {
            Self::TavernBenchShuffle => Self::AllPick,
            Self::AllPick => Self::TavernBenchShuffle,
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::TavernBenchShuffle => "Draft: Tavern Bench",
            Self::AllPick => "Draft: All Pick",
        }
    }
}

/// A Theme-Hero Card dealt in the Tavern Bench Shuffle or chosen in All Pick (`0..36`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HeroCard {
    /// Index into `MOBA_HEROES` (`0..36`).
    pub hero_idx: usize,
    /// Index into `MOBA_CLASSES` (`0..9`) for base animation/class visual compatibility.
    pub class_idx: usize,
    pub spec_name: &'static str,
    pub role: HeroRole,
    pub race: u8,
    pub sex: u8,
}

impl HeroCard {
    pub fn hero_def(&self) -> &'static MobaHeroDef {
        moba_hero_by_index(self.hero_idx)
    }

    pub fn class_def(&self) -> &'static super::data::MobaClassDef {
        moba_class_by_index(self.class_idx)
    }

    pub fn hero_type(&self) -> HeroType {
        self.hero_def().hero_type
    }

    pub fn damage_profile(&self) -> DamageProfile {
        self.hero_def().damage_profile
    }

    pub fn themes(&self) -> &'static [ThemeTag] {
        self.hero_def().themes
    }

    pub fn display_title(&self) -> String {
        let h = self.hero_def();
        format!(
            "{} — {} [{}: {} | {}]",
            h.name,
            h.title,
            h.hero_type.short_label(),
            h.damage_profile.label(),
            h.themes_summary()
        )
    }
}

/// Builds a `HeroCard` directly from `MOBA_HEROES[hero_idx]`.
pub fn card_for_hero_index(hero_idx: usize) -> HeroCard {
    let idx = hero_idx.min(MOBA_HEROES.len() - 1);
    let h = &MOBA_HEROES[idx];
    HeroCard {
        hero_idx: idx,
        class_idx: h.class_idx,
        spec_name: h.title,
        role: h.role,
        race: h.default_race(),
        sex: h.default_sex(),
    }
}

/// Builds a `HeroCard` for `class_idx` (`0..9`) and `race`/`sex`, selecting a matching hero from `MOBA_HEROES`.
pub fn card_for_class(class_idx: usize, race: u8, sex: u8) -> HeroCard {
    let c_idx = class_idx.min(MOBA_CLASSES.len() - 1);
    let hero_idx = MOBA_HEROES
        .iter()
        .position(|h| h.class_idx == c_idx && h.default_race() == race)
        .or_else(|| MOBA_HEROES.iter().position(|h| h.class_idx == c_idx))
        .unwrap_or(0);
    let h = &MOBA_HEROES[hero_idx];
    HeroCard {
        hero_idx,
        class_idx: c_idx,
        spec_name: h.title,
        role: h.role,
        race,
        sex,
    }
}

/// Pick a random `HeroCard` from `MOBA_HEROES` matching `target_role`.
pub fn random_card_for_role(target_role: HeroRole, _team: Team, rng: &mut Rng) -> HeroCard {
    let matching: Vec<usize> = MOBA_HEROES
        .iter()
        .enumerate()
        .filter(|(_, h)| h.role == target_role)
        .map(|(i, _)| i)
        .collect();
    let hero_idx = if matching.is_empty() {
        0
    } else {
        matching[rng.index(matching.len())]
    };
    card_for_hero_index(hero_idx)
}

/// Pick a random `HeroCard` from `MOBA_HEROES` matching `target_type` (`Brawler`, `Damage`, `Support`).
pub fn random_card_for_type(target_type: HeroType, rng: &mut Rng) -> HeroCard {
    let matching: Vec<usize> = MOBA_HEROES
        .iter()
        .enumerate()
        .filter(|(_, h)| h.hero_type == target_type)
        .map(|(i, _)| i)
        .collect();
    let hero_idx = if matching.is_empty() {
        0
    } else {
        matching[rng.index(matching.len())]
    };
    card_for_hero_index(hero_idx)
}

fn pick_distinct_card_for_role(
    target_role: HeroRole,
    used_heroes: &mut Vec<usize>,
    rng: &mut Rng,
) -> HeroCard {
    let candidates: Vec<usize> = MOBA_HEROES
        .iter()
        .enumerate()
        .filter(|(idx, h)| h.role == target_role && !used_heroes.contains(idx))
        .map(|(idx, _)| idx)
        .collect();
    let chosen = if !candidates.is_empty() {
        candidates[rng.index(candidates.len())]
    } else {
        let fallback: Vec<usize> = MOBA_HEROES
            .iter()
            .enumerate()
            .filter(|(_, h)| h.role == target_role)
            .map(|(idx, _)| idx)
            .collect();
        fallback[rng.index(fallback.len())]
    };
    used_heroes.push(chosen);
    card_for_hero_index(chosen)
}

/// Automatically builds a balanced 5-man team roster (`[lead_card, bot1, bot2, bot3, bot4]`)
/// guaranteeing:
/// - At least **1 `HeroType::Brawler` (`Tank`)**, **1 `HeroType::Support` (`Healer`)**, and **3 DPS**
///   (including at least **1 `HeroType::Damage` (`RangedDps`)** and **1 Melee `Brawler`**).
pub fn auto_balance_team_roster(lead_card: HeroCard, rng: &mut Rng, _team: Team) -> [HeroCard; 5] {
    let needed_roles: [HeroRole; 4] = match lead_card.role {
        HeroRole::Tank => [
            HeroRole::Healer,
            HeroRole::MeleeDps,
            HeroRole::RangedDps,
            HeroRole::RangedDps,
        ],
        HeroRole::Healer => [
            HeroRole::Tank,
            HeroRole::MeleeDps,
            HeroRole::RangedDps,
            HeroRole::RangedDps,
        ],
        HeroRole::MeleeDps => [
            HeroRole::Tank,
            HeroRole::Healer,
            HeroRole::RangedDps,
            HeroRole::RangedDps,
        ],
        HeroRole::RangedDps => [
            HeroRole::Tank,
            HeroRole::Healer,
            HeroRole::MeleeDps,
            HeroRole::RangedDps,
        ],
    };

    let mut used = vec![lead_card.hero_idx];
    [
        lead_card,
        pick_distinct_card_for_role(needed_roles[0], &mut used, rng),
        pick_distinct_card_for_role(needed_roles[1], &mut used, rng),
        pick_distinct_card_for_role(needed_roles[2], &mut used, rng),
        pick_distinct_card_for_role(needed_roles[3], &mut used, rng),
    ]
}

/// Checks if a 5-man team satisfies the 3-Type (`Brawler` + `Damage` + `Support`) composition rule.
pub fn is_roster_type_balanced(roster: &[HeroCard; 5]) -> bool {
    let brawlers = roster
        .iter()
        .filter(|c| c.hero_type() == HeroType::Brawler)
        .count();
    let damage = roster
        .iter()
        .filter(|c| c.hero_type() == HeroType::Damage)
        .count();
    let supports = roster
        .iter()
        .filter(|c| c.hero_type() == HeroType::Support)
        .count();
    brawlers >= 1 && damage >= 1 && supports >= 1
}

/// Check if a 5-man team satisfies both `1 Tank + 1 Healer + 3 DPS` and `1 Brawler + 1 Damage + 1 Support`.
pub fn is_roster_role_balanced(roster: &[HeroCard; 5]) -> bool {
    let tanks = roster.iter().filter(|c| c.role == HeroRole::Tank).count();
    let healers = roster.iter().filter(|c| c.role == HeroRole::Healer).count();
    let dps = roster.iter().filter(|c| c.role.is_dps()).count();
    tanks >= 1 && healers >= 1 && dps >= 3 && is_roster_type_balanced(roster)
}

/// Resource tracking the current Tavern Bench Shuffle / All-Pick state across the 36-Hero roster.
#[derive(Resource, Clone, Debug)]
pub struct TavernDraftState {
    pub mode: DraftMode,
    pub player_team: Team,
    pub player_hand: Vec<HeroCard>,
    pub selected_hand_idx: usize,
    pub bench: Vec<HeroCard>,
    pub rerolls_left: u8,
    pub alliance_roster: [HeroCard; 5],
    pub horde_roster: [HeroCard; 5],
}

impl Default for TavernDraftState {
    fn default() -> Self {
        let mut rng = Rng::default();
        Self::new_shuffle(&mut rng, Team::Alliance)
    }
}

impl TavernDraftState {
    /// Deals a 3-card hand (`1 Brawler`, `1 Damage`, `1 Support`) and 4 bench cards from `MOBA_HEROES`.
    pub fn new_shuffle(rng: &mut Rng, player_team: Team) -> Self {
        let hand = vec![
            random_card_for_type(HeroType::Brawler, rng),
            random_card_for_type(HeroType::Damage, rng),
            random_card_for_type(HeroType::Support, rng),
        ];

        let mut used_indices: Vec<usize> = hand.iter().map(|c| c.hero_idx).collect();
        let mut bench = Vec::with_capacity(MAX_BENCH_CARDS);
        while bench.len() < 4 {
            let idx = rng.index(MOBA_HEROES.len());
            if !used_indices.contains(&idx) {
                used_indices.push(idx);
                bench.push(card_for_hero_index(idx));
            }
        }

        let player_card = hand[0];
        let enemy_lead = random_card_for_role(HeroRole::Tank, player_team.opposite(), rng);
        let (alliance_roster, horde_roster) = match player_team {
            Team::Alliance => (
                auto_balance_team_roster(player_card, rng, Team::Alliance),
                auto_balance_team_roster(enemy_lead, rng, Team::Horde),
            ),
            Team::Horde => (
                auto_balance_team_roster(enemy_lead, rng, Team::Alliance),
                auto_balance_team_roster(player_card, rng, Team::Horde),
            ),
        };

        Self {
            mode: DraftMode::TavernBenchShuffle,
            player_team,
            player_hand: hand,
            selected_hand_idx: 0,
            bench,
            rerolls_left: INITIAL_REROLLS,
            alliance_roster,
            horde_roster,
        }
    }

    pub fn active_player_card(&self) -> HeroCard {
        self.player_hand
            .get(self.selected_hand_idx)
            .copied()
            .unwrap_or(self.alliance_roster[0])
    }

    pub fn select_hand_card(&mut self, hand_idx: usize, rng: &mut Rng) -> bool {
        if hand_idx >= self.player_hand.len() {
            return false;
        }
        self.selected_hand_idx = hand_idx;
        self.rebuild_rosters(rng);
        true
    }

    /// Rerolls the player's 3-card hand, moving the previously selected card onto the shared
    /// Team Bench (`bench`, capped at `MAX_BENCH_CARDS`), and deals 3 fresh cards (`Brawler`, `Damage`, `Support`).
    pub fn reroll_hand(&mut self, rng: &mut Rng) -> bool {
        if self.rerolls_left == 0 {
            return false;
        }
        self.rerolls_left -= 1;
        let old_card = self.active_player_card();
        if self.bench.len() >= MAX_BENCH_CARDS {
            self.bench.remove(0);
        }
        self.bench.push(old_card);

        self.player_hand = vec![
            random_card_for_type(HeroType::Brawler, rng),
            random_card_for_type(HeroType::Damage, rng),
            random_card_for_type(HeroType::Support, rng),
        ];
        self.selected_hand_idx = 0;
        self.rebuild_rosters(rng);
        true
    }

    /// Swaps the player's currently selected card with `bench[bench_idx]`.
    pub fn swap_with_bench(&mut self, bench_idx: usize, rng: &mut Rng) -> bool {
        if bench_idx >= self.bench.len() || self.player_hand.is_empty() {
            return false;
        }
        let hand_idx = self.selected_hand_idx.min(self.player_hand.len() - 1);
        std::mem::swap(&mut self.player_hand[hand_idx], &mut self.bench[bench_idx]);
        self.rebuild_rosters(rng);
        true
    }

    /// Cycles the active player card through all 36 heroes in `MOBA_HEROES` (`0..36`).
    pub fn cycle_hero(&mut self, rng: &mut Rng) -> HeroCard {
        let cur_idx = self.active_player_card().hero_idx;
        let next_idx = (cur_idx + 1) % MOBA_HEROES.len();
        self.set_hero_index(next_idx, rng)
    }

    /// Directly selects `MOBA_HEROES[hero_idx]` as the player's active hero card.
    pub fn set_hero_index(&mut self, hero_idx: usize, rng: &mut Rng) -> HeroCard {
        let card = card_for_hero_index(hero_idx);
        if self.player_hand.is_empty() {
            self.player_hand.push(card);
            self.selected_hand_idx = 0;
        } else {
            let idx = self.selected_hand_idx.min(self.player_hand.len() - 1);
            self.player_hand[idx] = card;
        }
        self.rebuild_rosters(rng);
        card
    }

    /// Sets the player's card from direct class/race/sex selection on the menu tower.
    pub fn set_all_pick_class(&mut self, class_idx: usize, race: u8, sex: u8, rng: &mut Rng) {
        let team = if [2, 5, 6, 8].contains(&race) {
            Team::Horde
        } else {
            Team::Alliance
        };
        self.player_team = team;
        let card = card_for_class(class_idx, race, sex);
        if self.player_hand.is_empty() {
            self.player_hand.push(card);
            self.selected_hand_idx = 0;
        } else {
            let idx = self.selected_hand_idx.min(self.player_hand.len() - 1);
            self.player_hand[idx] = card;
        }
        self.rebuild_rosters(rng);
    }

    fn rebuild_rosters(&mut self, rng: &mut Rng) {
        let lead = self.active_player_card();
        let enemy_lead = random_card_for_role(HeroRole::Tank, self.player_team.opposite(), rng);
        match self.player_team {
            Team::Alliance => {
                self.alliance_roster = auto_balance_team_roster(lead, rng, Team::Alliance);
                self.horde_roster = auto_balance_team_roster(enemy_lead, rng, Team::Horde);
            }
            Team::Horde => {
                self.alliance_roster = auto_balance_team_roster(enemy_lead, rng, Team::Alliance);
                self.horde_roster = auto_balance_team_roster(lead, rng, Team::Horde);
            }
        }
    }
}
