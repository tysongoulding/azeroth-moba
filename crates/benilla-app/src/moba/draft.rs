//! Dual-Mode Hero Draft & Team Composition Balancing (`TavernBenchShuffle` vs `AllPick`).
//!
//! In `TavernBenchShuffle` (HoTS ARAM Shuffle Pick + LoL ARAM Shared Reroll Bench):
//! - Deals 3 random Class/Spec Hero Cards to the player.
//! - Maintains a Shared Team Bench (`bench`) holding up to 6 unpicked/rerolled cards.
//! - Supports `reroll_hand()` (moves unpicked cards to the bench and deals 3 new cards) and
//!   `swap_with_bench(bench_idx)`.
//! - Automatically balances every 5-man team (`auto_balance_team_roster`) so every team of 5
//!   is guaranteed to field at least **1 Tank, 1 Healer, and 3 DPS**.

use bevy::prelude::*;

use super::data::{moba_class_by_index, HeroRole, Team, MOBA_CLASSES};
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

/// A Class/Spec Hero Card dealt in the Tavern Bench Shuffle or chosen in All Pick.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct HeroCard {
    /// Index into `MOBA_CLASSES` (`0..9`).
    pub class_idx: usize,
    pub spec_name: &'static str,
    pub role: HeroRole,
    pub race: u8,
    pub sex: u8,
}

impl HeroCard {
    pub fn class_def(&self) -> &'static super::data::MobaClassDef {
        moba_class_by_index(self.class_idx)
    }

    pub fn display_title(&self) -> String {
        format!(
            "{} {} ({})",
            self.spec_name,
            self.class_def().name,
            self.role.label()
        )
    }
}

/// Template pool of iconic Vanilla WoW Class/Spec archetypes across all 9 classes.
pub static TAVERN_POOL: &[(usize, &str, HeroRole)] = &[
    (0, "Protection", HeroRole::Tank),
    (0, "Arms", HeroRole::MeleeDps),
    (1, "Protection", HeroRole::Tank),
    (1, "Holy", HeroRole::Healer),
    (1, "Retribution", HeroRole::MeleeDps),
    (2, "Marksmanship", HeroRole::RangedDps),
    (2, "Beast Mastery", HeroRole::RangedDps),
    (3, "Subtlety", HeroRole::MeleeDps),
    (3, "Combat", HeroRole::MeleeDps),
    (4, "Discipline", HeroRole::Healer),
    (4, "Shadow", HeroRole::RangedDps),
    (5, "Restoration", HeroRole::Healer),
    (5, "Elemental", HeroRole::RangedDps),
    (6, "Frostfire", HeroRole::RangedDps),
    (7, "Destruction", HeroRole::RangedDps),
    (8, "Restoration", HeroRole::Healer),
    (8, "Feral Guardian", HeroRole::Tank),
    (8, "Balance", HeroRole::RangedDps),
];

fn make_card_from_template(tpl_idx: usize, team: Team, rng: &mut Rng) -> HeroCard {
    let &(class_idx, spec_name, role) = &TAVERN_POOL[tpl_idx % TAVERN_POOL.len()];
    let races = team.default_races();
    let race = races[rng.index(races.len())];
    let sex = (rng.next_u64() & 1) as u8;
    HeroCard {
        class_idx,
        spec_name,
        role,
        race,
        sex,
    }
}

pub fn card_for_class(class_idx: usize, race: u8, sex: u8) -> HeroCard {
    let idx = class_idx.min(MOBA_CLASSES.len() - 1);
    let def = &MOBA_CLASSES[idx];
    let spec_name = match def.id {
        1 => "Arms & Prot",
        2 => "Crusader",
        3 => "Marksmanship",
        4 => "Assassination",
        5 => "Holy Discipline",
        7 => "Storm Restoration",
        8 => "Archmage",
        9 => "Demonology",
        _ => "Archdruid",
    };
    HeroCard {
        class_idx: idx,
        spec_name,
        role: def.role,
        race,
        sex,
    }
}

/// Pick a random `HeroCard` matching `target_role` for `team`.
pub fn random_card_for_role(target_role: HeroRole, team: Team, rng: &mut Rng) -> HeroCard {
    let matching: Vec<usize> = TAVERN_POOL
        .iter()
        .enumerate()
        .filter(|(_, &(_, _, r))| r == target_role)
        .map(|(i, _)| i)
        .collect();
    let tpl_idx = if matching.is_empty() {
        0
    } else {
        matching[rng.index(matching.len())]
    };
    make_card_from_template(tpl_idx, team, rng)
}

/// Automatically builds a balanced 5-man team roster (`[lead_card, bot1, bot2, bot3, bot4]`)
/// guaranteeing at least **1 Tank, 1 Healer, and 3 DPS** (`MeleeDps` / `RangedDps`).
pub fn auto_balance_team_roster(lead_card: HeroCard, rng: &mut Rng, team: Team) -> [HeroCard; 5] {
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

    [
        lead_card,
        random_card_for_role(needed_roles[0], team, rng),
        random_card_for_role(needed_roles[1], team, rng),
        random_card_for_role(needed_roles[2], team, rng),
        random_card_for_role(needed_roles[3], team, rng),
    ]
}

/// Check if a 5-man team satisfies the `1 Tank + 1 Healer + 3 DPS` composition rule.
pub fn is_roster_role_balanced(roster: &[HeroCard; 5]) -> bool {
    let tanks = roster.iter().filter(|c| c.role == HeroRole::Tank).count();
    let healers = roster.iter().filter(|c| c.role == HeroRole::Healer).count();
    let dps = roster.iter().filter(|c| c.role.is_dps()).count();
    tanks >= 1 && healers >= 1 && dps >= 3
}

/// Resource tracking the current Tavern Bench Shuffle / All-Pick state.
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
    pub fn new_shuffle(rng: &mut Rng, player_team: Team) -> Self {
        let mut hand = Vec::with_capacity(HAND_SIZE);
        let mut used_classes = Vec::new();
        while hand.len() < HAND_SIZE {
            let tpl = rng.index(TAVERN_POOL.len());
            let card = make_card_from_template(tpl, player_team, rng);
            if !used_classes.contains(&card.class_idx) || hand.len() + 1 >= TAVERN_POOL.len() {
                used_classes.push(card.class_idx);
                hand.push(card);
            }
        }

        let mut bench = Vec::with_capacity(MAX_BENCH_CARDS);
        for _ in 0..4 {
            let tpl = rng.index(TAVERN_POOL.len());
            bench.push(make_card_from_template(tpl, player_team, rng));
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
    /// Team Bench (`bench`, capped at `MAX_BENCH_CARDS`), and deals 3 fresh cards.
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

        let mut new_hand = Vec::with_capacity(HAND_SIZE);
        let mut used = Vec::new();
        while new_hand.len() < HAND_SIZE {
            let tpl = rng.index(TAVERN_POOL.len());
            let card = make_card_from_template(tpl, self.player_team, rng);
            if !used.contains(&card.class_idx) {
                used.push(card.class_idx);
                new_hand.push(card);
            }
        }
        self.player_hand = new_hand;
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
