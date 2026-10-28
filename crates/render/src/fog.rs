//! Shared sky colors: one small GPU buffer for the background, fog and reflections.
use crate::environment::SkyLighting;
use bevy::{
    asset::RenderAssetUsages,
    prelude::*,
    render::{
        Render, RenderApp, RenderSystems,
        extract_resource::{ExtractResource, ExtractResourcePlugin},
        render_asset::RenderAssets,
        renderer::RenderQueue,
        storage::{GpuShaderBuffer, ShaderBuffer},
    },
};

#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub struct SkyPalette(pub Handle<ShaderBuffer>);

impl FromWorld for SkyPalette {
    fn from_world(world: &mut World) -> Self {
        Self(
            world
                .resource_mut::<Assets<ShaderBuffer>>()
                .add(ShaderBuffer::with_size(
                    4 * size_of::<Vec4>() as u64,
                    RenderAssetUsages::RENDER_WORLD,
                )),
        )
    }
}

pub fn register(app: &mut App) {
    app.init_resource::<SkyPalette>().add_plugins((
        ExtractResourcePlugin::<SkyPalette>::default(),
        ExtractResourcePlugin::<SkyLighting>::default(),
    ));
    if let Some(render_app) = app.get_sub_app_mut(RenderApp) {
        render_app.add_systems(
            Render,
            upload_palette.in_set(RenderSystems::PrepareBindGroups),
        );
    }
}

fn upload_palette(
    palette: Res<SkyPalette>,
    sky: Res<SkyLighting>,
    buffers: Res<RenderAssets<GpuShaderBuffer>>,
    queue: Res<RenderQueue>,
    mut uploaded: Local<bool>,
) {
    if palette.is_changed() {
        *uploaded = false;
    }
    if *uploaded && !sky.is_changed() {
        return;
    }
    let Some(buffer) = buffers.get(&palette.0) else {
        return;
    };
    let colors = [
        sky.zenith_color.to_linear().to_vec4(),
        sky.horizon_color.to_linear().to_vec4(),
        sky.sun_direction.extend(0.0),
        sky.sunset_color.to_linear().to_vec4(),
    ];
    // Keep the GPU allocation and all material bindings stable throughout the day.
    queue.write_buffer(&buffer.buffer, 0, bytemuck::cast_slice(&colors));
    *uploaded = true;
}
