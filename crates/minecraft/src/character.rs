//! Vanilla entity movement: an AABB moved axis by axis against block collision
//! boxes, at Minecraft's 20 ticks per second. No physics engine is involved, so
//! there are no internal-edge contacts between neighboring voxels or chunks.
#![allow(clippy::type_complexity)]

use bevy::{ecs::query::QueryData, math::DVec3, prelude::*};

use crate::voxel::{VoxelState, VoxelWorld};
use hollow_engine::fluids::FluidContactState;
use hollow_features::motion::FixedPosition;

/// All movement constants below are per vanilla game tick.
const GRAVITY: f64 = 0.08;
const VERTICAL_DRAG: f64 = 0.98;
const AIR_DRAG: f64 = 0.91;
const DEFAULT_SLIPPERINESS: f64 = 0.6;
const WALK_SPEED: f64 = 0.1;
const SPRINT_FACTOR: f64 = 1.3;
const SNEAK_FACTOR: f64 = 0.3;
const INPUT_SCALE: f64 = 0.98;
const AIR_SPEED: f64 = 0.02;
const AIR_SPRINT_SPEED: f64 = 0.026;
const JUMP_VELOCITY: f64 = 0.42;
const SPRINT_JUMP_BOOST: f64 = 0.2;
const STEP_HEIGHT: f64 = 0.6;
const AUTO_JUMP_HEIGHT: f64 = 1.2;
const AUTO_JUMP_PROBE: f64 = 0.3;
const SNEAK_EDGE_STEP: f64 = 0.05;
const FLUID_SWIM_UP: f64 = 0.04;
const FLUID_JUMP_THRESHOLD: f64 = 0.4;
const SHORE_JUMP: f64 = 0.3;
const CLIMB_SPEED: f64 = 0.15;
const CLIMB_RISE: f64 = 0.2;
const MIN_VELOCITY: f64 = 0.003;
/// Vanilla creative flight: horizontal acceleration (doubled when sprinting),
/// vertical push per tick, and the vertical drag that replaces gravity.
const FLY_SPEED: f64 = 0.05;
const FLY_VERTICAL: f64 = FLY_SPEED * 3.0;
const FLY_VERTICAL_DRAG: f64 = 0.6;
/// Vanilla `Mth.equal`: a smaller difference is not a collision.
const MOVE_EPSILON: f64 = 1e-5;
/// Touching faces do not overlap.
const CONTACT_EPSILON: f64 = 1e-7;

pub struct CharacterPlugin;

/// Vanilla player poses: each one has its own box height and eye height.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum Pose {
    #[default]
    Standing,
    Crouching,
    /// Swimming, or crawling where nothing taller fits.
    Swimming,
}

/// Shared immutable geometry and movement coefficients, selected at spawn.
#[derive(Debug)]
pub struct CharacterShape {
    pub half_width: f32,
    pub origin_height: f32,
    pub heights: [f32; 3],
    pub eyes: [f32; 3],
    pub step_height: f64,
    pub jump_velocity: f64,
    pub speed: f64,
}

pub static PLAYER_SHAPE: CharacterShape = CharacterShape {
    half_width: 0.3,
    origin_height: 0.9,
    heights: [1.8, 1.5, 0.6],
    eyes: [1.62, 1.27, 0.4],
    step_height: STEP_HEIGHT,
    jump_velocity: JUMP_VELOCITY,
    speed: 1.0,
};

/// A pointer avoids duplicating species geometry in every actor. Profiles have
/// static lifetime: reading a shape needs neither hashing nor reference counting.
#[derive(Component, Debug, Clone, Copy, Deref)]
#[require(FluidContactState)]
pub struct CharacterBody(pub &'static CharacterShape);

impl Default for CharacterBody {
    fn default() -> Self {
        Self(&PLAYER_SHAPE)
    }
}

impl CharacterBody {
    pub fn height(&self, pose: Pose) -> f32 {
        self.heights[pose as usize]
    }

    pub fn eye_height(&self, pose: Pose) -> f32 {
        self.eyes[pose as usize]
    }

    pub(crate) fn aabb(&self, origin: DVec3, pose: Pose) -> Aabb {
        let feet = origin - DVec3::Y * self.origin_height as f64;
        let half_width = self.half_width as f64;
        Aabb {
            min: feet - DVec3::new(half_width, 0.0, half_width),
            max: feet + DVec3::new(half_width, self.height(pose) as f64, half_width),
        }
    }

    fn eye_offset(&self, pose: Pose) -> f32 {
        self.eye_height(pose) - self.origin_height
    }
}

/// Controller velocity and pose animation, sampled at the fixed simulation rate.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterMotion {
    /// Blocks per tick.
    pub velocity: DVec3,
    /// Camera height above the origin: it eases toward the pose's eyes, as in vanilla.
    eye: f32,
    previous_eye: f32,
    walk_distance: f32,
    walk_delta: f32,
    bob: f32,
    previous_bob: f32,
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterState {
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub in_swimmable_fluid: bool,
    pub in_fluid: bool,
    pub eyes_in_swimmable_fluid: bool,
    pub swimming: bool,
    pub pose: Pose,
}

impl CharacterMotion {
    /// Minecraft's backwards walk interpolation and tick-smoothed bob amplitude.
    pub(crate) fn walking_animation(&self, alpha: f32) -> (f32, f32) {
        (
            -(self.walk_distance + self.walk_delta * alpha),
            self.previous_bob.lerp(self.bob, alpha),
        )
    }

    /// Eased eye height above the entity origin, for the camera.
    pub fn eye_offset(&self, alpha: f32) -> f32 {
        self.previous_eye.lerp(self.eye, alpha)
    }
}

#[derive(Component, Default, Debug, Clone, Copy, PartialEq)]
pub struct CharacterMovement {
    pub direction: Vec3,
    pub look_direction: Vec3,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

/// Actual movement outcomes, sampled once by fixed-step gameplay consumers.
#[derive(Component, Default, Clone, Copy)]
pub(crate) struct CharacterActivity {
    pub distance: f32,
    pub vertical: f32,
    pub sprinting: bool,
    pub jumped: bool,
    pub climbing: bool,
    pub simulated: bool,
}

#[derive(Component, Clone, Copy, PartialEq)]
pub struct Locomotion {
    /// Controllers may suspend travel without removing the character composition.
    pub enabled: bool,
    pub speed: f64,
    pub jump_boost: f64,
    pub sprint_allowed: bool,
    pub slow_falling: bool,
}

impl Default for Locomotion {
    fn default() -> Self {
        Self {
            enabled: true,
            speed: 1.0,
            jump_boost: 0.0,
            sprint_allowed: true,
            slow_falling: false,
        }
    }
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct AutoJump;

/// Creative flight: no gravity, jump and sneak move up and down, fluids are ignored.
/// Collisions stay.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Flying;

#[derive(Component, Default)]
#[require(
    FixedPosition,
    CharacterBody,
    CharacterMotion,
    CharacterState,
    CharacterMovement,
    CharacterActivity,
    Locomotion
)]
pub struct GameCharacter;

/// Native fixed-step motion, ordered after recovery and before presentation.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CharacterSystems;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.configure_sets(
            FixedUpdate,
            CharacterSystems.in_set(crate::simulation::SimulationSystems::Actors),
        )
        .add_systems(
            FixedUpdate,
            character_physics
                .in_set(CharacterSystems)
                .run_if(crate::simulation::running),
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Aabb {
    pub min: DVec3,
    pub max: DVec3,
}

impl Aabb {
    fn cell(cell: IVec3) -> Self {
        let min = cell.as_dvec3();
        Self {
            min,
            max: min + DVec3::ONE,
        }
    }

    fn moved(self, delta: DVec3) -> Self {
        Self {
            min: self.min + delta,
            max: self.max + delta,
        }
    }

    pub(crate) fn expanded_towards(self, delta: DVec3) -> Self {
        Self {
            min: self.min + delta.min(DVec3::ZERO),
            max: self.max + delta.max(DVec3::ZERO),
        }
    }

    pub(crate) fn intersects(&self, other: &Self) -> bool {
        (self.min + CONTACT_EPSILON).cmplt(other.max).all()
            && (self.max - CONTACT_EPSILON).cmpgt(other.min).all()
    }

    /// Clips a move along one axis so this box stops at the first face of `other`.
    fn clip(&self, other: &Self, axis: usize, delta: f64) -> f64 {
        let [a, b] = [(axis + 1) % 3, (axis + 2) % 3];
        if self.max[a] - CONTACT_EPSILON <= other.min[a]
            || self.min[a] + CONTACT_EPSILON >= other.max[a]
            || self.max[b] - CONTACT_EPSILON <= other.min[b]
            || self.min[b] + CONTACT_EPSILON >= other.max[b]
        {
            return delta;
        }
        if delta > 0.0 && self.max[axis] - CONTACT_EPSILON <= other.min[axis] {
            delta.min(other.min[axis] - self.max[axis])
        } else if delta < 0.0 && self.min[axis] + CONTACT_EPSILON >= other.max[axis] {
            delta.max(other.max[axis] - self.min[axis])
        } else {
            delta
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct FluidCell {
    cell: Aabb,
    /// Fluid surface height.
    top: f64,
    fluid: hollow_core::FluidId,
}

/// Collision boxes and fluids around one character for one tick.
#[derive(Default)]
pub(crate) struct Terrain {
    boxes: Vec<Aabb>,
    fluids: Vec<FluidCell>,
    climbables: Vec<Aabb>,
}

impl Terrain {
    pub(crate) fn gather(&mut self, world: &VoxelWorld, region: Aabb) {
        self.gather_region::<true>(world, region);
    }

    pub(crate) fn gather_collisions(&mut self, world: &VoxelWorld, region: Aabb) {
        self.gather_region::<false>(world, region);
    }

    fn gather_region<const ENVIRONMENT: bool>(&mut self, world: &VoxelWorld, region: Aabb) {
        self.boxes.clear();
        self.fluids.clear();
        self.climbables.clear();
        // Fences and walls rise 0.5 above their cell: read one more cell below.
        let min = region.min.floor().as_ivec3() - IVec3::Y;
        let max = region.max.floor().as_ivec3();
        // Columns are visited top to bottom: remember whether the cell above holds fluid.
        let mut above: Option<(IVec3, hollow_core::FluidId)> = None;
        world.visit_cells(min, max, |cell, state| {
            let Some(state) = state else {
                // Unloaded terrain is a wall: never fall through the world.
                self.boxes.push(Aabb::cell(cell));
                above = None;
                return;
            };
            if ENVIRONMENT {
                if crate::block_behavior::catalog::state_behavior(state).climbable {
                    self.climbables.push(Aabb::cell(cell));
                }
                let fluid = fluid_of(state);
                let covered = above
                    .is_some_and(|(position, id)| position == cell + IVec3::Y && fluid == Some(id));
                above = fluid.map(|id| (cell, id));
                if let Some(id) = fluid {
                    let height = if covered {
                        1.0
                    } else {
                        crate::content::fluid_catalog().fill_height(state)
                    };
                    self.fluids.push(FluidCell {
                        cell: Aabb::cell(cell),
                        top: cell.y as f64 + height,
                        fluid: id,
                    });
                }
            }
            if state.full_collision() {
                self.boxes.push(Aabb::cell(cell));
            } else if !state.collision_empty() {
                let origin = cell.as_dvec3();
                self.boxes
                    .extend(state.collision_boxes(cell).map(|shape| Aabb {
                        min: origin + shape.min.as_dvec3(),
                        max: origin + shape.max.as_dvec3(),
                    }));
            }
        });
    }

    pub(crate) fn collides(&self, body: &Aabb) -> bool {
        self.boxes.iter().any(|other| body.intersects(other))
    }

    pub(crate) fn has_fluid(&self, body: &Aabb) -> bool {
        self.fluids
            .iter()
            .any(|fluid| body.intersects(&fluid.cell) && body.min.y < fluid.top)
    }

    /// Dominant medium and its deepest surface above the feet, sampled once per tick.
    fn fluid_environment(&self, body: &Aabb) -> (Option<hollow_core::FluidId>, f64) {
        let body = Aabb {
            min: body.min + 0.001,
            max: body.max - 0.001,
        };
        self.fluids
            .iter()
            .filter(|f| body.intersects(&f.cell) && body.min.y < f.top)
            .max_by(|a, b| a.top.total_cmp(&b.top))
            .map_or((None, 0.0), |f| (Some(f.fluid), f.top - body.min.y))
    }
    fn fluid_at(&self, point: DVec3) -> hollow_core::FluidId {
        self.fluids
            .iter()
            .find(|fluid| {
                point.cmpge(fluid.cell.min).all()
                    && point.cmplt(fluid.cell.max).all()
                    && point.y < fluid.top
            })
            .map_or(hollow_core::FluidId::NONE, |fluid| fluid.fluid)
    }
    fn swimmable_at(&self, point: DVec3) -> bool {
        let fluid = self.fluid_at(point);
        fluid != hollow_core::FluidId::NONE
            && crate::content::fluid_catalog()
                .profile(fluid)
                .contact
                .swimmable
    }

    /// Vanilla `collideBoundingBox`: Y first, then the larger horizontal axis last.
    pub(crate) fn collide_box(&self, body: Aabb, delta: DVec3) -> DVec3 {
        let mut body = body;
        let mut result = DVec3::ZERO;
        let order = if delta.x.abs() < delta.z.abs() {
            [1, 2, 0]
        } else {
            [1, 0, 2]
        };
        for axis in order {
            if delta[axis] == 0.0 {
                continue;
            }
            let moved = self
                .boxes
                .iter()
                .fold(delta[axis], |moved, other| body.clip(other, axis, moved));
            let mut offset = DVec3::ZERO;
            offset[axis] = moved;
            body = body.moved(offset);
            result[axis] = moved;
        }
        result
    }

    /// Navigation uses the same collected shapes, clearance and axis clipping as travel.
    /// Returns feet elevation and water penalty for one cardinal transition.
    pub(crate) fn navigation_step(
        &self,
        from: DVec3,
        xz: DVec3,
        body: &CharacterBody,
        swimming: bool,
    ) -> Option<(f64, bool)> {
        let half = body.half_width as f64;
        let bounds_at = |y| {
            body.aabb(
                DVec3::new(xz.x, y + body.origin_height as f64, xz.z),
                Pose::Standing,
            )
        };
        let mut best = None;
        let mut consider = |y: f64| {
            if y > from.y + 1.00001 || y < from.y - 3.00001 || best.is_some_and(|(old, _)| y <= old)
            {
                return;
            }
            let next = bounds_at(y);
            if self.collides(&next) {
                return;
            }
            let (_, depth) = self.fluid_environment(&next);
            if depth > 0.6 && !swimming {
                return;
            }
            let current = body.aabb(from + DVec3::Y * body.origin_height as f64, Pose::Standing);
            let rise = (y - from.y).max(0.0);
            let lift = if rise > body.step_height { 1.25 } else { rise };
            // Check the upward and horizontal sweep separately, then the landing.
            let up = DVec3::Y * lift;
            if (self.collide_box(current, up) - up).length_squared() > 1e-8 {
                return;
            }
            let horizontal = (xz - from).with_y(0.0);
            let raised = current.moved(up);
            if (self.collide_box(raised, horizontal) - horizontal).length_squared() > 1e-8 {
                return;
            }
            let down = DVec3::Y * (y - from.y - lift);
            if (self.collide_box(raised.moved(horizontal), down) - down).length_squared() > 1e-8 {
                return;
            }
            best = Some((y, depth > 0.01));
        };
        for obstacle in &self.boxes {
            if obstacle.min.x < xz.x + half
                && obstacle.max.x > xz.x - half
                && obstacle.min.z < xz.z + half
                && obstacle.max.z > xz.z - half
                && obstacle.min.x <= xz.x
                && obstacle.max.x >= xz.x
                && obstacle.min.z <= xz.z
                && obstacle.max.z >= xz.z
            {
                consider(obstacle.max.y);
            }
        }
        if swimming {
            for fluid in &self.fluids {
                if xz.x >= fluid.cell.min.x
                    && xz.x < fluid.cell.max.x
                    && xz.z >= fluid.cell.min.z
                    && xz.z < fluid.cell.max.z
                    && crate::content::fluid_catalog()
                        .profile(fluid.fluid)
                        .contact
                        .swimmable
                {
                    consider(fluid.top - 0.3);
                }
            }
        }
        best
    }

    /// Recover restored contacts and items embedded by a block placement.
    pub(crate) fn depenetrate(&self, mut body: Aabb) -> DVec3 {
        let mut total = DVec3::ZERO;
        for _ in 0..4 {
            let offset = self
                .boxes
                .iter()
                .filter(|other| body.min.cmplt(other.max).all() && body.max.cmpgt(other.min).all())
                .flat_map(|other| {
                    (0..3).flat_map(move |axis| {
                        [
                            other.min[axis] - body.max[axis],
                            other.max[axis] - body.min[axis],
                        ]
                        .map(move |distance| {
                            let mut offset = DVec3::ZERO;
                            offset[axis] = distance;
                            offset
                        })
                    })
                })
                .min_by(|a, b| a.length_squared().total_cmp(&b.length_squared()));
            let Some(offset) = offset else {
                break;
            };
            body = body.moved(offset);
            total += offset;
        }
        total
    }

    /// Vanilla `Entity.collide`: when blocked sideways on the ground, also try the
    /// same move raised by the step height and keep whichever goes farther.
    fn collide(&self, body: Aabb, delta: DVec3, on_ground: bool, step_height: f64) -> DVec3 {
        let collided = self.collide_box(body, delta);
        let blocked_x = (collided.x - delta.x).abs() > MOVE_EPSILON;
        let blocked_z = (collided.z - delta.z).abs() > MOVE_EPSILON;
        let landing = (collided.y - delta.y).abs() > MOVE_EPSILON && delta.y < 0.0;
        if !(on_ground || landing) || !(blocked_x || blocked_z) {
            return collided;
        }
        let horizontal = DVec3::new(delta.x, 0.0, delta.z);
        let mut stepped = self.collide_box(body, horizontal.with_y(step_height));
        let lift = self.collide_box(body.expanded_towards(horizontal), DVec3::Y * step_height);
        if lift.y < step_height {
            let raised = self.collide_box(body.moved(lift), horizontal) + lift;
            if horizontal_length_squared(raised) > horizontal_length_squared(stepped) {
                stepped = raised;
            }
        }
        if horizontal_length_squared(stepped) > horizontal_length_squared(collided) {
            stepped + self.collide_box(body.moved(stepped), DVec3::Y * (delta.y - stepped.y))
        } else {
            collided
        }
    }

    /// Vanilla `maybeBackOffFromEdge`: a sneaking body never walks off a ledge.
    fn back_off_from_edge(&self, body: Aabb, mut delta: DVec3, step_height: f64) -> DVec3 {
        let down = -step_height;
        let free = |x: f64, z: f64| !self.collides(&body.moved(DVec3::new(x, down, z)));
        let toward_zero = |value: f64| {
            if value.abs() < SNEAK_EDGE_STEP {
                0.0
            } else {
                value - SNEAK_EDGE_STEP * value.signum()
            }
        };
        while delta.x != 0.0 && free(delta.x, 0.0) {
            delta.x = toward_zero(delta.x);
        }
        while delta.z != 0.0 && free(0.0, delta.z) {
            delta.z = toward_zero(delta.z);
        }
        while delta.x != 0.0 && delta.z != 0.0 && free(delta.x, delta.z) {
            delta.x = toward_zero(delta.x);
            delta.z = toward_zero(delta.z);
        }
        delta
    }
}

fn fluid_of(state: VoxelState) -> Option<hollow_core::FluidId> {
    let id = state.definition().fluid.id;
    (id != hollow_core::FluidId::NONE).then_some(id)
}

fn horizontal_length_squared(v: DVec3) -> f64 {
    v.x * v.x + v.z * v.z
}

/// Block properties under the feet: slipperiness, speed factor and jump factor.
fn ground_block(world: &VoxelWorld, body: &Aabb) -> (f64, f64, f64) {
    let center = (body.min + body.max) * 0.5;
    let at = |y: f64| {
        world
            .get(DVec3::new(center.x, y, center.z).floor().as_ivec3())
            .map(|kind| kind.physics())
    };
    let below = at(body.min.y - 0.500001);
    let slipperiness = below.map_or(DEFAULT_SLIPPERINESS, |block| block.slipperiness as f64);
    // Soul sand and honey slow the body standing in or on them.
    let factor = |pick: fn(&hollow_core::content::BlockDefinition) -> f32| {
        let feet = at(body.min.y).map_or(1.0, |block| pick(block) as f64);
        if feet == 1.0 {
            below.map_or(1.0, |block| pick(block) as f64)
        } else {
            feet
        }
    };
    (
        slipperiness,
        factor(|block| block.velocity_multiplier),
        factor(|block| block.jump_velocity_multiplier),
    )
}

/// One reusable borrow of a character's simulation state, with no extra ECS storage.
#[derive(QueryData)]
#[query_data(mutable)]
struct CharacterPhysics {
    transform: &'static Transform,
    motion: &'static mut CharacterMotion,
    fixed_position: &'static mut FixedPosition,
    state: &'static mut CharacterState,
    body: &'static CharacterBody,
    movement: &'static CharacterMovement,
    locomotion: &'static Locomotion,
    activity: &'static mut CharacterActivity,
    fluid_contact: &'static mut FluidContactState,
    auto_jump: Has<AutoJump>,
    flying: Has<Flying>,
}

/// Advance each character with one reusable collision scratch buffer.
fn character_physics(
    world: VoxelWorld,
    terrain: Local<bevy::utils::Parallel<Terrain>>,
    mut characters: Query<CharacterPhysics, With<crate::simulation::Simulating>>,
) {
    if characters.iter().len() < 128 {
        let mut terrain = terrain.borrow_local_mut();
        for mut character in &mut characters {
            character.step(&world, &mut terrain);
        }
    } else {
        characters
            .par_iter_mut()
            .batching_strategy(bevy::ecs::batching::BatchingStrategy::new().min_batch_size(64))
            .for_each_init(
                || terrain.borrow_local_mut(),
                |terrain, mut character| {
                    character.step(&world, terrain);
                },
            );
    }
}

/// Derived once per tick and kept on the stack, avoiding transient ECS changes.
#[derive(Clone, Copy)]
struct TravelFrame {
    velocity: DVec3,
    bounds: Aabb,
    input: DVec3,
    medium: Option<hollow_core::FluidId>,
    eye_fluid: hollow_core::FluidId,
    fluid_depth: f64,
    sneaking: bool,
    sprinting: bool,
    climbing: bool,
    slipperiness: f64,
    speed_factor: f64,
}

impl CharacterPhysicsItem<'_, '_> {
    fn step(&mut self, world: &VoxelWorld, terrain: &mut Terrain) {
        let Self {
            transform,
            motion,
            fixed_position,
            state,
            body,
            locomotion,
            activity,
            fluid_contact,
            ..
        } = self;
        **activity = CharacterActivity::default();
        if !locomotion.enabled {
            return;
        }
        let position = transform.translation;
        // Spawns, respawns and developer tools write the transform directly.
        let teleported = fixed_position.externally_moved(transform);
        if motion.is_added() || teleported {
            if teleported {
                let position = position.as_dvec3();
                if fixed_position.current != position {
                    fixed_position.current = position;
                }
                motion.velocity = DVec3::ZERO;
            }
            if world.collision_ready(position) {
                // Saved positions use f32. Restoring them as f64 can put the feet
                // just inside a supporting face, which axis clipping cannot stop.
                // Recover only penetration within one f32 rounding interval.
                let tolerance = (position.y.abs().next_up() - position.y.abs()) as f64;
                let aabb = body.aabb(fixed_position.current, state.pose);
                terrain.gather(&world, aabb);
                let support = terrain
                    .boxes
                    .iter()
                    .filter(|obstacle| {
                        let penetration = obstacle.max.y - aabb.min.y;
                        penetration > 0.0 && penetration <= tolerance && aabb.intersects(obstacle)
                    })
                    .map(|obstacle| obstacle.max.y)
                    .max_by(f64::total_cmp);
                if let Some(support) = support {
                    let y = support + body.origin_height as f64;
                    if fixed_position.current.y != y {
                        fixed_position.current.y = y;
                    }
                }
            }
            if fixed_position.previous != fixed_position.current {
                fixed_position.previous = fixed_position.current;
            }
            motion.eye = body.eye_offset(state.pose);
            motion.previous_eye = motion.eye;
            motion.walk_distance = 0.0;
            motion.walk_delta = 0.0;
            motion.bob = 0.0;
            motion.previous_bob = 0.0;
        }
        if fixed_position.previous != fixed_position.current {
            fixed_position.previous = fixed_position.current;
        }
        motion.previous_eye = motion.eye;
        if !world.collision_ready(fixed_position.current.as_vec3()) {
            motion.velocity = DVec3::ZERO;
            fluid_contact.set_if_neq(FluidContactState::default());
            fixed_position
                .bypass_change_detection()
                .acknowledge(transform);
            return;
        }
        let contact = self.travel(world, terrain);
        self.fluid_contact.set_if_neq(contact);
        self.fixed_position
            .bypass_change_detection()
            .acknowledge(self.transform);
    }

    /// One vanilla tick: sample the environment, apply intent, then integrate travel.
    fn travel(&mut self, world: &VoxelWorld, terrain: &mut Terrain) -> FluidContactState {
        let mut frame = self.prepare_travel(world, terrain);
        self.integrate_travel(terrain, &mut frame);
        self.finish_travel(frame)
    }

    fn prepare_travel(&mut self, world: &VoxelWorld, terrain: &mut Terrain) -> TravelFrame {
        let Self {
            motion,
            fixed_position,
            state,
            body,
            movement,
            locomotion,
            activity,
            auto_jump,
            flying,
            ..
        } = self;
        let (auto_jump, flying) = (*auto_jump, *flying);
        let mut velocity = motion.velocity;
        // AbstractClientPlayer.updateBob runs before travel, at 20 Hz.
        motion.previous_bob = motion.bob;
        let target_bob = if state.on_ground && !flying && state.pose != Pose::Swimming {
            (velocity.xz().length() as f32).min(0.1)
        } else {
            0.0
        };
        motion.bob += (target_bob - motion.bob) * 0.4;
        let mut aabb = body.aabb(fixed_position.current, state.pose);

        // Everything this tick can touch: the move, a step, a ledge probe, a jump,
        // and a standing box whatever the current pose.
        let reach = velocity.abs() + DVec3::new(0.6, 0.0, 0.6);
        let feet = aabb.min.y;
        let region = Aabb {
            min: aabb.min - reach - DVec3::Y * (body.step_height + 0.1),
            max: (aabb.max + reach).with_y(
                feet + reach.y + body.height(Pose::Standing) as f64 + AUTO_JUMP_HEIGHT + 0.5,
            ),
        };
        terrain.gather(world, region);

        let (medium, fluid_depth) = terrain.fluid_environment(&aabb);
        state.in_fluid = medium.is_some();
        state.in_swimmable_fluid = medium.is_some_and(|id| {
            crate::content::fluid_catalog()
                .profile(id)
                .contact
                .swimmable
        });
        let eyes = DVec3::new(
            fixed_position.current.x,
            feet + body.eye_height(state.pose) as f64,
            fixed_position.current.z,
        );
        let eye_fluid = terrain.fluid_at(eyes);
        state.eyes_in_swimmable_fluid = eye_fluid != hollow_core::FluidId::NONE
            && crate::content::fluid_catalog()
                .profile(eye_fluid)
                .contact
                .swimmable;

        velocity = DVec3::select(
            velocity.abs().cmplt(DVec3::splat(MIN_VELOCITY)),
            DVec3::ZERO,
            velocity,
        );

        let wish = Vec3::new(movement.direction.x, 0.0, movement.direction.z)
            .normalize_or_zero()
            .as_dvec3();
        // Sneaking while flying only descends.
        let sneaking = movement.sneak && !flying;
        // Vanilla `updateSwimming`: start with the eyes under water, keep swimming
        // while sprinting in water.
        let wants_sprint =
            movement.sprint && locomotion.sprint_allowed && !sneaking && wish != DVec3::ZERO;
        state.swimming = wants_sprint
            && !flying
            && state.in_swimmable_fluid
            && (state.swimming || state.eyes_in_swimmable_fluid);
        let desired = if state.swimming {
            Pose::Swimming
        } else if sneaking {
            Pose::Crouching
        } else {
            Pose::Standing
        };
        state.pose = choose_pose(terrain, body, fixed_position.current, desired);
        aabb = body.aabb(fixed_position.current, state.pose);
        let target_eye = body.eye_offset(state.pose);
        motion.eye += (target_eye - motion.eye) * 0.5;

        // Crouching and crawling move at 30 % and cannot sprint.
        let slow = state.pose == Pose::Crouching
            || (state.pose == Pose::Swimming && !state.in_swimmable_fluid);
        let sprinting = wants_sprint && !slow;
        let input = wish * INPUT_SCALE * locomotion.speed * if slow { SNEAK_FACTOR } else { 1.0 };
        let (slipperiness, speed_factor, jump_factor) = ground_block(world, &aabb);

        if flying {
            velocity.y +=
                FLY_VERTICAL * (movement.jump as i32 as f64 - movement.sneak as i32 as f64);
        } else if movement.jump {
            let depth = fluid_depth;
            if state.in_fluid && (!state.on_ground || depth > FLUID_JUMP_THRESHOLD) {
                velocity.y += FLUID_SWIM_UP;
            } else if state.on_ground {
                velocity.y = body.jump_velocity * jump_factor + locomotion.jump_boost;
                activity.jumped = true;
                if sprinting {
                    let facing =
                        Vec3::new(movement.look_direction.x, 0.0, movement.look_direction.z)
                            .normalize_or_zero()
                            .as_dvec3();
                    velocity += facing * SPRINT_JUMP_BOOST;
                }
            }
        } else if auto_jump
            && state.on_ground
            && !sneaking
            && !state.in_fluid
            && wish != DVec3::ZERO
            && should_auto_jump(terrain, aabb, wish, body.step_height)
        {
            velocity.y = body.jump_velocity * jump_factor + locomotion.jump_boost;
            activity.jumped = true;
        }
        if sneaking && state.in_fluid {
            velocity.y -= FLUID_SWIM_UP;
        }

        let climbing = if flying {
            false
        } else {
            let feet = Aabb {
                min: aabb.min + DVec3::splat(CONTACT_EPSILON),
                max: aabb.max.with_y(aabb.min.y + 0.1) - DVec3::splat(CONTACT_EPSILON),
            };
            terrain.climbables.iter().any(|cell| feet.intersects(cell))
        };
        if climbing {
            // Establish vertical intent before the fluid branch samples falling.
            velocity.y = velocity.y.max(if sneaking { 0.0 } else { -CLIMB_SPEED });
            if movement.jump {
                velocity.y = CLIMB_RISE;
            }
        }
        TravelFrame {
            velocity,
            bounds: aabb,
            input,
            medium,
            eye_fluid,
            fluid_depth,
            sneaking,
            sprinting,
            climbing,
            slipperiness,
            speed_factor,
        }
    }

    fn integrate_travel(&mut self, terrain: &Terrain, frame: &mut TravelFrame) {
        let Self {
            fixed_position,
            state,
            movement,
            locomotion,
            flying,
            ..
        } = self;
        let flying = *flying;
        let TravelFrame {
            mut velocity,
            bounds: mut aabb,
            input,
            medium,
            fluid_depth,
            sneaking,
            sprinting,
            climbing,
            slipperiness,
            speed_factor,
            ..
        } = *frame;
        let start_y = aabb.min.y;
        let falling = velocity.y <= 0.0;
        let fluid_movement = if flying {
            None
        } else {
            medium.map(|fluid| &crate::content::fluid_catalog().profile(fluid).movement)
        };
        // Force selection is content-specific; acceleration, collision and drag share one path.
        // These coefficients are tick-local, so changing medium never changes an archetype.
        let (acceleration, drag, gravity) = if flying {
            (
                FLY_SPEED * if sprinting { 2.0 } else { 1.0 },
                DVec3::new(AIR_DRAG, FLY_VERTICAL_DRAG, AIR_DRAG),
                0.0,
            )
        } else if let Some(movement_profile) = fluid_movement {
            if state.swimming {
                // Swimming follows the camera pitch.
                let look = movement.look_direction.normalize_or_zero().y as f64;
                let rate = if look < -0.2 { 0.085 } else { 0.06 };
                let head = DVec3::new(
                    fixed_position.current.x,
                    aabb.min.y + 0.9,
                    fixed_position.current.z,
                );
                if look <= 0.0 || movement.jump || terrain.swimmable_at(head) {
                    velocity.y += (look - velocity.y) * rate;
                }
            }
            let horizontal = if sprinting {
                movement_profile.sprint_drag
            } else {
                movement_profile.horizontal_drag
            };
            let vertical = if fluid_depth <= FLUID_JUMP_THRESHOLD {
                movement_profile.vertical_drag
            } else {
                movement_profile.deep_vertical_drag
            };
            (
                movement_profile.acceleration,
                DVec3::new(horizontal, vertical, horizontal),
                0.0,
            )
        } else {
            let (friction, speed) = if state.on_ground {
                let speed = WALK_SPEED * if sprinting { SPRINT_FACTOR } else { 1.0 };
                (
                    slipperiness * AIR_DRAG,
                    speed * (0.216 / slipperiness.powi(3)),
                )
            } else {
                (
                    AIR_DRAG,
                    if sprinting {
                        AIR_SPRINT_SPEED
                    } else {
                        AIR_SPEED
                    },
                )
            };
            let gravity = if locomotion.slow_falling && falling {
                0.01
            } else {
                GRAVITY
            };
            (
                speed,
                DVec3::new(friction, VERTICAL_DRAG, friction),
                gravity,
            )
        };
        velocity += input * acceleration;
        clamp_climbing(&mut velocity, climbing, sneaking);
        move_body(
            terrain,
            &mut aabb,
            &mut velocity,
            state,
            sneaking,
            speed_factor,
            self.body.step_height,
        );
        velocity.y -= gravity;
        velocity *= drag;
        if let Some(movement_profile) = fluid_movement {
            // Fluid gravity is applied after drag; air gravity is applied before it.
            if !sprinting || !movement_profile.sprint_cancels_gravity {
                velocity.y = if falling
                    && (velocity.y - 0.005).abs() >= MIN_VELOCITY
                    && (velocity.y - movement_profile.gravity).abs() < MIN_VELOCITY
                {
                    -MIN_VELOCITY
                } else {
                    velocity.y - movement_profile.gravity
                };
            }
            climb_out_of_fluid(
                terrain,
                aabb,
                &mut velocity,
                state,
                start_y,
                self.body.step_height,
            );
        }

        if climbing && !sneaking && (state.horizontal_collision || movement.jump) {
            velocity.y = CLIMB_RISE;
        }
        if climbing && sneaking {
            velocity.y = velocity.y.max(0.0);
        }
        frame.bounds = aabb;
        frame.velocity = velocity;
    }

    fn finish_travel(&mut self, frame: TravelFrame) -> FluidContactState {
        let Self {
            motion,
            fixed_position,
            body,
            activity,
            flying,
            ..
        } = self;
        let TravelFrame {
            velocity,
            bounds: aabb,
            medium,
            eye_fluid,
            fluid_depth,
            sprinting,
            climbing,
            ..
        } = frame;
        let flying = *flying;
        let position = DVec3::new(
            (aabb.min.x + aabb.max.x) * 0.5,
            aabb.min.y + body.origin_height as f64,
            (aabb.min.z + aabb.max.z) * 0.5,
        );
        if fixed_position.current != position {
            fixed_position.current = position;
        }
        motion.velocity = velocity;
        let displacement = fixed_position.current - fixed_position.previous;
        activity.distance = displacement.xz().length() as f32;
        activity.vertical = displacement.y as f32;
        activity.sprinting = sprinting;
        activity.climbing = climbing;
        activity.simulated = !flying;
        motion.walk_delta = activity.distance * 0.6;
        // Only the phase matters; wrapping preserves precision during long sessions.
        motion.walk_distance = (motion.walk_distance + motion.walk_delta).rem_euclid(2.0);
        FluidContactState {
            fluid: medium.unwrap_or(hollow_core::FluidId::NONE),
            eyes: eye_fluid,
            depth: fluid_depth as f32,
        }
    }
}

fn clamp_climbing(velocity: &mut DVec3, climbing: bool, sneaking: bool) {
    if climbing {
        velocity.x = velocity.x.clamp(-CLIMB_SPEED, CLIMB_SPEED);
        velocity.z = velocity.z.clamp(-CLIMB_SPEED, CLIMB_SPEED);
        velocity.y = velocity.y.max(if sneaking { 0.0 } else { -CLIMB_SPEED });
    }
}

/// Vanilla `Entity.move` for one tick of velocity.
fn move_body(
    terrain: &Terrain,
    aabb: &mut Aabb,
    velocity: &mut DVec3,
    state: &mut CharacterState,
    sneaking: bool,
    speed_factor: f64,
    step_height: f64,
) {
    let mut delta = *velocity;
    if sneaking && state.on_ground && delta.y <= 0.0 {
        delta = terrain.back_off_from_edge(*aabb, delta, step_height);
    }
    let moved = terrain.collide(*aabb, delta, state.on_ground, step_height);
    *aabb = aabb.moved(moved);

    let blocked = (moved - delta).abs().cmpgt(DVec3::splat(MOVE_EPSILON));
    state.horizontal_collision = blocked.x || blocked.z;
    state.on_ground = blocked.y && delta.y < 0.0;
    if blocked.x {
        velocity.x = 0.0;
    }
    if blocked.z {
        velocity.z = 0.0;
    }
    if blocked.y {
        velocity.y = 0.0;
    }
    velocity.x *= speed_factor;
    velocity.z *= speed_factor;
}

/// Swimming against a bank pops the body up onto it.
fn climb_out_of_fluid(
    terrain: &Terrain,
    aabb: Aabb,
    velocity: &mut DVec3,
    state: &CharacterState,
    start_y: f64,
    step_height: f64,
) {
    if !state.horizontal_collision {
        return;
    }
    let probe = aabb.moved(DVec3::new(
        velocity.x,
        velocity.y + step_height - (aabb.min.y - start_y),
        velocity.z,
    ));
    if !terrain.collides(&probe) && !terrain.has_fluid(&probe) {
        velocity.y = SHORE_JUMP;
    }
}

/// Vanilla `updatePlayerPose`: a pose whose box does not fit falls back to
/// crouching, then to crawling.
fn choose_pose(terrain: &Terrain, body: &CharacterBody, origin: DVec3, desired: Pose) -> Pose {
    let fits = |pose| !terrain.collides(&body.aabb(origin, pose));
    if fits(desired) {
        desired
    } else if fits(Pose::Crouching) {
        Pose::Crouching
    } else {
        Pose::Swimming
    }
}

/// A one-block rise ahead that a plain step cannot climb but a jump clears.
fn should_auto_jump(terrain: &Terrain, aabb: Aabb, wish: DVec3, step_height: f64) -> bool {
    let ahead = aabb.moved(wish * AUTO_JUMP_PROBE);
    terrain.collides(&ahead)
        && terrain.collides(&ahead.moved(DVec3::Y * step_height))
        && !terrain.collides(&ahead.moved(DVec3::Y * AUTO_JUMP_HEIGHT))
        && !terrain.collides(&aabb.moved(DVec3::Y * AUTO_JUMP_HEIGHT))
}
