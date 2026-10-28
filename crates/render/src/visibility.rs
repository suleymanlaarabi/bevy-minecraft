//! Conservative section connectivity, independent of camera orientation and shadows.

use super::build::{ChunkSections, poll_builds};
use crate::{RenderSource, VoxelViewer};
use bevy::{
    camera::visibility::{RenderLayers, VisibilitySystems},
    prelude::*,
};
use hollow_core::coordinates::SECTION_HEIGHT;
use hollow_core::{StateId, VoxelCell};
use hollow_engine::{
    chunk::{ChunkVoxels, VoxelChunk},
    edits::EditedChunk,
    streaming::{StreamOffsets, StreamState},
    world::{ChunkIndex, WorldLayout},
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Hidden from the world camera, still available to directional shadow views.
pub const OCCLUDED_LAYER: usize = 4;
pub const ALL_CONNECTIONS: u64 = (1 << 36) - 1;
const HORIZONTAL: [(usize, IVec2); 4] = [
    (0, IVec2::NEG_X),
    (1, IVec2::X),
    (4, IVec2::NEG_Y),
    (5, IVec2::Y),
];

/// Faces are -X, +X, -Y, +Y, -Z, +Z. Only full opaque cubes block traversal.
pub fn connectivity(visited: &mut [u64; 64], stack: &mut Vec<u16>) -> u64 {
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

/// One ring slot. Face pairs are transposed into vertical section masks.
#[derive(Default, Clone)]
struct Column {
    position: Option<IVec2>,
    connections: [[u32; 6]; 6],
    reached: [u32; 6],
    sent: [u32; 6],
    visible: u32,
    published: u32,
    dirty: u32,
    deferred: u32,
    publish: u32,
}

/// Opaque incremental graph; producers only invalidate affected column sections.
#[derive(Resource, Default)]
pub struct VisibilityGraph {
    columns: Vec<Column>,
    queued: Vec<u64>,
    queue: VecDeque<u32>,
    origin: IVec2,
    side: i32,
    mask: u32,
    min_y: i32,
    eye: Option<(IVec3, bool)>,
    initialize: Option<usize>,
    publish_cursor: usize,
    publish_remaining: usize,
    repeat: bool,
    conservative: bool,
}

impl VisibilityGraph {
    fn count(&self) -> usize {
        (self.side as usize).pow(2)
    }
    fn slot(&self, p: IVec2) -> Option<usize> {
        let local = p - self.origin;
        (local.cmpge(IVec2::ZERO).all() && local.cmplt(IVec2::splat(self.side)).all()).then(|| {
            p.x.rem_euclid(self.side) as usize
                + p.y.rem_euclid(self.side) as usize * self.side as usize
        })
    }
    fn position(&self, i: usize) -> IVec2 {
        let physical = IVec2::new(
            (i % self.side as usize) as i32,
            (i / self.side as usize) as i32,
        );
        self.origin + (physical - self.origin).rem_euclid(IVec2::splat(self.side))
    }
    fn enqueue(&mut self, i: usize) {
        let bit = 1 << (i % 64);
        if self.queued[i / 64] & bit == 0 {
            self.queued[i / 64] |= bit;
            self.queue.push_back(i as u32);
        }
    }
    pub fn invalidate_column(&mut self, position: IVec2, section_mask: u32) {
        if let Some(i) = self.slot(position)
            && i < self.columns.len()
        {
            self.columns[i].dirty |= section_mask & self.mask;
            self.columns[i].publish |= section_mask & self.mask;
            self.enqueue(i);
            self.publish_remaining = self.count();
            self.conservative = true;
        }
    }
    fn enter(&mut self, i: usize, face: usize, mask: u32) {
        let column = &mut self.columns[i];
        if mask & !column.reached[face] != 0 {
            column.reached[face] |= mask;
            column.visible |= mask;
            self.enqueue(i);
            self.publish_remaining = self.count();
        }
    }
    fn begin(&mut self) {
        self.queue.clear();
        self.queued.fill(0);
        self.initialize = Some(0);
        self.publish_remaining = self.count();
        self.repeat = false;
    }
    fn propagate(&mut self, i: usize) {
        let position = self.position(i);
        let seed = self
            .eye
            .filter(|(eye, valid)| *valid && position == IVec2::new(eye.x, eye.z))
            .map_or(0, |(eye, _)| 1 << eye.y);
        for exit in 0..6 {
            let column = &mut self.columns[i];
            let exits = (0..6).fold(0, |mask, entry| {
                mask | column.reached[entry] & column.connections[entry][exit]
            }) | seed;
            let exits = exits & !column.sent[exit];
            column.sent[exit] |= exits;
            if exits == 0 {
                continue;
            }
            match exit {
                2 => self.enter(i, 3, exits >> 1),
                3 => self.enter(i, 2, (exits << 1) & self.mask),
                _ => {
                    let offset = [
                        IVec2::NEG_X,
                        IVec2::X,
                        IVec2::ZERO,
                        IVec2::ZERO,
                        IVec2::NEG_Y,
                        IVec2::Y,
                    ][exit];
                    if let Some(next) = self.slot(position + offset) {
                        self.enter(next, exit ^ 1, exits);
                    }
                }
            }
        }
    }
    // The full 3x3x3 hull also covers diagonal overhangs and world boundaries.
    fn visible(&self, p: IVec2, overhang: bool) -> u32 {
        if self.eye.is_none_or(|(_, valid)| !valid) {
            return self.mask;
        }
        let mut mask = 0;
        let radius = i32::from(overhang);
        for z in -radius..=radius {
            for x in -radius..=radius {
                let other = self
                    .slot(p + IVec2::new(x, z))
                    .map_or(self.mask, |i| self.columns[i].visible);
                mask |= if overhang {
                    other | other << 1 | other >> 1 | 1 | (self.mask ^ (self.mask >> 1))
                } else {
                    other
                };
            }
        }
        mask & self.mask
    }
}

fn edited_topology(
    event: On<Insert<EditedChunk>>,
    chunks: Query<&VoxelChunk>,
    mut graph: ResMut<VisibilityGraph>,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        graph.invalidate_column(chunk.position, u32::MAX);
    }
}
fn removed_topology(
    event: On<Remove<(VoxelChunk, ChunkSections)>>,
    chunks: Query<&VoxelChunk>,
    mut graph: ResMut<VisibilityGraph>,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        graph.invalidate_column(chunk.position, u32::MAX);
    }
}

pub fn register<C: RenderSource>(app: &mut App) {
    app.init_resource::<VisibilityGraph>()
        .add_observer(edited_topology)
        .add_observer(removed_topology)
        .add_observer(
            |_: On<hollow_engine::session::SessionClosed>, mut graph: ResMut<VisibilityGraph>| {
                *graph = VisibilityGraph::default();
            },
        )
        .add_systems(
            PostUpdate,
            update_visibility::<C>
                .after(poll_builds::<C>)
                .after(bevy::transform::TransformSystems::Propagate)
                .before(VisibilitySystems::CheckVisibility)
                .run_if(resource_exists::<C>),
        );
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn update_visibility<C: RenderSource>(
    mut commands: Commands,
    camera: Option<Single<&GlobalTransform, With<VoxelViewer>>>,
    source: Res<C>,
    layout: Res<WorldLayout>,
    index: Res<ChunkIndex>,
    offsets: Res<StreamOffsets>,
    stream: Res<StreamState>,
    chunks: Query<(&ChunkVoxels<C::Cell>, Option<&ChunkSections>)>,
    mut layers: ParamSet<(
        Query<(Entity, Option<&mut RenderLayers>), (With<Camera3d>, With<VoxelViewer>)>,
        Query<Option<&mut RenderLayers>, With<Mesh3d>>,
    )>,
    mut graph: ResMut<VisibilityGraph>,
    mut sort_version: ResMut<super::sort::SortVersion>,
    #[cfg(feature = "dev")] diagnostics: Option<ResMut<crate::diagnostics::VoxelDiagnostics>>,
) {
    let started = Instant::now();
    let (Some(camera), Some(stream_center)) = (camera, stream.center) else {
        return;
    };
    let eye = camera.translation() - Vec3::Y * layout.min_y() as f32;
    let center = eye
        .floor()
        .as_ivec3()
        .div_euclid(IVec3::splat(SECTION_HEIGHT));
    let valid_eye = eye.is_finite()
        && (0.0..layout.height() as f32).contains(&eye.y)
        && index
            .get(&IVec2::new(center.x, center.z))
            .and_then(|e| chunks.get(*e).ok())
            .and_then(|(v, _)| {
                v.state(IVec3::new(
                    (eye.x.floor() as i32).rem_euclid(16),
                    eye.y.floor() as i32,
                    (eye.z.floor() as i32).rem_euclid(16),
                ))
            })
            .is_some_and(|state| {
                source.cell_info(StateId(state.id())).flags
                    & (super::models::cell::CUBE | super::models::cell::OCCLUDES)
                    != (super::models::cell::CUBE | super::models::cell::OCCLUDES)
            });
    let side = (offsets.radius * 2 + 1) as i32;
    let origin = stream_center - IVec2::splat(offsets.radius as i32);
    let g = &mut *graph;
    if g.side != side || g.mask != layout.mask() || g.min_y != layout.min_y() {
        *g = VisibilityGraph {
            columns: Vec::with_capacity((side as usize).pow(2)),
            queued: vec![0; (side as usize).pow(2).div_ceil(64)],
            queue: VecDeque::with_capacity((side as usize).pow(2)),
            side,
            mask: layout.mask(),
            min_y: layout.min_y(),
            origin,
            ..default()
        };
        g.conservative = true;
        g.begin();
    } else if g.origin != origin {
        g.origin = origin;
        g.conservative = true;
        g.begin();
    }
    if g.eye != Some((center, valid_eye)) {
        g.repeat |= g.eye.is_some();
        g.eye = Some((center, valid_eye));
        g.conservative = true;
        if valid_eye
            && let Some(i) = g.slot(IVec2::new(center.x, center.z))
            && i < g.columns.len()
        {
            // Old seeds remain until the next pass, so camera motion cannot
            // prevent the active traversal from completing conservatively.
            g.columns[i].visible |= 1 << center.y;
            g.enqueue(i);
        }
        g.publish_remaining = g.count();
    }
    #[cfg(feature = "dev")]
    let mut processed = 0u64;
    'work: while started.elapsed() < Duration::from_micros(500) {
        let initializing = g.initialize;
        let i = if let Some(i) = initializing {
            if i == g.count() {
                g.initialize = None;
                continue;
            }
            if i == g.columns.len() {
                g.columns.push(Column::default());
            }
            g.initialize = Some(i + 1);
            i
        } else if let Some(i) = g.queue.pop_front() {
            let i = i as usize;
            g.queued[i / 64] &= !(1 << (i % 64));
            i
        } else {
            if g.publish_remaining == 0 {
                g.conservative = false;
                if g.repeat {
                    g.begin();
                    continue;
                }
                break;
            }
            let cursor = g.publish_cursor;
            let position = g.position(cursor);
            if let Some((_, Some(sections))) =
                index.get(&position).and_then(|e| chunks.get(*e).ok())
            {
                let plain = g.visible(position, false);
                let hull = g.visible(position, true);
                let visible = (plain & !sections.overhanging_sections)
                    | (hull & sections.overhanging_sections);
                let column = &mut g.columns[cursor];
                let mut changed = (visible ^ column.published) | column.publish;
                let mut meshes = layers.p1();
                while changed != 0 {
                    if started.elapsed() >= Duration::from_micros(500) {
                        break 'work;
                    }
                    let y = changed.trailing_zeros() as usize;
                    let bit = 1 << y;
                    changed &= !bit;
                    column.published = (column.published & !bit) | (visible & bit);
                    column.publish &= !bit;
                    for entity in sections.camera_entities(y).into_iter().flatten() {
                        if let Ok(current) = meshes.get_mut(entity) {
                            let mut target = current.as_deref().cloned().unwrap_or_default();
                            let was_visible = target.intersects(&RenderLayers::layer(0));
                            let now_visible = visible & (1 << y) != 0;
                            target = target
                                .without(0)
                                .without(OCCLUDED_LAYER)
                                .with(if now_visible { 0 } else { OCCLUDED_LAYER });
                            if was_visible != now_visible && !g.conservative {
                                sort_version.0 = sort_version.0.wrapping_add(1);
                            }
                            if let Some(mut current) = current {
                                current.set_if_neq(target);
                            } else if !now_visible {
                                commands.entity(entity).try_insert(target);
                            }
                        }
                    }
                }
            }
            #[cfg(feature = "dev")]
            {
                processed += 1;
            }
            g.publish_cursor = (cursor + 1) % g.columns.len();
            g.publish_remaining -= 1;
            continue;
        };
        #[cfg(feature = "dev")]
        {
            processed += 1;
        }
        let position = g.position(i);
        if initializing.is_some() {
            if g.columns[i].position != Some(position) {
                g.columns[i] = Column {
                    position: Some(position),
                    dirty: g.mask,
                    publish: g.mask,
                    ..default()
                };
            }
            let c = &mut g.columns[i];
            c.reached = [0; 6];
            c.sent = [0; 6];
            c.visible = if valid_eye { 0 } else { g.mask };
            c.dirty |= c.deferred;
            c.deferred = 0;
        }
        let dirty = std::mem::take(&mut g.columns[i].dirty);
        if dirty != 0 {
            let data = index.get(&position).and_then(|e| chunks.get(*e).ok());
            let mut pending = dirty;
            while pending != 0 {
                if started.elapsed() >= Duration::from_micros(500) {
                    g.columns[i].dirty |= pending;
                    if initializing.is_some() {
                        g.initialize = Some(i);
                    } else {
                        g.enqueue(i);
                    }
                    break 'work;
                }
                let y = pending.trailing_zeros() as usize;
                let bit = 1 << y;
                pending &= pending - 1;
                let connections = data
                    .and_then(|(v, s)| {
                        s.filter(|s| {
                            s.content_revision == v.content_revision && s.built_sections & bit != 0
                        })
                    })
                    .map_or(ALL_CONNECTIONS, |s| s.sections[y].connectivity);
                let c = &mut g.columns[i];
                for pair in 0..36 {
                    let next = if connections & (1 << pair) != 0 {
                        bit
                    } else {
                        0
                    };
                    if initializing.is_some() {
                        c.connections[pair / 6][pair % 6] =
                            (c.connections[pair / 6][pair % 6] & !bit) | next;
                    } else {
                        if c.connections[pair / 6][pair % 6] & bit & !next != 0 {
                            c.deferred |= bit;
                            g.repeat = true;
                        }
                        c.connections[pair / 6][pair % 6] |= next;
                    }
                }
            }
        }
        if initializing.is_some() {
            for (face, offset) in HORIZONTAL {
                if g.slot(position + offset).is_none() {
                    g.enter(i, face, g.mask);
                }
            }
            g.enter(i, 3, g.mask ^ (g.mask >> 1));
            if valid_eye && position == IVec2::new(center.x, center.z) {
                // Camera seed exits are unconditional, including an enclosed eye cell.
                g.columns[i].visible |= 1 << center.y;
            }
        } else {
            g.propagate(i);
        }
    }
    for (entity, current) in &mut layers.p0() {
        let current_value = current.as_deref().cloned().unwrap_or_default();
        let target = if g.conservative {
            current_value.clone().with(OCCLUDED_LAYER)
        } else {
            current_value.clone().without(OCCLUDED_LAYER)
        };
        if current_value != target {
            sort_version.0 = sort_version.0.wrapping_add(1);
        }
        if let Some(mut current) = current {
            current.set_if_neq(target);
        } else if g.conservative {
            commands.entity(entity).try_insert(target);
        }
    }
    #[cfg(feature = "dev")]
    if let Some(mut diagnostics) = diagnostics {
        diagnostics.visibility_seconds += started.elapsed().as_secs_f64();
        diagnostics.visibility_columns = processed;
        diagnostics.visibility_remaining = g.queue.len()
            + g.initialize.map_or(0, |i| g.count() - i)
            + g.publish_remaining
            + if g.repeat { g.count() } else { 0 };
        diagnostics.visibility_conservative = g.conservative;
    }
}
