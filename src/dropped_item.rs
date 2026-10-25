use std::{collections::HashMap, time::Duration};

use avian3d::prelude::*;
use bevy::{light::NotShadowCaster, prelude::*};

use crate::{
    game::GameState,
    inventory::{ItemKind, ItemStack, PlayerInventory},
    player::Player,
    spatial::GameLayer,
    voxel::VoxelAssets,
};

const PICKUP_DELAY: Duration = Duration::from_millis(350);
const LIFETIME: Duration = Duration::from_secs(5 * 60);
const PICKUP_RADIUS: f32 = 1.25;
const DROP_VISUAL_SIZE: f32 = 0.28;
const DROP_COLLIDER_SIZE: f32 = 0.22;

#[derive(Event, Clone, Copy, Debug)]
pub struct SpawnDroppedItem {
    pub stack: ItemStack,
    pub position: Vec3,
    pub velocity: Vec3,
}

pub trait DroppedItemCommandsExt {
    fn spawn_dropped_item(&mut self, stack: ItemStack, position: Vec3, velocity: Vec3)
    -> &mut Self;
}

impl DroppedItemCommandsExt for Commands<'_, '_> {
    fn spawn_dropped_item(
        &mut self,
        stack: ItemStack,
        position: Vec3,
        velocity: Vec3,
    ) -> &mut Self {
        self.trigger(SpawnDroppedItem {
            stack,
            position,
            velocity,
        });
        self
    }
}

#[derive(Component)]
pub struct DroppedItem {
    stack: ItemStack,
    expires_at: Duration,
    pickup_after: Duration,
}

#[derive(Component)]
struct DroppedItemVisual {
    phase: f32,
}

#[derive(Resource, Default)]
struct DroppedItemAssets {
    meshes: HashMap<ItemKind, Handle<Mesh>>,
}

pub struct DroppedItemPlugin;

impl Plugin for DroppedItemPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DroppedItemAssets>()
            .add_observer(spawn_dropped_item)
            .add_observer(restore_dropped_item)
            .add_systems(
                Update,
                animate_dropped_item_visuals.run_if(crate::world::playing),
            )
            .add_systems(
                PostUpdate,
                pickup_dropped_items
                    .after(bevy::transform::TransformSystems::Propagate)
                    .run_if(crate::world::playing),
            )
            .add_systems(Update, cleanup_expired_drops.run_if(crate::world::playing));
    }
}

fn spawn_dropped_item(
    event: On<SpawnDroppedItem>,
    mut commands: Commands,
    time: Res<Time>,
    mut assets: ResMut<DroppedItemAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    voxel_assets: Res<VoxelAssets>,
) {
    spawn_item(
        &mut commands,
        &mut assets,
        &mut meshes,
        &voxel_assets,
        event.stack,
        event.position,
        event.velocity,
        time.elapsed() + LIFETIME,
        time.elapsed() + PICKUP_DELAY,
    );
}

#[derive(Event)]
struct RestoreDroppedItem(crate::world::SavedDrop);

pub(crate) fn restore_drop(commands: &mut Commands, drop: crate::world::SavedDrop) {
    commands.trigger(RestoreDroppedItem(drop));
}

fn restore_dropped_item(
    event: On<RestoreDroppedItem>,
    mut commands: Commands,
    time: Res<Time>,
    mut assets: ResMut<DroppedItemAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    voxel_assets: Res<VoxelAssets>,
) {
    let drop = &event.0;
    if let Ok(stack) = drop.stack.restore() {
        spawn_item(
            &mut commands,
            &mut assets,
            &mut meshes,
            &voxel_assets,
            stack,
            Vec3::from_array(drop.position),
            Vec3::from_array(drop.velocity),
            time.elapsed() + Duration::from_secs_f64(drop.remaining),
            time.elapsed() + Duration::from_secs_f64(drop.pickup_remaining),
        );
    }
}

impl DroppedItem {
    pub(crate) fn capture(
        &self,
        position: Vec3,
        velocity: Vec3,
        now: Duration,
    ) -> crate::world::SavedDrop {
        crate::world::SavedDrop {
            stack: crate::world::SavedStack::capture(self.stack),
            position: position.to_array(),
            velocity: velocity.to_array(),
            remaining: self.expires_at.saturating_sub(now).as_secs_f64(),
            pickup_remaining: self.pickup_after.saturating_sub(now).as_secs_f64(),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn spawn_item(
    commands: &mut Commands,
    assets: &mut DroppedItemAssets,
    meshes: &mut Assets<Mesh>,
    voxel_assets: &VoxelAssets,
    stack: ItemStack,
    position: Vec3,
    velocity: Vec3,
    expires_at: Duration,
    pickup_after: Duration,
) {
    let ItemKind::Block(kind) = stack.item();
    let mesh = assets
        .meshes
        .entry(stack.item())
        .or_insert_with(|| {
            meshes.add(crate::voxel::render::model_mesh::item_geometry(kind).into_mesh())
        })
        .clone();
    let root = commands
        .spawn((
            DroppedItem {
                stack: stack,
                expires_at,
                pickup_after,
            },
            RigidBody::Dynamic,
            Collider::cuboid(DROP_COLLIDER_SIZE, DROP_COLLIDER_SIZE, DROP_COLLIDER_SIZE),
            LinearVelocity(velocity),
            LockedAxes::ROTATION_LOCKED,
            LinearDamping(0.3),
            Friction::new(0.7),
            Restitution::new(0.05),
            CollisionLayers::new(
                GameLayer::DroppedItem,
                [GameLayer::Default, GameLayer::World],
            ),
            Transform::from_translation(position),
            Visibility::default(),
            DespawnOnExit(GameState::Game),
        ))
        .id();
    let visual = commands
        .spawn((
            DroppedItemVisual {
                phase: position.x * 0.73 + position.z * 0.37,
            },
            Transform::from_scale(Vec3::splat(DROP_VISUAL_SIZE)),
            Visibility::default(),
            ChildOf(root),
        ))
        .id();
    // The block mesh spans 0..1: center it on the spinning visual.
    commands.spawn((
        Mesh3d(mesh.clone()),
        MeshMaterial3d(if kind.name().contains("glass") || kind.name() == "ice" {
            voxel_assets.translucent.clone()
        } else {
            // Item models may hold cutout textures (flowers, torches).
            voxel_assets.models_cutout.clone()
        }),
        Transform::from_translation(Vec3::splat(-0.5)),
        ChildOf(visual),
        NotShadowCaster,
    ));
}

fn animate_dropped_item_visuals(
    time: Res<Time>,
    mut visuals: Query<(&DroppedItemVisual, &mut Transform)>,
) {
    let seconds = time.elapsed_secs();
    for (visual, mut transform) in &mut visuals {
        transform.rotation = Quat::from_rotation_y(seconds * 2.4 + visual.phase);
        transform.translation.y = (seconds * 2.0 + visual.phase).sin() * 0.03;
    }
}

fn pickup_dropped_items(
    spatial_query: SpatialQuery,
    time: Res<Time>,
    player: Single<(&GlobalTransform, &mut PlayerInventory), With<Player>>,
    mut drops: Query<&mut DroppedItem>,
    mut commands: Commands,
) {
    let (player_transform, mut inventory) = player.into_inner();
    let now = time.elapsed();
    spatial_query.shape_intersections_callback(
        &Collider::sphere(PICKUP_RADIUS),
        player_transform.translation(),
        Quat::IDENTITY,
        &SpatialQueryFilter::from_mask(GameLayer::DroppedItem),
        |entity| {
            let Ok(mut drop) = drops.get_mut(entity) else {
                return true;
            };
            if now < drop.pickup_after {
                return true;
            }
            let remaining =
                PlayerInventory::edit(&mut inventory, |inventory| inventory.insert(drop.stack));
            match remaining {
                None => commands.entity(entity).despawn(),
                Some(remaining) => drop.stack = remaining,
            }
            true
        },
    );
}

fn cleanup_expired_drops(
    time: Res<Time>,
    drops: Query<(Entity, &DroppedItem)>,
    mut commands: Commands,
    mut timer: Local<Option<Timer>>,
) {
    let timer =
        timer.get_or_insert_with(|| Timer::new(Duration::from_secs(1), TimerMode::Repeating));
    timer.tick(time.delta());
    if !timer.just_finished() {
        return;
    }
    let now = time.elapsed();
    for (entity, drop) in &drops {
        if now >= drop.expires_at {
            commands.entity(entity).despawn();
        }
    }
}
