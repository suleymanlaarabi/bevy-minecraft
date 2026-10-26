#[cfg(test)]
use super::coordinates::FULL_SKY;
use super::{
    VoxelState,
    blocks::VoxelKind,
    coordinates::{
        ALL_SECTIONS, CHUNK_HEIGHT, CHUNK_VOLUME, SECTION_COUNT, SECTION_HEIGHT, SECTION_VOLUME,
        cell_index, cell_position,
    },
    storage::{Layer, Section},
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
    pub(crate) states: Layer<VoxelState>,
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
    /// A uniform column with validated native states.
    #[cfg(test)]
    pub(crate) fn filled(kind: VoxelKind) -> Self {
        Self::from_layers(Layer::filled(kind.default_state()), Layer::filled(FULL_SKY))
    }

    /// Validates imported IDs once before they enter shared chunk storage.
    pub(crate) fn from_native(states: &[u16], light: &[u8]) -> Self {
        Self::from_layers(
            Layer::from_mapped(states, |id| {
                VoxelState::from_id(id).expect("Valid generated block state")
            }),
            Layer::from_cells(light),
        )
    }

    pub(super) fn from_layers(states: Layer<VoxelState>, light: Layer<u8>) -> Self {
        Self {
            states,
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

    /// Updates block light without discarding a pending skylight estimate.
    pub(crate) fn set_block_light(&mut self, index: usize, light: u8) {
        self.light
            .set(index, (self.light.get(index) & 0xf0) | light);
        if let Some(render) = &mut self.render_light {
            render.set(index, (render.get(index) & 0xf0) | light);
        }
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
        for section in 0..SECTION_COUNT {
            let states = self.states.section(section);
            if !states.any(|state| !state.native().collision_shapes.is_empty()) {
                continue;
            }
            let start = section * SECTION_VOLUME;
            let out = &mut classes[start..][..SECTION_VOLUME];
            match states {
                Section::Uniform(state) => {
                    let value = class(*state);
                    out.fill(value);
                    if value == PARTIAL_SHAPE {
                        partial.extend((0..SECTION_VOLUME).map(|offset| (start + offset, *state)));
                    }
                }
                Section::Dense(states) => {
                    for (offset, &state) in states.iter().enumerate() {
                        let value = class(state);
                        out[offset] = value;
                        if value == PARTIAL_SHAPE {
                            partial.push((start + offset, state));
                        }
                    }
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
        classes
            .iter()
            .enumerate()
            .filter(|&(_, &class)| class == FULL_CUBE)
            .filter_map(|(index, _)| {
                let (x, z, y) = (index & 15, (index >> 4) & 15, index >> 8);
                // Borders are kept conservatively; interior neighbors have known
                // linear offsets, without six coordinate conversions per cube.
                let boundary = x == 0
                    || x == 15
                    || z == 0
                    || z == 15
                    || y == 0
                    || y + 1 == CHUNK_HEIGHT as usize;
                (boundary
                    || classes[index - 1] != FULL_CUBE
                    || classes[index + 1] != FULL_CUBE
                    || classes[index - 16] != FULL_CUBE
                    || classes[index + 16] != FULL_CUBE
                    || classes[index - 256] != FULL_CUBE
                    || classes[index + 256] != FULL_CUBE)
                    .then(|| IVec3::new(x as i32, y as i32, z as i32))
            })
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
