use super::{
    SkyState,
    cycle::{MOON_LUX, SUN_LUX},
};
use crate::{game::GameState, settings::GraphicsSettings};
use bevy::{camera::visibility::RenderLayers, light::SunDisk, prelude::*};
use bevy_voxel::features::query::{set_field_if_neq, set_if_neq};
use bevy_voxel::render::{
    environment::{SkyPass, SkySettings},
    shadows::ShadowClipmap,
};

/// Output-space radiance of the sun's disk at its center: far past white, so bloom
/// gives it a halo.
const SUN_DISK_RADIANCE: f32 = 40.0;
/// A stylized disk large enough to read clearly at the player's wide field of view.
const SUN_RADIUS: f32 = 0.025;
/// Sine of the elevation where astronomical twilight ends, 18° below the horizon:
/// lower, a light no longer brightens the sky.
const TWILIGHT_END: f32 = -0.309;

use bevy_voxel::render::sky::{SkyAssets, SkyBackground};

#[derive(Component)]
pub(super) struct Sun;

/// The sun or the moon. [`update_lights`] is the only writer of its
/// `shadow_maps_enabled`.
#[derive(Component)]
pub(super) struct SkyLight;

/// The visible sun. It follows the smooth sun direction, while the light turns by
/// steps to keep shadows still, so Bevy's own disk (drawn at the light) is off.
#[derive(Component)]
pub(super) struct SunDiskMesh;

/// Read through the sky palette ([`bevy_voxel::render::environment::SkyLighting`]).
pub(crate) fn sun_settings(sky: &SkyState) -> SkySettings {
    let height = (sky.sun_direction.y * 5.0 + 0.2).clamp(0.0, 1.0);
    SkySettings {
        direction: sky.sun_direction.extend(SUN_RADIUS),
        color: (Vec3::new(1.0, 0.35 + 0.63 * height, 0.12 + 0.83 * height)
            * (SUN_DISK_RADIANCE * height))
            .extend(1.0),
        params: Vec4::ZERO,
    }
}

pub(super) fn spawn_sky(mut commands: Commands, mut assets: SkyAssets, sky: Res<SkyState>) {
    commands.spawn((
        SkyBackground,
        assets.surface(
            bevy_voxel::render::sky::background_mesh(),
            SkyPass::Background,
        ),
        bevy::camera::primitives::Aabb {
            center: bevy::math::Vec3A::ZERO,
            half_extents: bevy::math::Vec3A::ONE,
        },
        DespawnOnExit(GameState::Game),
    ));

    // Both lights reach the camera's layer and the terrain's shadow layer. Their
    // cascades are the camera-centered shadow clipmap of the voxel renderer.
    let layers = RenderLayers::from_layers(&[0, bevy_voxel::render::visibility::OCCLUDED_LAYER]);
    commands.spawn((
        Sun,
        SkyLight,
        DirectionalLight {
            illuminance: SUN_LUX,
            color: Color::srgb(1.0, 0.98, 0.95),
            // `update_lights` enables it before the first frame renders.
            shadow_maps_enabled: false,
            ..default()
        },
        // Keeps the disk's size for the light fading out at the horizon, but draws
        // nothing: `SkyPass::Sun` draws the sun.
        SunDisk {
            intensity: 0.0,
            ..SunDisk::EARTH
        },
        ShadowClipmap,
        light_transform(sky.sun_light_direction),
        layers.clone(),
        DespawnOnExit(GameState::Game),
    ));
    commands.spawn((
        SkyLight,
        DirectionalLight {
            illuminance: MOON_LUX,
            color: Color::srgb(0.62, 0.72, 1.0),
            shadow_maps_enabled: false,
            ..default()
        },
        // The moon's disk is drawn by `SkyPass::Moon`, with its phases.
        SunDisk::OFF,
        ShadowClipmap,
        light_transform(sky.moon_light_direction),
        layers,
        DespawnOnExit(GameState::Game),
    ));
    // Placed at infinity by its vertex shader: no bounding box can cull it.
    commands.spawn((
        SunDiskMesh,
        assets.surface(Rectangle::new(2.0, 2.0).into(), SkyPass::Sun),
        DespawnOnExit(GameState::Game),
    ));
}

/// Hides the visible sun below the horizon. Its position and color follow the sky
/// palette, written from [`sun_settings`].
pub(super) fn update_sun_disk(
    sky: Res<SkyState>,
    mut disks: Query<&mut Visibility, With<SunDiskMesh>>,
) {
    let settings = sun_settings(&sky);
    let shown = settings.direction.y > -settings.direction.w;
    let target = if shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    for mut visibility in disks.contiguous_iter_mut().expect("Dense sun disk query") {
        for index in 0..visibility.len() {
            set_if_neq(&mut visibility, index, target);
        }
    }
}

fn light_transform(toward_light: Vec3) -> Transform {
    Transform::default().looking_to(-toward_light, Vec3::Y)
}

/// Turns both lights and hands the shadow map to whichever is above the horizon.
/// Only one shadow map is ever rendered. A light past twilight is hidden: it no
/// longer lights the sky, and every lit pixel then loops over one light only.
pub(super) fn update_lights(
    sky: Res<SkyState>,
    settings: Res<GraphicsSettings>,
    mut lights: Query<
        (
            &mut DirectionalLight,
            &mut Transform,
            &mut Visibility,
            Has<Sun>,
        ),
        With<SkyLight>,
    >,
) {
    let shadows = settings.shadow_quality != crate::settings::ShadowQuality::Off;
    for (mut lights, mut transforms, mut visibility, is_sun) in
        lights.contiguous_iter_mut().expect("Dense sky light query")
    {
        let (direction, casts) = if is_sun {
            (sky.sun_light_direction, sky.sun_is_up())
        } else {
            (sky.moon_light_direction, !sky.sun_is_up())
        };
        let transform = light_transform(direction);
        let target = if direction.y < TWILIGHT_END {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        };
        let height = (direction.y * 5.0 + 0.2).clamp(0.0, 1.0);
        let color = if is_sun {
            Color::srgb(1.0, 0.35 + 0.63 * height, 0.12 + 0.83 * height)
        } else {
            Color::srgb(0.62, 0.72, 1.0)
        };
        let illuminance = if is_sun { SUN_LUX } else { MOON_LUX } * (0.75 * height);
        let shadow_maps_enabled = shadows && casts;
        for index in 0..lights.len() {
            set_if_neq(&mut transforms, index, transform);
            set_if_neq(&mut visibility, index, target);
            set_field_if_neq(&mut lights, index, |light| &mut light.color, color);
            set_field_if_neq(
                &mut lights,
                index,
                |light| &mut light.illuminance,
                illuminance,
            );
            set_field_if_neq(
                &mut lights,
                index,
                |light| &mut light.shadow_maps_enabled,
                shadow_maps_enabled,
            );
        }
    }
}
