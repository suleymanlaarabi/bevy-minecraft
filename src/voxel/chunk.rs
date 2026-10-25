#[cfg(test)]
use super::coordinates::FULL_SKY;
use super::{
    VoxelState,
    blocks::VoxelKind,
    coordinates::{
        ALL_SECTIONS, CHUNK_HEIGHT, CHUNK_VOLUME, SECTION_COUNT, SECTION_HEIGHT, SECTION_VOLUME,
        cell_index, cell_position,
    },
    storage::Layer,
};
use bevy::prelude::*;
use std::{collections::BTreeSet, sync::Arc};

/// Collision classes of [`ChunkVoxels::collision_classes`].
const FULL_CUBE: u8 = 1;
const PARTIAL_SHAPE: u8 = 2;

/// Chunk data, compressed by section. Cloning it is O(1); edits use copy-on-write
/// while a build reads it.
///
/// Neighbor borders are not copied here: mesh tasks read the neighbors' shared layers.
#[derive(Component, Clone, Debug)]
pub struct ChunkVoxels {
    cells: Layer<VoxelKind>,
    pub(crate) native_states: Option<Layer<u16>>,
    pub(crate) light: Layer<u8>,
    /// Rendering-only estimates while Pumpkin relights edits. Never seed the worker with these.
    render_light: Option<Layer<u8>>,
    /// Cells edited since generation. Pumpkin replays them whenever a chunk enters its
    /// lighting context, so they are kept for the chunk's whole life.
    pub(crate) native_edits: Arc<BTreeSet<usize>>,
    pub(crate) content_revision: u64,
    /// One bit per render section whose mesh is outdated.
    pub(crate) dirty_sections: u32,
    pub(crate) collider_dirty: bool,
    pub(crate) collider_built: bool,
    pub(crate) revision: u64,
    pub(crate) modified: bool,
    /// Disk revisions are independent of mesh invalidations and lighting task versions.
    pub(crate) save_revision: u64,
    pub(crate) saved_revision: u64,
}
impl ChunkVoxels {
    /// A column of `kind` under open sky, without Minecraft block states.
    #[cfg(test)]
    pub(crate) fn filled(kind: VoxelKind) -> Self {
        Self::from_layers(Layer::filled(kind), Layer::filled(FULL_SKY), None)
    }

    /// A generated column; the slices use Pumpkin's Y-major layout.
    pub(crate) fn generated(cells: &[VoxelKind], states: &[u16], light: &[u8]) -> Self {
        Self::from_layers(
            Layer::from_cells(cells),
            Layer::from_cells(light),
            Some(Layer::from_cells(states)),
        )
    }

    fn from_layers(
        cells: Layer<VoxelKind>,
        light: Layer<u8>,
        native_states: Option<Layer<u16>>,
    ) -> Self {
        Self {
            cells,
            native_states,
            light,
            render_light: None,
            native_edits: default(),
            content_revision: 0,
            dirty_sections: ALL_SECTIONS,
            collider_dirty: true,
            collider_built: false,
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

    /// Replaces authoritative light, clears estimates, and returns visually changed sections.
    pub(crate) fn replace_light(&mut self, light: Layer<u8>) -> u32 {
        if self.light.changed_sections(&light) != 0 {
            self.save_revision += 1;
        }
        let changed = self.render_light().changed_sections(&light);
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
        cell_index(local).map(|index| self.cells.get(index))
    }

    pub(crate) fn cells(&self) -> &Layer<VoxelKind> {
        &self.cells
    }

    /// Changes one cell. Out-of-bounds and identical values return `false`.
    pub fn set(&mut self, local: IVec3, kind: VoxelKind) -> bool {
        self.set_state(local, kind.default_state())
    }

    pub fn state(&self, local: IVec3) -> Option<VoxelState> {
        let index = cell_index(local)?;
        Some(
            self.native_states
                .as_ref()
                .and_then(|states| VoxelState::from_id(states.get(index)))
                .unwrap_or_else(|| self.cells.get(index).default_state()),
        )
    }

    pub fn set_state(&mut self, local: IVec3, state: VoxelState) -> bool {
        let Some(index) = cell_index(local) else {
            return false;
        };
        let old = self.state(local).expect("valid cell");
        if old == state {
            return false;
        }
        self.cells.set(index, state.kind());
        if self.native_states.is_none() {
            self.native_states = Some(Layer::from_cells(
                &self
                    .cells
                    .to_cells()
                    .iter()
                    .map(|kind| kind.default_state().id())
                    .collect::<Vec<_>>(),
            ));
        }
        self.native_states.as_mut().unwrap().set(index, state.id());
        Arc::make_mut(&mut self.native_edits).insert(index);
        self.content_revision = self.content_revision.wrapping_add(1);
        self.mark_sections(Self::sections_around(local.y));
        if old.native().collision_shapes != state.native().collision_shapes {
            self.collider_dirty = true;
        }
        self.modified = true;
        self.save_revision += 1;
        true
    }

    /// Collision class of every cell, read once per cell: 0 none, [`FULL_CUBE`]
    /// or [`PARTIAL_SHAPE`]. Partial cells also come back with their decoded
    /// state. Sections without a solid block are skipped.
    fn collision_classes(&self) -> (Vec<u8>, Vec<(usize, VoxelState)>) {
        let class = |state: VoxelState| {
            if state.full_collision() {
                FULL_CUBE
            } else if state.native().collision_shapes.is_empty() {
                0
            } else {
                PARTIAL_SHAPE
            }
        };
        let mut classes = vec![0u8; CHUNK_VOLUME];
        let mut partial = Vec::new();
        let mut states = [0u16; SECTION_VOLUME];
        let mut kinds = [VoxelKind::Air; SECTION_VOLUME];
        for section in 0..SECTION_COUNT {
            if !self.cells.section(section).any(VoxelKind::is_solid) {
                continue;
            }
            let start = section * SECTION_VOLUME;
            let out = &mut classes[start..][..SECTION_VOLUME];
            let mut classify = |offset: usize, state: VoxelState| {
                let value = class(state);
                out[offset] = value;
                if value == PARTIAL_SHAPE {
                    partial.push((start + offset, state));
                }
            };
            if let Some(native) = &self.native_states {
                native.section(section).copy_cells(0, &mut states);
                for (offset, &id) in states.iter().enumerate() {
                    if let Some(state) = VoxelState::from_id(id) {
                        classify(offset, state);
                    }
                }
            } else {
                self.cells.section(section).copy_cells(0, &mut kinds);
                for (offset, &kind) in kinds.iter().enumerate() {
                    classify(offset, kind.default_state());
                }
            }
        }
        (classes, partial)
    }

    /// Solid cells another body can touch. Cells enclosed by six solid neighbors of
    /// this chunk are left out: only a one-cell shell of the terrain is kept. Border
    /// cells stay, since the neighboring chunks are unknown here.
    #[cfg(test)]
    pub(crate) fn collider_voxels(&self) -> Vec<IVec3> {
        Self::shell(&self.collision_classes().0)
    }

    fn shell(classes: &[u8]) -> Vec<IVec3> {
        const NEIGHBORS: [IVec3; 6] = [
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Y,
            IVec3::NEG_Y,
            IVec3::Z,
            IVec3::NEG_Z,
        ];
        let solid =
            |local: IVec3| cell_index(local).is_some_and(|index| classes[index] == FULL_CUBE);
        classes
            .iter()
            .enumerate()
            .filter(|&(_, &class)| class == FULL_CUBE)
            .map(|(index, _)| cell_position(index))
            .filter(|&local| !NEIGHBORS.iter().all(|&offset| solid(local + offset)))
            .collect()
    }

    /// Full cubes retain the voxel collider; only partial shapes need compound boxes.
    pub(crate) fn collider(
        &self,
        chunk: IVec2,
    ) -> (
        Option<avian3d::prelude::Collider>,
        Option<avian3d::prelude::Collider>,
    ) {
        use avian3d::prelude::Collider;
        let (classes, partial) = self.collision_classes();
        let cubes = Self::shell(&classes);
        let mut parts = Vec::new();
        let terrain = (!cubes.is_empty()).then(|| Collider::voxels(Vec3::ONE, &cubes));
        let origin = IVec3::new(chunk.x * 16, 0, chunk.y * 16);
        let mut shapes = std::collections::HashMap::<[u32; 3], Collider>::new();
        for (index, state) in partial {
            let local = cell_position(index);
            for shape in state.collision_boxes(origin + local) {
                let size = shape.size();
                if size.min_element() <= 0.0 {
                    continue;
                }
                let collider = shapes
                    .entry(size.to_array().map(f32::to_bits))
                    .or_insert_with(|| Collider::cuboid(size.x, size.y, size.z))
                    .clone();
                parts.push((local.as_vec3() + shape.center(), Quat::IDENTITY, collider));
            }
        }
        let partial = (!parts.is_empty()).then(|| Collider::compound(parts));
        (terrain, partial)
    }

    #[cfg(test)]
    pub(crate) fn with_light(mut self, light: u8) -> Self {
        self.light = Layer::filled(light);
        self
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
