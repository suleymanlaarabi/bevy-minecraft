use super::{
    VoxelSettings, VoxelViewer,
    meshing::{ChunkMeshes, VoxelGeometry},
    rebuild::{ColliderUpdate, VoxelAssets},
};
use crate::game::GameState;
use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    platform::collections::HashMap,
    prelude::*,
    tasks::{AsyncComputeTaskPool, Task, futures::check_ready},
};
use std::sync::Arc;
#[cfg(feature = "dev")]
use std::time::{Duration, Instant};

const REGION_SIZE: i32 = 4;
const SETTLE_FRAMES: u8 = 4;
const MAX_PENDING_REGIONS: usize = 2;

/// Squared distance to the viewer, with coordinates as a deterministic tie-break.
type DistanceKey = (i32, i32, i32);

#[derive(Component)]
pub(crate) struct VoxelRenderRegion;

struct RegionEntities {
    terrain: Entity,
    water: Option<Entity>,
}

struct DirtyRegion {
    age: u8,
    revision: u64,
    /// Merges started before this revision still contain a removed chunk.
    floor: u64,
    /// Player edits bypass the merge limits so they appear without delay.
    urgent: bool,
}

struct DeferredCollider {
    revision: u64,
    entity: Entity,
    update: ColliderUpdate,
}

struct PreparedRegion {
    terrain: Mesh,
    water: Option<Mesh>,
    #[cfg(feature = "dev")]
    elapsed: Duration,
}

#[derive(Component)]
pub(crate) struct RegionBuild {
    position: IVec2,
    revision: u64,
    urgent: bool,
    task: Task<PreparedRegion>,
}

#[derive(Resource, Default)]
pub(crate) struct RenderRegions {
    pieces: HashMap<IVec2, HashMap<IVec2, Arc<ChunkMeshes>>>,
    entities: HashMap<IVec2, RegionEntities>,
    dirty: HashMap<IVec2, DirtyRegion>,
    pending: HashMap<IVec2, Entity>,
    /// Edit colliders wait for the merge that shows the same edit.
    colliders: HashMap<IVec2, Vec<DeferredCollider>>,
    next_revision: u64,
}

impl RenderRegions {
    fn mark_dirty(&mut self, region: IVec2, age: u8, urgent: bool, removal: bool) {
        // A new token also distinguishes deletion followed by recreation.
        self.next_revision = self
            .next_revision
            .checked_add(1)
            .expect("region revision exhausted");
        let revision = self.next_revision;
        let dirty = self.dirty.entry(region).or_insert(DirtyRegion {
            age,
            revision,
            floor: 0,
            urgent,
        });
        dirty.age = age;
        dirty.revision = revision;
        dirty.urgent |= urgent;
        if removal {
            dirty.floor = revision;
        }
    }

    #[cfg(test)]
    pub(crate) fn insert(&mut self, chunk: IVec2, meshes: ChunkMeshes) {
        self.insert_piece(chunk, meshes, false);
    }

    fn insert_piece(&mut self, chunk: IVec2, meshes: ChunkMeshes, urgent: bool) {
        let region = region_of(chunk);
        let replaced = self
            .pieces
            .entry(region)
            .or_default()
            .insert(chunk, Arc::new(meshes))
            .is_some();
        let age = if replaced || urgent { SETTLE_FRAMES } else { 0 };
        self.mark_dirty(region, age, urgent, false);
    }

    /// Stores a rebuilt chunk and returns its collider when it may apply right away.
    ///
    /// Edit colliders, and any collider queued behind one, wait for the merge that
    /// draws the same voxels so a block is never solid and invisible (or the reverse).
    pub(crate) fn insert_with_collider(
        &mut self,
        chunk: IVec2,
        meshes: ChunkMeshes,
        urgent: bool,
        entity: Entity,
        collider: ColliderUpdate,
    ) -> Option<ColliderUpdate> {
        self.insert_piece(chunk, meshes, urgent);
        let region = region_of(chunk);
        if matches!(collider, ColliderUpdate::Keep)
            || !(urgent || self.colliders.contains_key(&region))
        {
            return Some(collider);
        }
        self.colliders
            .entry(region)
            .or_default()
            .push(DeferredCollider {
                revision: self.next_revision,
                entity,
                update: collider,
            });
        None
    }

    pub(crate) fn remove(&mut self, chunk: IVec2) {
        let region = region_of(chunk);
        let Some(pieces) = self.pieces.get_mut(&region) else {
            return;
        };
        if pieces.remove(&chunk).is_none() {
            return;
        }
        if pieces.is_empty() {
            self.pieces.remove(&region);
        }
        self.mark_dirty(region, SETTLE_FRAMES, false, true);
    }

    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.pieces.is_empty()
            && self.dirty.is_empty()
            && self.pending.is_empty()
            && self.colliders.is_empty()
    }

    #[cfg(feature = "dev")]
    pub(crate) fn is_settled(&self) -> bool {
        self.dirty.is_empty() && self.pending.is_empty()
    }

    #[cfg(feature = "dev")]
    pub(crate) fn dirty_len(&self) -> usize {
        self.dirty.len()
    }

    #[cfg(feature = "dev")]
    pub(crate) fn pending_len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn clear(&mut self, commands: &mut Commands) {
        for entities in self.entities.values() {
            commands.entity(entities.terrain).try_despawn();
        }
        for &entity in self.pending.values() {
            commands.entity(entity).try_despawn();
        }
        let revision = self.next_revision;
        *self = Self::default();
        self.next_revision = revision;
    }

    fn apply_colliders(&mut self, commands: &mut Commands, region: IVec2, revision: u64) {
        let Some(colliders) = self.colliders.remove(&region) else {
            return;
        };
        let (ready, waiting): (Vec<_>, Vec<_>) = colliders
            .into_iter()
            .partition(|collider| collider.revision <= revision);
        for collider in ready {
            collider.update.apply(commands, collider.entity);
        }
        if !waiting.is_empty() {
            self.colliders.insert(region, waiting);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn rebuild_render_regions(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    assets: Res<VoxelAssets>,
    viewer: Option<Single<&Transform, With<VoxelViewer>>>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut regions: ResMut<RenderRegions>,
    mut builds: Query<(Entity, &mut RegionBuild)>,
    mut ready: Local<Vec<(DistanceKey, IVec2)>>,
    #[cfg(feature = "dev")] mut diagnostics: Option<ResMut<super::diagnostics::VoxelDiagnostics>>,
) {
    for dirty in regions.dirty.values_mut() {
        dirty.age = dirty.age.saturating_add(1);
    }

    // Empty regions need no merge and must invalidate any in-flight result.
    let empty: Vec<_> = regions
        .dirty
        .keys()
        .copied()
        .filter(|position| !regions.pieces.contains_key(position))
        .collect();
    for position in empty {
        if let Some(entity) = regions.pending.remove(&position) {
            commands.entity(entity).try_despawn();
        }
        if let Some(entities) = regions.entities.remove(&position) {
            commands.entity(entities.terrain).try_despawn();
        }
        regions.dirty.remove(&position);
        regions.colliders.remove(&position);
    }

    // Streaming merges apply one per frame; edit merges apply as soon as they finish.
    let mut applied = false;
    for (entity, mut build) in &mut builds {
        // A streaming merge still blocks an edit made after it started.
        let urgent = build.urgent
            || regions
                .dirty
                .get(&build.position)
                .is_some_and(|dirty| dirty.urgent);
        if regions.pending.get(&build.position) != Some(&entity) || (applied && !urgent) {
            continue;
        }
        let Some(output) = check_ready(&mut build.task) else {
            continue;
        };
        let position = build.position;
        regions.pending.remove(&position);
        commands.entity(entity).despawn();
        let Some(dirty) = regions.dirty.get(&position) else {
            continue;
        };
        let current = dirty.revision == build.revision;
        // An edit merge outdated by later edits is still newer than what is drawn, so
        // it is shown while the region stays dirty. Streaming waits for the next one.
        if build.revision < dirty.floor || !(current || urgent) {
            continue;
        }
        if current {
            regions.dirty.remove(&position);
        }
        #[cfg(feature = "dev")]
        if let Some(diagnostics) = diagnostics.as_deref_mut() {
            diagnostics.record_region(output.elapsed);
        }
        let terrain_mesh = mesh_assets.add(output.terrain);
        if let Some(mut entities) = regions.entities.remove(&position) {
            commands
                .entity(entities.terrain)
                .insert(Mesh3d(terrain_mesh));
            update_water(
                &mut commands,
                &mut mesh_assets,
                &assets,
                entities.terrain,
                &mut entities.water,
                output.water,
            );
            regions.entities.insert(position, entities);
        } else {
            let origin = position * REGION_SIZE * settings.chunk_size;
            let terrain = commands
                .spawn((
                    VoxelRenderRegion,
                    Mesh3d(terrain_mesh),
                    MeshMaterial3d(assets.terrain.clone()),
                    Transform::from_xyz(origin.x as f32, 0.0, origin.y as f32),
                    DespawnOnExit(GameState::Game),
                ))
                .id();
            let mut entities = RegionEntities {
                terrain,
                water: None,
            };
            update_water(
                &mut commands,
                &mut mesh_assets,
                &assets,
                terrain,
                &mut entities.water,
                output.water,
            );
            regions.entities.insert(position, entities);
        }
        regions.apply_colliders(&mut commands, position, build.revision);
        applied |= !urgent;
    }

    let available = MAX_PENDING_REGIONS.saturating_sub(regions.pending.len());
    let center = viewer.map_or(IVec2::ZERO, |viewer| {
        region_of(settings.chunk_at(viewer.translation))
    });
    ready.clear();
    let mut urgent = Vec::new();
    for (&position, dirty) in &regions.dirty {
        if regions.pending.contains_key(&position) {
            continue;
        }
        if dirty.urgent {
            urgent.push((position, dirty.revision));
        } else if available > 0 && dirty.age >= SETTLE_FRAMES {
            ready.push((
                (position.distance_squared(center), position.x, position.y),
                position,
            ));
        }
    }
    // Only the nearest `available` regions start, so a partial selection is enough.
    if available < ready.len() {
        ready.select_nth_unstable_by_key(available, |&(key, _)| key);
        ready.truncate(available);
    }
    let streaming = ready
        .iter()
        .map(|&(_, position)| (position, regions.dirty[&position].revision, false));
    let starts: Vec<_> = urgent
        .into_iter()
        .map(|(position, revision)| (position, revision, true))
        .chain(streaming)
        .collect();
    for (position, revision, urgent) in starts {
        let pieces = regions.pieces[&position]
            .iter()
            .map(|(&chunk, geometry)| (chunk, Arc::clone(geometry)))
            .collect();
        let chunk_size = settings.chunk_size;
        let task = AsyncComputeTaskPool::get()
            .spawn(async move { merge_region(position, chunk_size, pieces) });
        let entity = commands
            .spawn((
                RegionBuild {
                    position,
                    revision,
                    urgent,
                    task,
                },
                DespawnOnExit(GameState::Game),
            ))
            .id();
        regions.pending.insert(position, entity);
    }
}

fn merge_region(
    region: IVec2,
    chunk_size: i32,
    mut pieces: Vec<(IVec2, Arc<ChunkMeshes>)>,
) -> PreparedRegion {
    #[cfg(feature = "dev")]
    let started = Instant::now();
    pieces.sort_unstable_by_key(|(chunk, _)| (chunk.y, chunk.x));
    let (mut terrain_vertices, mut terrain_indices, mut water_vertices, mut water_indices) =
        (0, 0, 0, 0);
    for (_, meshes) in &pieces {
        let (vertices, indices) = meshes.0.buffer_lengths();
        terrain_vertices += vertices;
        terrain_indices += indices;
        if let Some(water) = &meshes.1 {
            let (vertices, indices) = water.buffer_lengths();
            water_vertices += vertices;
            water_indices += indices;
        }
    }
    let mut terrain = VoxelGeometry::with_capacity(terrain_vertices, terrain_indices);
    let mut water = VoxelGeometry::with_capacity(water_vertices, water_indices);
    for (chunk, meshes) in pieces {
        let local = (chunk - region * REGION_SIZE) * chunk_size;
        let offset = Vec3::new(local.x as f32, 0.0, local.y as f32);
        terrain.append(&meshes.0, offset);
        if let Some(piece) = &meshes.1 {
            water.append(piece, offset);
        }
    }
    PreparedRegion {
        terrain: terrain.into_mesh(),
        water: (!water.is_empty()).then(|| water.into_mesh()),
        #[cfg(feature = "dev")]
        elapsed: started.elapsed(),
    }
}

fn update_water(
    commands: &mut Commands,
    mesh_assets: &mut Assets<Mesh>,
    assets: &VoxelAssets,
    terrain: Entity,
    water_entity: &mut Option<Entity>,
    water: Option<Mesh>,
) {
    match (water, *water_entity) {
        (Some(mesh), Some(entity)) => {
            commands
                .entity(entity)
                .insert(Mesh3d(mesh_assets.add(mesh)));
        }
        (Some(mesh), None) => {
            *water_entity = Some(
                commands
                    .spawn((
                        Mesh3d(mesh_assets.add(mesh)),
                        MeshMaterial3d(assets.water.clone()),
                        ChildOf(terrain),
                        NotShadowCaster,
                        NotShadowReceiver,
                    ))
                    .id(),
            );
        }
        (None, Some(entity)) => {
            commands.entity(entity).despawn();
            *water_entity = None;
        }
        (None, None) => {}
    }
}

pub(crate) fn region_of(chunk: IVec2) -> IVec2 {
    IVec2::new(
        chunk.x.div_euclid(REGION_SIZE),
        chunk.y.div_euclid(REGION_SIZE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_backlog_respects_task_and_application_limits() {
        use bevy::ecs::system::RunSystemOnce;
        use std::time::{Duration, Instant};
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(VoxelSettings::default())
            .insert_resource(VoxelAssets {
                terrain: default(),
                water: default(),
            })
            .init_resource::<Assets<Mesh>>()
            .init_resource::<RenderRegions>();
        let world = app.world_mut();
        for x in 0..5 {
            world.resource_mut::<RenderRegions>().insert(
                IVec2::new(x * REGION_SIZE, 0),
                ChunkMeshes(VoxelGeometry::default(), None),
            );
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut previous = 0;
        loop {
            world.run_system_once(rebuild_render_regions).unwrap();
            let regions = world.resource::<RenderRegions>();
            assert!(regions.pending.len() <= MAX_PENDING_REGIONS);
            assert!(
                regions.entities.len() <= previous + 1,
                "more than one region applied in one frame"
            );
            previous = regions.entities.len();
            if regions.dirty.is_empty() && regions.pending.is_empty() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        assert_eq!(previous, 5);
    }

    #[test]
    fn recreated_region_rejects_old_merge_and_clear_cancels_pending_tasks() {
        use bevy::ecs::system::RunSystemOnce;
        use std::time::{Duration, Instant};
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(VoxelSettings::default())
            .insert_resource(VoxelAssets {
                terrain: default(),
                water: default(),
            })
            .init_resource::<Assets<Mesh>>()
            .init_resource::<RenderRegions>();
        let world = app.world_mut();
        let pack =
            super::super::material::TexturePack::load(super::super::TexturePackId::default());
        let solid = super::super::ChunkVoxels::generated(
            1,
            1,
            vec![super::super::VoxelKind::Stone],
            vec![super::super::VoxelKind::Air; 4],
        );
        world.resource_mut::<RenderRegions>().insert(
            IVec2::ZERO,
            super::super::meshing::build_chunk_mesh(&solid, &pack),
        );
        for _ in 0..SETTLE_FRAMES {
            world.run_system_once(rebuild_render_regions).unwrap();
        }
        assert_eq!(world.resource::<RenderRegions>().pending.len(), 1);
        let old_revision = world.resource::<RenderRegions>().dirty[&IVec2::ZERO].revision;
        world.resource_mut::<RenderRegions>().remove(IVec2::ZERO);
        world
            .resource_mut::<RenderRegions>()
            .insert(IVec2::ZERO, ChunkMeshes(VoxelGeometry::default(), None));
        assert_ne!(
            world.resource::<RenderRegions>().dirty[&IVec2::ZERO].revision,
            old_revision
        );
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            world.run_system_once(rebuild_render_regions).unwrap();
            let regions = world.resource::<RenderRegions>();
            if let Some(entities) = regions.entities.get(&IVec2::ZERO) {
                let mesh = &world.get::<Mesh3d>(entities.terrain).unwrap().0;
                assert_eq!(
                    world
                        .resource::<Assets<Mesh>>()
                        .get(mesh)
                        .unwrap()
                        .count_vertices(),
                    0
                );
            }
            if regions.dirty.is_empty() && regions.pending.is_empty() {
                break;
            }
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        world
            .resource_mut::<RenderRegions>()
            .insert(IVec2::ZERO, ChunkMeshes(VoxelGeometry::default(), None));
        world.run_system_once(rebuild_render_regions).unwrap();
        let pending: Vec<_> = world
            .resource::<RenderRegions>()
            .pending
            .values()
            .copied()
            .collect();
        assert_eq!(pending.len(), 1);
        world
            .run_system_once(
                |mut regions: ResMut<RenderRegions>, mut commands: Commands| {
                    regions.clear(&mut commands)
                },
            )
            .unwrap();
        assert!(world.resource::<RenderRegions>().pending.is_empty());
        assert!(
            pending
                .into_iter()
                .all(|entity| world.get_entity(entity).is_err())
        );
    }

    fn region_app() -> App {
        let mut app = App::new();
        app.add_plugins(TaskPoolPlugin::default())
            .insert_resource(VoxelSettings::default())
            .insert_resource(VoxelAssets {
                terrain: default(),
                water: default(),
            })
            .init_resource::<Assets<Mesh>>()
            .init_resource::<RenderRegions>();
        app
    }

    fn run_until(world: &mut World, mut done: impl FnMut(&mut World) -> bool) {
        use bevy::ecs::system::RunSystemOnce;
        use std::time::{Duration, Instant};
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            world.run_system_once(rebuild_render_regions).unwrap();
            if done(world) {
                return;
            }
            assert!(Instant::now() < deadline, "region merge did not finish");
            std::thread::yield_now();
        }
    }

    #[test]
    fn outdated_edit_merge_is_shown_while_the_region_stays_dirty() {
        use bevy::ecs::system::RunSystemOnce;
        let mut app = region_app();
        let world = app.world_mut();
        world.resource_mut::<RenderRegions>().insert_piece(
            IVec2::ZERO,
            ChunkMeshes(VoxelGeometry::default(), None),
            true,
        );
        world.run_system_once(rebuild_render_regions).unwrap();
        assert_eq!(world.resource::<RenderRegions>().pending.len(), 1);
        world.resource_mut::<RenderRegions>().insert_piece(
            IVec2::ZERO,
            ChunkMeshes(VoxelGeometry::default(), None),
            true,
        );
        run_until(world, |world| {
            world
                .resource::<RenderRegions>()
                .entities
                .contains_key(&IVec2::ZERO)
        });
        assert!(
            world
                .resource::<RenderRegions>()
                .dirty
                .contains_key(&IVec2::ZERO)
        );
        run_until(world, |world| {
            world.resource::<RenderRegions>().dirty.is_empty()
        });
    }

    #[test]
    fn edit_merges_skip_the_pending_limit_and_carry_their_collider() {
        use avian3d::prelude::Collider;
        use bevy::ecs::system::RunSystemOnce;
        let mut app = region_app();
        let world = app.world_mut();
        for x in 0..MAX_PENDING_REGIONS as i32 {
            let position = IVec2::new(10 + x, 0);
            let blocker = world
                .spawn(RegionBuild {
                    position,
                    revision: 0,
                    urgent: false,
                    task: AsyncComputeTaskPool::get().spawn(std::future::pending()),
                })
                .id();
            let mut regions = world.resource_mut::<RenderRegions>();
            regions.insert(
                position * REGION_SIZE,
                ChunkMeshes(VoxelGeometry::default(), None),
            );
            regions.pending.insert(position, blocker);
        }
        let chunk = world.spawn_empty().id();
        let deferred = world.resource_mut::<RenderRegions>().insert_with_collider(
            IVec2::ZERO,
            ChunkMeshes(VoxelGeometry::default(), None),
            true,
            chunk,
            ColliderUpdate::Replace(Collider::cuboid(1.0, 1.0, 1.0)),
        );
        assert!(deferred.is_none());
        world.run_system_once(rebuild_render_regions).unwrap();
        assert_eq!(
            world.resource::<RenderRegions>().pending.len(),
            MAX_PENDING_REGIONS + 1
        );
        assert!(world.get::<Collider>(chunk).is_none());
        run_until(world, |world| {
            world
                .resource::<RenderRegions>()
                .entities
                .contains_key(&IVec2::ZERO)
        });
        assert!(world.get::<Collider>(chunk).is_some());
        assert!(world.resource::<RenderRegions>().colliders.is_empty());
    }

    #[test]
    fn region_coordinates_handle_negative_chunks() {
        assert_eq!(region_of(IVec2::new(0, 0)), IVec2::ZERO);
        assert_eq!(region_of(IVec2::new(3, 3)), IVec2::ZERO);
        assert_eq!(region_of(IVec2::new(4, 4)), IVec2::ONE);
        assert_eq!(region_of(IVec2::new(-1, -1)), IVec2::NEG_ONE);
        assert_eq!(region_of(IVec2::new(-4, -4)), IVec2::NEG_ONE);
        assert_eq!(region_of(IVec2::new(-5, -5)), IVec2::splat(-2));
    }

    #[test]
    fn removals_drop_empty_region_storage() {
        let mut regions = RenderRegions::default();
        regions.insert(IVec2::ZERO, ChunkMeshes(VoxelGeometry::default(), None));
        regions.remove(IVec2::ZERO);

        assert!(regions.pieces.is_empty());
        assert_eq!(
            regions.dirty.get(&IVec2::ZERO).map(|dirty| dirty.age),
            Some(SETTLE_FRAMES)
        );
    }

    #[test]
    fn sixteen_chunks_share_one_render_region_and_edits_skip_debounce() {
        let mut regions = RenderRegions::default();
        for z in 0..REGION_SIZE {
            for x in 0..REGION_SIZE {
                regions.insert(
                    IVec2::new(x, z),
                    ChunkMeshes(VoxelGeometry::default(), None),
                );
            }
        }

        assert_eq!(regions.pieces.len(), 1);
        assert_eq!(regions.pieces[&IVec2::ZERO].len(), 16);
        assert_eq!(
            regions.dirty.get(&IVec2::ZERO).map(|dirty| dirty.age),
            Some(0)
        );

        regions.insert(
            IVec2::new(2, 2),
            ChunkMeshes(VoxelGeometry::default(), None),
        );
        assert_eq!(
            regions.dirty.get(&IVec2::ZERO).map(|dirty| dirty.age),
            Some(SETTLE_FRAMES)
        );
    }
}
