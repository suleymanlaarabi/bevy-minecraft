use bevy::{
    core_pipeline::{core_3d::extract_camera_prepass_phase, prepass::DepthPrepass},
    prelude::*,
    render::{
        Extract, ExtractSchedule, RenderApp,
        camera::{CameraMainPassTextureFormats, extract_cameras},
        render_resource::{TextureFormat, TextureFormatFeatureFlags, TextureUsages, WgpuFeatures},
        renderer::{RenderAdapter, RenderDevice},
        sync_world::RenderEntity,
        view::ExtractedView,
    },
    ui_render::RenderUiSystems,
};

/// Opt in when alpha is unused and the consumer's shaders tolerate packed HDR precision.
#[derive(Component)]
pub struct CompactHdr;
/// Opt in only when no shader or post-processing feature samples prepass depth.
#[derive(Component)]
pub struct UnsampledDepthPrepass;

pub(super) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.add_systems(
            ExtractSchedule,
            (
                skip_prepass_depth_copy.after(extract_camera_prepass_phase),
                compact_scene_target
                    .after(extract_cameras)
                    .before(RenderUiSystems::ExtractCameraViews),
            ),
        );
    }
}

/// Bevy copies the depth prepass into a separate sampled texture after the prepass,
/// for materials reading `depth_prepass_texture`. Nothing samples it here: no Hollow
/// shader does, and no Bevy feature that does (transmission, SSR, SSAO, decals, OIT,
/// TAA) is used. The prepass phases and early depth only need the main-world
/// `DepthPrepass`, so the render-world marker is dropped to skip the copy.
fn skip_prepass_depth_copy(
    cameras: Extract<Query<RenderEntity, (With<UnsampledDepthPrepass>, With<DepthPrepass>)>>,
    mut commands: Commands,
) {
    for entity in &cameras {
        commands.entity(entity).remove::<DepthPrepass>();
    }
}

/// Bevy's bloom already blurs the scene in this packed 32-bit float format.
const COMPACT_HDR: TextureFormat = TextureFormat::Rg11b10Ufloat;

/// Bevy gives an HDR camera 64-bit `Rgba16Float` color targets. Every scene pass
/// reads or writes them: the multisampled color, its resolve, bloom, tonemapping
/// and the final copy, which is most of the memory traffic of a frame on an
/// integrated GPU. The scene needs no alpha, and its smooth gradients are dithered
/// against the format's 6-bit mantissa (`hdr_dither` in `sky_color.wesl`), so the
/// packed format carries the same image with half the bytes.
fn compact_scene_target(
    cameras: Extract<Query<(RenderEntity, &Msaa), (With<CompactHdr>, With<bevy::camera::Hdr>)>>,
    device: Res<RenderDevice>,
    adapter: Res<RenderAdapter>,
    mut formats: ResMut<CameraMainPassTextureFormats>,
    mut supported: Local<Option<(u32, bool)>>,
    mut commands: Commands,
) {
    for (entity, msaa) in &cameras {
        let samples = msaa.samples();
        let compact = match *supported {
            Some((checked, compact)) if checked == samples => compact,
            _ => {
                let compact = compact_hdr_supported(&device, &adapter, samples);
                *supported = Some((samples, compact));
                compact
            }
        };
        let Some(format) = formats.get_mut(&entity).filter(|_| compact) else {
            continue;
        };
        *format = COMPACT_HDR;
        // `extract_cameras` inserts this frame's view with a command of its own.
        commands.entity(entity).queue(|mut camera: EntityWorldMut| {
            if let Some(mut view) = camera.get_mut::<ExtractedView>() {
                view.target_format = COMPACT_HDR;
            }
        });
    }
}

/// Rendering, blending and resolving into the packed format are optional features.
fn compact_hdr_supported(device: &RenderDevice, adapter: &RenderAdapter, samples: u32) -> bool {
    let features = device.features();
    if !features.contains(WgpuFeatures::RG11B10UFLOAT_RENDERABLE) {
        return false;
    }
    let format = if features.contains(WgpuFeatures::TEXTURE_ADAPTER_SPECIFIC_FORMAT_FEATURES) {
        adapter.get_texture_format_features(COMPACT_HDR)
    } else {
        COMPACT_HDR.guaranteed_format_features(features)
    };
    format
        .allowed_usages
        .contains(TextureUsages::RENDER_ATTACHMENT | TextureUsages::TEXTURE_BINDING)
        && format.flags.contains(TextureFormatFeatureFlags::BLENDABLE)
        && (samples == 1
            || format.flags.sample_count_supported(samples)
                && format
                    .flags
                    .contains(TextureFormatFeatureFlags::MULTISAMPLE_RESOLVE))
}
