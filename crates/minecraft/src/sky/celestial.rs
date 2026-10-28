#![allow(clippy::needless_arbitrary_self_type)]

use super::{
    SkyState,
    cycle::{MOON_LUX, SUN_LUX},
};
use crate::{game::GameState, settings::GraphicsSettings, voxel::SHADOW_LAYER};
use bevy::{
    camera::visibility::{NoFrustumCulling, RenderLayers},
    light::{
        Atmosphere, CascadeShadowConfigBuilder, NotShadowCaster, SunDisk,
        atmosphere::ScatteringMedium,
    },
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
};

/// Output-space radiance of the sun's disk at its center: far past white, so bloom
/// gives it a halo.
const SUN_DISK_RADIANCE: f32 = 40.0;
/// A stylized disk large enough to read clearly at the player's wide field of view.
const SUN_RADIUS: f32 = 0.025;
/// Sort key far behind every other transparent mesh, with the moon's.
const SUN_SORT_BIAS: f32 = -1.0e8;
/// Sine of the elevation where astronomical twilight ends, 18° below the horizon:
/// lower, a light no longer brightens the sky.
const TWILIGHT_END: f32 = -0.309;

#[derive(Component)]
pub(super) struct Sun;

/// The sun or the moon. [`update_lights`] is the only writer of its
/// `shadow_maps_enabled`.
#[derive(Component)]
pub(super) struct SkyLight;

#[derive(Clone, Copy, Debug, Default, PartialEq, ShaderType)]
struct SunSettings {
    /// Unit vector toward the sun in XYZ, angular radius of the disk in W.
    direction: Vec4,
    /// Output-space color of the disk's center in RGB.
    color: Vec4,
}

/// The visible sun. It follows the smooth sun direction, while the light turns by
/// steps to keep shadows still, so Bevy's own disk (drawn at the light) is off.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub(crate) struct SunMaterial {
    #[uniform(0)]
    settings: SunSettings,
}

sky_material!(
    SunMaterial,
    "shaders/sky_sun.wesl",
    AlphaMode::Add,
    SUN_SORT_BIAS,
    [ATTRIBUTE_POSITION => 0]
);

#[derive(Component)]
pub(super) struct SunDiskMesh;

fn sun_settings(sky: &SkyState) -> SunSettings {
    SunSettings {
        direction: sky.sun_direction.extend(SUN_RADIUS),
        color: (Vec3::new(1.0, 0.98, 0.95) * SUN_DISK_RADIANCE).extend(1.0),
    }
}

pub(super) fn spawn_sky(
    mut commands: Commands,
    mut media: ResMut<Assets<ScatteringMedium>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut suns: ResMut<Assets<SunMaterial>>,
    sky: Res<SkyState>,
) {
    // Earth's planet sits below the origin: world Y is the altitude in meters,
    // one block being one meter.
    commands.spawn((
        Atmosphere::earth(media.add(ScatteringMedium::default())),
        DespawnOnExit(GameState::Game),
    ));

    // Both lights reach the camera's layer and the terrain's shadow layer. Cascades
    // stop where terrain shadows stop mattering, keeping the near cascade sharp.
    let cascades = CascadeShadowConfigBuilder {
        num_cascades: 4,
        first_cascade_far_bound: 12.0,
        maximum_distance: 128.0,
        ..default()
    }
    .build();
    let layers = RenderLayers::from_layers(&[
        0,
        SHADOW_LAYER,
        crate::voxel::render::visibility::OCCLUDED_LAYER,
    ]);
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
        // nothing: `SunMaterial` draws the sun.
        SunDisk {
            intensity: 0.0,
            ..SunDisk::EARTH
        },
        cascades.clone(),
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
        // The moon's disk is drawn by `night::MoonMaterial`, with its phases.
        SunDisk::OFF,
        cascades,
        light_transform(sky.moon_light_direction),
        layers,
        DespawnOnExit(GameState::Game),
    ));
    // Placed at infinity by its vertex shader: no bounding box can cull it.
    commands.spawn((
        SunDiskMesh,
        Mesh3d(meshes.add(Rectangle::new(2.0, 2.0))),
        MeshMaterial3d(suns.add(SunMaterial {
            settings: sun_settings(&sky),
        })),
        Transform::default(),
        NoFrustumCulling,
        NotShadowCaster,
        DespawnOnExit(GameState::Game),
    ));
}

/// Moves the visible sun every frame the sky changes, and hides it below the horizon
/// where the atmosphere blocks it entirely.
pub(super) fn update_sun_disk(
    sky: Res<SkyState>,
    mut disks: Query<(&mut Visibility, &MeshMaterial3d<SunMaterial>), With<SunDiskMesh>>,
    mut materials: ResMut<Assets<SunMaterial>>,
) {
    if !sky.is_changed() {
        return;
    }
    let settings = sun_settings(&sky);
    let shown = settings.direction.y > -settings.direction.w;
    for (mut visibility, material) in &mut disks {
        visibility.set_if_neq(if shown {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        });
        if !shown {
            continue;
        }
        if let Some(mut sun) = materials.get_mut(&material.0) {
            sun.settings = settings;
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
    for (mut light, mut transform, mut visibility, is_sun) in &mut lights {
        let (direction, casts) = if is_sun {
            (sky.sun_light_direction, sky.sun_is_up())
        } else {
            (sky.moon_light_direction, !sky.sun_is_up())
        };
        transform.set_if_neq(light_transform(direction));
        visibility.set_if_neq(if direction.y < TWILIGHT_END {
            Visibility::Hidden
        } else {
            Visibility::Inherited
        });
        let shadow_maps_enabled = shadows && casts;
        if light.shadow_maps_enabled != shadow_maps_enabled {
            light.shadow_maps_enabled = shadow_maps_enabled;
        }
    }
}
