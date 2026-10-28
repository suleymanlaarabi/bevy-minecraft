use std::{collections::HashMap, fmt::Write};

use bevy::{picking::prelude::Pickable, prelude::*};
use pumpkin_data::{data_component_impl::IDSetContent, effect::StatusEffect as E};
use serde::Deserialize;

use super::{
    ActiveEffects, ArmorStats, DamageState, Hazards, Health, Nutrition, UsingItem, model::ItemUse,
};
use crate::{
    game::GameState,
    inventory::{HotbarSelection, InventoryState, ItemStack, PlayerInventory, catalog},
    player::Player,
};

const HUD_WIDTH: f32 = 472.0;

#[derive(Component, Clone, Default)]
#[require(
    Pickable::IGNORE,
    DespawnOnExit::<GameState>(GameState::Game),
    crate::game::GameplayHud
)]
pub(super) struct HudRoot;

#[derive(Clone, Copy, PartialEq, Eq)]
enum HeartKind {
    Normal,
    Poisoned,
    Withered,
    Absorbing,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fill {
    Empty,
    Half,
    Full,
}

fn fill(value: f32, index: usize) -> Fill {
    let remaining = value - index as f32 * 2.0;
    if remaining > 1.0 {
        Fill::Full
    } else if remaining > 0.0 {
        Fill::Half
    } else {
        Fill::Empty
    }
}

#[derive(Resource)]
pub(crate) struct HudAssets {
    image: Handle<Image>,
    hearts: [[[Rect; 2]; 3]; 4],
    food: [[Rect; 3]; 2],
    armor: [Rect; 3],
    air: [Rect; 2],
    container: Rect,
    crosshair: Rect,
    effects: [Option<Rect>; 256],
}

impl HudAssets {
    fn image(&self, rect: Rect) -> impl Scene {
        bsn! {
            ImageNode {
                image: {self.image.clone()},
                rect: {Some(rect)},
            }
        }
    }
}

#[derive(Deserialize)]
struct Manifest {
    sprites: HashMap<String, [u32; 4]>,
}

pub(super) fn load_assets(
    mut commands: Commands,
    server: Res<AssetServer>,
) -> Result<(), BevyError> {
    let path = crate::asset_directory().join("player/vanilla/survival/hud.json");
    let manifest: Manifest = serde_json::from_slice(&std::fs::read(path)?)?;
    let rect = |name: &str| -> Result<Rect, BevyError> {
        let &[x, y, width, height] = manifest.sprites.get(name).ok_or_else(|| {
            std::io::Error::other(format!(
                "Missing HUD sprite: {name}. Run scripts/import_player_assets.py."
            ))
        })?;
        Ok(Rect::from_corners(
            Vec2::new(x as f32, y as f32),
            Vec2::new((x + width) as f32, (y + height) as f32),
        ))
    };
    let mut hearts = [[[Rect::default(); 2]; 3]; 4];
    let mut food = [[Rect::default(); 3]; 2];
    let mut armor = [Rect::default(); 3];
    for (fullness, name) in ["empty", "half", "full"].into_iter().enumerate() {
        for (kind, prefix) in ["", "poisoned_", "withered_", "absorbing_"]
            .into_iter()
            .enumerate()
        {
            for (blink, suffix) in ["", "_blinking"].into_iter().enumerate() {
                // Empty foregrounds are hidden; Minecraft only supplies the container.
                hearts[kind][fullness][blink] = rect(&format!(
                    "gui/sprites/hud/heart/{}",
                    if fullness == 0 {
                        "container".into()
                    } else {
                        format!("{prefix}{name}{suffix}")
                    }
                ))?;
            }
        }
        for (hunger, suffix) in ["", "_hunger"].into_iter().enumerate() {
            food[hunger][fullness] = rect(&format!("gui/sprites/hud/food_{name}{suffix}"))?;
        }
        armor[fullness] = rect(&format!("gui/sprites/hud/armor_{name}"))?;
    }
    let mut effects = [None; 256];
    for id in 0..256 {
        if let Some(effect) = <E as IDSetContent>::from_id(id as u16) {
            effects[id] = Some(rect(&format!(
                "mob_effect/{}",
                effect.minecraft_name.trim_start_matches("minecraft:")
            ))?);
        }
    }
    commands.insert_resource(HudAssets {
        image: crate::load_nearest(&server, "player/vanilla/survival/hud.png"),
        hearts,
        food,
        armor,
        effects,
        container: rect("gui/sprites/hud/heart/container")?,
        crosshair: rect("gui/sprites/hud/crosshair")?,
        air: [
            rect("gui/sprites/hud/air")?,
            rect("gui/sprites/hud/air_empty")?,
        ],
    });
    Ok(())
}

#[derive(Component, Clone, Copy, Default)]
pub(super) enum View {
    Heart(usize),
    HeartBackground(usize),
    Food(usize),
    FoodBackground,
    Armor(usize),
    Air(usize),
    EffectGroup(usize),
    EffectImage(usize),
    EffectText(usize),
    #[default]
    Details,
    Flash,
    Blind,
}

fn icon(assets: &HudAssets, view: View, background: Option<View>) -> impl Scene {
    let background = background.map(|view| {
        bsn! {
            view
            @{ assets.image(assets.container) }
        }
    });
    bsn! {
        Node {
            width: px(22),
            height: px(22),
        }
        Pickable::IGNORE
        @background
        Children [
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
            }
            @{ assets.image(assets.container) }
            view
            Pickable::IGNORE
        ]
    }
}

fn icon_row(
    assets: &HudAssets,
    count: usize,
    view: fn(usize) -> View,
    background: Option<fn(usize) -> View>,
) -> impl Scene {
    let icons = (0..count)
        .map(|index| icon(assets, view(index), background.map(|view| view(index))))
        .collect::<Vec<_>>();
    bsn! {
        Node {
            display: Display::Grid,
            grid_template_columns: RepeatedGridTrack::px::<Vec<RepeatedGridTrack>>(10, 22.0),
        }
        Pickable::IGNORE
        Children [ {icons} ]
    }
}

fn column(top: impl Scene, bottom: impl Scene) -> impl Scene {
    bsn! {
        Node {
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
        }
        Pickable::IGNORE
        Children [
            @top
            --
            @bottom
        ]
    }
}

pub(super) fn spawn(
    mut commands: Commands,
    assets: Res<HudAssets>,
    existing: Query<(), With<View>>,
) {
    if !existing.is_empty() {
        return;
    }
    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
        }
        crate::game::GameplayHud { hide_with_inventory: true }
        HudRoot
        Children [
            Node {
                width: px(30),
                height: px(30),
            }
            @{ assets.image(assets.crosshair) }
            Pickable::IGNORE
        ]
    });
    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            position_type: PositionType::Absolute,
            align_items: AlignItems::End,
            justify_content: JustifyContent::Center,
            padding: UiRect::bottom(px(90)),
        }
        HudRoot
        Children [
            Node {
                width: px(HUD_WIDTH),
                max_width: percent(95),
                align_items: AlignItems::End,
                justify_content: JustifyContent::SpaceBetween,
            }
            Pickable::IGNORE
            Children [
                @{ column(icon_row(&assets, 10, View::Armor, None), icon_row(&assets, 40, View::Heart, Some(View::HeartBackground))) }
                --
                @{ column(icon_row(&assets, 10, View::Air, None), icon_row(&assets, 10, View::Food, Some(|_| View::FoodBackground))) }
            ]
        ]
    });
    let effects = (0usize..32)
        .map(|index| {
            bsn! {
                Node {
                    align_items: AlignItems::Center,
                    column_gap: px(8),
                    padding: UiRect::all(px(4)),
                }
                View::EffectGroup(index)
                BackgroundColor(Color::srgba(0.02, 0.025, 0.04, 0.72))
                Pickable::IGNORE
                Children [
                    Node {
                        width: px(28),
                        height: px(28),
                    }
                    @{ assets.image(assets.container) }
                    View::EffectImage(index)
                    Pickable::IGNORE
                    --
                    Text("")
                    TextFont { font_size: FontSize::Px(14.0) }
                    View::EffectText(index)
                    Pickable::IGNORE
                ]
            }
        })
        .collect::<Vec<_>>();
    commands.spawn_scene(bsn! {
        Node {
            position_type: PositionType::Absolute,
            top: px(16),
            right: px(16),
            flex_direction: FlexDirection::Column,
            row_gap: px(4),
        }
        HudRoot
        Children [ {effects} ]
    });
    commands.spawn_scene(bsn! {
        Node {
            position_type: PositionType::Absolute,
            left: px(20),
            top: px(20),
            max_width: percent(35),
            padding: UiRect::all(px(12)),
        }
        Text("")
        TextFont { font_size: FontSize::Px(16.0) }
        TextColor(Color::srgb(0.94, 0.94, 0.88))
        BackgroundColor(Color::srgba(0.025, 0.025, 0.04, 0.85))
        View::Details
        GlobalZIndex(120)
        HudRoot
    });
    for (view, z) in [(View::Flash, 40), (View::Blind, 35)] {
        commands.spawn((
            Node {
                width: percent(100),
                height: percent(100),
                position_type: PositionType::Absolute,
                ..default()
            },
            BackgroundColor(Color::NONE),
            view,
            GlobalZIndex(z),
            HudRoot,
        ));
    }
}

#[derive(PartialEq)]
struct HeartDisplay {
    health: Health,
    kind: HeartKind,
    blink: bool,
}

#[derive(Clone, Copy, PartialEq)]
struct EffectDisplay {
    id: u8,
    level: u8,
    seconds: u32,
}

#[derive(PartialEq)]
struct Details {
    stack: ItemStack,
    using: Option<ItemUse>,
    health: (f32, f32),
    nutrition: (u8, f32, f32),
}

#[derive(PartialEq)]
pub(super) struct HudDisplay {
    hearts: HeartDisplay,
    food: (u8, bool),
    armor: f32,
    air: u16,
    effects: [Option<EffectDisplay>; 32],
    details: Option<Details>,
    flash: f32,
    blind: f32,
}

#[allow(clippy::type_complexity)]
pub(super) fn sync(
    assets: Res<HudAssets>,
    player: Query<
        (
            &Health,
            &Nutrition,
            &DamageState,
            &ArmorStats,
            &Hazards,
            &ActiveEffects,
            &UsingItem,
            &PlayerInventory,
            &HotbarSelection,
        ),
        With<Player>,
    >,
    state: Res<State<InventoryState>>,
    cursor: Res<crate::inventory::InventoryCursor>,
    settings: Res<crate::settings::GraphicsSettings>,
    time: Res<Time>,
    new_roots: Query<(), Added<HudRoot>>,
    mut previous: Local<Option<HudDisplay>>,
    mut description: Local<String>,
    mut views: Query<(
        &View,
        &mut Node,
        Option<&mut ImageNode>,
        Option<&mut Text>,
        Option<&mut BackgroundColor>,
    )>,
) {
    let Ok((health, nutrition, damage, armor, hazards, effects, using, inventory, selection)) =
        player.single()
    else {
        return;
    };
    // Resolve each effect flag once, independently of the number of HUD entities.
    let poison = effects.level(E::POISON.id) > 0;
    let wither = effects.level(E::WITHER.id) > 0;
    let hunger = effects.level(E::HUNGER.id) > 0;
    let blind = effects.level(E::BLINDNESS.id) > 0;
    let nausea = settings.damage_camera_motion && effects.level(E::NAUSEA.id) > 0;
    let open = *state.get() == InventoryState::Open;
    let stack = if health.alive() && (open || using.0.is_some()) {
        let slot = if open {
            cursor.hovered_slot
        } else {
            Some(selection.0)
        };
        slot.and_then(|slot| inventory.slot(slot))
    } else {
        None
    };
    let mut visible_effects = effects.0.iter().filter(|effect| effect.show_icon);
    let next = HudDisplay {
        hearts: HeartDisplay {
            health: *health,
            kind: if poison {
                HeartKind::Poisoned
            } else if wither {
                HeartKind::Withered
            } else {
                HeartKind::Normal
            },
            blink: damage.flash_ticks > 0 && damage.flash_ticks % 4 < 2,
        },
        food: (nutrition.food, hunger),
        armor: armor.protection,
        air: hazards.air_used,
        effects: std::array::from_fn(|_| {
            visible_effects.next().map(|effect| EffectDisplay {
                id: effect.id,
                level: effect.amplifier + 1,
                seconds: effect.ticks.div_ceil(20),
            })
        }),
        details: stack.map(|stack| Details {
            stack,
            using: using.0,
            health: (health.current, health.maximum),
            nutrition: (nutrition.food, nutrition.saturation, nutrition.exhaustion),
        }),
        flash: if health.alive() {
            damage.flash_ticks as f32 / 10.0 * 0.15
        } else {
            0.0
        },
        blind: if !health.alive() {
            0.0
        } else if blind {
            0.55
        } else if nausea {
            0.08 + (time.elapsed_secs() * 1.8).sin() * 0.03
        } else {
            0.0
        },
    };
    let old = if !new_roots.is_empty() {
        None
    } else {
        previous.as_ref()
    };
    if old == Some(&next) {
        return;
    }
    let hearts_changed = old.is_none_or(|old| old.hearts != next.hearts);
    let food_changed = old.is_none_or(|old| old.food != next.food);
    let armor_changed = old.is_none_or(|old| old.armor != next.armor);
    let air_changed = old.is_none_or(|old| old.air != next.air);
    let details_changed = old.is_none_or(|old| old.details != next.details);
    if let Some(details) = &next.details
        && old
            .and_then(|old| old.details.as_ref())
            .is_none_or(|old| old.stack != details.stack)
    {
        *description = catalog::description(details.stack);
    }
    for (view, mut node, image, text, background) in &mut views {
        let mut sprite = None;
        let mut display = None;
        match *view {
            View::HeartBackground(index) if hearts_changed => {
                let hearts = (health.maximum / 2.0).ceil() as usize
                    + (health.absorption / 2.0).ceil() as usize;
                display = Some(index < hearts.min(40));
            }
            View::Heart(index) if hearts_changed => {
                let normal = (health.maximum / 2.0).ceil() as usize;
                let (kind, value, index) = if index >= normal {
                    (HeartKind::Absorbing, health.absorption, index - normal)
                } else {
                    (next.hearts.kind, health.current, index)
                };
                let fullness = fill(value, index);
                display = Some(fullness != Fill::Empty);
                sprite = Some(
                    assets.hearts[kind as usize][fullness as usize][usize::from(next.hearts.blink)],
                );
            }
            View::Food(index) if food_changed => {
                sprite = Some(
                    assets.food[usize::from(hunger)]
                        [fill(nutrition.food as f32, 9 - index) as usize],
                );
            }
            View::FoodBackground if food_changed => {
                sprite = Some(assets.food[0][Fill::Empty as usize]);
            }
            View::Armor(index) if armor_changed => {
                sprite = Some(assets.armor[fill(armor.protection, index) as usize]);
                display = Some(armor.protection > 0.0);
            }
            View::Air(index) if air_changed => {
                sprite =
                    Some(assets.air[usize::from(hazards.air_used >= (10 - index) as u16 * 30)]);
                display = Some(hazards.air_used > 0);
            }
            View::EffectGroup(index) | View::EffectImage(index) | View::EffectText(index)
                if old.is_none_or(|old| old.effects[index] != next.effects[index]) =>
            {
                let effect = next.effects[index];
                if matches!(*view, View::EffectGroup(_)) {
                    display = Some(effect.is_some());
                }
                if let Some(effect) = effect {
                    sprite = assets.effects[effect.id as usize];
                    if let Some(mut text) = text {
                        text.set_if_neq(Text::new(format!(
                            "{} {}  {}:{:02}",
                            catalog::effect_name(effect.id),
                            effect.level,
                            effect.seconds / 60,
                            effect.seconds % 60
                        )));
                    }
                }
            }
            View::Details if details_changed => {
                display = Some(next.details.is_some());
                if let Some(details) = &next.details
                    && let Some(mut text) = text
                {
                    let mut content = description.clone();
                    write!(
                        content,
                        "\nHealth {:.1}/{:.0} · Food {}/20\nSaturation {:.1} · Exhaustion {:.1}/4",
                        details.health.0,
                        details.health.1,
                        details.nutrition.0,
                        details.nutrition.1,
                        details.nutrition.2
                    )
                    .unwrap();
                    if let Some(action) = details.using {
                        write!(
                            content,
                            "\n{}… {:.0}%",
                            if action.drinking {
                                "Drinking"
                            } else {
                                "Eating"
                            },
                            using.progress() * 100.0
                        )
                        .unwrap();
                    }
                    text.set_if_neq(Text::new(content));
                }
            }
            View::Flash | View::Blind => {
                let (alpha, color) = if matches!(*view, View::Flash) {
                    (next.flash, Color::srgba(0.8, 0.0, 0.02, next.flash))
                } else {
                    (next.blind, Color::srgba(0.07, 0.0, 0.1, next.blind))
                };
                display = Some(alpha > 0.0);
                if let Some(mut background) = background {
                    background.set_if_neq(BackgroundColor(color));
                }
            }
            _ => {}
        }
        if let Some(shown) = display {
            let next = if shown { Display::Flex } else { Display::None };
            if node.display != next {
                node.display = next;
            }
        }
        if let (Some(rect), Some(mut image)) = (sprite, image)
            && image.rect != Some(rect)
        {
            image.rect = Some(rect);
        }
    }
    *previous = Some(next);
}
