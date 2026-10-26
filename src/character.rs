//! Vanilla entity movement: an AABB moved axis by axis against block collision
//! boxes, at Minecraft's 20 ticks per second. No physics engine is involved, so
//! there are no internal-edge contacts between neighboring voxels or chunks.
#![allow(clippy::type_complexity)]

use bevy::{math::DVec3, prelude::*};

use crate::voxel::{VoxelKind, VoxelState, VoxelWorld};

/// All movement constants below are per vanilla game tick.
const TICK: f32 = 1.0 / 20.0;
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
const FLUID_SPEED: f64 = 0.02;
const WATER_DRAG: f64 = 0.8;
const WATER_SPRINT_DRAG: f64 = 0.9;
const WATER_GRAVITY: f64 = GRAVITY / 16.0;
const LAVA_DRAG: f64 = 0.5;
const LAVA_GRAVITY: f64 = GRAVITY / 4.0;
const FLUID_SWIM_UP: f64 = 0.04;
const FLUID_JUMP_THRESHOLD: f64 = 0.4;
const SHORE_JUMP: f64 = 0.3;
const CLIMB_SPEED: f64 = 0.15;
const CLIMB_RISE: f64 = 0.2;
const SOURCE_FLUID_HEIGHT: f64 = 8.0 / 9.0;
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

impl Pose {
    pub const fn height(self) -> f32 {
        match self {
            Self::Standing => 1.8,
            Self::Crouching => 1.5,
            Self::Swimming => 0.6,
        }
    }

    pub const fn eye_height(self) -> f32 {
        match self {
            Self::Standing => 1.62,
            Self::Crouching => 1.27,
            Self::Swimming => 0.4,
        }
    }
}

/// Collision box footprint. The entity's `Transform::translation` sits `origin_height` above
/// the feet whatever the pose, so changing pose never moves the entity.
#[derive(Component, Debug, Clone, Copy)]
pub struct CharacterBody {
    pub half_width: f32,
    pub origin_height: f32,
}

impl Default for CharacterBody {
    fn default() -> Self {
        Self {
            half_width: 0.3,
            origin_height: 0.9,
        }
    }
}

impl CharacterBody {
    pub(crate) fn aabb(&self, origin: DVec3, pose: Pose) -> Aabb {
        let feet = origin - DVec3::Y * self.origin_height as f64;
        let half_width = self.half_width as f64;
        Aabb {
            min: feet - DVec3::new(half_width, 0.0, half_width),
            max: feet + DVec3::new(half_width, pose.height() as f64, half_width),
        }
    }

    fn eye_offset(&self, pose: Pose) -> f32 {
        pose.eye_height() - self.origin_height
    }
}

/// Tick-exact simulation state, in double precision so that resting contacts
/// stay exact far from the origin. `Transform::translation` is eased between two ticks.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterMotion {
    /// Entity origin at the last tick.
    pub position: DVec3,
    previous: DVec3,
    /// Blocks per tick.
    pub velocity: DVec3,
    elapsed: f32,
    /// Camera height above the origin: it eases toward the pose's eyes, as in vanilla.
    eye: f32,
    previous_eye: f32,
    walk_distance: f32,
    walk_delta: f32,
    bob: f32,
    previous_bob: f32,
    /// `Transform::translation` written last; any other value is a teleport.
    written: Option<Vec3>,
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterState {
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub in_water: bool,
    pub in_lava: bool,
    pub eyes_in_water: bool,
    pub swimming: bool,
    pub pose: Pose,
}

impl CharacterMotion {
    /// Minecraft's backwards walk interpolation and tick-smoothed bob amplitude.
    pub(crate) fn walking_animation(&self) -> (f32, f32) {
        let alpha = (self.elapsed / TICK).min(1.0);
        (
            -(self.walk_distance + self.walk_delta * alpha),
            self.previous_bob.lerp(self.bob, alpha),
        )
    }

    /// Eased eye height above the entity origin, for the camera.
    pub fn eye_offset(&self) -> f32 {
        self.previous_eye
            .lerp(self.eye, (self.elapsed / TICK).min(1.0))
    }
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterMovement {
    pub direction: Vec3,
    pub look_direction: Vec3,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct AutoJump;

/// Creative flight: no gravity, jump and sneak move up and down, fluids are ignored.
/// Collisions stay.
#[derive(Component, Default, Debug, Clone, Copy)]
pub struct Flying;

#[derive(Component, Default)]
#[require(
    Transform,
    CharacterBody,
    CharacterMotion,
    CharacterState,
    CharacterMovement
)]
pub struct GameCharacter;

/// Native fixed-step motion, ordered after recovery and before presentation.
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CharacterSystems;

/// Development tools can suspend motion without pausing world streaming.
#[derive(Resource)]
pub(crate) struct SimulationPaused;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(Time::<Fixed>::from_hz(20.0))
            .add_systems(
                FixedUpdate,
                character_physics
                    .in_set(CharacterSystems)
                    .run_if(crate::world::playing)
                    .run_if(not(resource_exists::<SimulationPaused>)),
            )
            .add_systems(
                RunFixedMainLoop,
                interpolate_characters
                    .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop)
                    .run_if(crate::world::playing)
                    .run_if(not(resource_exists::<SimulationPaused>)),
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
    lava: bool,
}

/// Collision boxes and fluids around one character for one tick.
#[derive(Default)]
pub(crate) struct Terrain {
    boxes: Vec<Aabb>,
    fluids: Vec<FluidCell>,
    climbables: Vec<Aabb>,
}

impl Terrain {
    fn gather(&mut self, world: &VoxelWorld, region: Aabb) {
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
        let mut above: Option<(IVec3, bool)> = None;
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
                let covered = above.is_some_and(|(position, lava)| {
                    position == cell + IVec3::Y && fluid == Some(lava)
                });
                above = fluid.map(|lava| (cell, lava));
                if let Some(lava) = fluid {
                    let height = if covered { 1.0 } else { SOURCE_FLUID_HEIGHT };
                    self.fluids.push(FluidCell {
                        cell: Aabb::cell(cell),
                        top: cell.y as f64 + height,
                        lava,
                    });
                }
            }
            if state.full_collision() {
                self.boxes.push(Aabb::cell(cell));
            } else if !state.native().collision_shapes.is_empty() {
                let origin = cell.as_dvec3();
                self.boxes
                    .extend(state.collision_boxes(cell).map(|shape| Aabb {
                        min: origin + shape.min.as_dvec3(),
                        max: origin + shape.max.as_dvec3(),
                    }));
            }
        });
    }

    fn collides(&self, body: &Aabb) -> bool {
        self.boxes.iter().any(|other| body.intersects(other))
    }

    fn has_fluid(&self, body: &Aabb) -> bool {
        self.fluids
            .iter()
            .any(|fluid| body.intersects(&fluid.cell) && body.min.y < fluid.top)
    }

    /// Deepest fluid surface above the feet, for water and lava.
    fn fluid_depths(&self, body: &Aabb) -> (f64, f64) {
        let body = Aabb {
            min: body.min + 0.001,
            max: body.max - 0.001,
        };
        let mut depths = (0.0f64, 0.0f64);
        for fluid in &self.fluids {
            if body.intersects(&fluid.cell) && fluid.top >= body.min.y {
                let depth = if fluid.lava {
                    &mut depths.1
                } else {
                    &mut depths.0
                };
                *depth = depth.max(fluid.top - body.min.y);
            }
        }
        depths
    }

    fn water_at(&self, point: DVec3) -> bool {
        self.fluids.iter().any(|fluid| {
            !fluid.lava
                && point.cmpge(fluid.cell.min).all()
                && point.cmplt(fluid.cell.max).all()
                && point.y < fluid.top
        })
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
    fn collide(&self, body: Aabb, delta: DVec3, on_ground: bool) -> DVec3 {
        let collided = self.collide_box(body, delta);
        let blocked_x = (collided.x - delta.x).abs() > MOVE_EPSILON;
        let blocked_z = (collided.z - delta.z).abs() > MOVE_EPSILON;
        let landing = (collided.y - delta.y).abs() > MOVE_EPSILON && delta.y < 0.0;
        if !(on_ground || landing) || !(blocked_x || blocked_z) {
            return collided;
        }
        let horizontal = DVec3::new(delta.x, 0.0, delta.z);
        let mut stepped = self.collide_box(body, horizontal.with_y(STEP_HEIGHT));
        let lift = self.collide_box(body.expanded_towards(horizontal), DVec3::Y * STEP_HEIGHT);
        if lift.y < STEP_HEIGHT {
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
    fn back_off_from_edge(&self, body: Aabb, mut delta: DVec3) -> DVec3 {
        let down = -STEP_HEIGHT;
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

fn fluid_of(state: VoxelState) -> Option<bool> {
    if state.kind() == VoxelKind::Lava {
        Some(true)
    } else if state.has_water() {
        Some(false)
    } else {
        None
    }
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
            .map(|kind| kind.block())
    };
    let below = at(body.min.y - 0.500001);
    let slipperiness = below.map_or(DEFAULT_SLIPPERINESS, |block| block.slipperiness as f64);
    // Soul sand and honey slow the body standing in or on them.
    let factor = |pick: fn(&pumpkin_data::Block) -> f32| {
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

/// Advances every character by one vanilla tick using Bevy's fixed schedule.
fn character_physics(
    world: VoxelWorld,
    mut terrain: Local<Terrain>,
    mut characters: Query<(
        &Transform,
        &mut CharacterMotion,
        &mut CharacterState,
        &CharacterBody,
        &CharacterMovement,
        Has<AutoJump>,
        Has<Flying>,
    )>,
) {
    for (transform, mut motion, mut state, body, movement, auto_jump, flying) in &mut characters {
        let position = transform.translation;
        // Spawns, respawns and developer tools write the transform directly.
        if motion
            .written
            .is_none_or(|written| written.distance_squared(position) > 1e-6)
        {
            motion.position = position.as_dvec3();
            if world.collision_ready(position) {
                // Saved positions use f32. Restoring them as f64 can put the feet
                // just inside a supporting face, which axis clipping cannot stop.
                // Recover only penetration within one f32 rounding interval.
                let tolerance = (position.y.abs().next_up() - position.y.abs()) as f64;
                let aabb = body.aabb(motion.position, state.pose);
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
                    motion.position.y = support + body.origin_height as f64;
                }
            }
            motion.previous = motion.position;
            motion.velocity = DVec3::ZERO;
            motion.elapsed = 0.0;
            motion.eye = body.eye_offset(state.pose);
            motion.previous_eye = motion.eye;
            motion.walk_distance = 0.0;
            motion.walk_delta = 0.0;
            motion.bob = 0.0;
            motion.previous_bob = 0.0;
        }
        motion.previous = motion.position;
        motion.previous_eye = motion.eye;
        if !world.collision_ready(motion.position.as_vec3()) {
            motion.velocity = DVec3::ZERO;
            motion.written = Some(position);
            continue;
        }
        tick(
            &world,
            &mut terrain,
            &mut motion,
            &mut state,
            body,
            movement,
            auto_jump,
            flying,
        );
        motion.written = Some(position);
    }
}

/// Present fixed-step motion every frame; mouse-driven rotation stays untouched.
fn interpolate_characters(
    time: Res<Time<Fixed>>,
    mut characters: Query<(&mut Transform, &mut CharacterMotion)>,
) {
    let alpha = time.overstep_fraction();
    for (mut transform, mut motion) in &mut characters {
        if motion
            .written
            .is_none_or(|written| written.distance_squared(transform.translation) > 1e-6)
        {
            continue;
        }
        motion.elapsed = alpha * TICK;
        let position = motion
            .previous
            .lerp(motion.position, alpha as f64)
            .as_vec3();
        if transform.translation != position {
            transform.translation = position;
        }
        motion.written = Some(position);
    }
}

/// One vanilla tick: `LivingEntity.aiStep` then `travel`.
#[allow(clippy::too_many_arguments)]
fn tick(
    world: &VoxelWorld,
    terrain: &mut Terrain,
    motion: &mut CharacterMotion,
    state: &mut CharacterState,
    body: &CharacterBody,
    movement: &CharacterMovement,
    auto_jump: bool,
    flying: bool,
) {
    let mut velocity = motion.velocity;
    // AbstractClientPlayer.updateBob runs before travel, at 20 Hz.
    motion.previous_bob = motion.bob;
    let target_bob = if state.on_ground && !flying && state.pose != Pose::Swimming {
        (velocity.xz().length() as f32).min(0.1)
    } else {
        0.0
    };
    motion.bob += (target_bob - motion.bob) * 0.4;
    let mut aabb = body.aabb(motion.position, state.pose);

    // Everything this tick can touch: the move, a step, a ledge probe, a jump,
    // and a standing box whatever the current pose.
    let reach = velocity.abs() + DVec3::new(0.6, 0.0, 0.6);
    let feet = aabb.min.y;
    let region = Aabb {
        min: aabb.min - reach - DVec3::Y * (STEP_HEIGHT + 0.1),
        max: (aabb.max + reach)
            .with_y(feet + reach.y + Pose::Standing.height() as f64 + AUTO_JUMP_HEIGHT + 0.5),
    };
    terrain.gather(world, region);

    let (water_depth, lava_depth) = terrain.fluid_depths(&aabb);
    state.in_water = water_depth > 0.0;
    state.in_lava = lava_depth > 0.0;
    let eyes = DVec3::new(
        motion.position.x,
        feet + state.pose.eye_height() as f64,
        motion.position.z,
    );
    state.eyes_in_water = terrain.water_at(eyes);

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
    let wants_sprint = movement.sprint && !sneaking && wish != DVec3::ZERO;
    state.swimming =
        wants_sprint && !flying && state.in_water && (state.swimming || state.eyes_in_water);
    let desired = if state.swimming {
        Pose::Swimming
    } else if sneaking {
        Pose::Crouching
    } else {
        Pose::Standing
    };
    state.pose = choose_pose(terrain, body, motion.position, desired);
    aabb = body.aabb(motion.position, state.pose);
    let target_eye = body.eye_offset(state.pose);
    motion.eye += (target_eye - motion.eye) * 0.5;

    // Crouching and crawling move at 30 % and cannot sprint.
    let slow = state.pose == Pose::Crouching || (state.pose == Pose::Swimming && !state.in_water);
    let sprinting = wants_sprint && !slow;
    let input = wish * INPUT_SCALE * if slow { SNEAK_FACTOR } else { 1.0 };
    let (slipperiness, speed_factor, jump_factor) = ground_block(world, &aabb);

    if flying {
        velocity.y += FLY_VERTICAL * (movement.jump as i32 as f64 - movement.sneak as i32 as f64);
    } else if movement.jump {
        let depth = if state.in_water {
            water_depth
        } else {
            lava_depth
        };
        if (state.in_water || state.in_lava) && (!state.on_ground || depth > FLUID_JUMP_THRESHOLD) {
            velocity.y += FLUID_SWIM_UP;
        } else if state.on_ground {
            velocity.y = JUMP_VELOCITY * jump_factor;
            if sprinting {
                let facing = Vec3::new(movement.look_direction.x, 0.0, movement.look_direction.z)
                    .normalize_or_zero()
                    .as_dvec3();
                velocity += facing * SPRINT_JUMP_BOOST;
            }
        }
    } else if auto_jump
        && state.on_ground
        && !sneaking
        && !state.in_water
        && !state.in_lava
        && wish != DVec3::ZERO
        && should_auto_jump(terrain, aabb, wish)
    {
        velocity.y = JUMP_VELOCITY * jump_factor;
    }
    if sneaking && (state.in_water || state.in_lava) {
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
    let start_y = aabb.min.y;
    let falling = velocity.y <= 0.0;
    if flying {
        let speed = FLY_SPEED * if sprinting { 2.0 } else { 1.0 };
        velocity += input * speed;
        move_body(
            terrain,
            &mut aabb,
            &mut velocity,
            state,
            false,
            speed_factor,
        );
        velocity.y *= FLY_VERTICAL_DRAG;
        velocity.x *= AIR_DRAG;
        velocity.z *= AIR_DRAG;
    } else if state.in_water {
        if state.swimming {
            // Swimming follows the camera pitch.
            let look = movement.look_direction.normalize_or_zero().y as f64;
            let rate = if look < -0.2 { 0.085 } else { 0.06 };
            let head = DVec3::new(motion.position.x, aabb.min.y + 0.9, motion.position.z);
            if look <= 0.0 || movement.jump || terrain.water_at(head) {
                velocity.y += (look - velocity.y) * rate;
            }
        }
        let drag = if sprinting {
            WATER_SPRINT_DRAG
        } else {
            WATER_DRAG
        };
        velocity += input * FLUID_SPEED;
        clamp_climbing(&mut velocity, climbing, sneaking);
        move_body(
            terrain,
            &mut aabb,
            &mut velocity,
            state,
            sneaking,
            speed_factor,
        );
        velocity *= DVec3::new(drag, WATER_DRAG, drag);
        if !sprinting {
            velocity.y = if falling
                && (velocity.y - 0.005).abs() >= MIN_VELOCITY
                && (velocity.y - WATER_GRAVITY).abs() < MIN_VELOCITY
            {
                -MIN_VELOCITY
            } else {
                velocity.y - WATER_GRAVITY
            };
        }
        climb_out_of_fluid(terrain, aabb, &mut velocity, state, start_y);
    } else if state.in_lava {
        velocity += input * FLUID_SPEED;
        clamp_climbing(&mut velocity, climbing, sneaking);
        move_body(
            terrain,
            &mut aabb,
            &mut velocity,
            state,
            sneaking,
            speed_factor,
        );
        velocity *= if lava_depth <= FLUID_JUMP_THRESHOLD {
            DVec3::new(LAVA_DRAG, WATER_DRAG, LAVA_DRAG)
        } else {
            DVec3::splat(LAVA_DRAG)
        };
        velocity.y -= LAVA_GRAVITY;
        climb_out_of_fluid(terrain, aabb, &mut velocity, state, start_y);
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
        velocity += input * speed;
        clamp_climbing(&mut velocity, climbing, sneaking);
        move_body(
            terrain,
            &mut aabb,
            &mut velocity,
            state,
            sneaking,
            speed_factor,
        );
        velocity.y = (velocity.y - GRAVITY) * VERTICAL_DRAG;
        velocity.x *= friction;
        velocity.z *= friction;
    }

    if climbing && !sneaking && (state.horizontal_collision || movement.jump) {
        velocity.y = CLIMB_RISE;
    }
    if climbing && sneaking {
        velocity.y = velocity.y.max(0.0);
    }
    motion.position = DVec3::new(
        (aabb.min.x + aabb.max.x) * 0.5,
        aabb.min.y + body.origin_height as f64,
        (aabb.min.z + aabb.max.z) * 0.5,
    );
    motion.velocity = velocity;
    motion.walk_delta = (motion.position - motion.previous).xz().length() as f32 * 0.6;
    // Only the phase matters; wrapping preserves precision during long sessions.
    motion.walk_distance = (motion.walk_distance + motion.walk_delta).rem_euclid(2.0);
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
) {
    let mut delta = *velocity;
    if sneaking && state.on_ground && delta.y <= 0.0 {
        delta = terrain.back_off_from_edge(*aabb, delta);
    }
    let moved = terrain.collide(*aabb, delta, state.on_ground);
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
) {
    if !state.horizontal_collision {
        return;
    }
    let probe = aabb.moved(DVec3::new(
        velocity.x,
        velocity.y + STEP_HEIGHT - (aabb.min.y - start_y),
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
fn should_auto_jump(terrain: &Terrain, aabb: Aabb, wish: DVec3) -> bool {
    let ahead = aabb.moved(wish * AUTO_JUMP_PROBE);
    terrain.collides(&ahead)
        && terrain.collides(&ahead.moved(DVec3::Y * STEP_HEIGHT))
        && !terrain.collides(&ahead.moved(DVec3::Y * AUTO_JUMP_HEIGHT))
        && !terrain.collides(&aabb.moved(DVec3::Y * AUTO_JUMP_HEIGHT))
}
