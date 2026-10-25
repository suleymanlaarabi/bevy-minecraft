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

    // The sky has changed on the first run, which applies everything once.
    if camera_liquid.0 == liquid && !sky.is_changed() && !settings.is_changed() {
        return;
    }
    camera_liquid.set_if_neq(CameraLiquid(liquid));

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
        // The fog only hides the edge of the loaded terrain, in the horizon's color:
        // the atmosphere's aerial perspective handles the haze before it. Like vanilla,
        // it only covers a thin band before that edge.
        let radius = terrain_radius(&settings);
        fog.color = sky.horizon_color;
        fog.directional_light_color = OPEN_AIR_SCATTERING;
        fog.falloff = FogFalloff::Linear {
            start: radius - (radius * 0.15).clamp(8.0, 64.0),
            end: radius,
        };
        ambient.color = sky.ambient_color;
        ambient.brightness = sky.ambient_brightness;
    }
}

/// Distance at which the loaded terrain ends and the open-air fog becomes opaque.
pub(crate) fn terrain_radius(settings: &GraphicsSettings) -> f32 {
    (settings.effective_view_distance() * CHUNK_SIZE as u32) as f32
}

/// Draws nothing but fully fogged color (`horizon_fog.wesl`): the fog shader's own
/// sun scattering included, so it blends seamlessly with fogged terrain beside it.
#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub(in crate::voxel) struct HorizonFogMaterial {}

impl Material for HorizonFogMaterial {
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
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Seen from inside.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// A fully fogged tube hanging from eye level around the camera, past the terrain
/// and the clouds. Below the horizon the atmosphere would otherwise show its bare
/// planet ground behind the terrain's edge, which never matches the fog.
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
) {
    let material = MeshMaterial3d(materials.add(HorizonFogMaterial::default()));
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

/// Keeps the skirt and the dome centered on the camera, the skirt's rim at eye level.
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
    skirt.set_if_neq(
        Transform::from_translation(center)
            // Deep enough that only steep downward views, always on terrain, miss it.
            .with_scale(Vec3::new(radius, radius * 3.0, radius)),
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
