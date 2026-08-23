use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{input::mouse::AccumulatedMouseMotion, prelude::*};

use crate::{
    game::GameState,
    voxel::{VoxelSettings, VoxelViewer},
};

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PlayerController>()
            .add_systems(Startup, setup_camera)
            .add_systems(OnEnter(GameState::Game), spawn_player)
            .add_systems(
                Update,
                (
                    update_grounded_state,
                    mouse_look,
                    player_movement,
                    sync_camera_position,
                )
                    .chain()
                    .run_if(in_state(GameState::Game)),
            );
    }
}

#[derive(Debug, Component)]
pub struct Player;

#[derive(Debug, Component, Reflect)]
#[reflect(Component)]
pub struct PlayerController {
    pub walk_speed: f32,
    pub sprint_speed: f32,
    pub sneak_speed: f32,
    pub jump_speed: f32,
    pub acceleration: f32,
    pub air_acceleration: f32,
    pub friction: f32,
    pub is_grounded: bool,
}

impl Default for PlayerController {
    fn default() -> Self {
        Self {
            walk_speed: 4.3,
            sprint_speed: 6.8,
            sneak_speed: 1.5,
            jump_speed: 8.2,
            acceleration: 60.0,
            air_acceleration: 15.0,
            friction: 25.0,
            is_grounded: false,
        }
    }
}

#[derive(Debug, Component, Default)]
pub struct PlayerCamera {
    pub pitch: f32,
}

#[derive(Debug, Component, Deref, DerefMut)]
pub struct CameraSensitivity(pub Vec2);

impl Default for CameraSensitivity {
    fn default() -> Self {
        Self(Vec2::new(0.0025, 0.002))
    }
}

use bevy::pbr::{DistanceFog, FogFalloff};

fn setup_camera(mut commands: Commands) {
    commands.spawn((
        PlayerCamera::default(),
        CameraSensitivity::default(),
        Camera3d::default(),
        AmbientLight {
            color: Color::srgb_u8(215, 235, 255),
            brightness: 500.0,
            ..default()
        },
        DistanceFog {
            color: Color::srgb_u8(195, 222, 255),
            falloff: FogFalloff::Linear {
                start: 160.0,
                end: 280.0,
            },
            ..default()
        },
        Projection::from(PerspectiveProjection {
            fov: 85.0_f32.to_radians(),
            ..default()
        }),
        Transform::from_xyz(0.0, 30.0, 0.0),
        Visibility::default(),
        VoxelViewer,
    ));
}

fn spawn_player(
    mut commands: Commands,
    voxel_settings: Option<Res<VoxelSettings>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut camera_query: Query<(&mut Transform, &mut PlayerCamera), Without<Player>>,
) {
    let spawn_pos = if let Some(settings) = voxel_settings {
        let center = settings.chunk_center(IVec2::ZERO);
        Vec3::new(center.x, settings.base_height + 15.0, center.z)
    } else {
        Vec3::new(0.0, 30.0, 0.0)
    };

    if let Ok((mut cam_transform, mut cam)) = camera_query.single_mut() {
        cam_transform.translation = spawn_pos + Vec3::new(0.0, 0.65, 0.0);
        cam_transform.rotation = Quat::IDENTITY;
        cam.pitch = 0.0;
    }

    commands.spawn((
        Player,
        PlayerController::default(),
        RigidBody::Dynamic,
        Collider::capsule(0.3, 1.2),
        LockedAxes::ROTATION_LOCKED,
        GravityScale(2.8),
        Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
        Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
        LinearVelocity::default(),
        ShapeCaster::new(
            Collider::sphere(0.25),
            Vec3::ZERO,
            Quat::IDENTITY,
            Dir3::NEG_Y,
        )
        .with_max_distance(0.75)
        .with_ignore_self(true),
        Transform::from_translation(spawn_pos),
        Visibility::default(),
        DespawnOnExit(GameState::Game),
    ));

    // Sun directional light
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            shadow_depth_bias: 0.02,
            shadow_normal_bias: 1.8,
            illuminance: 5_000.0,
            color: Color::srgb(1.0, 0.98, 0.94),
            ..default()
        },
        Transform::from_xyz(100.0, 200.0, 100.0).looking_at(Vec3::ZERO, Vec3::Y),
        DespawnOnExit(GameState::Game),
    ));

    // Minecraft-style square Sun in the sky
    commands.spawn((
        Mesh3d(meshes.add(Plane3d::default().mesh().size(50.0, 50.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 1.0, 0.9),
            unlit: true,
            ..default()
        })),
        Transform::from_xyz(250.0, 350.0, 250.0).looking_at(Vec3::ZERO, Vec3::Y),
        DespawnOnExit(GameState::Game),
    ));
}

fn sync_camera_position(
    player_query: Query<&Transform, With<Player>>,
    mut camera_query: Query<&mut Transform, (With<PlayerCamera>, Without<Player>)>,
) {
    if let Ok(player_transform) = player_query.single()
        && let Ok(mut camera_transform) = camera_query.single_mut()
    {
        camera_transform.translation = player_transform.translation + Vec3::new(0.0, 0.65, 0.0);
    }
}

fn update_grounded_state(
    mut player_query: Query<(&mut PlayerController, &ShapeHits), With<Player>>,
) {
    for (mut controller, hits) in &mut player_query {
        let mut grounded = false;
        for hit in hits.iter() {
            if hit.normal1.y > 0.5 {
                grounded = true;
                break;
            }
        }
        controller.is_grounded = grounded;
    }
}

fn mouse_look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_query: Query<
        (&mut Transform, &mut PlayerCamera, &CameraSensitivity),
        (With<PlayerCamera>, Without<Player>),
    >,
) {
    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    let Ok((mut camera_transform, mut player_camera, sensitivity)) = camera_query.single_mut()
    else {
        return;
    };

    let delta_yaw = -delta.x * sensitivity.x;
    let delta_pitch = -delta.y * sensitivity.y;

    if let Ok(mut player_transform) = player_query.single_mut() {
        player_transform.rotate_y(delta_yaw);
    }

    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
    player_camera.pitch = (player_camera.pitch + delta_pitch).clamp(-PITCH_LIMIT, PITCH_LIMIT);

    if let Ok(player_transform) = player_query.single() {
        camera_transform.rotation =
            player_transform.rotation * Quat::from_rotation_x(player_camera.pitch);
    }
}

fn player_movement(
    time: Res<Time>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut player_query: Query<(&mut LinearVelocity, &mut PlayerController, &Transform), With<Player>>,
) {
    let dt = time.delta_secs();
    if dt < 0.0 {
        return;
    }

    let Ok((mut velocity, mut controller, transform)) = player_query.single_mut() else {
        return;
    };

    // Calculate forward and right horizontal vectors from player body orientation
    let forward = transform.forward().as_vec3();
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    let mut wish_dir = Vec3::ZERO;
    // Support both WASD and ZQSD layouts
    if keyboard.pressed(KeyCode::KeyW) || keyboard.pressed(KeyCode::KeyZ) {
        wish_dir += forward_flat;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        wish_dir -= forward_flat;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        wish_dir += right_flat;
    }
    if keyboard.pressed(KeyCode::KeyA) || keyboard.pressed(KeyCode::KeyQ) {
        wish_dir -= right_flat;
    }

    let wish_dir = wish_dir.normalize_or_zero();

    // Determine target movement speed
    let is_sneaking = keyboard.pressed(KeyCode::ShiftLeft);
    let is_sprinting =
        keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ShiftRight);

    let target_speed = if is_sneaking {
        controller.sneak_speed
    } else if is_sprinting {
        controller.sprint_speed
    } else {
        controller.walk_speed
    };

    let target_horizontal_vel = wish_dir * target_speed;
    let current_horizontal_vel = Vec3::new(velocity.x, 0.0, velocity.z);

    if controller.is_grounded {
        if wish_dir != Vec3::ZERO {
            // Rapid acceleration towards target velocity
            let diff = target_horizontal_vel - current_horizontal_vel;
            let max_change = controller.acceleration * dt;
            let change = diff.clamp_length_max(max_change);
            velocity.x += change.x;
            velocity.z += change.z;
        } else {
            // Snappy ground friction
            let current_speed = current_horizontal_vel.length();
            let drop = current_speed * controller.friction * dt;
            let new_speed = (current_speed - drop).max(0.0);
            if current_speed > 0.001 {
                let factor = new_speed / current_speed;
                velocity.x *= factor;
                velocity.z *= factor;
            } else {
                velocity.x = 0.0;
                velocity.z = 0.0;
            }
        }

        // Jump impulse when grounded
        if keyboard.just_pressed(KeyCode::Space) || keyboard.pressed(KeyCode::Space) {
            velocity.y = controller.jump_speed;
            controller.is_grounded = false;
        }
    } else {
        // Air control: preserve momentum while allowing steering
        if wish_dir != Vec3::ZERO {
            let diff = target_horizontal_vel - current_horizontal_vel;
            let max_change = controller.air_acceleration * dt;
            let change = diff.clamp_length_max(max_change);
            velocity.x += change.x;
            velocity.z += change.z;
        }

        // Slight horizontal air drag
        const AIR_DRAG: f32 = 0.5;
        velocity.x -= velocity.x * AIR_DRAG * dt;
        velocity.z -= velocity.z * AIR_DRAG * dt;
    }
}
