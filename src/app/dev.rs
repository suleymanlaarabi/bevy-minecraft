//! Development-only diagnostics, enabled by the existing environment flags.

use crate::game::GameState;
use avian3d::prelude::PhysicsDiagnosticsPlugin;
use bevy::prelude::*;
use bevy::{
    dev_tools::fps_overlay::{FpsOverlayConfig, FpsOverlayPlugin, FrameTimeGraphConfig},
    diagnostic::{
        EntityCountDiagnosticsPlugin, LogDiagnosticsPlugin, SystemInformationDiagnosticsPlugin,
    },
    render::{
        diagnostic::{
            MeshAllocatorDiagnosticPlugin, RenderAssetDiagnosticPlugin, RenderDiagnosticsPlugin,
        },
        mesh::RenderMesh,
    },
};

pub(super) struct DevelopmentPlugin;

impl Plugin for DevelopmentPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(FpsOverlayPlugin {
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
        });
        if std::env::var_os("HOLLOW_DIAGNOSTICS").is_some() {
            app.add_plugins((
                PhysicsDiagnosticsPlugin,
                SystemInformationDiagnosticsPlugin,
                EntityCountDiagnosticsPlugin::default(),
                RenderDiagnosticsPlugin,
                MeshAllocatorDiagnosticPlugin,
                RenderAssetDiagnosticPlugin::<RenderMesh>::new(" meshes"),
                LogDiagnosticsPlugin {
                    wait_duration: core::time::Duration::from_secs(5),
                    ..default()
                },
            ));
        }
        if std::env::var_os("HOLLOW_DIAGNOSTICS_AUTOPLAY").is_some() {
            app.add_systems(Startup, dev_diagnostics_autoplay);
        }
    }
}

fn dev_diagnostics_autoplay(mut next_state: ResMut<NextState<GameState>>) {
    if std::env::var_os("HOLLOW_DIAGNOSTICS_AUTOPLAY").is_some() {
        next_state.set(GameState::Game);
    }
}
