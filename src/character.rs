use avian3d::{
    collision::collider::Collider,
    dynamics::rigid_body::{
        GravityScale, LinearDamping, LinearVelocity,
        forces::{Forces, WriteRigidBodyForces},
    },
    schedule::PhysicsSystems,
    spatial_query::{ShapeCaster, ShapeHits},
};
use bevy::prelude::*;

use crate::{
    game::GameState,
    voxel::{VoxelKind, VoxelWorld},
};

pub const CHARACTER_GRAVITY_SCALE: f32 = 2.8;
pub const CHARACTER_WATER_GRAVITY_SCALE: f32 = 0.15;
const WATER_DAMPING: f32 = 4.0;
const SWIM_VERTICAL_ACCELERATION: f32 = 8.0;

pub struct CharacterPlugin;

fn caster() -> ShapeCaster {
    ShapeCaster::new(
        Collider::sphere(0.25),
        Vec3::ZERO,
        Quat::IDENTITY,
        Dir3::NEG_Y,
    )
    .with_max_distance(0.75)
    .with_ignore_self(true)
}

#[derive(Component)]
pub struct OnGround;

#[derive(Component, Default)]
#[require(ShapeCaster = caster())]
pub struct OnGroundSensor;

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
        }
    }
}

#[derive(Component, Debug, Clone, Copy)]
pub struct CharacterBody {
    pub radius: f32,
    pub half_height: f32,
}

impl Default for CharacterBody {
    fn default() -> Self {
        Self {
            radius: 0.3,
            half_height: 0.9,
        }
    }
}

#[derive(Component, Default)]
#[require(
    OnGroundSensor,
    InWaterSensor,
    CharacterController,
    CharacterMovement,
    CharacterBody
)]
pub struct GameCharacter;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            FixedUpdate,
            (
                add_grounded_state,
                remove_grounded_state,
                add_in_water_state,
                remove_in_water_state,
            )
                .run_if(in_state(GameState::Game)),
        )
        .add_systems(
            FixedPostUpdate,
            character_swim_movement
                .after(PhysicsSystems::Prepare)
                .before(PhysicsSystems::StepSimulation)
                .run_if(in_state(GameState::Game)),
        )
        .add_systems(
            Update,
            character_land_movement.run_if(in_state(GameState::Game)),
        );
    }
}

fn add_grounded_state(
    mut query: Query<(Entity, &ShapeHits), (With<OnGroundSensor>, Without<OnGround>)>,
    mut commands: Commands,
) {
    for (entity, hits) in &mut query {
        if hits.iter().any(|hit| hit.normal1.y > 0.5) {
            commands.entity(entity).insert(OnGround);
        }
    }
}

fn remove_grounded_state(
    mut query: Query<(Entity, &ShapeHits), (With<OnGroundSensor>, With<OnGround>)>,
    mut commands: Commands,
) {
    for (entity, hits) in &mut query {
        if !hits.iter().any(|hit| hit.normal1.y > 0.5) {
            commands.entity(entity).remove::<OnGround>();
        }
    }
}

fn add_in_water_state(
    mut query: Query<(Entity, &Transform), (With<InWaterSensor>, Without<InWater>)>,
    voxel_world: VoxelWorld,

    mut commands: Commands,
) {
    for (entity, transform) in &mut query {
        if voxel_world
            .get_at(transform.translation)
            .is_some_and(|kind| kind.is_liquid())
        {
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
        if !voxel_world
            .get_at(transform.translation)
            .is_some_and(|kind| kind.is_liquid())
        {
            commands.entity(entity).remove::<InWater>();
        }
    }
}

fn character_land_movement(
    time: Res<Time>,
    mut characters: Query<
        (
            &mut LinearVelocity,
            &CharacterController,
            &CharacterMovement,
            &mut GravityScale,
            &mut LinearDamping,
            Has<OnGround>,
            Has<InWater>,
        ),
        With<GameCharacter>,
    >,
) {
    let dt = time.delta_secs();

    for (mut velocity, controller, movement, mut gravity, mut damping, grounded, in_water) in
        &mut characters
    {
        if in_water {
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
    mut characters: Query<
        (
            Forces,
            &CharacterController,
            &CharacterMovement,
            &CharacterBody,
            &Transform,
            &mut GravityScale,
            &mut LinearDamping,
        ),
        (With<GameCharacter>, With<InWater>),
    >,
) {
    for (mut forces, controller, movement, body, transform, mut gravity, mut damping) in
        &mut characters
    {
        let horizontal_direction =
            Vec3::new(movement.direction.x, 0.0, movement.direction.z).normalize_or_zero();

        // Sortie d'eau Minecraft-like
        if movement.jump
            && !movement.sneak
            && horizontal_direction != Vec3::ZERO
            && can_climb_shore(
                &voxel_world,
                transform.translation,
                horizontal_direction,
                *body,
            )
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

fn can_climb_shore(
    voxels: &VoxelWorld,
    position: Vec3,
    direction: Vec3,
    body: CharacterBody,
) -> bool {
    let feet = position - Vec3::Y * (body.half_height - 0.1);
    let ahead = feet + direction * (body.radius + 0.25);

    let solid_ledge = [0.1, 0.45, 0.8].into_iter().any(|y| {
        voxels
            .get_at(ahead + Vec3::Y * y)
            .is_some_and(VoxelKind::is_solid)
    });

    let body_clear = [1.1, 1.7].into_iter().all(|y| {
        voxels
            .get_at(ahead + Vec3::Y * y)
            .is_some_and(|kind| !kind.is_solid())
    });

    solid_ledge && body_clear
}
