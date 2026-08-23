use avian3d::prelude::*;
use bevy::{
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    prelude::*,
    window::{CursorOptions, WindowMode},
};

use crate::{
    player::PlayerPlugin,
    voxel::{VoxelPlugin, VoxelSettings},
};

mod player;
pub mod voxel;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    mode: WindowMode::BorderlessFullscreen(MonitorSelection::Primary),

                    ..default()
                }),
                primary_cursor_options: Some(CursorOptions {
                    visible: false,
                    ..default()
                }),
                ..default()
            }),
            PhysicsPlugins::default(),
            VoxelPlugin::new(VoxelSettings {
                view_distance: 20,
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
                        target_fps: 120.0,
                    },
                },
            },
            PlayerPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, close_on_esc)
        .run();
}

fn close_on_esc(
    keyboard_input: Res<ButtonInput<KeyCode>>,
    mut app_exit_events: MessageWriter<AppExit>,
) {
    if keyboard_input.just_pressed(KeyCode::Escape) {
        app_exit_events.write(AppExit::Success);
    }
}

fn setup(
    mut commands: Commands,
    voxel_settings: Res<VoxelSettings>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    commands.spawn((
        DirectionalLight {
            shadow_maps_enabled: false,
            contact_shadows_enabled: false,
            ..default()
        },
        Transform::from_xyz(20.0, 30.0, 20.0).looking_at(Vec3::new(8.0, 0.0, 8.0), Vec3::Y),
    ));

    let center = voxel_settings.chunk_center(IVec2::ZERO);
    commands.spawn((
        RigidBody::Dynamic,
        Collider::cuboid(1.0, 1.0, 1.0),
        Mesh3d(meshes.add(Cuboid::from_length(1.0))),
        MeshMaterial3d(materials.add(Color::srgb(0.8, 0.2, 0.15))),
        Transform::from_xyz(center.x, voxel_settings.base_height + 10.0, center.z),
    ));
}
