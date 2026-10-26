use super::{VoxelKind, VoxelState};
use bevy::prelude::*;
use hollow_worldgen::coordinates::{ALL_SECTIONS, CHUNK_HEIGHT, SECTION_HEIGHT, cell_index};
use hollow_worldgen::sections::Layer;
use std::{collections::BTreeSet, sync::Arc};

/// Chunk data, compressed by section. Cloning it is O(1); edits use copy-on-write
/// while a build reads it.
///
/// Neighbor borders are not copied here: mesh tasks read the neighbors' shared layers.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    pub(crate) states: Layer<VoxelState>,
    pub(crate) light: Layer<u8>,
    /// Rendering-only estimates while shared snapshots are relit. Never seed the worker with these.
    render_light: Option<Layer<u8>>,
    /// Cells edited since generation. Reconciliation rechecks them when a chunk enters
    /// its snapshot neighborhood, so they are kept for the chunk's whole life.
    pub(crate) native_edits: Arc<BTreeSet<usize>>,
    pub(crate) content_revision: u64,
    /// One bit per render section whose mesh is outdated.
    pub(crate) dirty_sections: u32,
    pub(crate) revision: u64,
    pub(crate) modified: bool,
    /// Disk revisions are independent of mesh invalidations and lighting task versions.
    pub(crate) save_revision: u64,
    pub(crate) saved_revision: u64,
}
impl ChunkVoxels {
    /// A uniform column with validated native states.

    pub(super) fn from_layers(states: Layer<VoxelState>, light: Layer<u8>) -> Self {
        Self {
            states,
            light,
            render_light: None,
            native_edits: default(),
            content_revision: 0,
            dirty_sections: ALL_SECTIONS,
            revision: 0,
            modified: false,
            save_revision: 0,
            saved_revision: 0,
        }
    }

    /// Sections whose faces or corner light can read cell `y`.
    pub(crate) const fn sections_around(y: i32) -> u32 {
        const fn section(y: i32) -> u32 {
            if 0 <= y && y < CHUNK_HEIGHT {
                1 << (y / SECTION_HEIGHT)
            } else {
                0
            }
        }
        section(y - 1) | section(y) | section(y + 1)
    }

    /// Requests new meshes for these sections.
    pub(crate) fn mark_sections(&mut self, sections: u32) {
        let sections = sections & ALL_SECTIONS;
        if sections == 0 {
            return;
        }
        self.dirty_sections |= sections;
        self.revision = self.revision.wrapping_add(1);
    }

    pub(crate) fn render_light(&self) -> &Layer<u8> {
        self.render_light.as_ref().unwrap_or(&self.light)
    }

    /// Displayed light of a cell, estimates included.
    pub(crate) fn render_light_at(&self, local: IVec3) -> Option<u8> {
        cell_index(local).map(|index| self.render_light().get(index))
    }

    /// The voxel edit already dirties this cell's sections and their border readers.
    pub(crate) fn estimate_light(&mut self, local: IVec3, light: u8) {
        let Some(index) = cell_index(local) else {
            return;
        };
        if self.render_light().get(index) == light {
            return;
        }
        self.render_light
            .get_or_insert_with(|| self.light.clone())
            .set(index, light);
    }

    /// Updates displayed block light without discarding a pending skylight estimate.
    pub(crate) fn set_block_light(&mut self, index: usize, light: u8) {
        let render = self.render_light.get_or_insert_with(|| self.light.clone());
        render.set(index, (render.get(index) & 0xf0) | light);
    }

    /// Replaces authoritative light, clears estimates, and returns visually changed sections.
    pub(crate) fn replace_light(&mut self, light: Layer<u8>) -> u32 {
        let authoritative_changed = self.light.changed_sections(&light);
        if authoritative_changed != 0 {
            self.save_revision += 1;
        }
        let changed = self
            .render_light
            .as_ref()
            .map_or(authoritative_changed, |render| {
                render.changed_sections(&light)
            });
        self.light = light;
        self.render_light = None;
        if changed == 0 {
            return 0;
        }
        // Light is drawn at face corners, which also sample the adjacent sections.
        let spread = changed | (changed << 1) | (changed >> 1);
        self.mark_sections(spread);
        spread & ALL_SECTIONS
    }

    pub fn get(&self, local: IVec3) -> Option<VoxelKind> {
        self.state(local).map(VoxelState::kind)
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        self.set_state(local, kind.default_state())
    }

    pub fn state(&self, local: IVec3) -> Option<VoxelState> {
        cell_index(local).map(|index| self.states.get(index))
    }

    pub fn set_state(&mut self, local: IVec3, state: VoxelState) -> bool {
        let Some(index) = cell_index(local) else {
            return false;
        };
        let old = self.state(local).expect("valid cell");
        if old == state {
            return false;
        }
        self.states.set(index, state);
        Arc::make_mut(&mut self.native_edits).insert(index);
        self.content_revision = self.content_revision.wrapping_add(1);
        self.mark_sections(Self::sections_around(local.y));
        self.modified = true;
        self.save_revision += 1;
        true
    }
}
#[derive(Component, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VoxelChunk {
    pub position: IVec2,
}

#[derive(
    Component, Clone, Copy, Debug, Default, bevy::render::extract_component::ExtractComponent,
)]
#[extract_app(bevy::render::RenderApp)]
pub struct VoxelViewer;
