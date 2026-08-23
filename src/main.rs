use avian3d::prelude::*;
use bevy::{
    camera_controller::free_camera::{FreeCamera, FreeCameraPlugin},
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    prelude::*,
};

use crate::voxel::{VoxelPlugin, VoxelSettings, VoxelViewer};

pub mod voxel;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins,
            PhysicsPlugins::default(),
            // PhysicsDebugPlugin,
            FreeCameraPlugin,
            VoxelPlugin::new(VoxelSettings {
                view_distance: 10,
                ..default()
            }),
            FpsOverlayPlugin {
                config: FpsOverlayConfig {
                    text_config: TextFont {
                        font_size: FontSize::Px(42.0),
                        ..default()
                    },
                    text_color: Color::WHITE,
                    refresh_interval: core::time::Duration::from_millis(100),
                    enabled: true,
                    frame_time_graph_config: FrameTimeGraphConfig {
                        enabled: true,
                        min_fps: 30.0,
                        target_fps: 144.0,
                    },
                },
            },
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(
    mut commands: Commands,
    voxel_settings: Res<VoxelSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: true,
            contact_shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(20.0, 30.0, 20.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
    ));

    commands.spawn((
        Camera3d::default(),
        VoxelViewer,
        Transform::from_xyz(24.0, 22.0, 28.0)
            .looking_at(voxel_settings.chunk_center(IVec2::ZERO), Vec3::Y),
        FreeCamera {
            walk_speed: 20.,
            ..default()
        },
    ));

    let center = voxel_settings.chunk_center(IVec2::ZERO);
    commands.spawn((
        Name::new("Physics test cube"),
        RigidBody::Dynamic,
        Collider::cuboid(1.0, 1.0, 1.0),
        Mesh3d(meshes.add(Cuboid::from_length(1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.2, 0.15))),
        Transform::from_xyz(center.x, voxel_settings.base_height + 10.0, center.z),
    ));
}
