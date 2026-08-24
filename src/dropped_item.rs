use std::{collections::HashMap, time::Duration};

use avian3d::prelude::*;
use bevy::{light::NotShadowCaster, prelude::*};

use crate::{
    game::GameState,
    inventory::{ItemKind, ItemStack, PlayerInventory},
    player::Player,
    spatial::GameLayer,
    voxel::{TexturePack, VoxelAssets, VoxelKind, build_block_item_mesh},
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
    spawned_at: Duration,
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
            .add_systems(PostStartup, prepare_dropped_item_meshes)
            .add_systems(
                Update,
                (animate_dropped_item_visuals, refresh_dropped_item_meshes)
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                PostUpdate,
                pickup_dropped_items.run_if(in_state(GameState::Game)),
            )
            .add_systems(
                Update,
                cleanup_expired_drops.run_if(in_state(GameState::Game)),
            );
    }
}

fn prepare_dropped_item_meshes(
    mut meshes: ResMut<Assets<Mesh>>,
    texture_pack: Res<TexturePack>,
    mut assets: ResMut<DroppedItemAssets>,
) {
    for kind in VoxelKind::ITEMIZABLE {
        let item = ItemKind::from_voxel(kind).expect("itemizable voxel has an item kind");
        assets
            .meshes
            .entry(item)
            .or_insert_with(|| meshes.add(build_block_item_mesh(kind, &texture_pack)));
    }
}

fn spawn_dropped_item(
    event: On<SpawnDroppedItem>,
    mut commands: Commands,
    time: Res<Time>,
    assets: Res<DroppedItemAssets>,
    voxel_assets: Res<VoxelAssets>,
) {
    let Some(mesh) = assets.meshes.get(&event.stack.item()) else {
        return;
    };
    let root = commands
        .spawn((
            DroppedItem {
                stack: event.stack,
                spawned_at: time.elapsed(),
            },
            RigidBody::Dynamic,
            Collider::cuboid(DROP_COLLIDER_SIZE, DROP_COLLIDER_SIZE, DROP_COLLIDER_SIZE),
            LinearVelocity(event.velocity),
            LockedAxes::ROTATION_LOCKED,
            LinearDamping(0.3),
            Friction::new(0.7),
            Restitution::new(0.05),
            CollisionLayers::new(
                GameLayer::DroppedItem,
                [GameLayer::Default, GameLayer::World],
            ),
            Transform::from_translation(event.position),
            Visibility::default(),
            DespawnOnExit(GameState::Game),
        ))
        .id();
    commands.spawn((
        DroppedItemVisual {
            phase: event.position.x * 0.73 + event.position.z * 0.37,
        },
        Mesh3d(mesh.clone()),
        MeshMaterial3d(voxel_assets.terrain.clone()),
        Transform::from_scale(Vec3::splat(DROP_VISUAL_SIZE)),
        ChildOf(root),
        NotShadowCaster,
    ));
}

fn refresh_dropped_item_meshes(
    texture_pack: Res<TexturePack>,
    assets: Res<DroppedItemAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if !texture_pack.is_changed() {
        return;
    }
    for (&item, handle) in &assets.meshes {
        let ItemKind::Block(kind) = item;
        if let Some(mut mesh) = meshes.get_mut(handle) {
            *mesh = build_block_item_mesh(kind, &texture_pack);
        }
    }
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
            if now.saturating_sub(drop.spawned_at) < PICKUP_DELAY {
                return true;
            }
            match inventory.insert(drop.stack) {
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
        if now.saturating_sub(drop.spawned_at) >= LIFETIME {
            commands.entity(entity).despawn();
        }
    }
}
