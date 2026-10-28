//! Streaming presentation is optional; simulation residency does not require meshes.
use crate::{
    RenderSource,
    build::{ChunkSections, NeedsChunkBuild},
};
use bevy::{ecs::entity::EntityIndexSet, prelude::*};
use hollow_core::coordinates::neighbor_slot;
use hollow_engine::{
    chunk::{ChunkVoxels, VoxelChunk},
    streaming::{AwaitingNeighbors, StreamOffsets, StreamState},
    world::{ChunkIndex, WorldLayout},
};
#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct RenderRequested;

/// Arrival notifications are reused by neighbor refresh and mesh readiness.
#[derive(Resource, Default)]
pub struct ChunkArrivals {
    pending: Vec<IVec2>,
    current: Vec<IVec2>,
}

pub fn track_chunk_arrival(
    event: On<Add<VoxelChunk>>,
    chunks: Query<&VoxelChunk>,
    mut arrivals: ResMut<ChunkArrivals>,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        arrivals.pending.push(chunk.position);
    }
}

/// Starts the first mesh of rendered chunks once their eight neighbors are loaded
/// (or outside the loaded area), so each is meshed once with complete borders.
pub fn release_ready_chunks<C: RenderSource>(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    offsets: Res<StreamOffsets>,
    state: Res<StreamState>,
    waiting: Query<(Entity, &VoxelChunk), With<AwaitingNeighbors>>,
    rendered: Query<(
        Entity,
        &VoxelChunk,
        Has<RenderRequested>,
        Option<&hollow_engine::blocks::LoadedColumn>,
    )>,
    arrivals: Option<Res<ChunkArrivals>>,
    mut voxels: Query<&mut ChunkVoxels<C::Cell>>,
    mut last_view: Local<Option<(IVec2, u32)>>,
    mut candidates: Local<EntityIndexSet>,
) {
    let Some(center) = state.center else {
        return;
    };
    let render_squared = (offsets.render_radius * offsets.render_radius) as i32;
    candidates.clear();
    if *last_view != Some((center, offsets.render_radius)) || arrivals.is_none() {
        candidates.extend(rendered.iter().map(|(entity, _, _, _)| entity));
        *last_view = Some((center, offsets.render_radius));
    } else if let Some(arrivals) = arrivals {
        for &position in &arrivals.current {
            for offset in NEIGHBOR_OFFSETS.into_iter().chain([IVec2::ZERO]) {
                if let Some(&entity) = index.get(&(position + offset)) {
                    candidates.insert(entity);
                }
            }
        }
    }
    // Arrival candidates also cover mesh readiness below. A view change alone
    // requires checking every loaded column for presentation transitions.
    for (entity, chunk, requested, owner) in rendered.iter_many_unique(&*candidates).matched() {
        let visible = hollow_core::coordinates::chunk_distance_squared(chunk.position, center)
            <= render_squared;
        if visible == requested {
            continue;
        }
        if visible {
            if let Ok(mut voxels) = voxels.get_mut(entity) {
                let mask = hollow_core::coordinates::section_mask(voxels.states.section_count());
                voxels.mark_sections(mask);
            }
            commands.entity(entity).insert(RenderRequested);
            if let Some(owner) = owner {
                commands.entity(owner.0).insert(RenderRequested);
            }
        } else {
            commands
                .entity(entity)
                .despawn_children()
                .remove::<(
                    ChunkSections,
                    crate::build::ChunkBuild,
                    NeedsChunkBuild,
                    RenderRequested,
                )>()
                .insert(AwaitingNeighbors);
            if let Some(owner) = owner {
                commands.entity(owner.0).remove::<RenderRequested>();
            }
        }
    }
    for (entity, chunk) in waiting.iter_many_unique(&*candidates).matched() {
        if hollow_core::coordinates::chunk_distance_squared(chunk.position, center) > render_squared
        {
            continue;
        }
        let ready = NEIGHBOR_OFFSETS.iter().all(|&offset| {
            let neighbor = chunk.position + offset;
            index.contains_key(&neighbor)
        });
        if ready {
            commands
                .entity(entity)
                .remove::<AwaitingNeighbors>()
                .insert(NeedsChunkBuild);
        }
    }
}

const NEIGHBOR_OFFSETS: [IVec2; 8] = [
    IVec2::new(-1, -1),
    IVec2::new(0, -1),
    IVec2::new(1, -1),
    IVec2::new(-1, 0),
    IVec2::new(1, 0),
    IVec2::new(-1, 1),
    IVec2::new(0, 1),
    IVec2::new(1, 1),
];

/// Remeshes chunks that were last built without a neighbor that just arrived.
pub fn refresh_neighbors_of_spawned<C: RenderSource>(
    index: Res<ChunkIndex>,
    mut chunks: Query<(
        &VoxelChunk,
        &mut ChunkVoxels<C::Cell>,
        Option<&ChunkSections>,
    )>,
    mut arrivals: ResMut<ChunkArrivals>,
) {
    let arrivals = &mut *arrivals;
    arrivals.current.clear();
    std::mem::swap(&mut arrivals.current, &mut arrivals.pending);
    for &position in &arrivals.current {
        for offset in NEIGHBOR_OFFSETS {
            let Some(&entity) = index.get(&(position + offset)) else {
                continue;
            };
            let Ok((_, mut voxels, Some(sections))) = chunks.get_mut(entity) else {
                continue;
            };
            if sections.missing_neighbors & (1 << neighbor_slot(-offset)) != 0 {
                let mask = hollow_core::coordinates::section_mask(voxels.states.section_count());
                voxels.mark_sections(mask);
            }
        }
    }
}

pub fn prepare_chunk(
    event: On<Add<VoxelChunk>>,
    chunks: Query<&VoxelChunk>,
    layout: Res<WorldLayout>,
    mut commands: Commands,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        commands.entity(event.entity).insert((
            Transform::from_xyz(
                (chunk.position.x * 16) as f32,
                layout.min_y() as f32,
                (chunk.position.y * 16) as f32,
            ),
            Visibility::default(),
            AwaitingNeighbors,
        ));
    }
}
pub fn remove_chunk(
    event: On<Remove<VoxelChunk>>,
    owner: Query<&hollow_engine::blocks::LoadedColumn>,
    mut commands: Commands,
) {
    let entity = event.entity;
    let owner = owner.get(entity).ok().map(|owner| owner.0);
    commands.queue(move |world: &mut World| {
        // A component removal keeps the provider entity alive; a despawn already
        // cleaned its linked children by the time this deferred command runs.
        if let Ok(mut entity) = world.get_entity_mut(entity) {
            entity.despawn_children().remove::<(
                ChunkSections,
                crate::build::ChunkBuild,
                NeedsChunkBuild,
                AwaitingNeighbors,
                RenderRequested,
            )>();
        }
        if let Some(owner) = owner
            && let Ok(mut owner) = world.get_entity_mut(owner)
        {
            owner.remove::<RenderRequested>();
        }
    });
}
