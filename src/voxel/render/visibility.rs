//! Conservative section connectivity, independent of camera orientation and shadows.

use super::build::{ChunkSections, poll_builds};
use crate::{
    player::PlayerCamera,
    voxel::{
        ChunkVoxels, VoxelChunk,
        coordinates::{CHUNK_HEIGHT, SECTION_COUNT, SECTION_HEIGHT},
        edits::EditedChunk,
        streaming::{ChunkIndex, StreamOffsets, StreamState},
    },
};
use bevy::{
    camera::visibility::{RenderLayers, VisibilitySystems},
    prelude::*,
};
use std::collections::VecDeque;

/// Hidden from the world camera, still available to directional shadow views.
pub(crate) const OCCLUDED_LAYER: usize = 4;
pub(super) const ALL_CONNECTIONS: u64 = (1 << 36) - 1;

/// Faces are -X, +X, -Y, +Y, -Z, +Z. Only full opaque cubes block traversal.
pub(super) fn connectivity(visited: &mut [u64; 64], stack: &mut Vec<u16>) -> u64 {
    if visited.iter().all(|&word| word == 0) {
        return ALL_CONNECTIONS;
    }
    if visited.iter().all(|&word| word == u64::MAX) {
        return 0;
    }
    let mut connections = 0;
    // Interior components cannot touch a section face. Start only at the six
    // boundary planes and stop once all face pairs are connected.
    for seed in (0..6).flat_map(|face| {
        (0..256).map(move |point| {
            let u = point & 15;
            let v = point >> 4;
            let edge = if face & 1 == 0 { 0 } else { 15 };
            match face / 2 {
                0 => edge | u << 4 | v << 8,
                1 => u | v << 4 | edge << 8,
                _ => u | edge << 4 | v << 8,
            }
        })
    }) {
        if visited[seed / 64] & (1 << (seed % 64)) != 0 {
            continue;
        }
        stack.clear();
        stack.push(seed as u16);
        visited[seed / 64] |= 1 << (seed % 64);
        let mut faces = 0u8;
        while let Some(cell) = stack.pop() {
            let cell = cell as usize;
            let x = cell & 15;
            let z = (cell >> 4) & 15;
            let y = cell >> 8;
            for (face, coordinate) in [x, x, y, y, z, z].into_iter().enumerate() {
                let boundary = if face & 1 == 0 {
                    coordinate == 0
                } else {
                    coordinate == 15
                };
                if boundary {
                    faces |= 1 << face;
                    continue;
                }
                let step = [1, 1, 256, 256, 16, 16][face];
                let neighbor = if face & 1 == 0 {
                    cell - step
                } else {
                    cell + step
                };
                let bit = 1 << (neighbor % 64);
                if visited[neighbor / 64] & bit == 0 {
                    visited[neighbor / 64] |= bit;
                    stack.push(neighbor as u16);
                }
            }
        }
        for face in 0..6 {
            if faces & (1 << face) != 0 {
                connections |= u64::from(faces) << (face * 6);
            }
        }
        if connections == ALL_CONNECTIONS {
            break;
        }
    }
    connections
}

/// Connectivity changes invalidate traversal independently of mesh replacement.
#[derive(Resource, Default)]
pub(in crate::voxel) struct TopologyVersion(u64);

impl TopologyVersion {
    pub(super) fn invalidate(&mut self) {
        self.0 = self.0.wrapping_add(1);
    }
}

fn edited_topology(_event: On<Insert<EditedChunk>>, mut version: ResMut<TopologyVersion>) {
    version.invalidate();
}

fn removed_topology(_event: On<Remove<VoxelChunk>>, mut version: ResMut<TopologyVersion>) {
    version.invalidate();
}

#[derive(Resource, Default)]
struct SectionGraph {
    center: Option<IVec3>,
    stream_center: Option<IVec2>,
    radius: u32,
    connections: Vec<u64>,
    visited: Vec<u16>,
    visible: Vec<bool>,
    previous_visible: Vec<bool>,
    valid_eye: bool,
    topology_version: u64,
    queue: VecDeque<usize>,
}

impl SectionGraph {
    /// Coalesce incoming faces before processing a section. Low bits remember
    /// scheduled exits; high bits accumulate the exits of its pending queue item.
    fn visit(&mut self, cell: usize, entry: u8) {
        self.visible[cell] = true;
        let exits = if entry == 6 {
            63
        } else {
            ((self.connections[cell] >> (entry * 6)) & 63) as u16
        };
        let exits = exits & !(self.visited[cell] & 63);
        if exits == 0 {
            return;
        }
        if self.visited[cell] >> 8 == 0 {
            self.queue.push_back(cell);
        }
        self.visited[cell] |= exits | exits << 8;
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<SectionGraph>()
        .init_resource::<TopologyVersion>()
        .add_observer(edited_topology)
        .add_observer(removed_topology)
        .add_systems(
            PostUpdate,
            update_visibility
                .after(poll_builds)
                .after(bevy::transform::TransformSystems::Propagate)
                .before(VisibilitySystems::CheckVisibility)
                .run_if(crate::world::streaming),
        );
}

fn update_visibility(
    mut commands: Commands,
    camera: Option<Single<&GlobalTransform, With<PlayerCamera>>>,
    index: Res<ChunkIndex>,
    offsets: Res<StreamOffsets>,
    stream: Res<StreamState>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels, Option<Ref<ChunkSections>>)>,
    changed: Query<(), Changed<ChunkSections>>,
    version: Res<TopologyVersion>,
    mut layers: Query<Option<&mut RenderLayers>, With<Mesh3d>>,
    mut graph: ResMut<SectionGraph>,
    mut sort_version: ResMut<super::sort::SortVersion>,
) {
    let (Some(camera), Some(stream_center)) = (camera, stream.center) else {
        return;
    };
    let eye = camera.translation();
    let center = (eye.floor().as_ivec3()).div_euclid(IVec3::splat(SECTION_HEIGHT));
    let eye_state = index
        .get(&IVec2::new(center.x, center.z))
        .and_then(|entity| chunks.get(*entity).ok())
        .and_then(|(_, voxels, _)| {
            voxels.state(IVec3::new(
                (eye.x.floor() as i32).rem_euclid(16),
                eye.y.floor() as i32,
                (eye.z.floor() as i32).rem_euclid(16),
            ))
        });
    let valid_eye = eye.is_finite()
        && (0.0..CHUNK_HEIGHT as f32).contains(&eye.y)
        && eye_state.is_some_and(|state| {
            let flags = super::models::cell_info(state).flags;
            flags & (super::models::cell::CUBE | super::models::cell::OCCLUDES)
                != (super::models::cell::CUBE | super::models::cell::OCCLUDES)
        });
    let rebuild = !(graph.center == Some(center)
        && graph.stream_center == Some(stream_center)
        && graph.radius == offsets.radius
        && graph.valid_eye == valid_eye
        && graph.topology_version == version.0);
    if !rebuild && changed.is_empty() {
        return;
    }
    let radius = offsets.radius as i32;
    let side = (radius * 2 + 1) as usize;
    let plane = side * side;
    let count = plane * SECTION_COUNT;
    let origin = stream_center - IVec2::splat(radius);
    let previous_side = if rebuild {
        (graph.radius * 2 + 1) as usize
    } else {
        side
    };
    let previous_origin = if rebuild {
        graph
            .stream_center
            .map(|center| center - IVec2::splat(graph.radius as i32))
    } else {
        Some(origin)
    };
    if rebuild {
        graph.topology_version = version.0;
        graph.valid_eye = valid_eye;
        graph.center = Some(center);
        graph.stream_center = Some(stream_center);
        graph.radius = offsets.radius;
        graph.connections.resize(count, ALL_CONNECTIONS);
        graph.connections.fill(ALL_CONNECTIONS);
        graph.visited.resize(count, 0);
        graph.visited.fill(0);
        {
            let graph = &mut *graph;
            std::mem::swap(&mut graph.previous_visible, &mut graph.visible);
        }
        graph.visible.resize(count, false);
        graph.visible.fill(false);
        graph.queue.clear();
        let column = |position: IVec2| {
            let local = position - origin;
            (local.cmpge(IVec2::ZERO).all() && local.cmplt(IVec2::splat(side as i32)).all())
                .then(|| local.x as usize + local.y as usize * side)
        };
        for (chunk, voxels, sections) in &chunks {
            let (Some(column), Some(sections)) = (column(chunk.position), sections) else {
                continue;
            };
            if sections.content_revision != voxels.content_revision {
                continue;
            }
            for y in 0..SECTION_COUNT {
                if sections.built_sections & (1 << y) != 0 {
                    graph.connections[column + y * plane] = sections.connectivity[y];
                }
            }
        }
        // Unknown space beyond the loaded area may connect passages. Seed its
        // boundary too, avoiding assumptions about terrain that has not arrived.
        for y in 0..SECTION_COUNT {
            for cell in 0..side {
                graph.visit(cell * side + y * plane, 0);
                graph.visit(side - 1 + cell * side + y * plane, 1);
                graph.visit(cell + y * plane, 4);
                graph.visit(cell + (side - 1) * side + y * plane, 5);
            }
        }
        for cell in 0..plane {
            graph.visit(cell + (SECTION_COUNT - 1) * plane, 3);
        }
        if valid_eye && let Some(column) = column(IVec2::new(center.x, center.z)) {
            graph.visit(column + center.y as usize * plane, 6);
        } else {
            graph.visible.fill(true);
            graph.queue.clear();
        }
        while let Some(cell) = graph.queue.pop_front() {
            let exits = (graph.visited[cell] >> 8) as u8;
            graph.visited[cell] &= 63;
            let x = cell % side;
            let z = (cell / side) % side;
            let y = cell / plane;
            for face in 0..6 {
                if exits & (1 << face) == 0 {
                    continue;
                }
                let coordinate = [x, x, y, y, z, z][face];
                let size = if face / 2 == 1 { SECTION_COUNT } else { side };
                if (face & 1 == 0 && coordinate == 0) || (face & 1 != 0 && coordinate + 1 == size) {
                    continue;
                }
                let step = [1, 1, plane, plane, side, side][face];
                let neighbor = if face & 1 == 0 {
                    cell - step
                } else {
                    cell + step
                };
                graph.visit(neighbor, (face ^ 1) as u8);
            }
        }
    }
    for (chunk, _, sections) in &chunks {
        let Some(sections) = sections else {
            continue;
        };
        if !rebuild && !sections.is_changed() {
            continue;
        }
        for y in 0..SECTION_COUNT {
            let overhang = sections.overhanging_sections & (1 << y) != 0;
            let visible =
                section_visible(&graph.visible, origin, side, chunk.position, y, overhang);
            let previous = previous_origin.map(|origin| {
                section_visible(
                    if rebuild {
                        &graph.previous_visible
                    } else {
                        &graph.visible
                    },
                    origin,
                    previous_side,
                    chunk.position,
                    y,
                    overhang,
                )
            });
            if !sections.is_changed() && previous == Some(visible) {
                continue;
            }
            if previous != Some(visible) {
                sort_version.0 = sort_version.0.wrapping_add(1);
            }
            let target = RenderLayers::layer(if visible { 0 } else { OCCLUDED_LAYER });
            for entity in sections.camera_entities(y).into_iter().flatten() {
                match layers.get_mut(entity) {
                    Ok(Some(mut current)) => {
                        current.set_if_neq(target.clone());
                    }
                    Ok(None) if !visible => {
                        commands.entity(entity).try_insert(target.clone());
                    }
                    _ => {}
                }
            }
        }
    }
}

/// Packed models can extend beyond their owner section, into any adjacent one.
/// Keep such batches if any section overlapped by their conservative hull is visible.
fn section_visible(
    visible: &[bool],
    origin: IVec2,
    side: usize,
    position: IVec2,
    y: usize,
    overhang: bool,
) -> bool {
    let at = |position: IVec2, y: i32| {
        let local = position - origin;
        if !(0..SECTION_COUNT as i32).contains(&y)
            || !local.cmpge(IVec2::ZERO).all()
            || !local.cmplt(IVec2::splat(side as i32)).all()
        {
            return true;
        }
        visible
            .get(local.x as usize + local.y as usize * side + y as usize * side * side)
            .copied()
            .unwrap_or(true)
    };
    if at(position, y as i32) {
        return true;
    }
    if overhang {
        for dy in -1..=1 {
            for z in -1..=1 {
                for x in -1..=1 {
                    if at(position + IVec2::new(x, z), y as i32 + dy) {
                        return true;
                    }
                }
            }
        }
    }
    false
}
