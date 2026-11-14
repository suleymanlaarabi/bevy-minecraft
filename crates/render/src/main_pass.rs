//! One main render pass per 3D view instead of Bevy's two.
//!
//! Bevy draws the opaque and the transparent phases in separate passes over the
//! same attachments. Ending a multisampled pass resolves the whole color target, so
//! every frame with water or clouds resolved it twice, the first time for nothing,
//! and stored and reloaded the multisampled attachments in between. Nothing here
//! runs between the two phases (no transmission, atmosphere or order-independent
//! transparency), so both draw in a single pass with a single resolve.

use bevy::{
    camera::{MainPassResolutionOverride, Viewport},
    core_pipeline::{
        Core3dSystems,
        core_3d::{
            AlphaMask3d, Opaque3d, Transparent3d, main_opaque_pass_3d, main_transparent_pass_3d,
        },
        schedule::Core3d,
    },
    ecs::schedule::ScheduleCleanupPolicy,
    prelude::*,
    render::{
        RenderApp, RenderStartup,
        camera::ExtractedCamera,
        diagnostic::RecordDiagnostics,
        render_phase::{ViewBinnedRenderPhases, ViewSortedRenderPhases},
        render_resource::{RenderPassDescriptor, StoreOp},
        renderer::{RenderContext, ViewQuery},
        view::{ExtractedView, ViewDepthStencilTexture, ViewTarget},
    },
};

pub(crate) fn register(app: &mut App) {
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        // Once every plugin has added its passes and their resources exist.
        render.add_systems(RenderStartup, merge_main_passes);
    }
}

fn merge_main_passes(world: &mut World) {
    world.schedule_scope(Core3d, |world, schedule| {
        // The system sets stay, so Bevy's own ordering against these passes still holds.
        let policy = ScheduleCleanupPolicy::RemoveSystemsOnly;
        if !matches!(
            schedule.remove_systems_in_set(main_opaque_pass_3d, world, policy),
            Ok(1)
        ) {
            error!("Bevy's main 3D passes are kept: the opaque pass was not found");
            return;
        }
        if !matches!(
            schedule.remove_systems_in_set(main_transparent_pass_3d, world, policy),
            Ok(1)
        ) {
            error!("Bevy's transparent 3D pass was not found beside the merged pass");
        }
        schedule.add_systems(
            main_pass_3d
                .in_set(Core3dSystems::MainPass)
                .after(main_opaque_pass_3d)
                .before(main_transparent_pass_3d),
        );
    });
}

fn main_pass_3d(
    world: &World,
    view: ViewQuery<(
        &ExtractedCamera,
        &ExtractedView,
        &ViewTarget,
        &ViewDepthStencilTexture,
        Option<&MainPassResolutionOverride>,
    )>,
    opaque: Res<ViewBinnedRenderPhases<Opaque3d>>,
    alpha_mask: Res<ViewBinnedRenderPhases<AlphaMask3d>>,
    transparent: Res<ViewSortedRenderPhases<Transparent3d>>,
    mut ctx: RenderContext,
) {
    let entity = view.entity();
    let (camera, extracted, target, depth, resolution_override) = view.into_inner();
    let retained = &extracted.retained_view_entity;
    let (Some(opaque), Some(alpha_mask)) = (opaque.get(retained), alpha_mask.get(retained)) else {
        return;
    };
    let transparent = transparent
        .get(retained)
        .filter(|phase| !phase.items.is_empty());

    let diagnostics = ctx.diagnostic_recorder();
    let diagnostics = diagnostics.as_deref();
    let mut pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
        label: Some("main_pass_3d"),
        color_attachments: &[Some(target.get_color_attachment())],
        depth_stencil_attachment: Some(depth.get_attachment(StoreOp::Store)),
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    let span = diagnostics.pass_span(&mut pass, "main_pass_3d");
    if let Some(viewport) =
        Viewport::from_viewport_and_override(camera.viewport.as_ref(), resolution_override)
    {
        pass.set_camera_viewport(&viewport);
    }
    if !opaque.is_empty()
        && let Err(err) = opaque.render(&mut pass, world, entity)
    {
        error!("Error encountered while rendering the opaque phase {err:?}");
    }
    if !alpha_mask.is_empty()
        && let Err(err) = alpha_mask.render(&mut pass, world, entity)
    {
        error!("Error encountered while rendering the alpha mask phase {err:?}");
    }
    if let Some(transparent) = transparent
        && let Err(err) = transparent.render(&mut pass, world, entity)
    {
        error!("Error encountered while rendering the transparent phase {err:?}");
    }
    span.end(&mut pass);
}
