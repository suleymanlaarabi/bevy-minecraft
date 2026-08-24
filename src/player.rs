use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{input::mouse::AccumulatedMouseMotion, prelude::*};

use crate::{
    character::{CHARACTER_GRAVITY_SCALE, CHARACTER_WATER_GRAVITY_SCALE, OnGround, OnGroundSensor},
    game::GameState,
    spatial::Follow,
    voxel::{VoxelKind, VoxelSettings, VoxelViewer, VoxelWorld},
};

const PLAYER_RADIUS: f32 = 0.3;
const PLAYER_CAPSULE_LENGTH: f32 = 1.2;
const PLAYER_EYE_HEIGHT: f32 = 0.65;
const WATER_DAMPING: f32 = 4.0;
const WATER_EYE_CLEARANCE: f32 = 0.25;
const WATER_SURFACE_BAND: f32 = 0.12;
const WATER_DEEP_SAMPLE: f32 = 0.5;
const SWIM_VERTICAL_ACCELERATION: f32 = 8.0;
const SWIM_SURFACE_ACCELERATION: f32 = 3.0;
const SHORE_PROBE_DISTANCE: f32 = PLAYER_RADIUS + 0.2;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PlayerController>()
            .add_systems(Startup, setup_camera)
            .add_systems(OnEnter(GameState::Game), spawn_player)
            .add_systems(
                Update,
                (mouse_look, player_movement)
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
    pub swim_speed: f32,
    pub swim_sprint_speed: f32,
    surface_rising: bool,
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
            swim_speed: 2.2,
            swim_sprint_speed: 5.612,
            surface_rising: false,
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

type PlayerCameraQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut Transform,
        &'static mut PlayerCamera,
        &'static CameraSensitivity,
    ),
    (With<PlayerCamera>, Without<Player>),
>;

type PlayerMovementQuery<'w, 's> = Query<
    'w,
    's,
    (
        &'static mut LinearVelocity,
        &'static mut PlayerController,
        &'static Transform,
        &'static mut GravityScale,
        &'static mut LinearDamping,
        &'static mut ConstantLinearAcceleration,
        Has<OnGround>,
    ),
    With<Player>,
>;

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
    mut camera_query: Query<(Entity, &mut Transform, &mut PlayerCamera), Without<Player>>,
) {
    let spawn_pos = if let Some(settings) = voxel_settings {
        let center = settings.chunk_center(IVec2::ZERO);
        Vec3::new(center.x, settings.base_height + 15.0, center.z)
    } else {
        Vec3::new(0.0, 30.0, 0.0)
    };

    let player_entity = commands
        .spawn((
            Player,
            PlayerController::default(),
            RigidBody::Dynamic,
            Collider::capsule(PLAYER_RADIUS, PLAYER_CAPSULE_LENGTH),
            LockedAxes::ROTATION_LOCKED,
            GravityScale(CHARACTER_GRAVITY_SCALE),
            LinearDamping::default(),
            ConstantLinearAcceleration::default(),
            Friction::ZERO.with_combine_rule(CoefficientCombine::Min),
            Restitution::ZERO.with_combine_rule(CoefficientCombine::Min),
            LinearVelocity::default(),
            OnGroundSensor,
            Transform::from_translation(spawn_pos),
            Visibility::default(),
            DespawnOnExit(GameState::Game),
        ))
        .id();

    if let Ok((cam_entity, mut cam_transform, mut cam)) = camera_query.single_mut() {
        cam_transform.translation = spawn_pos + Vec3::Y * PLAYER_EYE_HEIGHT;
        cam_transform.rotation = Quat::IDENTITY;
        cam.pitch = 0.0;

        commands
            .entity(cam_entity)
            .insert(Follow::new(player_entity, Vec3::Y * PLAYER_EYE_HEIGHT));
    }

    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center
        }
        GlobalZIndex(100)
        Children [
            Node {
                width: px(32),
                height: px(32),
                position_type: PositionType::Relative
            }
            Children [
                Node {
                    width: px(2),
                    height: px(18),
                    position_type: PositionType::Absolute,
                    left: px(15),
                    top: px(7)
                }
                BackgroundColor(Color::WHITE),

                Node {
                    width: px(18),
                    height: px(2),
                    position_type: PositionType::Absolute,
                    left: px(7),
                    top: px(15)
                }
                BackgroundColor(Color::WHITE)
            ]
        ]
        DespawnOnExit::<GameState>(GameState::Game)
    });

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

fn mouse_look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mut player_query: Query<&mut Transform, With<Player>>,
    mut camera_query: PlayerCameraQuery,
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
    voxel_world: VoxelWorld,
    camera_query: Query<&Transform, (With<PlayerCamera>, Without<Player>)>,
    mut player_query: PlayerMovementQuery,
) {
    let dt = time.delta_secs();
    if dt < 0.0 {
        return;
    }

    let Ok((
        mut velocity,
        mut controller,
        transform,
        mut gravity_scale,
        mut damping,
        mut acceleration,
        is_on_ground,
    )) = player_query.single_mut()
    else {
        return;
    };

    // Calculate forward and right horizontal vectors from player body orientation
    let forward = transform.forward().as_vec3();
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    // Support ZQSD layouts.
    let forward_input =
        i8::from(keyboard.pressed(KeyCode::KeyW)) - i8::from(keyboard.pressed(KeyCode::KeyS));
    let right_input =
        i8::from(keyboard.pressed(KeyCode::KeyD)) - i8::from(keyboard.pressed(KeyCode::KeyA));

    // Determine target movement speed
    let is_sneaking = keyboard.pressed(KeyCode::ShiftLeft);
    let is_sprinting = keyboard.pressed(KeyCode::ControlLeft);

    let in_water = voxel_world
        .get_at(transform.translation)
        .is_some_and(|kind| kind.is_liquid());
    if in_water {
        let eye_position = transform.translation + Vec3::Y * PLAYER_EYE_HEIGHT;
        let surface_probe = eye_position - Vec3::Y * WATER_EYE_CLEARANCE;
        let water_above_target = voxel_world
            .get_at(surface_probe + Vec3::Y * WATER_SURFACE_BAND)
            .is_some_and(|kind| kind.is_liquid());
        let water_below_target = voxel_world
            .get_at(surface_probe - Vec3::Y * WATER_SURFACE_BAND)
            .is_some_and(|kind| kind.is_liquid());
        let eyes_submerged = voxel_world
            .get_at(eye_position)
            .is_some_and(|kind| kind.is_liquid());
        let deeply_submerged = voxel_world
            .get_at(eye_position + Vec3::Y * WATER_DEEP_SAMPLE)
            .is_some_and(|kind| kind.is_liquid());
        let holding_space = keyboard.pressed(KeyCode::Space) && !is_sneaking;
        if !holding_space {
            controller.surface_rising = false;
        } else if water_above_target {
            controller.surface_rising = true;
        } else if !water_below_target {
            controller.surface_rising = false;
        }

        let swim_sprinting = is_sprinting && !is_sneaking && forward_input > 0;
        let horizontal_direction = (forward_flat * forward_input as f32
            + right_flat * right_input as f32)
            .normalize_or_zero();
        let swim_forward = if swim_sprinting && eyes_submerged {
            camera_query
                .single()
                .map_or(forward_flat, |camera| camera.forward().as_vec3())
        } else {
            forward_flat
        };
        let direction = (swim_forward * forward_input as f32 + right_flat * right_input as f32)
            .normalize_or_zero();
        let climbing_shore = holding_space
            && horizontal_direction != Vec3::ZERO
            && can_climb_shore(&voxel_world, transform.translation, horizontal_direction);
        if climbing_shore {
            velocity.y = velocity.y.max(controller.jump_speed);
        }
        let speed = if swim_sprinting {
            controller.swim_sprint_speed
        } else {
            controller.swim_speed
        };

        gravity_scale.0 = CHARACTER_WATER_GRAVITY_SCALE;
        damping.0 = WATER_DAMPING;
        let vertical_acceleration = if is_sneaking {
            -SWIM_VERTICAL_ACCELERATION
        } else if climbing_shore {
            0.0
        } else if controller.surface_rising {
            if deeply_submerged {
                SWIM_VERTICAL_ACCELERATION
            } else {
                SWIM_SURFACE_ACCELERATION
            }
        } else {
            0.0
        };
        acceleration.0 = swim_acceleration(direction, speed, vertical_acceleration);
        // controller.is_grounded = false;
        return;
    }

    controller.surface_rising = false;
    gravity_scale.0 = CHARACTER_GRAVITY_SCALE;
    damping.0 = 0.0;
    acceleration.0 = Vec3::ZERO;

    let wish_dir =
        (forward_flat * forward_input as f32 + right_flat * right_input as f32).normalize_or_zero();

    let target_speed = if is_sneaking {
        controller.sneak_speed
    } else if is_sprinting {
        controller.sprint_speed
    } else {
        controller.walk_speed
    };

    let target_horizontal_vel = wish_dir * target_speed;
    let current_horizontal_vel = Vec3::new(velocity.x, 0.0, velocity.z);

    if is_on_ground {
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

fn swim_acceleration(direction: Vec3, speed: f32, vertical: f32) -> Vec3 {
    direction * speed * WATER_DAMPING + Vec3::Y * vertical
}

fn can_climb_shore(voxels: &VoxelWorld, position: Vec3, direction: Vec3) -> bool {
    let obstacle = position + direction * SHORE_PROBE_DISTANCE;
    has_shore_clearance([
        voxels.get_at(obstacle),
        voxels.get_at(obstacle + Vec3::Y),
        voxels.get_at(obstacle + Vec3::Y * 2.0),
    ])
}

fn has_shore_clearance(samples: [Option<VoxelKind>; 3]) -> bool {
    samples[0].is_some_and(VoxelKind::is_solid)
        && samples[1..]
            .iter()
            .all(|kind| kind.is_some_and(|kind| !kind.is_solid()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swimming_acceleration_uses_avian_damping_and_controls() {
        let forward = swim_acceleration(Vec3::NEG_Z, 2.2, 0.0);
        assert_eq!(forward, Vec3::new(0.0, 0.0, -8.8));

        let ascend = swim_acceleration(Vec3::ZERO, 0.0, SWIM_VERTICAL_ACCELERATION);
        let descend = swim_acceleration(Vec3::ZERO, 0.0, -SWIM_VERTICAL_ACCELERATION);
        assert_eq!(ascend.y, SWIM_VERTICAL_ACCELERATION);
        assert_eq!(descend.y, -SWIM_VERTICAL_ACCELERATION);
    }

    #[test]
    fn shore_exit_requires_a_solid_step_and_two_clear_voxels() {
        assert!(has_shore_clearance([
            Some(VoxelKind::Sand),
            Some(VoxelKind::Air),
            Some(VoxelKind::Air),
        ]));
        assert!(!has_shore_clearance([
            Some(VoxelKind::Sand),
            Some(VoxelKind::Stone),
            Some(VoxelKind::Air),
        ]));
        assert!(!has_shore_clearance([
            Some(VoxelKind::Water),
            Some(VoxelKind::Air),
            Some(VoxelKind::Air),
        ]));
    }
}
