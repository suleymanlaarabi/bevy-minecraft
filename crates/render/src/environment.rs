use crate::{RenderSource, VoxelViewer};
use bevy::{
    camera::Exposure,
    light::{NotShadowCaster, NotShadowReceiver},
    mesh::{CylinderAnchor, MeshVertexBufferLayoutRef},
    pbr::{DistanceFog, FogFalloff, Material, MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    render::storage::ShaderBuffer,
    shader::ShaderRef,
};
use hollow_core::{StateId, VoxelCell, coordinates::CHUNK_SIZE};
use hollow_engine::{
    session::SessionMember,
    world::{VoxelWorld, WorldLayout},
};

/// Marks open-air fog and supplies directional scattering for standard materials.
/// Liquids have none, which also switches voxel fog to a spherical falloff.
pub const OPEN_AIR_SCATTERING: Color = Color::srgba(1.0, 0.9, 0.75, 0.3);

/// The liquid the camera is in, if any.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct CameraLiquid(pub Option<hollow_core::FluidId>);

/// The camera's surroundings: the day cycle's light and fog in the open, or the
/// murk of the liquid it is in. The only writer of these camera components.
pub fn update_camera_environment<C: RenderSource>(
    voxel_world: VoxelWorld<C::Cell>,
    source: Res<C>,
    sky: Res<SkyLighting>,
    settings: Res<EnvironmentSettings>,
    mut camera: Single<
        (
            &Transform,
            &mut DistanceFog,
            &mut AmbientLight,
            &mut Exposure,
        ),
        With<VoxelViewer>,
    >,
    mut camera_liquid: ResMut<CameraLiquid>,
    effects: Res<ViewEffects>,
    mut previous_effects: Local<ViewEffects>,
) {
    let effect_flags = *effects;
    let liquid = voxel_world
        .state(camera.0.translation.floor().as_ivec3())
        .and_then(|state| {
            let fluid = source.content().state(StateId(state.id())).fluid.id;
            (fluid != hollow_core::FluidId::NONE && source.fluids().profile(fluid).fog.is_some())
                .then_some(fluid)
        });

    // A newly spawned camera also needs initialization when the day is frozen.
    if camera_liquid.0 == liquid
        && !sky.is_changed()
        && !settings.is_changed()
        && !camera.1.is_added()
        && *previous_effects == effect_flags
    {
        return;
    }
    camera_liquid.set_if_neq(CameraLiquid(liquid));
    *previous_effects = effect_flags;

    let (_, fog, ambient, exposure) = &mut *camera;
    exposure.ev100 = sky.ev100;
    // Ambient brightness is pre-exposure: liquids keep a steady look at any hour by
    // giving it in output space.
    let output = 1.0 / sky.exposure();
    if let Some(fluid) = liquid {
        let profile = source
            .fluids()
            .profile(fluid)
            .fog
            .expect("Camera medium with configured fog");
        let daylight = if profile.daylight {
            0.15 + 0.85 * (1.0 - sky.night)
        } else {
            1.0
        };
        let tint = Color::srgb_from_array(profile.color).to_linear();
        fog.color = LinearRgba::rgb(
            tint.red * daylight,
            tint.green * daylight,
            tint.blue * daylight,
        )
        .into();
        fog.directional_light_color = Color::NONE;
        fog.falloff = FogFalloff::Linear {
            start: profile.start,
            end: profile.end,
        };
        ambient.color = Color::srgb_from_array(profile.ambient);
        ambient.brightness = profile.brightness
            * if profile.daylight {
                sky.ambient_brightness
            } else {
                output
            };
    } else {
        // Streaming is centered on the viewer's chunk, not their exact position.
        // One chunk diagonal keeps the opaque boundary inside the loaded circle
        // throughout movement across that chunk, including negative coordinates.
        let radius = terrain_radius(&settings) - CHUNK_SIZE as f32 * std::f32::consts::SQRT_2;
        fog.color = sky.horizon_color;
        fog.directional_light_color = OPEN_AIR_SCATTERING;
        fog.falloff = FogFalloff::Linear {
            start: radius * 0.6,
            end: radius,
        };
        ambient.color = sky.ambient_color;
        ambient.brightness = sky.ambient_brightness;
    }
    if effect_flags.minimum_ambient > 0.0 {
        ambient.color = Color::WHITE;
        ambient.brightness = ambient
            .brightness
            .max(effect_flags.minimum_ambient * output);
    }
    if effect_flags.fog_distance.is_some() {
        fog.color = Color::BLACK;
        fog.directional_light_color = Color::NONE;
        fog.falloff = FogFalloff::Linear {
            start: 0.0,
            end: effect_flags.fog_distance.unwrap_or_default(),
        };
    }
}

/// Nominal loaded terrain radius, before the fog's chunk-alignment safety margin.
pub fn terrain_radius(settings: &EnvironmentSettings) -> f32 {
    settings.terrain_radius
}

/// Lower-sky haze and the opaque immersion dome share the sky palette.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct HorizonFogMaterial {
    #[storage(11, read_only, visibility(fragment))]
    fog: Handle<ShaderBuffer>,
}

impl Material for HorizonFogMaterial {
    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
    }
    fn depth_bias(&self) -> f32 {
        // Draw behind all world translucency, despite being camera-centered.
        -1_000_000.0
    }
    fn fragment_shader() -> ShaderRef {
        "embedded://hollow_render/shaders/horizon_fog.wesl".into()
    }
    fn enable_prepass() -> bool {
        false
    }
    fn enable_shadows() -> bool {
        false
    }
    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from inside.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Lower-sky haze behind the terrain and clouds, with an invisible upper rim.
#[derive(Component)]
pub struct HorizonSkirt;

/// A fully fogged sphere just past the end of a liquid's fog hides the sky underwater.
#[derive(Component)]
pub struct LiquidDome;

pub fn spawn_horizon_skirt(
    mut commands: Commands,
    session: Res<hollow_engine::session::ActiveSession>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HorizonFogMaterial>>,
    fog: Res<super::fog::SkyPalette>,
) {
    let material = MeshMaterial3d(materials.add(HorizonFogMaterial { fog: fog.0.clone() }));
    let tube = Cylinder::new(1.0, 1.0)
        .mesh()
        .resolution(64)
        .without_caps()
        .anchor(CylinderAnchor::Top);
    commands.spawn((
        HorizonSkirt,
        Mesh3d(meshes.add(tube)),
        material.clone(),
        Transform::default(),
        NotShadowCaster,
        NotShadowReceiver,
        SessionMember(session.0),
    ));
    commands.spawn((
        LiquidDome,
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(32, 16))),
        material,
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
        NotShadowReceiver,
        SessionMember(session.0),
    ));
}

/// Keeps the skirt and dome camera-centered, the skirt's rim above its alpha fade.
/// Reads last frame's camera placement: a frame's lag at this distance is invisible.
pub fn follow_horizon_skirt(
    settings: Res<EnvironmentSettings>,
    liquid: Res<CameraLiquid>,
    layout: Res<WorldLayout>,
    camera: Single<(&GlobalTransform, &DistanceFog), With<VoxelViewer>>,
    mut skirt: Single<&mut Transform, (With<HorizonSkirt>, Without<LiquidDome>)>,
    mut dome: Single<(&mut Transform, &mut Visibility), With<LiquidDome>>,
) {
    let (camera, fog) = *camera;
    let center = camera.translation();

    // Clouds below a flying camera reach past the terrain: stay behind them too.
    let mut radius = terrain_radius(&settings);
    radius = radius.max(settings.horizon_radius);
    let radius = radius + 1.0;
    let rim_height = radius * 0.10510424;
    // Reach below the whole terrain column even from a high flying camera.
    let depth = (radius * 3.0).max(center.y + rim_height + layout.height() as f32);
    skirt.set_if_neq(
        Transform::from_translation(center + Vec3::Y * rim_height)
            // The rim is six degrees above eye level; haze vanishes at three.
            .with_scale(Vec3::new(radius, depth, radius)),
    );

    let (dome_transform, visibility) = &mut *dome;
    let fog_end = match fog.falloff {
        FogFalloff::Linear { end, .. } if liquid.0.is_some() => Some(end),
        _ => None,
    };
    visibility.set_if_neq(if fog_end.is_some() {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    });
    if let Some(end) = fog_end {
        // Flat faces dip inside the sphere by under 1%.
        dome_transform.set_if_neq(
            Transform::from_translation(center).with_scale(Vec3::splat(end * 1.02 + 0.5)),
        );
    }
}

#[derive(Resource, Clone, Copy, PartialEq)]
pub struct EnvironmentSettings {
    pub terrain_radius: f32,
    pub horizon_radius: f32,
    pub shadows_enabled: bool,
}
impl Default for EnvironmentSettings {
    fn default() -> Self {
        Self {
            terrain_radius: 128.0,
            horizon_radius: 128.0,
            shadows_enabled: true,
        }
    }
}
#[derive(Resource, Clone, Copy, PartialEq, bevy::render::extract_resource::ExtractResource)]
#[extract_app(bevy::render::RenderApp)]
pub struct SkyLighting {
    pub night: f32,
    pub ev100: f32,
    pub ambient_color: Color,
    pub ambient_brightness: f32,
    pub horizon_color: Color,
    pub zenith_color: Color,
    pub sun_direction: Vec3,
    pub sunset_color: Color,
}
impl SkyLighting {
    fn exposure(&self) -> f32 {
        bevy::camera::Exposure { ev100: self.ev100 }.exposure()
    }
}
impl Default for SkyLighting {
    fn default() -> Self {
        Self {
            night: 0.0,
            ev100: 0.0,
            ambient_color: Color::WHITE,
            ambient_brightness: 0.5,
            horizon_color: Color::srgb(0.6, 0.75, 0.9),
            zenith_color: Color::srgb(0.35, 0.6, 1.0),
            sun_direction: Vec3::Y,
            sunset_color: Color::BLACK,
        }
    }
}
#[derive(Resource, Default, Clone, Copy, PartialEq)]
pub struct ViewEffects {
    pub minimum_ambient: f32,
    pub fog_distance: Option<f32>,
}
