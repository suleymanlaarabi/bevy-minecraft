use crate::voxel::{VoxelKind, VoxelViewer, VoxelWorld};
use bevy::{
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};

pub(in crate::voxel) fn update_underwater_effect(
    voxel_world: VoxelWorld,
    mut camera: Single<(&Transform, &mut DistanceFog, &mut AmbientLight), With<VoxelViewer>>,
    mut clear_color: ResMut<ClearColor>,
    mut previous: Local<Option<Option<VoxelKind>>>,
) {
    let liquid = voxel_world
        .state(camera.0.translation.floor().as_ivec3())
        .and_then(|state| {
            if state.kind() == VoxelKind::Lava {
                Some(VoxelKind::Lava)
            } else if state.has_water() {
                Some(VoxelKind::Water)
            } else {
                None
            }
        });

    if *previous == Some(liquid) {
        return;
    }
    *previous = Some(liquid);

    let (_, fog, ambient) = &mut *camera;
    if liquid == Some(VoxelKind::Lava) {
        fog.color = Color::srgb_u8(210, 80, 10);
        fog.falloff = FogFalloff::Linear {
            start: 0.0,
            end: 1.5,
        };
        ambient.color = Color::srgb_u8(255, 140, 60);
        ambient.brightness = 400.0;
        clear_color.0 = Color::srgb_u8(210, 80, 10);
    } else if liquid.is_some() {
        // A darkened vanilla water tint (#3F76E4).
        fog.color = Color::srgb_u8(24, 62, 138);
        fog.falloff = FogFalloff::Linear {
            start: 2.0,
            end: 40.0,
        };
        ambient.color = Color::srgb_u8(95, 140, 215);
        ambient.brightness = 280.0;
        clear_color.0 = Color::srgb_u8(24, 62, 138);
    } else {
        fog.color = Color::srgb_u8(195, 222, 255);
        fog.falloff = FogFalloff::Linear {
            start: 160.0,
            end: 280.0,
        };
        ambient.color = Color::srgb_u8(215, 235, 255);
        ambient.brightness = 500.0;
        clear_color.0 = Color::srgb_u8(148, 195, 255);
    }
}
