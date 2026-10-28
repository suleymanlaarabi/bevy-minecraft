//! Prepared content access. Implementations are monomorphized in mesh workers.
use crate::models::{CellInfo, ModelQuad};
use bevy::prelude::*;
use hollow_core::{BlockId, FluidId, StateId, VoxelCell, content::ContentCatalog};
use hollow_engine::fluids::FluidCatalog;
pub trait RenderSource: Resource + Clone {
    type Cell: VoxelCell;
    fn content(&self) -> &ContentCatalog;
    fn fluids(&self) -> &FluidCatalog;
    fn cell_info(&self, state: StateId) -> CellInfo;
    fn visit_quads(&self, state: StateId, world: IVec3, visit: impl FnMut(&ModelQuad));
    fn fluid_texture(&self, fluid: FluidId, flowing: bool) -> u32;
    fn tint_index(&self, block: BlockId) -> u8;
    fn opaque_texture(&self, slot: u32) -> bool;
    fn visual_offset(&self, _state: StateId, _world: IVec3) -> Vec3 {
        Vec3::ZERO
    }
    fn has_imported_models(&self) -> bool {
        true
    }
    fn entity_rendered(&self, _state: StateId) -> bool {
        false
    }
    fn shadow_directions(&self) -> [bool; 3] {
        [false; 3]
    }
}
