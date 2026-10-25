//! Asynchronous terrain colliders around moving bodies.

use super::{
    ChunkVoxels, VoxelChunk, VoxelViewer, coordinates::chunk_at, edits::EditedChunk,
    streaming::ChunkIndex,
};
use avian3d::prelude::*;
use bevy::{
    platform::collections::HashSet,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};

#[derive(Component)]
pub(crate) struct ColliderBuild {
    task: Task<(Option<Collider>, Option<Collider>)>,
    content_revision: u64,
}

#[derive(Component)]
pub(crate) struct PartialCollider(Entity);

/// Arrival/edit observers enqueue invalidations even when a mesh finishes first.
#[derive(Resource, Default)]
pub(super) struct ColliderRequests(pub(super) HashSet<Entity>);

pub(super) fn invalidate_collider(
    event: On<Insert<EditedChunk>>,
    mut requests: ResMut<ColliderRequests>,
) {
    requests.0.insert(event.entity);
}

/// Chunks within this Chebyshev distance of a moving body or viewer get colliders.
pub(super) const COLLIDER_RADIUS: i32 = 2;
/// Colliders are dropped only beyond this distance, so bodies near a border do not churn.
const COLLIDER_KEEP_RADIUS: i32 = 4;

/// Prefetch collision around moving bodies and the viewer, including spawn loading.
pub(crate) fn request_chunk_colliders(
    mut commands: Commands,
    index: Res<ChunkIndex>,
    bodies: Query<(&GlobalTransform, &RigidBody)>,
    viewers: Query<&GlobalTransform, With<VoxelViewer>>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels, Has<Collider>, Has<ColliderBuild>)>,
    mut requests: ResMut<ColliderRequests>,
    builds: Query<(), With<ColliderBuild>>,
    loaded: Query<
        (Entity, &VoxelChunk, Option<&PartialCollider>),
        Or<(With<Collider>, With<ColliderBuild>)>,
    >,
    mut centers: Local<(Vec<IVec2>, Vec<IVec2>)>,
    mut near: Local<HashSet<IVec2>>,
    mut keep: Local<HashSet<IVec2>>,
    mut demand: Local<HashSet<Entity>>,
    mut candidates: Local<Vec<(i32, Entity)>>,
) {
    // The areas only change when a body or viewer enters another chunk.
    let (previous, current) = &mut *centers;
    current.clear();
    current.extend(
        bodies
            .iter()
            .filter(|(_, body)| !body.is_static())
            .map(|(transform, _)| chunk_at(transform.translation())),
    );
    current.extend(
        viewers
            .iter()
            .map(|transform| chunk_at(transform.translation())),
    );
    current.sort_unstable_by_key(|position| (position.x, position.y));
    current.dedup();
    if current != previous {
        std::mem::swap(current, previous);
        near.clear();
        keep.clear();
        for &center in previous.iter() {
            for x in -COLLIDER_KEEP_RADIUS..=COLLIDER_KEEP_RADIUS {
                for z in -COLLIDER_KEEP_RADIUS..=COLLIDER_KEEP_RADIUS {
                    let position = center + IVec2::new(x, z);
                    keep.insert(position);
                    if x.abs().max(z.abs()) <= COLLIDER_RADIUS {
                        near.insert(position);
                    }
                }
            }
        }
        demand.retain(|entity| {
            chunks
                .get(*entity)
                .is_ok_and(|(chunk, _, _, _)| near.contains(&chunk.position))
        });
        demand.extend(
            near.iter()
                .filter_map(|position| index.get(position).copied()),
        );
        // Colliders are only requested near bodies, so none can leave `keep` meanwhile.
        for (entity, chunk, partial) in &loaded {
            if !keep.contains(&chunk.position) {
                if let Some(partial) = partial {
                    commands.entity(partial.0).try_despawn();
                }
                commands.entity(entity).remove::<PartialCollider>();
                commands
                    .entity(entity)
                    .try_remove::<(Collider, RigidBody, ColliderBuild)>();
            }
        }
    }
    for entity in requests.0.drain() {
        if chunks
            .get(entity)
            .is_ok_and(|(chunk, _, _, _)| near.contains(&chunk.position))
        {
            demand.insert(entity);
        }
    }
    let pool = AsyncComputeTaskPool::get();
    let available = (pool.thread_num() / 2)
        .max(1)
        .saturating_sub(builds.iter().count());
    let viewer_center = viewers
        .iter()
        .next()
        .map(|viewer| chunk_at(viewer.translation()));
    candidates.clear();
    demand.retain(|&entity| {
        let Ok((chunk, voxels, has_collider, building)) = chunks.get(entity) else {
            return false;
        };
        if !near.contains(&chunk.position) {
            return false;
        }
        if building {
            return true;
        }
        if (has_collider || voxels.collider_built) && !voxels.collider_dirty {
            return false;
        }
        let distance = viewer_center.map_or(0, |center| chunk.position.distance_squared(center));
        candidates.push((distance, entity));
        true
    });
    if available < candidates.len() && available > 0 {
        candidates.select_nth_unstable(available);
    }
    candidates.truncate(available);
    for &(_, entity) in candidates.iter() {
        let Ok((chunk, voxels, _, _)) = chunks.get(entity) else {
            continue;
        };
        let snapshot = voxels.clone();
        let position = chunk.position;
        commands.entity(entity).try_insert(ColliderBuild {
            content_revision: voxels.content_revision,
            task: pool.spawn(async move { snapshot.collider(position) }),
        });
    }
}

/// New colliders applied per frame away from the viewer: the rest wait in their task.
const COLLIDERS_PER_FRAME: usize = 1;

pub(crate) fn poll_chunk_colliders(
    mut commands: Commands,
    viewer: Option<Single<&GlobalTransform, With<VoxelViewer>>>,
    mut chunks: Query<(
        Entity,
        &VoxelChunk,
        &mut ChunkVoxels,
        &mut ColliderBuild,
        Has<Collider>,
        Option<&PartialCollider>,
    )>,
) {
    let viewer_chunk = viewer.map(|viewer| chunk_at(viewer.translation()));
    let mut applied = 0;
    for (entity, chunk, mut voxels, mut build, has_collider, old_partial) in &mut chunks {
        if build.content_revision != voxels.content_revision {
            // Cancel stale queued work immediately. It must neither occupy a
            // scheduling slot nor consume this frame's integration allowance.
            commands.entity(entity).remove::<ColliderBuild>();
            continue;
        }
        // Edited colliders and the viewer's own chunk never wait: something may stand on them.
        let urgent = has_collider || viewer_chunk == Some(chunk.position);
        if !urgent && applied >= COLLIDERS_PER_FRAME {
            continue;
        }
        let Some((terrain, partial)) = check_ready(&mut build.task) else {
            continue;
        };
        if !urgent {
            applied += 1;
        }
        commands.entity(entity).remove::<ColliderBuild>();
        voxels.bypass_change_detection().collider_dirty = false;
        voxels.bypass_change_detection().collider_built = terrain.is_none() && partial.is_none();
        // A Voxels shape cannot be a leaf of a Compound: Parry dispatches those
        // leaves to its convex-only contact path. Keep terrain as a root collider.
        let (collider, child) = if terrain.is_some() {
            (terrain, partial)
        } else {
            (partial, None)
        };
        if let Some(old) = old_partial {
            commands.entity(old.0).try_despawn();
        }
        commands.entity(entity).remove::<PartialCollider>();
        // Only chunks with a collider are (static) bodies: avian skips the rest.
        match collider {
            Some(collider) => commands
                .entity(entity)
                .try_insert((RigidBody::Static, collider)),
            None => commands
                .entity(entity)
                .try_remove::<(Collider, RigidBody)>(),
        };
        if let Some(partial) = child {
            let child = commands
                .spawn((
                    partial,
                    ColliderOf { body: entity },
                    Transform::default(),
                    ChildOf(entity),
                    CollisionLayers::new(
                        crate::spatial::GameLayer::World,
                        [
                            crate::spatial::GameLayer::Default,
                            crate::spatial::GameLayer::Player,
                            crate::spatial::GameLayer::DroppedItem,
                        ],
                    ),
                    Friction::new(0.8),
                ))
                .id();
            commands.entity(entity).insert(PartialCollider(child));
        }
    }
}
