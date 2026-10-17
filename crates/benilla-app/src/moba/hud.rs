//! 5v5 Mid Brawl HUD Overlay:
//! - Top 5v5 Scoreboard Bar: Alliance vs. Horde Kills, Shared Team Levels (`Lv 1..20`) & XP bar,
//!   Tower/Keep/Barracks/General status pips, Match Clock, and 4-Game Event Objective Countdown
//! - Bottom Hybrid Action Bar: `[AUTO]`, `[Q]`, `[W]`, `[E]`, `[R]` (Ultimate), `[F]` (Mobility),
//!   `[1/2/3]` (Active Shop Items), `[B]` (Buy Next Item & Call Goblin Courier), `[G]` (Team Glyph / Buyback)
//! - Non-Pausing 3-Card Milestone Tray above the action bar when a milestone card pick is pending

use bevy::prelude::*;

use super::bots::recommended_item_order_for_role;
use super::data::{MobaAbilitySlot, Team};
use super::{GameMode, MobaMatch};
use crate::survivors::hud::HudFont;
use crate::survivors::Phase;

const GOLD: Color = Color::srgb(1.0, 0.82, 0.0);
const ALLIANCE_BLUE: Color = Color::srgb(0.25, 0.58, 1.0);
const HORDE_RED: Color = Color::srgb(0.95, 0.25, 0.22);
const PANEL_BG: Color = Color::srgba(0.05, 0.05, 0.08, 0.88);
const PANEL_EDGE: Color = Color::srgb(0.55, 0.45, 0.25);

#[derive(Component)]
pub(crate) struct MobaHudRoot;

#[derive(Component)]
pub(crate) struct MobaScoreboardText;

#[derive(Component)]
pub(crate) struct MobaObjectiveText;

#[derive(Component)]
pub(crate) struct MobaBannerText;

#[derive(Component)]
pub(crate) struct MobaHeroStatusText;

#[derive(Component)]
pub(crate) struct MobaActionBarText;

#[derive(Component)]
pub(crate) struct MobaCardTray;

#[derive(Component, Clone, Copy)]
pub(crate) struct MobaCardButton(pub usize);

#[derive(Component, Clone, Copy)]
pub(crate) enum MobaHudButton {
    CastSlot(MobaAbilitySlot),
    UseItem(usize),
    QuickBuyOrCourier,
    GlyphOrBuyback,
}

pub(crate) fn setup_moba_hud(
    mut commands: Commands,
    font: Option<Res<HudFont>>,
    asset_server: Res<AssetServer>,
) {
    let f = font
        .map(|hf| hf.0.clone())
        .unwrap_or_else(|| crate::char_select::wow_font(&asset_server));

    commands
        .spawn((
            MobaHudRoot,
            Node {
                position_type: PositionType::Absolute,
                width: Val::Percent(100.0),
                height: Val::Percent(100.0),
                flex_direction: FlexDirection::Column,
                justify_content: JustifyContent::SpaceBetween,
                align_items: AlignItems::Center,
                padding: UiRect::all(Val::Px(8.0)),
                display: Display::None,
                ..default()
            },
            GlobalZIndex(405),
            Visibility::Hidden,
            bevy::ui::FocusPolicy::Pass,
        ))
        .with_children(|root| {
            // --- Top 5v5 Scoreboard & Objective Bar ---
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    padding: UiRect::axes(Val::Px(18.0), Val::Px(6.0)),
                    row_gap: Val::Px(3.0),
                    border: UiRect::all(Val::Px(1.5)),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                },
                BackgroundColor(PANEL_BG),
                BorderColor::all(PANEL_EDGE),
            ))
            .with_children(|top| {
                top.spawn((
                    MobaScoreboardText,
                    Text::new("ALLIANCE Lv 1 (0)  |  00:00  |  HORDE Lv 1 (0)"),
                    TextFont {
                        font: f.clone(),
                        font_size: 16.0,
                        ..default()
                    },
                    TextColor(GOLD),
                ));
                top.spawn((
                    MobaObjectiveText,
                    Text::new("Next River Rune in 90s"),
                    TextFont {
                        font: f.clone(),
                        font_size: 13.0,
                        ..default()
                    },
                    TextColor(Color::srgb(0.82, 0.9, 1.0)),
                ));
            });

            // --- Center Announcement Banner ---
            root.spawn((
                MobaBannerText,
                Text::new(""),
                TextFont {
                    font: f.clone(),
                    font_size: 24.0,
                    ..default()
                },
                TextColor(GOLD),
            ));

            // --- Bottom Stack: Non-Pausing 3-Card Milestone Tray + Action Bar ---
            root.spawn((
                Node {
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    row_gap: Val::Px(8.0),
                    margin: UiRect::bottom(Val::Px(12.0)),
                    ..default()
                },
                bevy::ui::FocusPolicy::Pass,
            ))
            .with_children(|bottom| {
                // Non-Pausing 3-Card Milestone Tray
                bottom.spawn((
                    MobaCardTray,
                    Node {
                        flex_direction: FlexDirection::Row,
                        column_gap: Val::Px(10.0),
                        display: Display::None,
                        ..default()
                    },
                ));

                // Hero Vitals, Gold, Stash & Action Hotbar
                bottom
                    .spawn((
                        Node {
                            flex_direction: FlexDirection::Column,
                            align_items: AlignItems::Center,
                            padding: UiRect::axes(Val::Px(16.0), Val::Px(8.0)),
                            row_gap: Val::Px(6.0),
                            border: UiRect::all(Val::Px(1.5)),
                            border_radius: BorderRadius::all(Val::Px(6.0)),
                            ..default()
                        },
                        BackgroundColor(PANEL_BG),
                        BorderColor::all(PANEL_EDGE),
                    ))
                    .with_children(|bar| {
                        bar.spawn((
                            MobaHeroStatusText,
                            Text::new(""),
                            TextFont {
                                font: f.clone(),
                                font_size: 14.0,
                                ..default()
                            },
                            TextColor(Color::WHITE),
                        ));
                        bar.spawn((
                            MobaActionBarText,
                            Text::new(""),
                            TextFont {
                                font: f.clone(),
                                font_size: 13.0,
                                ..default()
                            },
                            TextColor(GOLD),
                        ));
                        // Interactive Quick-Buttons Row
                        bar.spawn(Node {
                            flex_direction: FlexDirection::Row,
                            column_gap: Val::Px(6.0),
                            ..default()
                        })
                        .with_children(|btns| {
                            for (label, action) in [
                                ("Q", MobaHudButton::CastSlot(MobaAbilitySlot::Q)),
                                ("W", MobaHudButton::CastSlot(MobaAbilitySlot::W)),
                                ("E", MobaHudButton::CastSlot(MobaAbilitySlot::E)),
                                (
                                    "R (Ult)",
                                    MobaHudButton::CastSlot(MobaAbilitySlot::UltimateR),
                                ),
                                (
                                    "F (Move)",
                                    MobaHudButton::CastSlot(MobaAbilitySlot::MobilityF),
                                ),
                                ("1 (Item)", MobaHudButton::UseItem(0)),
                                ("2 (Item)", MobaHudButton::UseItem(1)),
                                ("3 (Item)", MobaHudButton::UseItem(2)),
                                ("B (Shop/Courier)", MobaHudButton::QuickBuyOrCourier),
                                ("G (Glyph/Buyback)", MobaHudButton::GlyphOrBuyback),
                            ] {
                                btns.spawn((
                                    Button,
                                    action,
                                    Node {
                                        padding: UiRect::axes(Val::Px(8.0), Val::Px(4.0)),
                                        border: UiRect::all(Val::Px(1.0)),
                                        border_radius: BorderRadius::all(Val::Px(4.0)),
                                        ..default()
                                    },
                                    BackgroundColor(Color::srgba(0.16, 0.12, 0.08, 0.95)),
                                    BorderColor::all(PANEL_EDGE),
                                ))
                                .with_children(|b| {
                                    b.spawn((
                                        Text::new(label),
                                        TextFont {
                                            font: f.clone(),
                                            font_size: 12.0,
                                            ..default()
                                        },
                                        TextColor(Color::WHITE),
                                    ));
                                });
                            }
                        });
                    });
            });
        });
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn refresh_moba_hud(
    mut commands: Commands,
    phase: Res<State<Phase>>,
    mode: Res<GameMode>,
    moba: Res<MobaMatch>,
    font: Option<Res<HudFont>>,
    mut root_q: Query<(&mut Visibility, &mut Node), With<MobaHudRoot>>,
    mut texts: ParamSet<(
        Query<&mut Text, With<MobaScoreboardText>>,
        Query<&mut Text, With<MobaObjectiveText>>,
        Query<&mut Text, With<MobaBannerText>>,
        Query<&mut Text, With<MobaHeroStatusText>>,
        Query<&mut Text, With<MobaActionBarText>>,
    )>,
    mut tray_q: Query<(Entity, &mut Node), (With<MobaCardTray>, Without<MobaHudRoot>)>,
    mut last_card_count: Local<usize>,
) {
    let show = *mode == GameMode::Moba5v5 && *phase.get() == Phase::Playing;
    for (mut vis, mut node) in &mut root_q {
        let want_vis = if show {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
        let want_disp = if show { Display::Flex } else { Display::None };
        if *vis != want_vis {
            *vis = want_vis;
        }
        if node.display != want_disp {
            node.display = want_disp;
        }
    }
    if !show {
        return;
    }

    let secs = moba.elapsed as u32;
    let a_standing = moba
        .structures
        .iter()
        .filter(|s| s.team == Team::Alliance && !s.destroyed)
        .count();
    let h_standing = moba
        .structures
        .iter()
        .filter(|s| s.team == Team::Horde && !s.destroyed)
        .count();

    if let Ok(mut t) = texts.p0().single_mut() {
        **t = format!(
            "ALLIANCE Lv {} ({:.0}%) | Kills {} | Keeps {}/4   [{:02}:{:02}]   Keeps {}/4 | Kills {} | HORDE Lv {} ({:.0}%)",
            moba.alliance_prog.level,
            moba.alliance_prog.xp_fraction() * 100.0,
            moba.alliance_kills,
            a_standing,
            secs / 60,
            secs % 60,
            h_standing,
            moba.horde_kills,
            moba.horde_prog.level,
            moba.horde_prog.xp_fraction() * 100.0,
        );
    }

    if let Ok(mut t) = texts.p1().single_mut() {
        **t = format!(
            "{}   |   Map: {}",
            moba.objectives.hud_status(moba.elapsed),
            moba.map_def().name
        );
    }

    if let Ok(mut t) = texts.p2().single_mut() {
        **t = moba
            .banner
            .as_ref()
            .map(|(msg, _)| msg.clone())
            .unwrap_or_default();
    }

    let Some(hero) = moba.player_hero() else {
        return;
    };
    let team_lvl = moba.team_level(hero.team);
    let st = hero.stats(team_lvl);

    if let Ok(mut t) = texts.p3().single_mut() {
        let state_tag = if let Some(rev) = hero.aegis_revive_t {
            format!("AEGIS REVIVING ({:.1}s)", rev)
        } else if hero.dead {
            format!("DEAD (Respawn {:.1}s)", hero.respawn_timer)
        } else if hero.unstoppable_t > 0.0 {
            format!("UNSTOPPABLE ({:.1}s)", hero.unstoppable_t)
        } else if hero.anti_magic_shell_t > 0.0 && hero.anti_magic_shell > 0.0 {
            format!(
                "ANTI-MAGIC ({:.0} | {:.1}s)",
                hero.anti_magic_shell, hero.anti_magic_shell_t
            )
        } else if hero.stun_t > 0.0 {
            format!("STUNNED ({:.1}s)", hero.stun_t)
        } else if hero.fear_t > 0.0 {
            format!("FEARED ({:.1}s)", hero.fear_t)
        } else if hero.root_t > 0.0 {
            format!("ROOTED ({:.1}s)", hero.root_t)
        } else {
            "READY".to_string()
        };
        let aegis_str = if hero.has_aegis_soulstone {
            " [AEGIS SOULSTONE]"
        } else {
            ""
        };
        let eff_arm = hero.effective_armor(team_lvl);
        let eff_mr = hero.effective_magic_resist(team_lvl);
        **t = format!(
            "{} [{}] {}  |  HP {:.0}/{:.0}  |  Power {:.0}/{:.0}  |  Arm {:.0} / MR {:.0}  |  Gold {}g  |  K/D/A {}/{}/{}  |  Items {}/6 (Stash {})",
            hero.card.display_title(),
            state_tag,
            aegis_str,
            hero.hp,
            st.max_hp,
            hero.power,
            st.max_power,
            eff_arm,
            eff_mr,
            hero.gold,
            hero.kills,
            hero.deaths,
            hero.assists,
            hero.inventory.len(),
            hero.stash.len()
        );
    }

    if let Ok(mut t) = texts.p4().single_mut() {
        let h_def = hero.hero_def();
        let ult_name = hero
            .chosen_ultimate
            .map(|idx| h_def.ultimates[idx.min(1)].name)
            .unwrap_or("Locked (Lv 10)");
        let fmt_cd = |cd: f32| {
            if cd <= 0.05 {
                "RDY".to_string()
            } else {
                format!("{:.1}s", cd)
            }
        };
        **t = format!(
            "[AUTO] {}  |  [Q] {}: {}  |  [W] {}: {}  |  [E] {}: {}  |  [R] {}: {}  |  [F] {}: {}",
            h_def.auto_primary.name,
            h_def.q_skill.name,
            fmt_cd(hero.q_cd),
            h_def.w_skill.name,
            fmt_cd(hero.w_cd),
            h_def.e_skill.name,
            fmt_cd(hero.e_cd),
            ult_name,
            fmt_cd(hero.r_cd),
            h_def.mobility.name,
            fmt_cd(hero.f_cd),
        );
    }

    // Rebuild Non-Pausing 3-Card Milestone Tray when pending offers change
    let pending_len = hero.pending_card_offers.len();
    if let Ok((tray_entity, mut tray_node)) = tray_q.single_mut() {
        tray_node.display = if pending_len > 0 {
            Display::Flex
        } else {
            Display::None
        };
        if *last_card_count != pending_len {
            *last_card_count = pending_len;
            commands.entity(tray_entity).despawn_related::<Children>();
            if let Some(offer) = hero.pending_card_offers.first() {
                if let Some(font) = &font {
                    let f = &font.0;
                    commands.entity(tray_entity).with_children(|tray| {
                        for (idx, card) in offer.iter().enumerate() {
                            let rc = card.rarity.color();
                            tray.spawn((
                                Button,
                                MobaCardButton(idx),
                                Node {
                                    width: Val::Px(220.0),
                                    flex_direction: FlexDirection::Column,
                                    padding: UiRect::all(Val::Px(10.0)),
                                    row_gap: Val::Px(4.0),
                                    border: UiRect::all(Val::Px(2.0)),
                                    border_radius: BorderRadius::all(Val::Px(6.0)),
                                    ..default()
                                },
                                BackgroundColor(Color::srgba(0.06, 0.05, 0.09, 0.95)),
                                BorderColor::all(rc),
                            ))
                            .with_children(|c| {
                                c.spawn((
                                    Text::new(format!(
                                        "[Alt+{}] {} ({})",
                                        idx + 1,
                                        card.title,
                                        card.rarity.label()
                                    )),
                                    TextFont {
                                        font: f.clone(),
                                        font_size: 14.0,
                                        ..default()
                                    },
                                    TextColor(rc),
                                ));
                                c.spawn((
                                    Text::new(card.subtitle.clone()),
                                    TextFont {
                                        font: f.clone(),
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(GOLD),
                                ));
                                c.spawn((
                                    Text::new(card.desc.clone()),
                                    TextFont {
                                        font: f.clone(),
                                        font_size: 12.0,
                                        ..default()
                                    },
                                    TextColor(Color::WHITE),
                                ));
                            });
                        }
                    });
                }
            }
        }
    }
}

/// Handles keyboard hotkeys (`Q`, `W`, `E`, `R`, `F`, `1/2/3`, `B`, `G`, `Alt+1/2/3`) and HUD button clicks.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
pub(crate) fn moba_hud_inputs(
    keys: Res<ButtonInput<KeyCode>>,
    phase: Res<State<Phase>>,
    mode: Res<GameMode>,
    mut moba: ResMut<MobaMatch>,
    card_btns: Query<(&Interaction, &MobaCardButton), Changed<Interaction>>,
    hud_btns: Query<(&Interaction, &MobaHudButton), Changed<Interaction>>,
) {
    if *mode != GameMode::Moba5v5 || *phase.get() != Phase::Playing {
        return;
    }

    // 1. Non-pausing 3-Card Milestone picks via click or Alt+1/2/3
    let mut card_pick = None;
    for (interaction, btn) in &card_btns {
        if *interaction == Interaction::Pressed {
            card_pick = Some(btn.0);
        }
    }
    let alt = keys.any_pressed([KeyCode::AltLeft, KeyCode::AltRight]);
    if alt {
        for (i, k) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3]
            .into_iter()
            .enumerate()
        {
            if keys.just_pressed(k) {
                card_pick = Some(i);
            }
        }
    }
    if let Some(idx) = card_pick {
        moba.pick_player_card(idx);
        return;
    }

    // 2. Ability hotkeys (Q, W, E, R, F)
    for (key, slot) in [
        (KeyCode::KeyQ, MobaAbilitySlot::Q),
        (KeyCode::KeyW, MobaAbilitySlot::W),
        (KeyCode::KeyE, MobaAbilitySlot::E),
        (KeyCode::KeyR, MobaAbilitySlot::UltimateR),
        (KeyCode::KeyF, MobaAbilitySlot::MobilityF),
    ] {
        if keys.just_pressed(key) {
            moba.cast_player_ability(slot);
        }
    }

    // 3. Active Item hotkeys (1, 2, 3 when Alt is not held)
    if !alt {
        for (slot_idx, key) in [KeyCode::Digit1, KeyCode::Digit2, KeyCode::Digit3]
            .into_iter()
            .enumerate()
        {
            if keys.just_pressed(key) {
                moba.use_player_item(slot_idx);
            }
        }
    }

    // 4. `B`: Quick-Buy next recommended item & Call Goblin Courier if stash has items
    let mut trigger_shop = keys.just_pressed(KeyCode::KeyB);
    let mut trigger_glyph = keys.just_pressed(KeyCode::KeyG);

    for (interaction, btn) in &hud_btns {
        if *interaction == Interaction::Pressed {
            match *btn {
                MobaHudButton::CastSlot(slot) => {
                    moba.cast_player_ability(slot);
                }
                MobaHudButton::UseItem(slot) => {
                    moba.use_player_item(slot);
                }
                MobaHudButton::QuickBuyOrCourier => trigger_shop = true,
                MobaHudButton::GlyphOrBuyback => trigger_glyph = true,
            }
        }
    }

    if trigger_shop {
        moba.player_quick_buy_and_courier();
    }
    if trigger_glyph {
        moba.player_glyph_or_buyback();
    }

    let _ = (ALLIANCE_BLUE, HORDE_RED, recommended_item_order_for_role);
}
