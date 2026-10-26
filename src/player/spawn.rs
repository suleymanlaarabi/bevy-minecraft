use super::{Crosshair, Player, PlayerCamera};
use crate::{
    game::GameState,
    spatial::{FollowOffset, FollowedBy},
    voxel::{VoxelLoading, VoxelSettings, VoxelViewer},
};
use bevy::{
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

/// Vanilla eyes sit 1.62 above the feet; the player's origin is its box center.
const PLAYER_EYE_HEIGHT: f32 = 0.72;

/// The spawn point search generates terrain, so it runs off the main thread while a
/// loading screen shows. The player waits for both the search and area loading.
#[derive(Component)]
pub(super) struct SpawnSearch(Task<std::io::Result<Vec3>>);

/// Temporary streaming viewer at the chosen spawn, without a player to simulate.
#[derive(Component)]
pub(super) struct PendingSpawn;

#[derive(Component, Clone, Default)]
pub(super) struct LoadingScreen;

#[derive(Component, Clone, Default)]
pub(super) struct LoadingMessage;

pub(super) fn start_spawn_search(
    mut commands: Commands,
    voxel_settings: Option<Res<VoxelSettings>>,
    budget: Res<hollow_worldgen::tasks::VoxelTaskBudget>,
    generator: Option<Res<crate::voxel::generation::WorldGenerator>>,
    session: Option<Res<crate::world::WorldSession>>,
) {
    let fallback = voxel_settings.map_or(Vec3::new(0.0, 30.0, 0.0), |settings| {
        let center = settings.chunk_center(IVec2::ZERO);
        Vec3::new(center.x, settings.base_height + 15.0, center.z)
    });
    let generator = generator.map(|generator| generator.clone());
    let saved_position = session.and_then(|s| {
        s.resume
            .player
            .as_ref()
            .map(|p| Vec3::from_array(p.position))
    });
    let budget = budget.clone();
    let task = AsyncComputeTaskPool::get().spawn(async move {
        if let Some(position) = saved_position {
            Ok(position)
        } else if let Some(generator) = generator {
            generator.try_spawn_position(budget).await
        } else {
            Ok(fallback)
        }
    });
    commands.spawn((SpawnSearch(task), DespawnOnExit(GameState::Game)));
    let message = if saved_position.is_some() {
        "Restoring player…"
    } else {
        "Finding a spawn point…"
    };

    // The player camera does not exist yet: the loading screen brings its own.
    commands.spawn((Camera2d, LoadingScreen, DespawnOnExit(GameState::Game)));
    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center,
        }
        BackgroundColor(Color::srgb_u8(15, 15, 18))
        LoadingScreen
        DespawnOnExit::<GameState>(GameState::Game)
        Children [
            Text(message)
            LoadingMessage
            TextFont { font_size: FontSize::Px(28.0) }
            TextColor(Color::WHITE)
        ]
    });
}

pub(super) fn finish_spawn_search(
    mut commands: Commands,
    mut searches: Query<(Entity, &mut SpawnSearch)>,
) {
    for (entity, mut search) in &mut searches {
        let Some(position) = check_ready(&mut search.0) else {
            continue;
        };
        let position = match position {
            Ok(position) => position,
            Err(error) => {
                crate::world::report_load_error(&mut commands, error.to_string());
                commands.entity(entity).despawn();
                continue;
            }
        };
        commands.entity(entity).remove::<SpawnSearch>().insert((
            PendingSpawn,
            VoxelViewer,
            Transform::from_translation(position),
            GlobalTransform::from_translation(position),
        ));
    }
}

pub(super) fn finish_spawn_loading(
    mut commands: Commands,
    loading: VoxelLoading,
    spawns: Query<(Entity, &Transform), With<PendingSpawn>>,
    screens: Query<Entity, With<LoadingScreen>>,
    session: Option<Res<crate::world::WorldSession>>,
    mut cursor: ResMut<crate::inventory::InventoryCursor>,
    mut next_phase: ResMut<NextState<crate::world::WorldSessionPhase>>,
    mut messages: Query<&mut Text, With<LoadingMessage>>,
) {
    for (entity, transform) in &spawns {
        let (ready, stage) = loading.progress(transform.translation);
        for mut text in &mut messages {
            if text.0 != stage {
                text.0 = stage.into();
            }
        }
        if !ready {
            continue;
        }
        commands.entity(entity).despawn();
        for screen in &screens {
            commands.entity(screen).despawn();
        }
        let saved = session.as_ref().and_then(|s| s.resume.player.as_ref());
        spawn_player(&mut commands, transform.translation, saved);
        if let Some(session) = &session {
            if let Some(stack) = saved
                .and_then(|p| p.cursor.as_ref())
                .and_then(|s| s.restore().ok())
            {
                cursor.stack = Some(stack);
            }
            for drop in &session.resume.drops {
                crate::dropped_item::restore_drop(&mut commands, drop.clone());
            }
        }
        crate::world::spawn_save_indicator(&mut commands);
        next_phase.set(crate::world::WorldSessionPhase::Playing);
    }
}

fn spawn_player(
    commands: &mut Commands,
    spawn_pos: Vec3,
    saved: Option<&crate::world::SavedPlayer>,
) {
    let rotation = saved.map_or(Quat::IDENTITY, |p| Quat::from_array(p.rotation).normalize());
    let pitch = saved.map_or(0.0, |p| p.pitch);
    let inventory = saved.and_then(|p| p.inventory().ok()).unwrap_or_default();
    let grid = crate::inventory::CraftingGrid {
        slots: std::array::from_fn(|index| {
            saved
                .and_then(|player| player.crafting[index].as_ref())
                .and_then(|stack| stack.restore().ok())
        }),
        ..default()
    };
    let player_transform = Transform {
        translation: spawn_pos,
        rotation,
        ..default()
    };
    let camera_transform = Transform {
        translation: spawn_pos + Vec3::Y * PLAYER_EYE_HEIGHT,
        rotation: rotation * Quat::from_rotation_x(pitch),
        ..default()
    };
    commands.spawn_scene_list(bsn! {
        Player
        inventory
        grid
        crate::inventory::HotbarSelection({saved.map_or(0, |saved| saved.selected)})
        player_transform
        GlobalTransform::from_translation(spawn_pos)
        FollowedBy [
            PlayerCamera { pitch }
            camera_transform
            GlobalTransform::from_translation(spawn_pos + Vec3::Y * PLAYER_EYE_HEIGHT)
            FollowOffset({Vec3::Y * PLAYER_EYE_HEIGHT})
        ]
    });

    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center
        }
        Crosshair
        Children [
            Node {
                width: px(32),
                height: px(32),
                position_type: PositionType::Relative
            }
            Children [
                Node {
                    width: px(2),
                    height: px(18),
                    position_type: PositionType::Absolute,
                    left: px(15),
                    top: px(7)
                }
                BackgroundColor(Color::WHITE)
                --

                Node {
                    width: px(18),
                    height: px(2),
                    position_type: PositionType::Absolute,
                    left: px(7),
                    top: px(15)
                }
                BackgroundColor(Color::WHITE)
            ]
        ]
        DespawnOnExit::<GameState>(GameState::Game)
    });
}
