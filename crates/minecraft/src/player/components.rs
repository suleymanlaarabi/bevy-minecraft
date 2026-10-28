use super::interaction;
use crate::{
    character::{AutoJump, GameCharacter},
    game::GameState,
    inventory::PlayerInventory,
    voxel::VoxelViewer,
};
use bevy::{
    camera::Exposure,
    core_pipeline::tonemapping::Tonemapping,
    pbr::{AtmosphereSettings, DistanceFog, FogFalloff},
    prelude::*,
};

#[derive(Component, Default, Debug, Clone)]
#[require(
    PlayerInventory,
    crate::dropped_item::ItemCollector,
    crate::audio::MovementAudio,
    crate::audio::EquipmentAudio,
    crate::inventory::HotbarSelection,
    crate::inventory::CraftingResult,
    crate::inventory::CraftingGrid,
    super::safety::PlayerCheckpoint,
    GameCharacter,
    AutoJump,
    crate::survival::Living,
    crate::survival::Nutrition,
    crate::survival::UsingItem,
    crate::survival::RespawnPoint,
    Visibility::default(),
    DespawnOnExit::<GameState>(GameState::Game)
)]
pub struct Player;

#[derive(Debug, Component, Default, Clone)]
#[require(
    CameraSensitivity,
    interaction::BlockTargetState,
    interaction::BreakingState,
    super::skin::ArmSwing,
    Camera3d,
    SpatialListener,
    IsDefaultUiCamera,
    // Exposure, ambient light and fog follow the day cycle
    // (`crate::voxel::render::environment`); these are the first frame's values.
    AmbientLight,
    Exposure,
    DistanceFog {
        // Forward scattering around the sun and moon (`voxel_fog.wesl`).
        directional_light_color: crate::voxel::OPEN_AIR_SCATTERING,
        directional_light_exponent: 12.0,
        falloff: FogFalloff::Linear {
            start: 100.0,
            end: 160.0,
        },
        ..default()
    },
    // Physical sky and aerial perspective. Terrain ends within a few hundred
    // meters, so the aerial-view slices only need to cover the clouds' reach.
    AtmosphereSettings {
        aerial_view_lut_max_distance: 2_000.0,
        ..default()
    },
    Tonemapping::AgX,
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

impl PlayerCamera {
    /// Eye orientation: the body's yaw followed by the camera pitch.
    pub fn look(&self, body: Quat) -> Quat {
        body * Quat::from_rotation_x(self.pitch)
    }
}

#[derive(Debug, Component, Deref, DerefMut)]
pub struct CameraSensitivity(pub Vec2);

impl Default for CameraSensitivity {
    fn default() -> Self {
        Self(Vec2::new(0.0025, 0.002))
    }
}
