//! Sun and moon shadows as a cached clipmap instead of Bevy's view-frustum cascades.
//!
//! Each cascade is a light-aligned box centered on the camera, so it depends on
//! neither the camera's rotation nor its field of view and stays valid while the
//! camera turns. Only the nearest cascade is redrawn every frame; the others take
//! turns, one per frame, and keep their matrix and shadow map in between. A wide
//! field of view also made Bevy's frustum fit about half as sharp for the same map.
//!
//! Terrain and model shaders pick the first box holding a fragment
//! (`block_lighting.wesl`); Bevy's own materials keep its view-depth split, whose
//! bounds are shrunk here so that every depth range stays inside its box.

use crate::VoxelViewer;
use bevy::{
    light::{
        CascadeShadowConfig, Cascades, DirectionalLightShadowMap, SimulationLightSystems,
        cascade::Cascade,
    },
    pbr::{LightEntity, ViewLightEntities},
    prelude::*,
    render::{
        RenderApp,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        sync_world::MainEntity,
        view::ExtractedView,
    },
};

/// Half the side of each cascade's box, in blocks. Every level is sharper than the
/// frustum cascade it replaces (12, 26, 58 and 128 blocks deep).
const HALF_EXTENTS: [f32; 4] = [12.0, 28.0, 64.0, 144.0];
/// The cascade redrawn beside the nearest one, by frame: the second at half rate,
/// the two widest at quarter rate.
const ROTATION: [usize; 4] = [1, 2, 1, 3];
/// Fraction of a box's half extent the camera may drift from its center before the
/// cascade is redrawn out of turn (teleports, very low frame rates).
const MAX_DRIFT: f32 = 0.1;
/// Part of a box that Bevy's view-depth split may use: the rest absorbs the drift of
/// a cached cascade and the filter kernel.
const DEPTH_SPLIT_REACH: f32 = 0.85;

/// Marks a directional light whose cascades this module owns. One such light casts
/// shadows at a time (the sun or the moon).
#[derive(Component, Clone, Copy, Debug, Default)]
pub struct ShadowClipmap;

/// Cascades of the viewer's camera drawn this frame; the others keep their map.
#[derive(Resource, Clone, Copy, Default, ExtractResource)]
#[extract_app(RenderApp)]
pub(crate) struct ShadowRefresh {
    camera: Option<Entity>,
    drawn: u8,
}
impl ShadowRefresh {
    /// Whether `view` is a cascade that keeps last frame's shadow map.
    pub fn cached(&self, view: &ExtractedView, light: &LightEntity) -> bool {
        let LightEntity::Directional { cascade_index, .. } = light else {
            return false;
        };
        self.camera.is_some_and(|camera| {
            view.retained_view_entity.auxiliary_entity == Some(MainEntity::from(camera))
                && self.drawn & (1 << cascade_index) == 0
        })
    }
}

#[derive(Default)]
struct ClipmapState {
    light: Option<Entity>,
    camera: Option<Entity>,
    map_size: usize,
    frame: usize,
    cascades: Vec<Cascade>,
    /// Camera position each cascade was last centered on.
    centers: [Vec3; HALF_EXTENTS.len()],
}

pub(crate) fn register(app: &mut App) {
    app.init_resource::<ShadowRefresh>()
        .add_plugins(ExtractResourcePlugin::<ShadowRefresh>::default())
        .add_systems(
            PostUpdate,
            update_shadow_clipmap
                .after(SimulationLightSystems::UpdateDirectionalLightCascades)
                .before(SimulationLightSystems::UpdateLightFrusta),
        );
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        // `keep_cached_cascades` is scheduled with the section draws it gates.
        render.init_resource::<ShadowRefresh>();
    }
}

fn update_shadow_clipmap(
    map: Res<DirectionalLightShadowMap>,
    camera: Option<Single<(Entity, &GlobalTransform, &Projection), With<VoxelViewer>>>,
    mut lights: Query<
        (
            Entity,
            &GlobalTransform,
            &DirectionalLight,
            &mut CascadeShadowConfig,
            &mut Cascades,
        ),
        With<ShadowClipmap>,
    >,
    mut refresh: ResMut<ShadowRefresh>,
    mut state: Local<ClipmapState>,
) {
    // Bevy has just rebuilt its frustum cascades for every active camera.
    let casting = camera
        .as_deref()
        .and_then(|&(viewer, transform, projection)| {
            lights
                .iter_mut()
                .find(|(_, _, light, _, cascades)| {
                    light.shadow_maps_enabled && cascades.cascades.contains_key(&viewer)
                })
                .map(|light| (viewer, transform.translation(), projection, light))
        });
    let Some((viewer, eye, projection, (light, light_transform, _, mut config, mut cascades))) =
        casting
    else {
        *state = default();
        *refresh = default();
        return;
    };

    // A fragment at view depth `d` lies up to `d * reach` from the camera, at the
    // corners of the screen. Bevy's split never falls back to a wider cascade, so
    // each depth range must fit its box; past the last box there is no shadow either
    // way, and its range keeps the whole depth of the box.
    let reach = match projection {
        Projection::Perspective(perspective) => {
            let tan = (perspective.fov * 0.5).tan();
            (1.0 + tan * tan * (1.0 + perspective.aspect_ratio * perspective.aspect_ratio)).sqrt()
        }
        _ => 1.0,
    };
    let mut bounds = HALF_EXTENTS.map(|half| half * DEPTH_SPLIT_REACH / reach);
    bounds[HALF_EXTENTS.len() - 1] *= reach;
    if config.bounds != bounds {
        config.bounds = bounds.to_vec();
    }

    let count = HALF_EXTENTS.len();
    let reset = state.light != Some(light)
        || state.camera != Some(viewer)
        || state.map_size != map.size
        || state.cascades.len() != count;
    if reset {
        state.cascades.resize(count, Cascade::default());
    }
    state.light = Some(light);
    state.camera = Some(viewer);
    state.map_size = map.size;
    state.frame = state.frame.wrapping_add(1);

    let rotation = light_transform.rotation();
    let turn = ROTATION[state.frame % ROTATION.len()];
    let mut drawn = 0u8;
    for (index, half) in HALF_EXTENTS.into_iter().enumerate() {
        let drifted = (eye - state.centers[index]).abs().max_element() > half * MAX_DRIFT;
        if reset || index == 0 || index == turn || drifted {
            state.cascades[index] = clipmap_cascade(eye, rotation, half, map.size as f32);
            state.centers[index] = eye;
            drawn |= 1 << index;
        }
    }
    cascades.cascades.insert(viewer, state.cascades.clone());
    *refresh = ShadowRefresh {
        camera: Some(viewer),
        drawn,
    };
}

/// A box of half side `half` around `eye`, aligned with the light and snapped to
/// its own texels. Mirrors Bevy's `calculate_cascade`, which keeps the cascade
/// matrix numerically stable by building it directly from the light's rotation.
fn clipmap_cascade(eye: Vec3, rotation: Quat, half: f32, map_size: f32) -> Cascade {
    let world_from_light = Mat4::from_quat(rotation);
    let light_from_world = world_from_light.transpose();
    let texel_size = 2.0 * half / map_size;
    let center = (light_from_world.transform_point3(eye) / texel_size).floor() * texel_size;
    // The light looks along -Z: the near plane is the box's face toward the light.
    let near_plane_center = center + Vec3::Z * half;
    let cascade_from_world = Mat4::from_cols(
        light_from_world.x_axis,
        light_from_world.y_axis,
        light_from_world.z_axis,
        (-near_plane_center).extend(1.0),
    );
    let world_from_cascade = Mat4::from_cols(
        world_from_light.x_axis,
        world_from_light.y_axis,
        world_from_light.z_axis,
        world_from_light * near_plane_center.extend(1.0),
    );
    // Reverse Z over the box's depth, as Bevy's orthographic cascades.
    let clip_from_cascade = Mat4::from_cols(
        Vec4::new(1.0 / half, 0.0, 0.0, 0.0),
        Vec4::new(0.0, 1.0 / half, 0.0, 0.0),
        Vec4::new(0.0, 0.0, 0.5 / half, 0.0),
        Vec4::new(0.0, 0.0, 1.0, 1.0),
    );
    Cascade {
        world_from_cascade,
        clip_from_cascade,
        clip_from_world: clip_from_cascade * cascade_from_world,
        texel_size,
    }
}

/// Bevy draws the shadow pass of every cascade a camera lists, clearing its map
/// first. Cached cascades leave the list: their map and matrix stay last drawn.
pub(crate) fn keep_cached_cascades(
    refresh: Res<ShadowRefresh>,
    mut cameras: Query<&mut ViewLightEntities>,
    cascades: Query<(&ExtractedView, &LightEntity)>,
) {
    for mut lights in &mut cameras {
        lights.lights.retain(|&entity| {
            cascades
                .get(entity)
                .map_or(true, |(view, light)| !refresh.cached(view, light))
        });
    }
}
