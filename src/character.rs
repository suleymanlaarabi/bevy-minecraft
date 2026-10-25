#![allow(clippy::type_complexity)]

use avian3d::{
    collision::collider::Collider,
    dynamics::rigid_body::{
        GravityScale, LinearDamping, LinearVelocity,
        forces::{Forces, ReadRigidBodyForces, WriteRigidBodyForces},
    },
    prelude::{NarrowPhaseConfig, PhysicsLengthUnit, Position, Rotation},
    schedule::PhysicsSystems,
    spatial_query::{ShapeCastConfig, SpatialQuery, SpatialQueryFilter},
};
use bevy::prelude::*;

use crate::{spatial::GameLayer, voxel::VoxelWorld};

pub const CHARACTER_GRAVITY_SCALE: f32 = 2.8;
pub const CHARACTER_WATER_GRAVITY_SCALE: f32 = 0.15;
const WATER_DAMPING: f32 = 4.0;
const SWIM_VERTICAL_ACCELERATION: f32 = 8.0;
const STEP_CLEARANCE: f32 = 0.02;
// Rest beyond Avian's contact tolerance so an edge below the feet cannot
// create a speculative sideways contact and steal walking speed.
const CONTACT_CLEARANCE: f32 = 0.005;
const GROUND_REACH: f32 = 0.1;
// Ignore tiny upward corrections from the contact solver, but never a jump.
const GROUND_UP_SPEED: f32 = 0.5;

pub struct CharacterPlugin;

#[derive(Component)]
pub struct OnGround;

#[derive(Component, Default)]
#[require(GroundSupport)]
pub struct OnGroundSensor;

#[derive(Component, Default)]
struct GroundSupport(Option<f32>);

#[derive(Component)]
pub struct InWater;

#[derive(Component, Default)]
pub struct InWaterSensor;

#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub struct CharacterController {
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub sneak_speed: f32,
    pub jump_speed: f32,

    pub acceleration: f32,
    pub air_acceleration: f32,
    pub friction: f32,

    pub swim_speed: f32,
    pub swim_sprint_speed: f32,

    pub max_step_height: f32,
    /// Ledges climbed directly, without applying the jump impulse.
    pub walk_step_height: f32,
    pub step_probe_distance: f32,
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterMovement {
    pub direction: Vec3,
    pub look_direction: Vec3,
    pub jump: bool,
    pub sprint: bool,
    pub sneak: bool,
}

impl Default for CharacterController {
    fn default() -> Self {
        Self {
            walk_speed: 4.3,
            sprint_speed: 6.8,
            sneak_speed: 1.5,
            jump_speed: 8.2,
            acceleration: 60.0,
            air_acceleration: 15.0,
            friction: 25.0,
            swim_speed: 2.2,
            swim_sprint_speed: 5.612,
            max_step_height: 1.0,
            walk_step_height: 0.6,
            step_probe_distance: 0.25,
        }
    }
}

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct AutoJump;

#[derive(Component, Default)]
#[require(OnGroundSensor, InWaterSensor, CharacterController, CharacterMovement)]
pub struct GameCharacter;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (add_in_water_state, remove_in_water_state).run_if(crate::world::playing),
        )
        .add_systems(
            FixedPostUpdate,
            (
                update_grounded_state,
                (character_land_movement, character_swim_movement),
            )
                .chain()
                .after(PhysicsSystems::Prepare)
                .before(PhysicsSystems::StepSimulation)
                .run_if(crate::world::playing),
        )
        .add_systems(
            FixedPostUpdate,
            follow_ground
                .after(PhysicsSystems::StepSimulation)
                .before(PhysicsSystems::Writeback)
                .run_if(crate::world::playing),
        );
    }
}

fn update_grounded_state(
    spatial_query: SpatialQuery,
    narrow_phase: Res<NarrowPhaseConfig>,
    length_unit: Res<PhysicsLengthUnit>,
    mut characters: Query<
        (
            Entity,
            &Position,
            &Rotation,
            &Collider,
            &LinearVelocity,
            &mut GroundSupport,
            Has<OnGround>,
        ),
        With<OnGroundSensor>,
    >,
    mut commands: Commands,
) {
    let skin = narrow_phase.contact_tolerance * length_unit.0 + CONTACT_CLEARANCE;
    for (entity, position, rotation, collider, velocity, mut support, was_grounded) in
        &mut characters
    {
        // Read the current physics pose and the full footprint. A narrower
        // sphere misses support at corners, and last tick's ShapeHits lag a step.
        support.0 = if velocity.y <= GROUND_UP_SPEED {
            ground_height(
                &spatial_query,
                entity,
                collider,
                position.0,
                rotation.0,
                skin,
            )
        } else {
            None
        };
        let grounded = support.0.is_some();
        if grounded != was_grounded {
            if grounded {
                commands.entity(entity).insert(OnGround);
            } else {
                commands.entity(entity).remove::<OnGround>();
            }
        }
    }
}

fn add_in_water_state(
    mut query: Query<(Entity, &Transform), (With<InWaterSensor>, Without<InWater>)>,
    voxel_world: VoxelWorld,

    mut commands: Commands,
) {
    for (entity, transform) in &mut query {
        if voxel_world.is_liquid_at(transform.translation) {
            commands.entity(entity).insert(InWater);
        }
    }
}

fn remove_in_water_state(
    mut query: Query<(Entity, &Transform), (With<InWaterSensor>, With<InWater>)>,
    voxel_world: VoxelWorld,

    mut commands: Commands,
) {
    for (entity, transform) in &mut query {
        if !voxel_world.is_liquid_at(transform.translation) {
            commands.entity(entity).remove::<InWater>();
        }
    }
}

fn character_land_movement(
    mut commands: Commands,
    voxel_world: crate::voxel::VoxelWorld,
    time: Res<Time>,
    spatial_query: SpatialQuery,
    narrow_phase: Res<NarrowPhaseConfig>,
    length_unit: Res<PhysicsLengthUnit>,
    mut characters: Query<
        (
            Entity,
            &Position,
            &Rotation,
            &Collider,
            &mut LinearVelocity,
            &CharacterController,
            &CharacterMovement,
            &GroundSupport,
            &mut GravityScale,
            &mut LinearDamping,
            Has<OnGround>,
            Has<AutoJump>,
        ),
        (With<GameCharacter>, Without<InWater>),
    >,
) {
    let dt = time.delta_secs();
    let skin = narrow_phase.contact_tolerance * length_unit.0 + CONTACT_CLEARANCE;

    for (
        entity,
        position,
        rotation,
        collider,
        mut velocity,
        controller,
        movement,
        support,
        mut gravity,
        mut damping,
        grounded,
        auto_jump,
    ) in &mut characters
    {
        if !voxel_world.collision_ready_for_motion(position.0, velocity.0, dt) {
            gravity.0 = 0.0;
            velocity.0 = Vec3::ZERO;
            continue;
        }
        gravity.0 = CHARACTER_GRAVITY_SCALE;
        damping.0 = 0.0;

        let wish_dir =
            Vec3::new(movement.direction.x, 0.0, movement.direction.z).normalize_or_zero();

        let speed = if movement.sneak {
            controller.sneak_speed
        } else if movement.sprint {
            controller.sprint_speed
        } else {
            controller.walk_speed
        };

        let target = wish_dir * speed;
        let current = Vec3::new(velocity.x, 0.0, velocity.z);

        if grounded {
            if wish_dir != Vec3::ZERO {
                let change = (target - current).clamp_length_max(controller.acceleration * dt);

                velocity.x += change.x;
                velocity.z += change.z;
            } else {
                let speed = current.length();

                if speed > 0.001 {
                    let new_speed = (speed - speed * controller.friction * dt).max(0.0);

                    let factor = new_speed / speed;
                    velocity.x *= factor;
                    velocity.z *= factor;
                } else {
                    velocity.x = 0.0;
                    velocity.z = 0.0;
                }
            }

            if movement.jump {
                velocity.y = controller.jump_speed;
            } else if velocity.y <= GROUND_UP_SPEED {
                let horizontal_velocity = Vec3::new(velocity.x, 0.0, velocity.z);
                let travel = horizontal_velocity * dt;
                // Do not raise the body before its next footprint reaches the
                // ledge: otherwise gravity pulls it back onto the lower floor.
                let probe = travel.length().min(controller.step_probe_distance);
                let step = step_height(
                    &spatial_query,
                    entity,
                    collider,
                    position.0,
                    rotation.0,
                    horizontal_velocity.normalize_or_zero(),
                    probe,
                    controller.walk_step_height,
                    skin,
                );
                let auto_jump = step.is_none()
                    && auto_jump
                    && wish_dir != Vec3::ZERO
                    && step_height(
                        &spatial_query,
                        entity,
                        collider,
                        position.0,
                        rotation.0,
                        wish_dir,
                        controller.step_probe_distance,
                        controller.max_step_height,
                        skin,
                    )
                    .is_some_and(|height| height > controller.walk_step_height + CONTACT_CLEARANCE);
                if auto_jump {
                    velocity.y = controller.jump_speed;
                } else if let Some(height) = step.or(support.0) {
                    if height.abs() > 0.0001 {
                        // SpatialQuery reads Position for all colliders. Queue
                        // the correction; the chain flushes it before physics.
                        commands
                            .entity(entity)
                            .insert(Position(position.0 + Vec3::Y * height));
                    }
                    // Hold the verified support height through this tick. A
                    // gravity impulse here would hit the riser's edge again,
                    // losing horizontal speed and making the player bob.
                    gravity.0 = 0.0;
                    velocity.y = 0.0;
                }
            }
        } else if wish_dir != Vec3::ZERO {
            let change = (target - current).clamp_length_max(controller.air_acceleration * dt);

            velocity.x += change.x;
            velocity.z += change.z;
        }
    }
}

fn character_swim_movement(
    voxel_world: VoxelWorld,
    time: Res<Time>,
    spatial_query: SpatialQuery,
    narrow_phase: Res<NarrowPhaseConfig>,
    length_unit: Res<PhysicsLengthUnit>,
    mut characters: Query<
        (
            Entity,
            Forces,
            &CharacterController,
            &CharacterMovement,
            &Collider,
            &Position,
            &Rotation,
            &mut GravityScale,
            &mut LinearDamping,
        ),
        (With<GameCharacter>, With<InWater>),
    >,
) {
    let skin = narrow_phase.contact_tolerance * length_unit.0 + CONTACT_CLEARANCE;
    for (
        entity,
        mut forces,
        controller,
        movement,
        collider,
        position,
        rotation,
        mut gravity,
        mut damping,
    ) in &mut characters
    {
        let velocity = forces.linear_velocity();
        if !voxel_world.collision_ready_for_motion(position.0, velocity, time.delta_secs()) {
            gravity.0 = 0.0;
            *forces.linear_velocity_mut() = Vec3::ZERO;
            continue;
        }
        let horizontal_direction =
            Vec3::new(movement.direction.x, 0.0, movement.direction.z).normalize_or_zero();

        if movement.jump
            && !movement.sneak
            && horizontal_direction != Vec3::ZERO
            && step_height(
                &spatial_query,
                entity,
                collider,
                position.0,
                rotation.0,
                horizontal_direction,
                controller.step_probe_distance,
                controller.max_step_height,
                skin,
            )
            .is_some()
        {
            gravity.0 = CHARACTER_GRAVITY_SCALE;
            damping.0 = 0.0;

            let velocity = forces.linear_velocity_mut();
            let shore_velocity = horizontal_direction * controller.walk_speed;

            velocity.x = shore_velocity.x;
            velocity.z = shore_velocity.z;
            velocity.y = velocity.y.max(controller.jump_speed);

            continue;
        }

        gravity.0 = CHARACTER_WATER_GRAVITY_SCALE;
        damping.0 = WATER_DAMPING;

        let look = movement.look_direction.normalize_or_zero();
        let look_flat = Vec3::new(look.x, 0.0, look.z).normalize_or_zero();

        let sprint_swimming =
            movement.sprint && !movement.sneak && horizontal_direction.dot(look_flat) > 0.5;

        let direction = if sprint_swimming {
            look
        } else {
            horizontal_direction
        };

        let speed = if sprint_swimming {
            controller.swim_sprint_speed
        } else {
            controller.swim_speed
        };

        let vertical = movement.jump as i8 as f32 - movement.sneak as i8 as f32;

        forces.apply_linear_acceleration(
            direction * speed * WATER_DAMPING + Vec3::Y * vertical * SWIM_VERTICAL_ACCELERATION,
        );
    }
}

/// Snap down only after horizontal integration has cleared the old floor.
/// Applying a projected descent before moving would put the rear of the body
/// inside the higher block, making the solver push it up and slow it down.
fn follow_ground(
    mut commands: Commands,
    spatial_query: SpatialQuery,
    narrow_phase: Res<NarrowPhaseConfig>,
    length_unit: Res<PhysicsLengthUnit>,
    mut characters: Query<
        (Entity, &Position, &Rotation, &Collider, &mut LinearVelocity),
        (With<GameCharacter>, With<OnGround>, Without<InWater>),
    >,
) {
    let skin = narrow_phase.contact_tolerance * length_unit.0 + CONTACT_CLEARANCE;
    for (entity, position, rotation, collider, mut velocity) in &mut characters {
        if velocity.y > GROUND_UP_SPEED {
            continue;
        }
        if let Some(height) = ground_height(
            &spatial_query,
            entity,
            collider,
            position.0,
            rotation.0,
            skin,
        ) && height < -0.0001
        {
            commands
                .entity(entity)
                .insert(Position(position.0 + Vec3::Y * height));
            velocity.y = 0.0;
        }
    }
}

/// Vertical correction to rest on a nearby floor with a small stable gap.
fn ground_height(
    spatial_query: &SpatialQuery,
    entity: Entity,
    collider: &Collider,
    position: Vec3,
    rotation: Quat,
    skin: f32,
) -> Option<f32> {
    let filter = SpatialQueryFilter::from_excluded_entities([entity])
        .with_mask([GameLayer::Default, GameLayer::World]);
    let hit = spatial_query.cast_shape(
        collider,
        position + Vec3::Y * STEP_CLEARANCE,
        rotation,
        Dir3::NEG_Y,
        &ShapeCastConfig::from_max_distance(STEP_CLEARANCE + GROUND_REACH),
        &filter,
    )?;
    // An initial overlap with the side of a higher step is not floor support.
    (hit.distance > CONTACT_CLEARANCE && hit.normal1.y > 0.5)
        .then_some(STEP_CLEARANCE - hit.distance + skin)
}

/// Returns the supported rise after an up/forward/down sweep. Every leg of
/// the path must fit the whole body, including its head, before moving it.
fn step_height(
    spatial_query: &SpatialQuery,
    entity: Entity,
    collider: &Collider,
    position: Vec3,
    rotation: Quat,
    direction: Vec3,
    probe_distance: f32,
    max_height: f32,
    skin: f32,
) -> Option<f32> {
    let direction = Dir3::new(direction).ok()?;
    let config = ShapeCastConfig::from_max_distance(probe_distance);
    // Loose items must never act as stairs or trigger an automatic jump.
    let filter = SpatialQueryFilter::from_excluded_entities([entity])
        .with_mask([GameLayer::Default, GameLayer::World]);
    // The grounded body already has a stable clearance. Raising this initial
    // sweep would hide risers smaller than that extra offset.
    let origin = position;

    let obstacle =
        spatial_query.cast_shape(collider, origin, rotation, direction, &config, &filter)?;

    // Ignore floor contacts, but accept shallow approaches to a riser as well
    // as head-on ones; a steep dot threshold catches the feet while strafing.
    if obstacle.normal1.dot(*direction) >= -0.001 {
        return None;
    }

    let up_config = ShapeCastConfig {
        ignore_origin_penetration: true,
        ..ShapeCastConfig::from_max_distance(max_height)
    };
    let lift = spatial_query
        .cast_shape(collider, origin, rotation, Dir3::Y, &up_config, &filter)
        .map_or(max_height, |ceiling| (ceiling.distance - skin).max(0.0));
    if lift <= CONTACT_CLEARANCE {
        return None;
    }
    let raised = origin + Vec3::Y * lift;
    if spatial_query
        .cast_shape(collider, raised, rotation, direction, &config, &filter)
        .is_some()
    {
        return None;
    }
    let landing = spatial_query.cast_shape(
        collider,
        raised + *direction * probe_distance,
        rotation,
        Dir3::NEG_Y,
        &ShapeCastConfig::from_max_distance(lift + STEP_CLEARANCE),
        &filter,
    )?;
    let height = lift - landing.distance + skin;
    (landing.normal1.y > 0.5
        && height > CONTACT_CLEARANCE
        && height <= max_height + CONTACT_CLEARANCE)
        .then_some(height)
}
