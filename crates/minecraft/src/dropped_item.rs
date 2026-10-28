use hollow_features::motion::FixedPosition;
use std::{collections::HashMap, time::Duration};

use bevy::{
    ecs::{batching::BatchingStrategy, storage::SparseSet},
    light::NotShadowCaster,
    math::DVec3,
    prelude::*,
    utils::Parallel,
};

use crate::{
    character::{Aabb, CharacterSystems, Terrain},
    game::GameState,
    inventory::{ItemKind, ItemStack, PlayerInventory},
    voxel::{VoxelAssets, VoxelWorld, chunk_at, render::models},
};

const PICKUP_DELAY: Duration = Duration::from_millis(350);
const LIFETIME: Duration = Duration::from_secs(5 * 60);
const DROP_VISUAL_SIZE: f32 = 0.28;
const DROP_COLLIDER_SIZE: f32 = 0.22;

#[derive(Event, Clone, Copy, Debug)]
pub struct SpawnDroppedItem {
    pub stack: ItemStack,
    pub position: Vec3,
    pub velocity: Vec3,
    pub lifetime: Duration,
    pub pickup_delay: Duration,
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
            lifetime: LIFETIME,
            pickup_delay: PICKUP_DELAY,
        });
        self
    }
}

/// Any actor with this capability can collect drops; controllers need no pickup logic.
#[derive(Component)]
#[require(FixedPosition, PlayerInventory = PlayerInventory::empty())]
pub struct ItemCollector {
    pub radius: f32,
}
impl Default for ItemCollector {
    fn default() -> Self {
        Self { radius: 1.25 }
    }
}

/// React after the inventory transaction; presentation does not own collection rules.
#[derive(EntityEvent)]
pub struct ItemCollected {
    pub entity: Entity,
    pub stack: ItemStack,
}

#[derive(Component)]
pub struct DroppedItem {
    stack: ItemStack,
    remaining: Duration,
    pickup_remaining: Duration,
}

/// Fixed-step state; sleeping items only check the revisions of touching columns.
#[derive(Component)]
#[require(FixedPosition)]
pub(crate) struct DropMotion {
    velocity: Vec3,
    support: Option<[Option<(Entity, u64)>; 4]>,
}

impl DropMotion {
    fn bounds(position: DVec3) -> Aabb {
        let half = DVec3::splat(DROP_COLLIDER_SIZE as f64 * 0.5);
        Aabb {
            min: position - half,
            max: position + half,
        }
    }

    fn support_revisions(position: DVec3, world: &VoxelWorld) -> [Option<(Entity, u64)>; 4] {
        let bounds = Self::bounds(position);
        let min = chunk_at(bounds.min.as_vec3());
        let max = chunk_at(bounds.max.as_vec3());
        let first = world.column_revision(min);
        let x = if min.x == max.x {
            first
        } else {
            world.column_revision(IVec2::new(max.x, min.y))
        };
        let z = if min.y == max.y {
            first
        } else {
            world.column_revision(IVec2::new(min.x, max.y))
        };
        let last = if min.x == max.x {
            z
        } else if min.y == max.y {
            x
        } else {
            world.column_revision(max)
        };
        [first, x, z, last]
    }

    fn step(
        &mut self,
        position: &mut FixedPosition,
        world: &VoxelWorld,
        terrain: &mut Terrain,
        dt: f32,
        damping: f32,
    ) {
        position.previous = position.current;
        if self
            .support
            .is_some_and(|support| support == Self::support_revisions(position.current, world))
        {
            return;
        }
        if !world.collision_ready(position.current.as_vec3()) {
            return;
        }
        self.support = None;
        self.velocity.y -= 9.81 * dt;
        self.velocity *= damping;
        let bounds = Self::bounds(position.current);
        let delta = (self.velocity * dt).as_dvec3();
        terrain.gather_collisions(world, bounds.expanded_towards(delta));
        let correction = terrain.depenetrate(bounds);
        position.current += correction;
        let moved = terrain.collide_box(Self::bounds(position.current), delta);
        position.current += moved;
        let blocked = (moved - delta).abs().cmpgt(DVec3::splat(1e-7));
        if blocked.x {
            self.velocity.x = 0.0;
        }
        if blocked.z {
            self.velocity.z = 0.0;
        }
        if blocked.y {
            self.velocity.y = if delta.y < 0.0 {
                -self.velocity.y * 0.05
            } else {
                0.0
            };
            if self.velocity.y < 0.15 {
                self.velocity.y = 0.0;
            }
        }
        if blocked.y && delta.y < 0.0 {
            self.velocity.x *= 0.7;
            self.velocity.z *= 0.7;
            if self.velocity.length_squared() < 0.0025 {
                self.velocity = Vec3::ZERO;
                self.support = Some(Self::support_revisions(position.current, world));
            }
        }
    }
}

#[derive(Component)]
struct DroppedItemVisual {
    phase: f32,
}

#[derive(Resource, Default)]
struct DroppedItemAssets {
    meshes: SparseSet<ItemKind, Handle<Mesh>>,
    // Generational image asset IDs are sparse; share one material per actual image.
    materials: HashMap<AssetId<Image>, Handle<StandardMaterial>>,
}

pub struct DroppedItemPlugin;

impl Plugin for DroppedItemPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DroppedItemAssets>()
            .add_observer(spawn_dropped_item)
            .add_systems(
                Update,
                animate_dropped_item_visuals.run_if(crate::world::playing),
            )
            .add_systems(
                FixedUpdate,
                (
                    simulate_dropped_items,
                    collect_dropped_items.after(CharacterSystems),
                )
                    .chain()
                    .in_set(crate::simulation::SimulationSystems::Actors)
                    .run_if(crate::simulation::running),
            )
            .add_systems(
                PostUpdate,
                sync_drop_visibility
                    .after(crate::voxel::VoxelPipeline)
                    .before(bevy::transform::TransformSystems::Propagate),
            );
    }
}

fn spawn_dropped_item(
    event: On<SpawnDroppedItem>,
    mut commands: Commands,
    columns: Res<crate::block_behavior::containers::GameplayColumns>,
    active: Query<(), With<crate::simulation::Simulating>>,
    mut assets: ResMut<DroppedItemAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    voxel_assets: Res<VoxelAssets>,
    render_source: Res<crate::voxel::render::MinecraftRenderSource>,
    icons: Res<crate::inventory::icons::BlockIcons>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let stack = event.stack;
    let position = event.position;
    let velocity = event.velocity;
    let remaining = event.lifetime;
    let pickup_remaining = event.pickup_delay;
    let mesh = assets
        .meshes
        .get_or_insert_with(stack.item(), || {
            meshes.add(match stack.item().block_kind() {
                Some(kind) => hollow_render::model_mesh::state_geometry(
                    &*render_source,
                    hollow_core::StateId(kind.default_state().id()),
                    IVec3::ZERO,
                )
                .into_mesh(),
                None => {
                    let mut mesh = Rectangle::new(1.0, 1.0).mesh().build();
                    if let Some((offset, scale)) = icons.item_uv(stack.item())
                        && let Some(bevy::mesh::VertexAttributeValues::Float32x2(uvs)) =
                            mesh.attribute_mut(Mesh::ATTRIBUTE_UV_0)
                    {
                        for uv in uvs {
                            *uv = (offset + Vec2::from_array(*uv) * scale).to_array();
                        }
                    }
                    mesh
                }
            })
        })
        .clone();
    let root = commands
        .spawn((
            DroppedItem {
                stack: stack,
                remaining,
                pickup_remaining,
            },
            DropMotion {
                velocity,
                support: None,
            },
            FixedPosition::new(position),
            Transform::from_translation(position),
            Visibility::default(),
            DespawnOnExit(GameState::Game),
        ))
        .id();
    if let Some(&owner) = columns.get(&chunk_at(position)) {
        commands
            .entity(root)
            .insert(crate::simulation::SimulatedIn(owner));
        if active.contains(owner) {
            commands.entity(root).insert(crate::simulation::Simulating);
        }
    }
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
    match stack.item().block_kind() {
        Some(kind) => {
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(if models::translucent_kind(kind) {
                    voxel_assets.translucent.clone()
                } else {
                    voxel_assets.models_cutout.clone()
                }),
                Transform::from_translation(Vec3::splat(-0.5)),
                ChildOf(visual),
                NotShadowCaster,
            ));
        }
        None => {
            let texture = icons.item_image(stack.item()).cloned().unwrap_or_default();
            let material = assets
                .materials
                .entry(texture.id())
                .or_insert_with(|| {
                    materials.add(StandardMaterial {
                        base_color_texture: Some(texture),
                        alpha_mode: AlphaMode::Mask(0.5),
                        cull_mode: None,
                        perceptual_roughness: 1.0,
                        reflectance: 0.0,
                        ..default()
                    })
                })
                .clone();
            commands.spawn((
                Mesh3d(mesh),
                MeshMaterial3d(material),
                Transform::default(),
                ChildOf(visual),
                NotShadowCaster,
            ));
        }
    }
}
pub(crate) fn restore_drop(commands: &mut Commands, drop: crate::world::SavedDrop) {
    if let Ok(stack) = drop.stack.restore() {
        commands.trigger(SpawnDroppedItem {
            stack,
            position: Vec3::from_array(drop.position),
            velocity: Vec3::from_array(drop.velocity),
            lifetime: Duration::from_secs_f64(drop.remaining),
            pickup_delay: Duration::from_secs_f64(drop.pickup_remaining),
        });
    }
}

impl DroppedItem {
    pub(crate) fn capture(
        &self,
        motion: &DropMotion,
        position: &FixedPosition,
    ) -> crate::world::SavedDrop {
        crate::world::SavedDrop {
            stack: crate::world::SavedStack::capture(self.stack),
            position: position.current.as_vec3().to_array(),
            velocity: motion.velocity.to_array(),
            remaining: self.remaining.as_secs_f64(),
            pickup_remaining: self.pickup_remaining.as_secs_f64(),
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

/// Reuse each worker's collision buffer instead of allocating one per item/tick.
fn simulate_dropped_items(
    world: VoxelWorld,
    time: Res<Time<Fixed>>,
    terrain: Local<Parallel<Terrain>>,
    mut drops: Query<(&mut DropMotion, &mut FixedPosition), With<crate::simulation::Simulating>>,
) {
    let dt = time.delta_secs();
    let damping = (-0.3 * dt).exp();
    // Small groups stay on this thread; larger groups share persistent worker buffers.
    if drops.iter().len() < 128 {
        let mut terrain = terrain.borrow_local_mut();
        for (mut motion, mut position) in &mut drops {
            motion.step(&mut position, &world, &mut terrain, dt, damping);
        }
    } else {
        drops
            .par_iter_mut()
            .batching_strategy(BatchingStrategy::new().min_batch_size(64))
            .for_each_init(
                || terrain.borrow_local_mut(),
                |terrain, (mut motion, mut position)| {
                    motion.step(&mut position, &world, terrain, dt, damping)
                },
            );
    }
}

/// Timers, collection and expiry share one pass over active drop state.
fn collect_dropped_items(
    time: Res<Time<Fixed>>,
    mut collectors: Query<(Entity, &FixedPosition, &ItemCollector, &mut PlayerInventory)>,
    mut drops: Query<
        (Entity, &FixedPosition, &mut DroppedItem),
        With<crate::simulation::Simulating>,
    >,
    mut commands: Commands,
) {
    let half = DVec3::splat(DROP_COLLIDER_SIZE as f64 * 0.5);
    for (entity, position, mut drop) in &mut drops {
        drop.remaining = drop.remaining.saturating_sub(time.delta());
        drop.pickup_remaining = drop.pickup_remaining.saturating_sub(time.delta());
        if drop.pickup_remaining.is_zero() {
            // Collector counts are small; a spatial index would add maintenance and storage.
            for (collector, origin, capability, mut inventory) in &mut collectors {
                // Sphere/box overlap without allocating a spatial-query structure.
                let distance = (origin.current - position.current).abs() - half;
                let radius = capability.radius as f64;
                if !radius.is_finite()
                    || radius < 0.0
                    || distance.max(DVec3::ZERO).length_squared() > radius * radius
                {
                    continue;
                }
                let stack = drop.stack;
                let remaining =
                    PlayerInventory::edit(&mut inventory, |inventory| inventory.insert(stack));
                if remaining == Some(stack) {
                    continue;
                }
                commands.trigger(ItemCollected {
                    entity: collector,
                    stack: stack
                        .with_count(stack.count() - remaining.map_or(0, |s| s.count()))
                        .expect("A successful collection transfers at least one item"),
                });
                if let Some(remaining) = remaining {
                    drop.stack = remaining;
                } else {
                    drop.remaining = Duration::ZERO;
                    break;
                }
            }
        }
        if drop.remaining.is_zero() {
            commands.entity(entity).despawn();
        }
    }
}

fn sync_drop_visibility(
    rendered: Query<(), With<hollow_render::streaming::RenderRequested>>,
    mut drops: Query<(&crate::simulation::SimulatedIn, &mut Visibility), With<DroppedItem>>,
) {
    for (owner, mut visibility) in &mut drops {
        visibility.set_if_neq(if rendered.contains(owner.0) {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
    }
}
