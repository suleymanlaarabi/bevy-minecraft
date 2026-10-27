use bevy_ecs::prelude::*;
use bevy_math::prelude::*;
use hollow_core::VoxelCell;
use hollow_core::coordinates::{ALL_SECTIONS, CHUNK_HEIGHT, SECTION_HEIGHT, cell_index};
use hollow_core::sections::Layer;

/// Chunk data, compressed by section. Cloning it is O(1); edits use copy-on-write
/// while a build reads it.
///
/// Neighbor borders are not copied here: mesh tasks read the neighbors' shared layers.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels<T: VoxelCell> {
    pub states: Layer<T>,
    pub light: Layer<u8>,
    /// Rendering-only estimates while shared snapshots are relit. Never seed the worker with these.
    render_light: Option<Layer<u8>>,
    pub content_revision: u64,
    /// One bit per render section whose mesh is outdated.
    pub dirty_sections: u32,
    pub revision: u64,
    /// Remains set after saving: lighting must recognize edited chunks on arrival.
    pub modified: bool,
    /// Disk revisions are independent of mesh invalidations and lighting task versions.
    pub save_revision: u64,
    pub saved_revision: u64,
}
impl<T: VoxelCell> ChunkVoxels<T> {
    /// A uniform column with the application's cell representation.

    pub fn from_layers(states: Layer<T>, light: Layer<u8>) -> Self {
        Self {
            states,
            light,
            render_light: None,
            content_revision: 0,
            dirty_sections: ALL_SECTIONS,
            revision: 0,
            modified: false,
            save_revision: 0,
            saved_revision: 0,
        }
    }

    pub fn encode_saved(&self) -> Vec<u8> {
        hollow_core::codec::TerrainRecord {
            states: self.states.clone(),
            light: self.light.clone(),
            modified: self.modified,
        }
        .encode()
    }
    pub fn decode_saved(bytes: &[u8]) -> std::io::Result<Self> {
        let record = hollow_core::codec::TerrainRecord::<T>::decode(
            bytes,
            hollow_core::coordinates::SECTION_COUNT,
        )?;
        let mut chunk = Self::from_layers(record.states, record.light);
        chunk.modified = record.modified;
        chunk.save_revision = 1;
        chunk.saved_revision = 1;
        Ok(chunk)
    }

    /// Sections whose faces or corner light can read cell `y`.
    pub const fn sections_around(y: i32) -> u32 {
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
    pub fn mark_sections(&mut self, sections: u32) {
        let sections = sections & ALL_SECTIONS;
        if sections == 0 {
            return;
        }
        self.dirty_sections |= sections;
        self.revision = self.revision.wrapping_add(1);
    }

    pub fn render_light(&self) -> &Layer<u8> {
        self.render_light.as_ref().unwrap_or(&self.light)
    }

    /// Displayed light of a cell, estimates included.
    pub fn render_light_at(&self, local: IVec3) -> Option<u8> {
        cell_index(local).map(|index| self.render_light().get(index))
    }

    /// The voxel edit already dirties this cell's sections and their border readers.
    pub fn estimate_light(&mut self, local: IVec3, light: u8) {
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
    pub fn set_block_light(&mut self, index: usize, light: u8) {
        let render = self.render_light.get_or_insert_with(|| self.light.clone());
        render.set(index, (render.get(index) & 0xf0) | light);
    }

    /// Replaces authoritative light, clears estimates, and returns visually changed sections.
    pub fn replace_light(&mut self, light: Layer<u8>) -> u32 {
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

    pub fn get(&self, local: IVec3) -> Option<T::Kind> {
        self.state(local).map(T::kind)
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: T::Kind) -> bool {
        self.set_state(local, T::default_state(kind))
    }

    pub fn state(&self, local: IVec3) -> Option<T> {
        cell_index(local).map(|index| self.states.get(index))
    }

    pub fn set_state(&mut self, local: IVec3, state: T) -> bool {
        let Some(index) = cell_index(local) else {
            return false;
        };
        let old = self.state(local).expect("valid cell");
        if old == state {
            return false;
        }
        self.states.set(index, state);
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
