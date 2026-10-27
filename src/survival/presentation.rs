use bevy::{
    prelude::*,
    window::{CursorGrabMode, CursorOptions},
};

use super::{
    ActiveEffects, DamageApplied, DamageState, Hazards, Health, Nutrition, RespawnPoint, UsingItem,
};
use crate::{
    game::GameState,
    inventory::InventoryState,
    player::Player,
    voxel::{VoxelLoading, VoxelViewer},
};

#[derive(Message, Clone, Copy)]
pub(super) enum SoundCue {
    Eat,
    Drink,
    Burp,
    Teleport,
}

pub(super) fn play_audio(
    mut commands: Commands,
    player: Query<Entity, With<Player>>,
    mut cues: MessageReader<SoundCue>,
    mut impacts: MessageReader<DamageApplied>,
) {
    use crate::audio::Sound;
    use pumpkin_data::damage::DamageType as D;
    for cue in cues.read() {
        commands.trigger(Sound::event(match cue {
            SoundCue::Eat => crate::audio::SoundId::EntityGenericEat,
            SoundCue::Drink => crate::audio::SoundId::EntityGenericDrink,
            SoundCue::Burp => crate::audio::SoundId::EntityPlayerBurp,
            SoundCue::Teleport => crate::audio::SoundId::ItemChorusFruitTeleport,
        }));
    }
    let local = player.single().ok();
    for impact in impacts.read().filter(|impact| Some(impact.target) == local) {
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
}

#[derive(Component)]
pub(super) struct RespawnLoader(Entity);

pub(super) fn respawn_input(
    players: Query<Ref<Health>, With<Player>>,
    mut cursors: Query<&mut CursorOptions>,
    inventory: Res<State<InventoryState>>,
    panel: Option<Res<State<crate::devtools::DevPanelState>>>,
    mut next_inventory: ResMut<NextState<InventoryState>>,
) {
    let Ok(health) = players.single() else {
        return;
    };
    if !health.alive() {
        if *inventory.get() == InventoryState::Open {
            next_inventory.set(InventoryState::Closed);
        }
    }
    if health.is_changed() || !health.alive() {
        for mut cursor in &mut cursors {
            let visible = *inventory.get() == InventoryState::Open
                || panel
                    .as_ref()
                    .is_some_and(|panel| *panel.get() == crate::devtools::DevPanelState::Open);
            cursor.visible = visible;
            cursor.grab_mode = if visible {
                CursorGrabMode::None
            } else {
                CursorGrabMode::Locked
            };
        }
    }
}

pub(super) fn begin_respawn(
    mut players: Query<(Entity, &Health, &RespawnPoint, &mut Transform), With<Player>>,
    loaders: Query<(), With<RespawnLoader>>,
    mut commands: Commands,
) {
    if !loaders.is_empty() {
        return;
    }
    let Ok((entity, health, point, mut transform)) = players.single_mut() else {
        return;
    };
    if health.alive() {
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
pub(super) fn finish_respawn(
    loading: VoxelLoading,
    viewer: Single<&GlobalTransform, With<VoxelViewer>>,
    world: crate::voxel::VoxelWorld,
    loaders: Query<(Entity, &RespawnLoader, &Transform), With<RespawnLoader>>,
    mut players: Query<
        (
            &mut Transform,
            &mut Health,
            &mut Nutrition,
            &mut ActiveEffects,
            &mut DamageState,
            &mut Hazards,
            &mut UsingItem,
            &mut crate::character::CharacterMotion,
            &mut crate::character::CharacterState,
            &crate::character::CharacterBody,
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
        let Ok((
            mut transform,
            mut health,
            mut nutrition,
            mut effects,
            mut damage,
            mut hazards,
            mut using,
            mut motion,
            mut character,
            body,
        )) = players.get_mut(target.0)
        else {
            commands.entity(loader).despawn();
            continue;
        };
        let Some(safe_position) = super::placement::respawn(&world, position.translation, body)
        else {
            commands.entity(loader).despawn();
            crate::world::report_load_error(
                &mut commands,
                "No safe respawn position near the saved spawn point.".into(),
            );
            continue;
        };
        transform.translation = safe_position;
        *health = Health::default();
        *nutrition = Nutrition::default();
        *effects = ActiveEffects::default();
        *damage = DamageState::default();
        *hazards = Hazards::default();
        *motion = crate::character::CharacterMotion::default();
        *character = crate::character::CharacterState::default();
        using.0 = None;
        commands.entity(loader).despawn();
    }
}
