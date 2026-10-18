//! The run's pick screen, laid out like the 1.12 character create and select glue screens and built
//! from their own widgets and art ([`crate::glue`]):
//! - In **5v5 Mid Brawl (`GameMode::Moba5v5`)**:
//!   - **Left Frame (`MobaRosterRoot`)**: Interactive **36-Hero Theme Roster** with 3-Type Filter
//!     Tabs (`All (36)`, `Brawler (14)`, `Damage (12)`, `Support (10)`) and clickable hero cards
//!     for every single one of the 36 characters in [`crate::moba::data::MOBA_HEROES`].
//!   - **Right Frame (`MobaRightRoot`)**: **Hero Dossier & Ability Kit Inspector** showing the
//!     selected hero's `HeroType`, `DamageProfile`, `ThemeTag`s, `HP / Armor / Magic Resist`, and
//!     all 7 abilities (`Auto`, `Q`, `W`, `E`, `R1`, `R2`, `F`), plus the 4 Mid-Only Battlegrounds.
//! - In **Warcraft Survivors (`GameMode::SurvivorsSolo`)**:
//!   - **Left Frame (`SurvivorsTowerRoot`)**: Classic race, gender, and class tower.
//!   - **Right Frame (`SurvivorsZoneRoot`)**: Classic 10-battleground zone list.

use bevy::prelude::*;
use bevy::ui_render::ui_material::MaterialNode;

use super::hud::{looks_for, rgb, start_run, HudFont, MenuRoot};
use super::{data, Phase, Rng, Run, Selection};
use crate::char_select::race_name;
use crate::glue::art::{class_tc, race_tc, tc_rect, GlueArt, DIM, GOLD, NAME_EDGE};
use crate::glue::backdrop::{backdrop_border, tiled_bg_node};
use crate::glue::widgets::{
    abs, glue_button, icon_button, outlined_text, GlueBtnKind, GlueDisabled, GlueText, Hilight,
    HoverLabel, LockHighlight,
};
use crate::glue::GlueClicks;
use crate::moba::data::{HeroType, MOBA_HEROES, MOBA_MAPS};

/// The create screen's two race columns.
const ALLIANCE: [u8; 4] = [1, 3, 4, 7];
const HORDE: [u8; 4] = [2, 5, 6, 8];

/// `CharacterSelectCharacterFrame`'s border and fill tints.
const FRAME_BORDER: Color = Color::srgb(0.8, 0.8, 0.8);
const FRAME_FILL: Color = Color::srgba(0.09, 0.09, 0.09, 0.88);

/// The tower ends under the class grid and Randomize, where the create screen's appearance dials
/// would start: there is nothing to customise here.
const TOWER_H: f32 = 590.0;

/// The zone rows: the character-select row, compacted so every zone fits the 642-tall frame.
const ROW_TOP: f32 = 44.0;
const ROW_PITCH: f32 = 36.0;
const ROW_H: f32 = 34.0;
const ROW_W: f32 = 228.0;

/// Active `HeroType` filter tab in the 36-Hero MOBA Roster menu (`None` = All 36).
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct MobaMenuFilter(pub Option<HeroType>);

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum MenuAction {
    Race(u8),
    Sex(u8),
    Class(usize),
    Map(usize),
    SelectHero(usize),
    FilterHeroType(Option<HeroType>),
    Randomize,
    ToggleGameMode,
    ToggleDraftMode,
    CycleHero,
    RerollBench,
    SwapBench(usize),
    Enter,
}

/// On [`MenuRoot`] once the glue art is in and the screen is built.
#[derive(Component)]
pub(super) struct MenuBuilt;

#[derive(Component)]
pub(super) struct SurvivorsTowerRoot;

#[derive(Component)]
pub(super) struct SurvivorsZoneRoot;

#[derive(Component)]
pub(super) struct MobaRosterRoot;

#[derive(Component)]
pub(super) struct MobaRightRoot;

#[derive(Component)]
pub(super) struct HeroDossierText;

/// A race button's face, re-cropped to the picked gender.
#[derive(Component)]
pub(super) struct RaceFace(u8);

#[derive(Component)]
pub(super) struct PickName;

#[derive(Component)]
pub(super) struct ModeBannerText;

#[derive(Component)]
pub(super) struct BenchInfoText;

/// The saved pick, `benilla-config/survivors-pick.txt` (`localStorage` on the page):
/// `class race sex map`, restored at the menu so a reload keeps the last hero and zone.
fn pick_path() -> Option<std::path::PathBuf> {
    crate::local_state::home().map(|h| h.join("survivors-pick.txt"))
}

fn save_pick(sel: &Selection) {
    let (Some(class), Some(&(_, race, sex))) = (sel.class, sel.look.and_then(|i| sel.looks.get(i)))
    else {
        return;
    };
    if let Some(path) = pick_path() {
        let _ =
            crate::local_state::write_atomic(&path, &format!("{class} {race} {sex} {}", sel.map));
    }
}

/// Restore the saved pick once the creature data can resolve its looks; a stale or missing file
/// leaves the default.
pub(super) fn restore_pick(
    creatures: Option<Res<crate::entities::Creatures>>,
    mut selection: ResMut<Selection>,
    mut done: Local<bool>,
) {
    if *done {
        return;
    }
    let Some(creatures) = creatures.as_deref() else {
        return;
    };
    *done = true;
    let Some(text) = pick_path().and_then(|p| crate::local_state::read_to_string(&p).ok()) else {
        return;
    };
    let v: Vec<usize> = text
        .split_whitespace()
        .filter_map(|t| t.parse().ok())
        .collect();
    let [class, race, sex, map] = v[..] else {
        return;
    };
    if map < data::MAPS.len() {
        selection.map = map;
        selection.stage = map;
        selection.version += 1;
    }
    if class < data::CLASSES.len() {
        let all = all_looks(creatures);
        if let Some(l) = find(&all[class], Some(race as u8), Some(sex as u8)) {
            set(&mut selection, &all, class, l);
        }
    }
}

/// Every class's looks, `(display, race, sex)`, indexed like [`data::CLASSES`].
fn all_looks(creatures: &crate::entities::Creatures) -> Vec<Vec<(u32, u8, u8)>> {
    data::CLASSES
        .iter()
        .map(|c| looks_for(c, creatures))
        .collect()
}

fn find(looks: &[(u32, u8, u8)], race: Option<u8>, sex: Option<u8>) -> Option<usize> {
    looks
        .iter()
        .position(|&(_, r, s)| race.is_none_or(|x| x == r) && sex.is_none_or(|x| x == s))
}

fn set(sel: &mut Selection, all: &[Vec<(u32, u8, u8)>], class: usize, look: usize) {
    sel.class = Some(class);
    sel.looks = all[class].clone();
    sel.look = Some(look);
    sel.version += 1;
}

fn sync_selection_from_card(
    selection: &mut Selection,
    card: crate::moba::draft::HeroCard,
    all: Option<&[Vec<(u32, u8, u8)>]>,
) {
    let c = card.class_idx.min(data::CLASSES.len().saturating_sub(1));
    if let Some(all) = all {
        if let Some(looks) = all.get(c) {
            if let Some(l) = find(looks, Some(card.race), Some(card.sex))
                .or_else(|| find(looks, Some(card.race), None))
                .or_else(|| find(looks, None, None))
            {
                set(selection, all, c, l);
                return;
            }
        }
    }
    selection.class = Some(c);
    selection.version += 1;
}

/// Build the screen into [`MenuRoot`] once the glue art is in, so the glue art is there.
pub(super) fn build_menu(
    mut commands: Commands,
    roots: Query<Entity, (With<MenuRoot>, Without<MenuBuilt>)>,
    assets: Option<ResMut<benilla_assets::WorldAssets>>,
    mut art: ResMut<GlueArt>,
    mut images: ResMut<Assets<Image>>,
    mut add_mats: ResMut<Assets<crate::glue::add_material::AddUiMaterial>>,
    font: Res<HudFont>,
) {
    let Ok(root) = roots.single() else { return };
    let Some(mut assets) = assets else { return };
    art.ensure_loaded(&mut assets, &mut images, &mut add_mats);
    let f = &font.0;
    commands.entity(root).insert(MenuBuilt).with_children(|ui| {
        // 1. Survivors Classic Left Tower & Right Zone Frame
        tower(ui, &art, f);
        zone_frame(ui, &art, f);

        // 2. MOBA 36-Hero Roster Frame (Left) & Hero Dossier + Battleground Frame (Right)
        moba_roster_frame(ui, &art, f);
        moba_right_frame(ui, &art, f);

        // `CharacterCreateWoWLogo`, 256×128 at TOPLEFT (3,-7), after the tower so it draws over
        // the border art, as the create screen orders it.
        if let Some(logo) = &art.logo {
            ui.spawn((
                ImageNode::new(logo.clone()),
                abs(1.0, 3.0, 7.0, 180.0, 90.0),
            ));
        }

        // --- Top-Center Mode & Tavern Bench Bar (5v5 Mid Brawl vs Survivors Solo) ---
        ui.spawn(Node {
            position_type: PositionType::Absolute,
            top: Val::Px(10.0),
            width: Val::Percent(100.0),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            row_gap: Val::Px(5.0),
            ..default()
        })
        .with_children(|top| {
            outlined_text(
                top,
                Node::default(),
                (),
                ModeBannerText,
                GlueText {
                    text: "Mode: 5v5 Mid Brawl (36 Theme Heroes)",
                    size: 15.0,
                    color: GOLD,
                    wrap: false,
                },
                f,
                1.0,
            );
            top.spawn(Node {
                flex_direction: FlexDirection::Row,
                column_gap: Val::Px(6.0),
                ..default()
            })
            .with_children(|row| {
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::ToggleGameMode,
                    "Switch Mode",
                    112.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::ToggleDraftMode,
                    "Draft Mode",
                    108.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::CycleHero,
                    "Next Hero",
                    104.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::Randomize,
                    "Random Hero",
                    112.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::RerollBench,
                    "Reroll Hand",
                    108.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
                glue_button(
                    row,
                    &art,
                    f,
                    MenuAction::SwapBench(0),
                    "Swap Bench",
                    108.0,
                    26.0,
                    GlueBtnKind::Small,
                    1.0,
                );
            });
            outlined_text(
                top,
                Node::default(),
                (),
                BenchInfoText,
                GlueText {
                    text: "",
                    size: 11.5,
                    color: Color::WHITE,
                    wrap: false,
                },
                f,
                1.0,
            );
        });

        // `CharSelectCharacterName`, GlueFontNormalHuge at BOTTOM (0,94).
        ui.spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(94.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|c| {
            outlined_text(
                c,
                Node::default(),
                (),
                PickName,
                GlueText {
                    text: "",
                    size: 20.0,
                    color: GOLD,
                    wrap: false,
                },
                f,
                1.0,
            );
        });
        // `CharSelectEnterWorldButton`, 200×60 at BOTTOM (0,24).
        ui.spawn(Node {
            position_type: PositionType::Absolute,
            bottom: Val::Px(24.0),
            width: Val::Percent(100.0),
            justify_content: JustifyContent::Center,
            ..default()
        })
        .with_children(|c| {
            glue_button(
                c,
                &art,
                f,
                MenuAction::Enter,
                "Enter World",
                200.0,
                56.0,
                GlueBtnKind::Normal,
                1.0,
            );
        });
    });
}

/// Interactive 36-Hero Roster Frame on the Left side of the Menu (`GameMode::Moba5v5`).
fn moba_roster_frame(ui: &mut ChildSpawnerCommands, art: &GlueArt, f: &Handle<Font>) {
    let px = Val::Px;
    ui.spawn((
        MobaRosterRoot,
        Node {
            position_type: PositionType::Absolute,
            left: px(12.0),
            top: px(72.0),
            width: px(332.0),
            height: px(616.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(10.0)),
            row_gap: px(5.0),
            ..default()
        },
    ))
    .with_children(|frame| {
        if let (Some(bg), Some(border)) = (&art.tooltip_bg, &art.name_border) {
            frame.spawn((
                tiled_bg_node(bg.clone(), NAME_EDGE, 1.0, FRAME_FILL),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(4.0),
                    right: px(4.0),
                    top: px(4.0),
                    bottom: px(4.0),
                    ..default()
                },
            ));
            backdrop_border(frame, border, NAME_EDGE, FRAME_BORDER);
        } else {
            frame.spawn((BackgroundColor(FRAME_FILL), crate::glue::widgets::overlay()));
        }

        // Header
        outlined_text(
            frame,
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                margin: UiRect::top(px(2.0)),
                ..default()
            },
            (),
            (),
            GlueText {
                text: "36 Theme Heroes — Select Character",
                size: 14.0,
                color: GOLD,
                wrap: false,
            },
            f,
            1.0,
        );

        // 4 Type Filter Tabs: All (36), Brawler (14), Damage (12), Support (10)
        frame
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(4.0),
                margin: UiRect::bottom(px(2.0)),
                ..default()
            })
            .with_children(|tabs| {
                for (filter, label) in [
                    (None, "All (36)"),
                    (Some(HeroType::Brawler), "Brawler (14)"),
                    (Some(HeroType::Damage), "Damage (12)"),
                    (Some(HeroType::Support), "Support (10)"),
                ] {
                    tabs.spawn((
                        MenuAction::FilterHeroType(filter),
                        Button,
                        LockHighlight::default(),
                        Node {
                            flex_grow: 1.0,
                            height: px(22.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border: UiRect::all(px(1.0)),
                            border_radius: BorderRadius::all(px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.16, 0.14, 0.10, 0.92)),
                        BorderColor::all(Color::srgb(0.45, 0.38, 0.22)),
                    ))
                    .with_children(|btn| {
                        outlined_text(
                            btn,
                            Node::default(),
                            (),
                            (),
                            GlueText {
                                text: label,
                                size: 10.5,
                                color: GOLD,
                                wrap: false,
                            },
                            f,
                            1.0,
                        );
                    });
                }
            });

        // 2-Column Grid of All 36 Heroes (`0..36`)
        frame
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::SpaceBetween,
                column_gap: px(4.0),
                row_gap: px(3.0),
                ..default()
            })
            .with_children(|grid| {
                for h in &MOBA_HEROES {
                    let type_badge = h.hero_type.short_label();
                    let dmg_short = match h.damage_profile {
                        crate::moba::data::DamageProfile::Physical => "Phys",
                        crate::moba::data::DamageProfile::Magic => "Mag",
                        crate::moba::data::DamageProfile::Hybrid => "Hyb",
                    };
                    let top_line = format!("[{type_badge}] {}", h.name);
                    let sub_themes = h
                        .themes
                        .iter()
                        .take(3)
                        .map(|t| t.label())
                        .collect::<Vec<_>>()
                        .join("·");
                    let bottom_line = format!("{dmg_short} | {sub_themes}");

                    grid.spawn((
                        MenuAction::SelectHero(h.idx),
                        Button,
                        LockHighlight::default(),
                        Node {
                            width: px(152.0),
                            height: px(27.0),
                            flex_direction: FlexDirection::Column,
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::FlexStart,
                            padding: UiRect::axes(px(5.0), px(1.0)),
                            border: UiRect::all(px(1.0)),
                            border_radius: BorderRadius::all(px(3.0)),
                            ..default()
                        },
                        BackgroundColor(Color::srgba(0.13, 0.12, 0.15, 0.90)),
                        BorderColor::all(Color::srgb(0.35, 0.32, 0.25)),
                    ))
                    .with_children(|card_btn| {
                        outlined_text(
                            card_btn,
                            Node::default(),
                            (),
                            (),
                            GlueText {
                                text: &top_line,
                                size: 10.5,
                                color: rgb(h.color),
                                wrap: false,
                            },
                            f,
                            1.0,
                        );
                        outlined_text(
                            card_btn,
                            Node::default(),
                            (),
                            (),
                            GlueText {
                                text: &bottom_line,
                                size: 8.8,
                                color: Color::srgb(0.80, 0.80, 0.84),
                                wrap: false,
                            },
                            f,
                            1.0,
                        );
                    });
                }
            });
    });
}

/// Right Frame in `GameMode::Moba5v5`: Hero Kit & Theme Dossier + 4 Mid-Only Battlegrounds.
fn moba_right_frame(ui: &mut ChildSpawnerCommands, art: &GlueArt, f: &Handle<Font>) {
    let px = Val::Px;
    ui.spawn((
        MobaRightRoot,
        Node {
            position_type: PositionType::Absolute,
            right: px(12.0),
            top: px(72.0),
            width: px(340.0),
            height: px(616.0),
            flex_direction: FlexDirection::Column,
            padding: UiRect::all(px(12.0)),
            row_gap: px(8.0),
            ..default()
        },
    ))
    .with_children(|frame| {
        if let (Some(bg), Some(border)) = (&art.tooltip_bg, &art.name_border) {
            frame.spawn((
                tiled_bg_node(bg.clone(), NAME_EDGE, 1.0, FRAME_FILL),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(4.0),
                    right: px(4.0),
                    top: px(4.0),
                    bottom: px(4.0),
                    ..default()
                },
            ));
            backdrop_border(frame, border, NAME_EDGE, FRAME_BORDER);
        } else {
            frame.spawn((BackgroundColor(FRAME_FILL), crate::glue::widgets::overlay()));
        }

        // Top section: Selected Hero Dossier & Full 7-Ability Kit
        outlined_text(
            frame,
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            (),
            (),
            GlueText {
                text: "Hero Dossier — Themes & Abilities",
                size: 14.0,
                color: GOLD,
                wrap: false,
            },
            f,
            1.0,
        );

        frame
            .spawn((
                Node {
                    width: Val::Percent(100.0),
                    min_height: px(390.0),
                    padding: UiRect::all(px(8.0)),
                    border: UiRect::all(px(1.0)),
                    border_radius: BorderRadius::all(px(4.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.05, 0.07, 0.90)),
                BorderColor::all(Color::srgb(0.42, 0.35, 0.20)),
            ))
            .with_children(|box_node| {
                box_node.spawn((
                    HeroDossierText,
                    Text::new(""),
                    TextFont {
                        font: f.clone(),
                        font_size: 11.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.94, 0.92, 0.86)),
                ));
            });

        // Bottom section: 4 Single-Lane MOBA Battlegrounds
        outlined_text(
            frame,
            Node {
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                margin: UiRect::top(px(2.0)),
                ..default()
            },
            (),
            (),
            GlueText {
                text: "Choose 5v5 Mid Battleground",
                size: 13.5,
                color: GOLD,
                wrap: false,
            },
            f,
            1.0,
        );

        frame
            .spawn(Node {
                width: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                row_gap: px(4.0),
                ..default()
            })
            .with_children(|maps_col| {
                for (i, m) in MOBA_MAPS.iter().enumerate() {
                    maps_col
                        .spawn((
                            MenuAction::Map(i),
                            Button,
                            LockHighlight::default(),
                            Node {
                                width: Val::Percent(100.0),
                                height: px(31.0),
                                flex_direction: FlexDirection::Column,
                                justify_content: JustifyContent::Center,
                                padding: UiRect::axes(px(8.0), px(2.0)),
                                border: UiRect::all(px(1.0)),
                                border_radius: BorderRadius::all(px(3.0)),
                                ..default()
                            },
                            BackgroundColor(Color::srgba(0.14, 0.12, 0.10, 0.90)),
                            BorderColor::all(Color::srgb(0.38, 0.33, 0.22)),
                        ))
                        .with_children(|b| {
                            outlined_text(
                                b,
                                Node::default(),
                                (),
                                (),
                                GlueText {
                                    text: m.name,
                                    size: 12.0,
                                    color: GOLD,
                                    wrap: false,
                                },
                                f,
                                1.0,
                            );
                            outlined_text(
                                b,
                                Node::default(),
                                (),
                                (),
                                GlueText {
                                    text: m.zone,
                                    size: 9.5,
                                    color: Color::srgb(0.82, 0.82, 0.85),
                                    wrap: false,
                                },
                                f,
                                1.0,
                            );
                        });
                }
            });
    });
}

/// `CharacterCreateConfigurationFrame`, 206×600 at TOPLEFT (28,-74): the race columns under the
/// faction banners, the gender pair, the class grid, the class text and Randomize.
fn tower(ui: &mut ChildSpawnerCommands, art: &GlueArt, f: &Handle<Font>) {
    let px = Val::Px;
    ui.spawn((
        SurvivorsTowerRoot,
        Node {
            position_type: PositionType::Absolute,
            left: px(28.0),
            top: px(74.0),
            width: px(206.0),
            height: px(TOWER_H),
            ..default()
        },
    ))
    .with_children(|tower| {
        if let Some(bg) = &art.tower_bg {
            tower.spawn((
                ImageNode::new(bg.clone()),
                abs(1.0, -3.0, 0.0, 218.0, TOWER_H + 6.0),
            ));
        }
        // The create tower's top piece, then its foot piece cut to end where the class grid does.
        if let Some((border, size)) = &art.tower_border {
            for (top, height, tc) in [
                (0.0, 236.0, [0.0, 0.875, 0.0, 0.9375]),
                (
                    236.0,
                    TOWER_H - 236.0 + 6.0,
                    [
                        0.0,
                        0.875,
                        1.0 - (TOWER_H - 236.0 + 6.0) / 256.0 * 0.8203125 / (210.0 / 256.0),
                        1.0,
                    ],
                ),
            ] {
                tower.spawn((
                    ImageNode {
                        image: border.clone(),
                        rect: Some(tc_rect(*size, tc)),
                        ..default()
                    },
                    abs(1.0, -9.0, top, 224.0, height),
                ));
            }
        }
        if let Some(banners) = &art.banners {
            tower.spawn((
                ImageNode::new(banners.clone()),
                abs(1.0, -27.0, 60.0, 256.0, 259.0),
            ));
        }
        for (label, center) in [("Alliance", 51.0), ("Horde", 151.0)] {
            outlined_text(
                tower,
                Node {
                    justify_content: JustifyContent::Center,
                    ..abs(1.0, center - 60.0, 40.0, 120.0, 17.0)
                },
                (),
                (),
                GlueText {
                    text: label,
                    size: 15.0,
                    color: GOLD,
                    wrap: false,
                },
                f,
                1.0,
            );
        }

        for (faction, left) in [(ALLIANCE, 33.0), (HORDE, 127.0)] {
            tower
                .spawn(Node {
                    position_type: PositionType::Absolute,
                    left: px(left),
                    top: px(68.0),
                    flex_direction: FlexDirection::Column,
                    row_gap: px(5.0),
                    ..default()
                })
                .with_children(|col| {
                    for race in faction {
                        let face = art.races.as_ref().and_then(|(sheet, size)| {
                            race_tc(race, 0).map(|tc| (sheet.clone(), tc_rect(*size, tc)))
                        });
                        icon_button(
                            col,
                            f,
                            MenuAction::Race(race),
                            Some(RaceFace(race)),
                            face,
                            None::<()>,
                            race_name(race),
                            art,
                            1.0,
                        );
                    }
                });
        }

        tower
            .spawn(Node {
                position_type: PositionType::Absolute,
                left: px(53.0),
                top: px(303.0),
                flex_direction: FlexDirection::Row,
                column_gap: px(5.0),
                ..default()
            })
            .with_children(|g| {
                for (sex, label) in [(0u8, "Male"), (1u8, "Female")] {
                    let face = art.gender.as_ref().map(|(sheet, size)| {
                        let half = if sex == 0 {
                            [0.0, 0.5, 0.0, 1.0]
                        } else {
                            [0.5, 1.0, 0.0, 1.0]
                        };
                        (sheet.clone(), tc_rect(*size, half))
                    });
                    icon_button(
                        g,
                        f,
                        MenuAction::Sex(sex),
                        None::<()>,
                        face,
                        None::<()>,
                        label,
                        art,
                        1.0,
                    );
                }
            });

        tower
            .spawn(Node {
                position_type: PositionType::Absolute,
                left: px(103.0 - (3.0 * 48.0 + 2.0 * 4.0) / 2.0),
                width: px(3.0 * 48.0 + 2.0 * 4.0),
                top: px(368.0),
                flex_direction: FlexDirection::Row,
                flex_wrap: FlexWrap::Wrap,
                justify_content: JustifyContent::Center,
                column_gap: px(4.0),
                row_gap: px(4.0),
                ..default()
            })
            .with_children(|grid| {
                for (i, class) in data::CLASSES.iter().enumerate() {
                    let face = art.classes.as_ref().and_then(|(sheet, size)| {
                        class_tc(class.id).map(|tc| (sheet.clone(), tc_rect(*size, tc)))
                    });
                    icon_button(
                        grid,
                        f,
                        MenuAction::Class(i),
                        None::<()>,
                        face,
                        None::<()>,
                        class.name,
                        art,
                        1.0,
                    );
                }
            });

        tower
            .spawn(Node {
                position_type: PositionType::Absolute,
                left: px(30.0),
                top: px(TOWER_H - 46.0),
                ..default()
            })
            .with_children(|r| {
                glue_button(
                    r,
                    art,
                    f,
                    MenuAction::Randomize,
                    "Randomize",
                    146.0,
                    30.0,
                    GlueBtnKind::Small,
                    1.0,
                );
            });
    });
}

/// The zone list in `CharacterSelectCharacterFrame` (260×642 at TOPRIGHT (-5,-15)), one
/// character-row per zone, lowest level first.
fn zone_frame(ui: &mut ChildSpawnerCommands, art: &GlueArt, f: &Handle<Font>) {
    let px = Val::Px;
    let mut order: Vec<(usize, u32, u32)> = (0..data::MAPS.len())
        .map(|i| {
            let levels = data::roster(i).enemies.iter().map(|e| e.level);
            let lo = levels.clone().min().unwrap_or(1);
            let hi = levels.max().unwrap_or(lo);
            (i, lo, hi)
        })
        .collect();
    order.sort_by_key(|&(i, lo, _)| (lo, data::MAPS[i].name));

    ui.spawn((
        SurvivorsZoneRoot,
        Node {
            position_type: PositionType::Absolute,
            right: px(5.0),
            top: px(15.0),
            width: px(260.0),
            height: px(642.0),
            ..default()
        },
    ))
    .with_children(|frame| {
        if let (Some(bg), Some(border)) = (&art.tooltip_bg, &art.name_border) {
            frame.spawn((
                tiled_bg_node(bg.clone(), NAME_EDGE, 1.0, FRAME_FILL),
                Node {
                    position_type: PositionType::Absolute,
                    left: px(10.0),
                    right: px(5.0),
                    top: px(4.0),
                    bottom: px(9.0),
                    ..default()
                },
            ));
            backdrop_border(frame, border, NAME_EDGE, FRAME_BORDER);
        } else {
            frame.spawn((BackgroundColor(FRAME_FILL), crate::glue::widgets::overlay()));
        }
        outlined_text(
            frame,
            Node {
                position_type: PositionType::Absolute,
                top: px(12.0),
                width: Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            (),
            (),
            GlueText {
                text: "Choose a Zone",
                size: 18.0,
                color: DIM,
                wrap: false,
            },
            f,
            1.0,
        );
        for (row, &(i, lo, hi)) in order.iter().enumerate() {
            let map = &data::MAPS[i];
            frame
                .spawn((
                    MenuAction::Map(i),
                    Button,
                    LockHighlight::default(),
                    Node {
                        position_type: PositionType::Absolute,
                        left: px(24.0),
                        top: px(ROW_TOP + row as f32 * ROW_PITCH),
                        width: px(ROW_W),
                        height: px(ROW_H),
                        ..default()
                    },
                ))
                .with_children(|b| {
                    if let Some(hi) = &art.select_highlight {
                        b.spawn((
                            Hilight,
                            Visibility::Hidden,
                            MaterialNode(hi.clone()),
                            abs(1.0, -20.0, -6.0, ROW_W + 20.0, ROW_H + 12.0),
                        ));
                    }
                    outlined_text(
                        b,
                        abs(1.0, 0.0, 2.0, ROW_W, 16.0),
                        (),
                        (),
                        GlueText {
                            text: map.name,
                            size: 15.0,
                            color: GOLD,
                            wrap: false,
                        },
                        f,
                        1.0,
                    );
                    let levels = if lo == hi {
                        format!("Level {lo}")
                    } else {
                        format!("Level {lo}-{hi}")
                    };
                    outlined_text(
                        b,
                        abs(1.0, 0.0, 19.0, ROW_W, 14.0),
                        (),
                        (),
                        GlueText {
                            text: &format!("{levels}  {}", map.zone),
                            size: 12.0,
                            color: Color::WHITE,
                            wrap: false,
                        },
                        f,
                        1.0,
                    );
                });
        }
    });
}

#[allow(clippy::too_many_arguments)]
pub(super) fn menu_clicks(
    clicks: Res<GlueClicks>,
    actions: Query<(Entity, &MenuAction)>,
    phase: Res<State<Phase>>,
    creatures: Option<Res<crate::entities::Creatures>>,
    mut mode: ResMut<crate::moba::GameMode>,
    mut draft: ResMut<crate::moba::draft::TavernDraftState>,
    mut filter: ResMut<MobaMenuFilter>,
    mut moba: ResMut<crate::moba::MobaMatch>,
    mut selection: ResMut<Selection>,
    mut run: ResMut<Run>,
    mut rng: ResMut<Rng>,
    mut next: ResMut<NextState<Phase>>,
) {
    if *phase.get() != Phase::Menu {
        return;
    }
    let Some(action) = actions
        .iter()
        .find(|(e, _)| clicks.hit(*e))
        .map(|(_, a)| *a)
    else {
        return;
    };
    let all = creatures.as_deref().map(all_looks);
    let cur = selection
        .look
        .and_then(|i| selection.looks.get(i))
        .map(|&(_, r, s)| (r, s));
    let (race, sex) = (cur.map(|c| c.0), cur.map_or(0, |c| c.1));

    match action {
        MenuAction::ToggleGameMode => {
            *mode = match *mode {
                crate::moba::GameMode::SurvivorsSolo => {
                    *draft = crate::moba::draft::TavernDraftState::new_shuffle(
                        &mut rng,
                        crate::moba::data::Team::Alliance,
                    );
                    crate::moba::GameMode::Moba5v5
                }
                crate::moba::GameMode::Moba5v5 => crate::moba::GameMode::SurvivorsSolo,
            };
            selection.version += 1;
        }
        MenuAction::ToggleDraftMode => {
            draft.mode = match draft.mode {
                crate::moba::draft::DraftMode::TavernBenchShuffle => {
                    crate::moba::draft::DraftMode::AllPick
                }
                crate::moba::draft::DraftMode::AllPick => {
                    crate::moba::draft::DraftMode::TavernBenchShuffle
                }
            };
            selection.version += 1;
        }
        MenuAction::FilterHeroType(new_filter) => {
            filter.0 = new_filter;
            selection.version += 1;
        }
        MenuAction::SelectHero(hero_idx) => {
            let card = draft.set_hero_index(hero_idx, &mut rng);
            sync_selection_from_card(&mut selection, card, all.as_deref());
        }
        MenuAction::CycleHero => {
            let card = draft.cycle_hero(&mut rng);
            sync_selection_from_card(&mut selection, card, all.as_deref());
        }
        MenuAction::RerollBench => {
            if draft.reroll_hand(&mut rng) {
                let card = draft.active_player_card();
                sync_selection_from_card(&mut selection, card, all.as_deref());
            }
        }
        MenuAction::SwapBench(idx) => {
            if draft.swap_with_bench(idx, &mut rng) {
                let card = draft.active_player_card();
                sync_selection_from_card(&mut selection, card, all.as_deref());
            }
        }
        MenuAction::Map(i) => {
            if selection.map != i {
                selection.map = i;
                selection.version += 1;
            }
        }
        MenuAction::Enter => {
            let started = if *mode == crate::moba::GameMode::Moba5v5 {
                crate::moba::start_moba_match(&mut moba, &draft, &mut selection, &mut run, &mut rng)
            } else {
                start_run(&mut run, &mut selection)
            };
            if started {
                next.set(Phase::Playing);
            }
        }
        MenuAction::Randomize => {
            if *mode == crate::moba::GameMode::Moba5v5 {
                let random_hero = rng.index(MOBA_HEROES.len());
                let card = draft.set_hero_index(random_hero, &mut rng);
                sync_selection_from_card(&mut selection, card, all.as_deref());
            } else if let Some(all) = all.as_deref() {
                let picks: Vec<(usize, usize)> = all
                    .iter()
                    .enumerate()
                    .flat_map(|(c, looks)| (0..looks.len()).map(move |l| (c, l)))
                    .collect();
                if !picks.is_empty() {
                    let (c, l) = picks[rng.index(picks.len())];
                    set(&mut selection, all, c, l);
                }
            }
        }
        MenuAction::Class(c) => {
            let Some(all) = all.as_deref() else { return };
            let looks = &all[c];
            let l = find(looks, race, Some(sex))
                .or_else(|| find(looks, race, None))
                .or_else(|| find(looks, None, Some(sex)))
                .or_else(|| find(looks, None, None));
            if let Some(l) = l {
                set(&mut selection, all, c, l);
                if *mode == crate::moba::GameMode::Moba5v5 {
                    let picked_race = looks[l].1;
                    let picked_sex = looks[l].2;
                    draft.set_all_pick_class(c, picked_race, picked_sex, &mut rng);
                }
            }
        }
        MenuAction::Race(r) => {
            let Some(all) = all.as_deref() else { return };
            let classes = selection.class.into_iter().chain(0..all.len());
            for c in classes {
                let l = find(&all[c], Some(r), Some(sex)).or_else(|| find(&all[c], Some(r), None));
                if let Some(l) = l {
                    set(&mut selection, all, c, l);
                    if *mode == crate::moba::GameMode::Moba5v5 {
                        let picked_sex = all[c][l].2;
                        draft.set_all_pick_class(c, r, picked_sex, &mut rng);
                    }
                    break;
                }
            }
        }
        MenuAction::Sex(s) => {
            let Some(all) = all.as_deref() else { return };
            if let Some(c) = selection.class {
                if let Some(l) = find(&all[c], race, Some(s)) {
                    set(&mut selection, all, c, l);
                }
            }
        }
    }
    save_pick(&selection);
}

fn format_hero_dossier(h: &crate::moba::heroes::MobaHeroDef) -> String {
    format!(
        "#{:02} {} — {}\n\
         Type: {}  |  Damage: {}  |  Power: {}\n\
         Themes: [{}]\n\
         Base Stats: {:.0} HP  |  {:.0} Armor  |  {:.0} MR  |  {:.0} yd Range\n\n\
         [Auto] {} ({:.1}s):\n  {}\n\
         [Q] {} ({:.1}s, {:.0}p):\n  {}\n\
         [W] {} ({:.1}s, {:.0}p):\n  {}\n\
         [E] {} ({:.1}s, {:.0}p):\n  {}\n\
         [R1 — Lv10] {} ({:.0}s):\n  {}\n\
         [R2 — Lv10] {} ({:.0}s):\n  {}\n\
         [F — Mobility] {} ({:.1}s):\n  {}",
        h.idx + 1,
        h.name,
        h.title,
        h.hero_type.label(),
        h.damage_profile.label(),
        h.power_type.label(),
        h.themes_summary(),
        h.base_hp,
        h.base_armor,
        h.base_magic_resist,
        h.attack_range,
        h.auto_primary.name,
        h.auto_primary.cooldown,
        h.auto_primary.desc,
        h.q_skill.name,
        h.q_skill.cooldown,
        h.q_skill.power_cost,
        h.q_skill.desc,
        h.w_skill.name,
        h.w_skill.cooldown,
        h.w_skill.power_cost,
        h.w_skill.desc,
        h.e_skill.name,
        h.e_skill.cooldown,
        h.e_skill.power_cost,
        h.e_skill.desc,
        h.ultimates[0].name,
        h.ultimates[0].cooldown,
        h.ultimates[0].desc,
        h.ultimates[1].name,
        h.ultimates[1].cooldown,
        h.ultimates[1].desc,
        h.mobility.name,
        h.mobility.cooldown,
        h.mobility.desc,
    )
}

/// The screen's per-frame state: the locked highlights, hover labels, which races and classes
/// the pick allows, the gender on the race faces, the 36-Hero MOBA Roster & Dossier, and Enter World.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(super) fn refresh_menu_ui(
    selection: Res<Selection>,
    mode: Res<crate::moba::GameMode>,
    draft: Res<crate::moba::draft::TavernDraftState>,
    filter: Res<MobaMenuFilter>,
    creatures: Option<Res<crate::entities::Creatures>>,
    art: Res<GlueArt>,
    mut mode_roots: ParamSet<(
        Query<&mut Visibility, With<SurvivorsTowerRoot>>,
        Query<&mut Visibility, With<SurvivorsZoneRoot>>,
        Query<&mut Visibility, With<MobaRosterRoot>>,
        Query<&mut Visibility, With<MobaRightRoot>>,
    )>,
    mut buttons: Query<(
        &MenuAction,
        &Interaction,
        &Children,
        Option<&mut LockHighlight>,
        Option<&mut GlueDisabled>,
        Option<&mut BorderColor>,
        Option<&mut BackgroundColor>,
        &mut Node,
        &mut Visibility,
    )>,
    mut labels: Query<
        &mut Visibility,
        (
            With<HoverLabel>,
            Without<MenuAction>,
            Without<SurvivorsTowerRoot>,
            Without<SurvivorsZoneRoot>,
            Without<MobaRosterRoot>,
            Without<MobaRightRoot>,
        ),
    >,
    mut faces: Query<(&RaceFace, &mut ImageNode)>,
    mut texts: ParamSet<(
        Query<(&mut Text, &mut TextColor), With<PickName>>,
        Query<&mut Text, With<ModeBannerText>>,
        Query<&mut Text, With<BenchInfoText>>,
        Query<&mut Text, With<HeroDossierText>>,
    )>,
    mut cache: Local<(Option<u32>, bool, Vec<Vec<(u32, u8, u8)>>)>,
) {
    let have = creatures.is_some();
    if cache.0 != Some(selection.version) || cache.1 != have {
        *cache = (
            Some(selection.version),
            have,
            creatures.as_deref().map(all_looks).unwrap_or_default(),
        );
    }
    let all = &cache.2;
    let cur = selection
        .look
        .and_then(|i| selection.looks.get(i))
        .map(|&(_, r, s)| (r, s));

    let is_moba = *mode == crate::moba::GameMode::Moba5v5;
    let surv_vis = if is_moba {
        Visibility::Hidden
    } else {
        Visibility::Inherited
    };
    let moba_vis = if is_moba {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut v in &mut mode_roots.p0() {
        if *v != surv_vis {
            *v = surv_vis;
        }
    }
    for mut v in &mut mode_roots.p1() {
        if *v != surv_vis {
            *v = surv_vis;
        }
    }
    for mut v in &mut mode_roots.p2() {
        if *v != moba_vis {
            *v = moba_vis;
        }
    }
    for mut v in &mut mode_roots.p3() {
        if *v != moba_vis {
            *v = moba_vis;
        }
    }

    let active_hero_idx = draft.active_player_card().hero_idx;

    for (action, interaction, children, lock, disabled, border_color, bg_color, mut node, mut vis) in
        &mut buttons
    {
        let selected = match *action {
            MenuAction::Race(r) => cur.is_some_and(|c| c.0 == r),
            MenuAction::Sex(s) => cur.is_some_and(|c| c.1 == s),
            MenuAction::Class(i) => selection.class == Some(i),
            MenuAction::Map(i) => {
                if is_moba {
                    selection.map % MOBA_MAPS.len() == i
                } else {
                    selection.map == i
                }
            }
            MenuAction::SelectHero(idx) => is_moba && active_hero_idx == idx,
            MenuAction::FilterHeroType(f_opt) => is_moba && filter.0 == f_opt,
            MenuAction::ToggleGameMode
            | MenuAction::ToggleDraftMode
            | MenuAction::CycleHero
            | MenuAction::RerollBench
            | MenuAction::SwapBench(_)
            | MenuAction::Randomize
            | MenuAction::Enter => false,
        };
        if let Some(mut lock) = lock {
            if lock.0 != selected {
                lock.0 = selected;
            }
        }
        if let Some(mut bc) = border_color {
            if matches!(
                *action,
                MenuAction::SelectHero(_) | MenuAction::FilterHeroType(_) | MenuAction::Map(_)
            ) {
                let target_border = if selected {
                    GOLD
                } else if *interaction == Interaction::Hovered {
                    Color::srgb(0.75, 0.65, 0.40)
                } else {
                    Color::srgb(0.35, 0.32, 0.25)
                };
                *bc = BorderColor::all(target_border);
            }
        }
        if let Some(mut bg) = bg_color {
            if matches!(
                *action,
                MenuAction::SelectHero(_) | MenuAction::FilterHeroType(_) | MenuAction::Map(_)
            ) {
                let target_bg = if selected {
                    Color::srgba(0.28, 0.22, 0.08, 0.96)
                } else if *interaction == Interaction::Hovered {
                    Color::srgba(0.20, 0.18, 0.14, 0.94)
                } else {
                    Color::srgba(0.13, 0.12, 0.15, 0.90)
                };
                if bg.0 != target_bg {
                    bg.0 = target_bg;
                }
            }
        }
        if let Some(mut disabled) = disabled {
            let off = match *action {
                MenuAction::Enter => !is_moba && cur.is_none(),
                MenuAction::ToggleDraftMode | MenuAction::CycleHero => !is_moba,
                MenuAction::RerollBench => {
                    !is_moba
                        || draft.mode != crate::moba::draft::DraftMode::TavernBenchShuffle
                        || draft.rerolls_left == 0
                }
                MenuAction::SwapBench(idx) => !is_moba || idx >= draft.bench.len(),
                _ => false,
            };
            if disabled.0 != off {
                disabled.0 = off;
            }
        }
        match *action {
            MenuAction::SelectHero(idx) => {
                let matches_filter = filter
                    .0
                    .is_none_or(|ht| MOBA_HEROES[idx].hero_type == ht);
                let display = if matches_filter {
                    Display::Flex
                } else {
                    Display::None
                };
                let want_h = if filter.0.is_some() {
                    Val::Px(36.0)
                } else {
                    Val::Px(27.0)
                };
                if node.display != display {
                    node.display = display;
                }
                if node.height != want_h {
                    node.height = want_h;
                }
            }
            // As on the create screen, a class the race cannot be leaves the grid.
            MenuAction::Class(i) => {
                let ok = cur.is_none_or(|(r, _)| {
                    all.get(i).is_some_and(|l| find(l, Some(r), None).is_some())
                });
                let display = if ok { Display::Flex } else { Display::None };
                if node.display != display {
                    node.display = display;
                }
            }
            MenuAction::Race(r) => {
                let ok = all.is_empty() || all.iter().any(|l| find(l, Some(r), None).is_some());
                let want = if ok {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *vis != want {
                    *vis = want;
                }
            }
            _ => {}
        }
        let show = selected || *interaction != Interaction::None || art.races.is_none();
        for child in children {
            if let Ok(mut label) = labels.get_mut(*child) {
                let want = if show {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *label != want {
                    *label = want;
                }
            }
        }
    }

    if let (Some((_, size)), Some((_, sex))) = (&art.races, cur) {
        for (face, mut img) in &mut faces {
            if let Some(tc) = race_tc(face.0, sex) {
                let rect = Some(tc_rect(*size, tc));
                if img.rect != rect {
                    img.rect = rect;
                }
            }
        }
    }

    let class = selection.class_def();
    if let Ok((mut t, mut color)) = texts.p0().single_mut() {
        let (want_text, want_color) = if is_moba {
            let card = draft.active_player_card();
            let h = card.hero_def();
            (card.display_title(), rgb(h.color))
        } else {
            let txt = match (class, cur) {
                (Some(c), Some((r, _))) => format!("{} {}", race_name(r), c.name),
                _ => String::new(),
            };
            (txt, class.map_or(GOLD, |c| rgb(c.color)))
        };
        if t.0 != want_text {
            t.0 = want_text;
        }
        if color.0 != want_color {
            color.0 = want_color;
        }
    }

    if let Ok(mut t) = texts.p1().single_mut() {
        let want = if is_moba {
            let d = match draft.mode {
                crate::moba::draft::DraftMode::TavernBenchShuffle => "Tavern Bench Shuffle",
                crate::moba::draft::DraftMode::AllPick => "All Pick",
            };
            format!("Mode: 5v5 Mid Brawl ({d} — 36 Theme Heroes)")
        } else {
            "Mode: Warcraft Survivors Solo".to_string()
        };
        if t.0 != want {
            t.0 = want;
        }
    }

    if let Ok(mut t) = texts.p2().single_mut() {
        let want = if is_moba {
            let active_h = draft.active_player_card().hero_def();
            let bench_str = if draft.bench.is_empty() {
                "Empty".to_string()
            } else {
                draft
                    .bench
                    .iter()
                    .map(|b| {
                        let bh = b.hero_def();
                        format!("{} ({})", bh.name, bh.hero_type.short_label())
                    })
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            format!(
                "Selected: #{} {} [{} · {}]  |  Rerolls: {}  |  Bench: {bench_str}",
                active_h.idx + 1,
                active_h.name,
                active_h.hero_type.label(),
                active_h.damage_profile.label(),
                draft.rerolls_left
            )
        } else {
            String::new()
        };
        if t.0 != want {
            t.0 = want;
        }
    }

    if let Ok(mut t) = texts.p3().single_mut() {
        let want = if is_moba {
            let active_h = draft.active_player_card().hero_def();
            format_hero_dossier(active_h)
        } else {
            String::new()
        };
        if t.0 != want {
            t.0 = want;
        }
    }
}
