use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{
    input::mouse::AccumulatedMouseMotion,
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};

use crate::{
    character::{
        CHARACTER_GRAVITY_SCALE, CHARACTER_WATER_GRAVITY_SCALE, CharacterController,
        CharacterMovement, GameCharacter, InWater,
    },
    game::GameState,
    spatial::{FollowOffset, FollowedBy},
    voxel::{VoxelKind, VoxelSettings, VoxelViewer, VoxelWorld},
};

const PLAYER_RADIUS: f32 = 0.3;
const PLAYER_CAPSULE_LENGTH: f32 = 1.2;
const PLAYER_EYE_HEIGHT: f32 = 0.65;
const WATER_DAMPING: f32 = 4.0;
const SWIM_VERTICAL_ACCELERATION: f32 = 8.0;
const PLAYER_HALF_HEIGHT: f32 = PLAYER_CAPSULE_LENGTH * 0.5 + PLAYER_RADIUS;
const SHORE_PROBE_DISTANCE: f32 = PLAYER_RADIUS + 0.25;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Game), spawn_player)
            .add_systems(
                Update,
                (mouse_look, player_input).run_if(in_state(GameState::Game)),
            )
            .add_systems(
                FixedPostUpdate,
                swim_movement
                    .after(PhysicsSystems::Prepare)
                    .before(PhysicsSystems::StepSimulation)
                    .run_if(in_state(GameState::Game)),
            );
    }
}

fn default_friction() -> Friction {
    Friction::ZERO.with_combine_rule(CoefficientCombine::Min)
}

fn default_restitution() -> Restitution {
    Restitution::ZERO.with_combine_rule(CoefficientCombine::Min)
}

#[derive(Component, Default, Debug, Clone)]
#[require(
    GameCharacter,
    RigidBody::Dynamic,
    Collider::capsule(PLAYER_RADIUS, PLAYER_CAPSULE_LENGTH),
    LockedAxes::ROTATION_LOCKED,
    GravityScale(CHARACTER_GRAVITY_SCALE),
    LinearDamping::default(),
    Friction = default_friction(),
    Restitution = default_restitution(),
    Visibility::default(),
    DespawnOnExit::<GameState>(GameState::Game)
)]
pub struct Player;

#[derive(Debug, Component, Default, Clone)]
#[require(
    CameraSensitivity,
    Camera3d,
    IsDefaultUiCamera,
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
    DespawnOnExit::<GameState>(GameState::Game),
)]
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

fn spawn_player(mut commands: Commands, voxel_settings: Option<Res<VoxelSettings>>) {
    let spawn_pos = if let Some(settings) = voxel_settings {
        let center = settings.chunk_center(IVec2::ZERO);
        Vec3::new(center.x, settings.base_height + 15.0, center.z)
    } else {
        Vec3::new(0.0, 30.0, 0.0)
    };

    commands.spawn_scene_list(bsn! {
        Player
        Transform::from_translation(spawn_pos)
        FollowedBy [
            PlayerCamera
            FollowOffset({Vec3::Y * PLAYER_EYE_HEIGHT}),
        ]
    });

    commands.spawn_scene(bsn! {
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center
        }
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
}

fn mouse_look(
    accumulated_mouse_motion: Res<AccumulatedMouseMotion>,
    mut player: Single<&mut Transform, (With<Player>, Without<PlayerCamera>)>,
    mut camera: Single<
        (&mut Transform, &mut PlayerCamera, &CameraSensitivity),
        (With<PlayerCamera>, Without<Player>),
    >,
) {
    let delta = accumulated_mouse_motion.delta;
    if delta == Vec2::ZERO {
        return;
    }

    let (camera_transform, player_camera, sensitivity) = &mut *camera;
    player.rotate_y(-delta.x * sensitivity.x);

    const PITCH_LIMIT: f32 = FRAC_PI_2 - 0.01;
    player_camera.pitch =
        (player_camera.pitch - delta.y * sensitivity.y).clamp(-PITCH_LIMIT, PITCH_LIMIT);
    camera_transform.rotation = player.rotation * Quat::from_rotation_x(player_camera.pitch);
}

fn player_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    camera: Single<&Transform, With<PlayerCamera>>,
    player: Single<(&Transform, &mut CharacterMovement), With<Player>>,
) {
    let (transform, mut movement) = player.into_inner();

    let forward = transform.forward().as_vec3();
    let forward = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();

    let right = transform.right().as_vec3();
    let right = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    movement.direction = (forward * axis(&keyboard, KeyCode::KeyW, KeyCode::KeyS)
        + right * axis(&keyboard, KeyCode::KeyD, KeyCode::KeyA))
    .normalize_or_zero();

    movement.look_direction = camera.forward().as_vec3();

    movement.jump = keyboard.pressed(KeyCode::Space);
    movement.sneak = keyboard.pressed(KeyCode::ShiftLeft);
    movement.sprint = keyboard.pressed(KeyCode::ControlLeft);
}

fn swim_movement(
    keyboard: Res<ButtonInput<KeyCode>>,
    voxel_world: VoxelWorld,
    camera: Single<&Transform, (With<PlayerCamera>, Without<Player>)>,
    player: Single<
        (
            Forces,
            &CharacterController,
            &Transform,
            &mut GravityScale,
            &mut LinearDamping,
            Has<InWater>,
        ),
        With<Player>,
    >,
) {
    let (mut forces, controller, transform, mut gravity, mut damping, in_water) =
        player.into_inner();

    if !in_water {
        return;
    }

    let forward = transform.forward().as_vec3();
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();
    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    let forward_input = axis(&keyboard, KeyCode::KeyW, KeyCode::KeyS);
    let right_input = axis(&keyboard, KeyCode::KeyD, KeyCode::KeyA);
    let sneaking = keyboard.pressed(KeyCode::ShiftLeft);
    let sprinting = keyboard.pressed(KeyCode::ControlLeft);
    let holding_jump = keyboard.pressed(KeyCode::Space) && !sneaking;

    let horizontal_direction =
        (forward_flat * forward_input + right_flat * right_input).normalize_or_zero();

    // Minecraft-like shore exit: when swimming against a one-block ledge while
    // holding jump, launch upward and slightly onto the block. This bypasses
    // water damping for that physics step so the exit cannot be cancelled.
    if holding_jump
        && horizontal_direction != Vec3::ZERO
        && can_climb_shore(&voxel_world, transform.translation, horizontal_direction)
    {
        gravity.0 = CHARACTER_GRAVITY_SCALE;
        damping.0 = 0.0;

        let velocity = forces.linear_velocity_mut();
        let shore_velocity = horizontal_direction * controller.walk_speed;
        velocity.x = shore_velocity.x;
        velocity.z = shore_velocity.z;
        velocity.y = velocity.y.max(controller.jump_speed);
        return;
    }

    gravity.0 = CHARACTER_WATER_GRAVITY_SCALE;
    damping.0 = WATER_DAMPING;

    let swim_sprinting = sprinting && !sneaking && forward_input > 0.0;
    let swim_forward = if swim_sprinting {
        camera.forward().as_vec3()
    } else {
        forward_flat
    };
    let direction = (swim_forward * forward_input + right_flat * right_input).normalize_or_zero();
    let speed = if swim_sprinting {
        controller.swim_sprint_speed
    } else {
        controller.swim_speed
    };
    let vertical = axis(&keyboard, KeyCode::Space, KeyCode::ShiftLeft);

    forces.apply_linear_acceleration(
        direction * speed * WATER_DAMPING + Vec3::Y * vertical * SWIM_VERTICAL_ACCELERATION,
    );
}

fn can_climb_shore(voxels: &VoxelWorld, position: Vec3, direction: Vec3) -> bool {
    // The transform is at the capsule centre. Probe from close to the feet,
    // not from the centre of the body.
    let feet = position - Vec3::Y * (PLAYER_HALF_HEIGHT - 0.1);
    let ahead = feet + direction * SHORE_PROBE_DISTANCE;

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

fn axis(input: &ButtonInput<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    (input.pressed(positive) as i8 - input.pressed(negative) as i8) as f32
}
