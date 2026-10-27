//! Terrain-renderer adapter for the shared fluid surface service.
use super::{
    meshing::Padded,
    models::{self, ModelQuad},
};
use bevy::prelude::*;
use hollow_core::{ShapeId, shape::BlockBox};
use hollow_engine::fluids::surface::{self, FluidSurfaceSample, FluidSurfaceWorld};
struct Surface<'a>(&'a Padded);
impl FluidSurfaceWorld for Surface<'_> {
    fn sample(&self, position: IVec3) -> FluidSurfaceSample {
        let sample = self.0.get(position);
        FluidSurfaceSample {
            fluid: sample.fluid_id(),
            amount: sample.fluid_amount(),
            falling: sample.fluid_falling(),
            greedy: sample.ocean_surface(),
            occludes: sample.hides_fluid(),
            shape: if sample.fluid_occluder() {
                sample.state.definition().selection
            } else {
                ShapeId(0)
            },
        }
    }
    fn shapes(&self, id: ShapeId) -> &[BlockBox] {
        hollow_worldgen::content::catalog().shapes(id)
    }
    fn has_partial_occluders(&self) -> bool {
        self.0.fluid_occluders
    }
}
pub(super) fn explicit_surface(padded: &Padded, cell: IVec3) -> bool {
    surface::explicit_surface(&Surface(padded), crate::content::fluid_catalog(), cell)
}
pub(super) fn visit(padded: &Padded, cell: IVec3, mut emit: impl FnMut(&ModelQuad)) {
    let catalog = crate::content::fluid_catalog();
    let family = padded.get(cell).fluid_id();
    let render = catalog
        .profile(family)
        .render
        .expect("Surface classified by shared renderer");
    surface::visit_surface(&Surface(padded), catalog, cell, |quad| {
        emit(&ModelQuad {
            positions: quad.positions,
            normal: quad.normal,
            uv: quad.uv,
            texture: models::fluid_texture(family, quad.flowing),
            tinted: false,
            cull: [0; 3],
            translucent: render.translucent,
            effect: render.effect,
            light_emission: render.emission,
            shade_up: false,
        });
    });
}
