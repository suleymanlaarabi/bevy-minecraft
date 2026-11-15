//! Parallel fixed-step collision and revision-based sleeping for passive voxel bodies.
use crate::character::{Aabb, Terrain};
use bevy::{
    ecs::{batching::BatchingStrategy, query::ArchetypeFilter},
    math::DVec3,
    prelude::*,
    utils::Parallel,
};
use hollow_core::VoxelCell;
use hollow_engine::world::VoxelWorld;
use hollow_features::motion::FixedPosition;

pub struct BallisticProfile {
    pub half_extent: DVec3,
    pub gravity: f32,
    pub air_drag: f32,
    pub restitution: f32,
    pub minimum_bounce: f32,
    pub ground_drag: f32,
    pub sleep_speed_squared: f32,
}
pub trait BallisticRules: Send + Sync + 'static {
    const PROFILE: BallisticProfile;
}

/// Fixed-step state; sleeping items only check the revisions of touching columns.
#[derive(Component)]
#[require(FixedPosition)]
pub struct BallisticMotion {
    pub velocity: Vec3,
    support: Option<[Option<(Entity, u64)>; 4]>,
}

impl BallisticMotion {
    pub fn new(velocity: Vec3) -> Self {
        Self {
            velocity,
            support: None,
        }
    }

    fn bounds<R: BallisticRules>(position: DVec3) -> Aabb {
        let half = R::PROFILE.half_extent;
        Aabb {
            min: position - half,
            max: position + half,
        }
    }

    fn support_revisions<T: VoxelCell, R: BallisticRules>(
        position: DVec3,
        world: &VoxelWorld<T>,
    ) -> [Option<(Entity, u64)>; 4] {
        let bounds = Self::bounds::<R>(position);
        world.region_revisions(bounds.min.as_vec3(), bounds.max.as_vec3())
    }

    fn step<T: VoxelCell, R: BallisticRules>(
        &mut self,
        position: &mut Mut<FixedPosition>,
        world: &VoxelWorld<T>,
        terrain: &mut Terrain,
        dt: f32,
        damping: f32,
    ) {
        position.bypass_change_detection().previous = position.current;
        if self.support.is_some_and(|support| {
            support == Self::support_revisions::<T, R>(position.current, world)
        }) {
            return;
        }
        if !world.collision_ready(position.current.as_vec3()) {
            return;
        }
        self.support = None;
        self.velocity.y -= R::PROFILE.gravity * dt;
        self.velocity *= damping;
        let bounds = Self::bounds::<R>(position.current);
        let delta = (self.velocity * dt).as_dvec3();
        terrain.gather_collisions(world, bounds.expanded_towards(delta));
        let correction = terrain.depenetrate(bounds);
        let corrected = position.current + correction;
        let moved = terrain.collide_box(Self::bounds::<R>(corrected), delta);
        let next = corrected + moved;
        if position.current != next {
            position.current = next;
        }
        let blocked = (moved - delta).abs().cmpgt(DVec3::splat(1e-7));
        if blocked.x {
            self.velocity.x = 0.0;
        }
        if blocked.z {
            self.velocity.z = 0.0;
        }
        if blocked.y {
            self.velocity.y = if delta.y < 0.0 {
                -self.velocity.y * R::PROFILE.restitution
            } else {
                0.0
            };
            if self.velocity.y < R::PROFILE.minimum_bounce {
                self.velocity.y = 0.0;
            }
        }
        if blocked.y && delta.y < 0.0 {
            self.velocity.x *= R::PROFILE.ground_drag;
            self.velocity.z *= R::PROFILE.ground_drag;
            if self.velocity.length_squared() < R::PROFILE.sleep_speed_squared {
                self.velocity = Vec3::ZERO;
                self.support = Some(Self::support_revisions::<T, R>(position.current, world));
            }
        }
    }
}

/// Reuse each worker's collision buffer instead of allocating one per item/tick.
pub fn simulate_bodies<T: VoxelCell, R: BallisticRules, F: ArchetypeFilter>(
    world: VoxelWorld<T>,
    time: Res<Time<Fixed>>,
    terrain: Local<Parallel<Terrain>>,
    mut drops: Query<(&mut BallisticMotion, &mut FixedPosition), F>,
) {
    let dt = time.delta_secs();
    let damping = (-R::PROFILE.air_drag * dt).exp();
    // Small groups stay on this thread; larger groups share persistent worker buffers.
    if drops.iter().len() < 128 {
        let mut terrain = terrain.borrow_local_mut();
        for (mut motion, mut position) in &mut drops {
            motion.step::<T, R>(&mut position, &world, &mut terrain, dt, damping);
        }
    } else {
        drops
            .par_iter_mut()
            .batching_strategy(BatchingStrategy::new().min_batch_size(64))
            .for_each_init(
                || terrain.borrow_local_mut(),
                |terrain, (mut motion, mut position)| {
                    motion.step::<T, R>(&mut position, &world, terrain, dt, damping)
                },
            );
    }
}
