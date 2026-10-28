//! Terrain-renderer adapter for the shared fluid surface service.
use super::{meshing::Padded, models::ModelQuad};
use bevy::prelude::*;
use hollow_core::{ShapeId, shape::BlockBox};
use hollow_engine::fluids::surface::{self, FluidSurfaceSample, FluidSurfaceWorld};
struct Surface<'a, C: crate::source::RenderSource>(&'a Padded, &'a C);
impl<C: crate::source::RenderSource> FluidSurfaceWorld for Surface<'_, C> {
    fn sample(&self, position: IVec3) -> FluidSurfaceSample {
        let sample = self.0.get(position);
        FluidSurfaceSample {
            fluid: sample.fluid_id(),
            amount: sample.fluid_amount(),
            falling: sample.fluid_falling(),
            greedy: sample.ocean_surface(),
            occludes: sample.hides_fluid(),
            shape: if sample.fluid_occluder() {
                self.1.content().state(sample.state).selection
            } else {
                ShapeId(0)
            },
        }
    }
    fn shapes(&self, id: ShapeId) -> &[BlockBox] {
        self.1.content().shapes(id)
    }
    fn has_partial_occluders(&self) -> bool {
        self.0.fluid_occluders
    }
}
pub fn explicit_surface<C: crate::source::RenderSource>(
    padded: &Padded,
    source: &C,
    cell: IVec3,
) -> bool {
    surface::explicit_surface(&Surface(padded, source), source.fluids(), cell)
}
pub fn visit<C: crate::source::RenderSource>(
    padded: &Padded,
    source: &C,
    cell: IVec3,
    mut emit: impl FnMut(&ModelQuad),
) {
    let catalog = source.fluids();
    let family = padded.get(cell).fluid_id();
    let render = catalog
        .profile(family)
        .render
        .expect("Surface classified by shared renderer");
    surface::visit_surface(&Surface(padded, source), catalog, cell, |quad| {
        emit(&ModelQuad {
            positions: quad.positions,
            normal: quad.normal,
            uv: quad.uv,
            texture: source.fluid_texture(family, quad.flowing),
            tinted: false,
            cull: [0; 3],
            translucent: render.translucent,
            effect: render.effect,
            light_emission: render.emission,
            shade_up: false,
        });
    });
}
