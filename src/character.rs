use avian3d::{
    collision::collider::Collider,
    dynamics::rigid_body::{GravityScale, LinearDamping, LinearVelocity},
    spatial_query::{ShapeCaster, ShapeHits},
};
use bevy::prelude::*;

use crate::{game::GameState, voxel::VoxelWorld};

pub const CHARACTER_GRAVITY_SCALE: f32 = 2.8;
pub const CHARACTER_WATER_GRAVITY_SCALE: f32 = 0.15;

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

#[derive(Component, Default)]
#[require(OnGroundSensor, InWaterSensor, CharacterController, CharacterMovement)]
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
