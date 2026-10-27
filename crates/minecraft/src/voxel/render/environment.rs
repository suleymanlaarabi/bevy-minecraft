use super::fog::FogMaterialKey;
use crate::{
    game::GameState,
    settings::GraphicsSettings,
    sky::SkyState,
    voxel::{CHUNK_SIZE, VoxelKind, VoxelViewer, VoxelWorld},
};
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

/// Forward scattering of the sun and moon in open-air fog (`voxel_fog.wesl`). Liquids
/// have none, which also switches that shader to a spherical fog.
pub(crate) const OPEN_AIR_SCATTERING: Color = Color::srgba(1.0, 0.9, 0.75, 0.3);

/// The liquid the camera is in, if any.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(in crate::voxel) struct CameraLiquid(pub Option<VoxelKind>);

/// The camera's surroundings: the day cycle's light and fog in the open, or the
/// murk of the liquid it is in. The only writer of these camera components.
pub(in crate::voxel) fn update_camera_environment(
    voxel_world: VoxelWorld,
    sky: Res<SkyState>,
    settings: Res<GraphicsSettings>,
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
    effects: Query<&crate::survival::ActiveEffects, With<crate::player::Player>>,
    mut previous_effects: Local<(bool, bool)>,
) {
    use pumpkin_data::effect::StatusEffect as E;
    let effect_flags = effects.single().map_or((false, false), |effects| {
        (
            effects.level(E::NIGHT_VISION.id) > 0,
            effects.level(E::BLINDNESS.id) > 0,
        )
    });
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
    if liquid == Some(VoxelKind::Lava) {
        fog.color = Color::srgb_u8(210, 80, 10);
        fog.directional_light_color = Color::NONE;
        fog.falloff = FogFalloff::Linear {
            start: 0.0,
            end: 1.5,
        };
        ambient.color = Color::srgb_u8(255, 140, 60);
        ambient.brightness = 0.28 * output;
    } else if liquid.is_some() {
        // A darkened vanilla water tint (#3F76E4), dimmer at night.
        let daylight = 0.15 + 0.85 * (1.0 - sky.night);
        let tint = Color::srgb_u8(24, 62, 138).to_linear();
        fog.color = LinearRgba::rgb(
            tint.red * daylight,
            tint.green * daylight,
            tint.blue * daylight,
        )
        .into();
        fog.directional_light_color = Color::NONE;
        fog.falloff = FogFalloff::Linear {
            start: 2.0,
            end: 40.0,
        };
        ambient.color = Color::srgb_u8(95, 140, 215);
        ambient.brightness = sky.ambient_brightness * 0.55;
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
    if effect_flags.0 {
        ambient.color = Color::WHITE;
        ambient.brightness = ambient.brightness.max(0.65 * output);
    }
    if effect_flags.1 {
        fog.color = Color::BLACK;
        fog.directional_light_color = Color::NONE;
        fog.falloff = FogFalloff::Linear {
            start: 0.0,
            end: 5.0,
        };
    }
}

/// Nominal loaded terrain radius, before the fog's chunk-alignment safety margin.
pub(crate) fn terrain_radius(settings: &GraphicsSettings) -> f32 {
    (settings.effective_view_distance() * CHUNK_SIZE as u32) as f32
}

/// Draws lower-sky haze or the opaque immersion dome after the atmosphere pass.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
#[bind_group_data(FogMaterialKey)]
pub(in crate::voxel) struct HorizonFogMaterial {
    #[storage(11, read_only, visibility(fragment))]
    fog: Handle<ShaderBuffer>,
    #[texture(12, visibility(fragment))]
    #[sampler(13, visibility(fragment))]
    fog_sky: Option<Handle<Image>>,
    #[texture(14, dimension = "3d", visibility(fragment))]
    #[sampler(15, visibility(fragment))]
    fog_aerial: Option<Handle<Image>>,
}

impl From<&HorizonFogMaterial> for super::fog::FogMaterialKey {
    fn from(material: &HorizonFogMaterial) -> Self {
        Self(material.fog_sky.is_some())
    }
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
        "shaders/horizon_fog.wesl".into()
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
        key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        super::fog::specialize_fog(descriptor, key.bind_group_data.0);
        // Seen from inside.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Lower-sky haze behind the terrain and clouds, with an invisible upper rim.
#[derive(Component)]
pub(in crate::voxel) struct HorizonSkirt;

/// A fully fogged sphere just past the end of a liquid's fog: the atmosphere draws
/// the sky over every background pixel, so nothing else would hide it underwater.
#[derive(Component)]
pub(in crate::voxel) struct LiquidDome;

pub(in crate::voxel) fn spawn_horizon_skirt(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<HorizonFogMaterial>>,
    fog: Res<super::fog::FogLut>,
) {
    let material = MeshMaterial3d(materials.add(HorizonFogMaterial {
        fog: fog.0.clone(),
        fog_sky: fog.1.clone(),
        fog_aerial: fog.2.clone(),
    }));
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
        DespawnOnExit(GameState::Game),
    ));
    commands.spawn((
        LiquidDome,
        Mesh3d(meshes.add(Sphere::new(1.0).mesh().uv(32, 16))),
        material,
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
        NotShadowReceiver,
        DespawnOnExit(GameState::Game),
    ));
}

/// Keeps the skirt and dome camera-centered, the skirt's rim above its alpha fade.
/// Reads last frame's camera placement: a frame's lag at this distance is invisible.
pub(in crate::voxel) fn follow_horizon_skirt(
    settings: Res<GraphicsSettings>,
    liquid: Res<CameraLiquid>,
    camera: Single<(&GlobalTransform, &DistanceFog), With<VoxelViewer>>,
    mut skirt: Single<&mut Transform, (With<HorizonSkirt>, Without<LiquidDome>)>,
    mut dome: Single<(&mut Transform, &mut Visibility), With<LiquidDome>>,
) {
    let (camera, fog) = *camera;
    let center = camera.translation();

    // Clouds below a flying camera reach past the terrain: stay behind them too.
    let mut radius = terrain_radius(&settings);
    if settings.clouds {
        radius = radius.max(crate::sky::cloud_reach(&settings));
    }
    let radius = radius + 1.0;
    let rim_height = radius * 0.10510424;
    // Reach below the whole terrain column even from a high flying camera.
    let depth =
        (radius * 3.0).max(center.y + rim_height + hollow_core::coordinates::CHUNK_HEIGHT as f32);
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
