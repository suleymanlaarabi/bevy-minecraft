use std::f32::consts::FRAC_PI_2;

use avian3d::prelude::*;
use bevy::{
    color::palettes::css::BLACK,
    input::mouse::AccumulatedMouseMotion,
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
    transform::TransformSystems,
};

use crate::{
    character::{AutoJump, CHARACTER_GRAVITY_SCALE, CharacterMovement, GameCharacter},
    game::GameState,
    spatial::{FollowOffset, FollowedBy},
    voxel::{VoxelChunk, VoxelSettings, VoxelViewer},
};

const PLAYER_RADIUS: f32 = 0.3;
const PLAYER_CAPSULE_LENGTH: f32 = 1.2;
const PLAYER_EYE_HEIGHT: f32 = 0.65;
const BLOCK_REACH: f32 = 5.0;
const BLOCK_OUTLINE_SIZE: f32 = 1.01;
const HIT_EPSILON: f32 = 0.001;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Game), spawn_player)
            .add_systems(
                RunFixedMainLoop,
                (mouse_look, player_input)
                    .chain()
                    .in_set(RunFixedMainLoopSystems::BeforeFixedMainLoop)
                    .run_if(in_state(GameState::Game)),
            )
            .add_systems(
                PostUpdate,
                draw_targeted_block
                    .after(TransformSystems::Propagate)
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
    AutoJump,
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
    let forward_flat = Vec3::new(forward.x, 0.0, forward.z).normalize_or_zero();

    let right = transform.right().as_vec3();
    let right_flat = Vec3::new(right.x, 0.0, right.z).normalize_or_zero();

    movement.direction = (forward_flat * axis(&keyboard, KeyCode::KeyW, KeyCode::KeyS)
        + right_flat * axis(&keyboard, KeyCode::KeyD, KeyCode::KeyA))
    .normalize_or_zero();

    movement.look_direction = camera.forward().as_vec3();

    movement.jump = keyboard.pressed(KeyCode::Space);
    movement.sneak = keyboard.pressed(KeyCode::ShiftLeft);
    movement.sprint = keyboard.pressed(KeyCode::ControlLeft);
}

fn draw_targeted_block(
    mut gizmos: Gizmos,
    spatial_query: SpatialQuery,
    camera: Single<&GlobalTransform, With<PlayerCamera>>,
    chunks: Query<(), With<VoxelChunk>>,
) {
    let Some(voxel) = targeted_voxel(
        &spatial_query,
        &chunks,
        camera.translation(),
        camera.forward(),
    ) else {
        return;
    };

    gizmos.cube(
        Transform::from_translation(voxel.as_vec3() + Vec3::splat(0.5))
            .with_scale(Vec3::splat(BLOCK_OUTLINE_SIZE)),
        BLACK,
    );
}

fn targeted_voxel(
    spatial_query: &SpatialQuery,
    chunks: &Query<(), With<VoxelChunk>>,
    origin: Vec3,
    direction: Dir3,
) -> Option<IVec3> {
    let hit = spatial_query.cast_ray_predicate(
        origin,
        direction,
        BLOCK_REACH,
        false,
        &SpatialQueryFilter::DEFAULT,
        &|entity| chunks.contains(entity),
    )?;
    let hit_point = origin + direction * hit.distance;

    Some(voxel_from_hit(hit_point, hit.normal))
}

fn voxel_from_hit(hit_point: Vec3, normal: Vec3) -> IVec3 {
    (hit_point - normal * HIT_EPSILON).floor().as_ivec3()
}

fn axis(input: &ButtonInput<KeyCode>, positive: KeyCode, negative: KeyCode) -> f32 {
    (input.pressed(positive) as i8 - input.pressed(negative) as i8) as f32
}
