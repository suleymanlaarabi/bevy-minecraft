//! Optional local-player presentation and safe respawning over the living-entity simulation.
use bevy::prelude::*;

use super::{DamageApplied, Health, RespawnPoint, UsingItem};
use crate::{
    game::GameState,
    inventory::InventoryState,
    player::Player,
    voxel::{VoxelLoading, VoxelViewer},
};

/// Installed by PlayerPlugin; living entities do not require a HUD or player camera.
pub struct PlayerSurvivalPlugin;
impl Plugin for PlayerSurvivalPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, super::hud::load_assets)
            .add_systems(
                OnEnter(crate::world::WorldSessionPhase::Playing),
                super::hud::spawn,
            )
            .add_systems(
                Update,
                (begin_respawn, finish_respawn)
                    .chain()
                    .run_if(crate::world::playing),
            )
            .add_systems(
                PostUpdate,
                super::hud::sync
                    .run_if(crate::world::playing)
                    .before(bevy::ui::UiSystems::Content),
            )
            .add_observer(play_audio);
    }
}

fn play_audio(impact: On<DamageApplied>, mut commands: Commands, player: Query<(), With<Player>>) {
    use crate::audio::Sound;
    use pumpkin_data::damage::DamageType as D;
    if !player.contains(impact.target) {
        return;
    }
    let name = if impact.fatal {
        crate::audio::SoundId::EntityPlayerDeath
    } else if impact.source == &D::DROWN {
        crate::audio::SoundId::EntityPlayerHurtDrown
    } else if [
        &D::ON_FIRE,
        &D::IN_FIRE,
        &D::LAVA,
        &D::HOT_FLOOR,
        &D::CAMPFIRE,
    ]
    .contains(&impact.source)
    {
        crate::audio::SoundId::EntityPlayerHurtOnFire
    } else if impact.source == &D::FALL {
        if impact.lost_health > 4.0 {
            crate::audio::SoundId::EntityPlayerBigFall
        } else {
            crate::audio::SoundId::EntityPlayerSmallFall
        }
    } else {
        crate::audio::SoundId::EntityPlayerHurt
    };
    commands.trigger(Sound::event(name));
}

#[derive(Component)]
pub(super) struct RespawnLoader(Entity);

fn begin_respawn(
    mut players: Query<(Entity, &Health, &RespawnPoint, &mut Transform), With<Player>>,
    loaders: Query<(), With<RespawnLoader>>,
    inventory: Res<State<InventoryState>>,
    mut next_inventory: ResMut<NextState<InventoryState>>,
    mut commands: Commands,
) {
    let Ok((entity, health, point, mut transform)) = players.single_mut() else {
        return;
    };
    if health.alive() {
        return;
    }
    if *inventory.get() == InventoryState::Open {
        next_inventory.set(InventoryState::Closed);
    }
    if !loaders.is_empty() {
        return;
    }
    // The player's existing camera remains the only streaming viewer. Moving the
    // dead body redirects it to the spawn while input and physics remain stopped.
    transform.translation = Vec3::from_array(point.0);
    commands.spawn((
        RespawnLoader(entity),
        Transform::from_translation(Vec3::from_array(point.0)),
        GlobalTransform::from_translation(Vec3::from_array(point.0)),
        DespawnOnExit(GameState::Game),
    ));
}

#[allow(clippy::type_complexity)]
fn finish_respawn(
    loading: VoxelLoading,
    viewer: Single<&GlobalTransform, With<VoxelViewer>>,
    world: crate::voxel::VoxelWorld,
    loaders: Query<(Entity, &RespawnLoader, &Transform), With<RespawnLoader>>,
    mut players: Query<
        (
            &mut Transform,
            &mut hollow_features::motion::FixedPosition,
            &crate::character::CharacterBody,
            &RespawnPoint,
        ),
        (With<Player>, Without<RespawnLoader>),
    >,
    mut commands: Commands,
) {
    for (loader, target, position) in &loaders {
        let view_position = viewer.translation();
        if view_position
            .xz()
            .distance_squared(position.translation.xz())
            > 16.0
            || !loading.progress(view_position).0
        {
            continue;
        }
        commands.entity(loader).despawn();
        let Ok((mut transform, mut fixed_position, body, point)) = players.get_mut(target.0) else {
            continue;
        };
        let Some(safe_position) = super::placement::respawn(&world, position.translation, body)
        else {
            commands.trigger(crate::world::LoadError(
                "No safe respawn position near the saved spawn point.".into(),
            ));
            continue;
        };
        transform.translation = safe_position;
        fixed_position.teleport(safe_position);
        commands.entity(target.0).insert((
            super::SurvivalSnapshot {
                respawn: point.clone(),
                ..default()
            },
            crate::character::CharacterMotion::default(),
            crate::character::CharacterState::default(),
            UsingItem::default(),
        ));
    }
}
