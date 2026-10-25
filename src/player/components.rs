use super::interaction;
use crate::{
    character::{AutoJump, CHARACTER_GRAVITY_SCALE, GameCharacter},
    game::GameState,
    inventory::PlayerInventory,
    spatial::GameLayer,
    voxel::VoxelViewer,
};
use avian3d::prelude::*;
use bevy::{
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};

const PLAYER_WIDTH: f32 = 0.6;
const PLAYER_HEIGHT: f32 = 1.8;

fn default_friction() -> Friction {
    Friction::ZERO.with_combine_rule(CoefficientCombine::Min)
}

fn default_restitution() -> Restitution {
    Restitution::ZERO.with_combine_rule(CoefficientCombine::Min)
}

#[derive(Component, Default, Debug, Clone)]
#[require(
    PlayerInventory,
    super::safety::PlayerCheckpoint,
    GameCharacter,
    AutoJump,
    RigidBody::Dynamic,
    // Physics ticks at 64 Hz: ease the position the camera follows between ticks.
    // Rotation is driven by the mouse every frame and must not be eased.
    TranslationInterpolation,
    Collider::cuboid(PLAYER_WIDTH, PLAYER_HEIGHT, PLAYER_WIDTH),
    LockedAxes::ROTATION_LOCKED,
    GravityScale(CHARACTER_GRAVITY_SCALE),
    LinearDamping::default(),
    Friction = default_friction(),
    Restitution = default_restitution(),
    CollisionLayers::new(GameLayer::Player, [GameLayer::Default, GameLayer::World]),
    Visibility::default(),
    DespawnOnExit::<GameState>(GameState::Game)
)]
pub struct Player;

#[derive(Debug, Component, Default, Clone)]
#[require(
    CameraSensitivity,
    interaction::BlockTargetState,
    interaction::BreakingState,
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

#[derive(Component, Clone, Default)]
pub(crate) struct Crosshair;
