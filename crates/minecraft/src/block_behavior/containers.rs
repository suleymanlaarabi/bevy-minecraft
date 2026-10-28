//! ECS owns gameplay data through unloads; transfer records are immutable snapshots.
use super::catalog::{chest_partner, profile, state_behavior};
use crate::inventory::{InventorySession, InventoryStation, StationViewers};
use crate::{
    inventory::slots::ItemSlots,
    voxel::{ChunkVoxels, VoxelState, VoxelWorld},
};
use bevy::{ecs::system::SystemParam, prelude::*};
use hollow_features::motion::FixedPosition;
use std::io;

pub(crate) const CONTAINER_SIZE: usize = 27;
// Retired stateful blocks can retain slot arrays and sparse ECS/index storage.
// Charge a populated inventory even when this particular block owns no slots.
pub(crate) const RETAINED_BLOCK_BYTES: usize =
    std::mem::size_of::<[Option<crate::inventory::ItemStack>; CONTAINER_SIZE]>() + 256;
pub(crate) type ContainerInventory = ItemSlots<CONTAINER_SIZE>;

#[derive(Component)]
#[require(
    ContainerInventory,
    super::visuals::InstanceLight,
    InventoryStation::Container
)]
pub(crate) struct Chest;
#[derive(Component, Default)]
pub(crate) struct ManualOpen;
#[derive(Component, Default)]
pub(crate) struct Door;
#[derive(Component, Default)]
pub(crate) struct Gate;
pub use hollow_engine::blocks::{
    BlockEntities, BlockEntity, BlockLookup, BlockOf, Dormant, LoadedColumn, LoadedTerrain,
};
pub type BlockState = hollow_engine::blocks::BlockState<VoxelState>;
#[derive(Component)]
#[relationship(relationship_target = ChestPartners)]
pub(crate) struct PairedWith(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = PairedWith)]
pub(crate) struct ChestPartners(Vec<Entity>);
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct TopologyDirty;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct AvailableContainer;

pub use hollow_engine::blocks::ColumnOwners as GameplayColumns;
pub type GameplayColumn = hollow_engine::simulation::ColumnData<crate::creatures::ColumnActors>;
pub(crate) fn column(world: &mut World, position: IVec2) -> Entity {
    let entity = hollow_engine::blocks::column(world, position);
    if world.get::<GameplayColumn>(entity).is_none() {
        world.entity_mut(entity).insert(GameplayColumn {
            position,
            revision: 0,
            saved_revision: 0,
            initialized: false,
            application: default(),
        });
    }
    entity
}
/// Chest-specific topology, inventory and presentation are installed together.
pub struct ChestPlugin;
impl Plugin for ChestPlugin {
    fn build(&self, app: &mut App) {
        let mut registrations = app
            .world_mut()
            .resource_mut::<hollow_engine::blocks::BlockRegistrations>();
        for kind in crate::voxel::VoxelKind::ALL
            .into_iter()
            .filter(|kind| profile(*kind).chest)
        {
            registrations.register(
                hollow_core::BlockId(kind.index() as u32),
                hollow_engine::blocks::BlockFactory {
                    attach: |entity, _| {
                        entity.insert((Chest, TopologyDirty, super::visuals::PrepareView));
                    },
                    detach: |entity| {
                        entity.remove::<(Chest, InventoryStation)>();
                    },
                    interactive: true,
                    refresh_on_state_change: true,
                },
            );
        }
        app.init_resource::<super::visuals::ChestMeshes>()
            .add_observer(pairing_removed)
            .add_systems(
                PostUpdate,
                update_topology
                    .in_set(super::GameplaySystems::Topology)
                    .run_if(crate::world::streaming),
            );
        super::visuals::register(app);
    }
}

pub(crate) fn top_clear(world: &VoxelWorld, position: IVec3) -> bool {
    position.y + 1 >= hollow_worldgen::coordinates::CHUNK_HEIGHT
        || world
            .state(position + IVec3::Y)
            .is_some_and(|state| !state.is_side_solid(pumpkin_data::BlockDirection::Down))
}

#[derive(Clone, Default)]
pub(crate) struct PreparedColumn {
    pub entries: Vec<(IVec3, VoxelState, ContainerInventory)>,
    pub revision: u64,
    pub simulation: crate::simulation::ColumnSimulation,
    pub drops: Vec<crate::world::SavedDrop>,
    pub creatures: Vec<crate::creatures::SavedCreature>,
    pub spawners: Vec<crate::creatures::SavedSpawner>,
    pub biomes: std::sync::Arc<[u8]>,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct ColumnRecord {
    containers: Vec<ContainerRecord>,
    simulation: crate::simulation::SimulationSnapshot,
    drops: Vec<crate::world::SavedDrop>,
    creatures: Vec<crate::creatures::SavedCreature>,
    spawners: Vec<crate::creatures::SavedSpawner>,
    biomes: Vec<u8>,
}
type ContainerRecord = ([i32; 3], [Option<crate::world::SavedStack>; CONTAINER_SIZE]);
impl PreparedColumn {
    /// Runs only on a chunk-copy worker. Uniform sections are classified once.
    pub fn discover(
        voxels: &ChunkVoxels,
        chunk: IVec2,
        catalog: &hollow_engine::blocks::BlockCatalog,
    ) -> Self {
        use hollow_core::coordinates::{CHUNK_SIZE, SECTION_HEIGHT};
        use hollow_worldgen::coordinates::SECTION_COUNT;
        let mut result = Self::default();
        let origin = IVec3::new(chunk.x * CHUNK_SIZE, 0, chunk.y * CHUNK_SIZE);
        for section in 0..SECTION_COUNT {
            if !voxels.states.section(section).any(|state| {
                catalog.materialized(hollow_core::BlockId(state.kind().index() as u32))
            }) {
                continue;
            }
            for y in section as i32 * SECTION_HEIGHT..(section as i32 + 1) * SECTION_HEIGHT {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let local = IVec3::new(x, y, z);
                        if voxels
                            .state(local)
                            .map(|state| state.kind())
                            .is_some_and(|kind| {
                                catalog.materialized(hollow_core::BlockId(kind.index() as u32))
                            })
                        {
                            result.entries.push((
                                origin + local,
                                voxels.state(local).expect("Discovered state"),
                                default(),
                            ));
                        }
                    }
                }
            }
        }
        result
    }

    pub fn restore(&mut self, bytes: &[u8], chunk: IVec2) -> io::Result<()> {
        let mut record: ColumnRecord = serde_json::from_slice(bytes).map_err(io::Error::other)?;
        self.simulation = crate::simulation::ColumnSimulation::restore(
            record.simulation,
            hollow_worldgen::coordinates::SECTION_COUNT,
        )?;
        for drop in &record.drops {
            drop.validate()?;
            if crate::voxel::chunk_at(Vec3::from_array(drop.position)) != chunk {
                return Err(io::Error::other("Dropped item belongs to another column."));
            }
        }
        for creature in &record.creatures {
            creature.validate(chunk)?;
        }
        if !record.biomes.is_empty()
            && record.biomes.len() != hollow_worldgen::coordinates::SECTION_COUNT * 64
        {
            return Err(io::Error::other("Invalid column biomes."));
        }
        self.entries
            .sort_unstable_by_key(|(p, _, _)| (p.y, p.z, p.x));
        record.spawners.sort_unstable_by_key(|s| s.position);
        if record
            .spawners
            .windows(2)
            .any(|pair| pair[0].position == pair[1].position)
        {
            return Err(io::Error::other("Duplicate spawner record."));
        }
        for spawner in &record.spawners {
            let position = IVec3::from_array(spawner.position);
            let matches = self
                .entries
                .binary_search_by_key(&(position.y, position.z, position.x), |(p, _, _)| {
                    (p.y, p.z, p.x)
                })
                .ok()
                .is_some_and(|index| {
                    self.entries[index].1.kind()
                        == crate::voxel::VoxelKind::from_native(pumpkin_data::Block::SPAWNER.id)
                });
            if !matches
                || pumpkin_data::entity::EntityType::from_raw(spawner.species.0).is_none()
                || crate::voxel::chunk_at(position.as_vec3()) != chunk
                || !(0..hollow_worldgen::coordinates::CHUNK_HEIGHT).contains(&position.y)
                || spawner.delay > 800
            {
                return Err(io::Error::other("Invalid saved spawner."));
            }
        }
        self.creatures = record.creatures;
        self.spawners = record.spawners;
        if !record.biomes.is_empty() {
            self.biomes = record.biomes.into();
        }
        self.drops = record.drops;
        let records = record.containers;
        let mut seen = vec![false; self.entries.len()];
        for (position, slots) in records {
            let position = IVec3::from_array(position);
            let entry = self
                .entries
                .binary_search_by_key(&(position.y, position.z, position.x), |(p, _, _)| {
                    (p.y, p.z, p.x)
                })
                .map_err(|_| io::Error::other("Container data does not match its block."))?;
            if !profile(self.entries[entry].1.kind()).chest {
                return Err(io::Error::other(
                    "Container data belongs to a non-container block.",
                ));
            }
            if seen[entry] {
                return Err(io::Error::other("Duplicate container record."));
            }
            seen[entry] = true;
            let inventory = &mut self.entries[entry].2;
            for (index, stack) in slots.into_iter().enumerate() {
                if let Some(saved) = stack {
                    inventory.set(index, Some(saved.restore()?));
                }
            }
        }
        Ok(())
    }
}

#[derive(SystemParam)]
pub(crate) struct TopologyUpdates<'w, 's> {
    index: Res<'w, GameplayColumns>,
    blocks: Query<'w, 's, &'static BlockEntities>,
    chests: Query<'w, 's, (), With<Chest>>,
}
impl TopologyUpdates<'_, '_> {
    fn neighbors(&self, position: IVec2, commands: &mut Commands) {
        for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            if let Some(&owner) = self.index.get(&(position + offset))
                && let Ok(blocks) = self.blocks.get(owner)
            {
                for entity in blocks.iter() {
                    if self.chests.contains(entity) {
                        commands.entity(entity).try_insert(TopologyDirty);
                    }
                }
            }
        }
    }
}

pub(crate) fn spawn_for_chunk(
    commands: &mut Commands,
    terrain: Entity,
    position: IVec2,
    prepared: PreparedColumn,
) {
    // Identity creation and bundle factories share one immediate structural boundary.
    commands.queue(move |world: &mut World| {
        let owner = column(world, position);
        if !world
            .get::<GameplayColumn>(owner)
            .expect("Gameplay column")
            .initialized
        {
            world
                .get_mut::<GameplayColumn>(owner)
                .expect("Gameplay column")
                .application = crate::creatures::ColumnActors {
                drops: prepared.drops,
                creatures: prepared.creatures,
            };
            world
                .entity_mut(owner)
                .insert(crate::creatures::spawning::ColumnBiomes(prepared.biomes));
            world.entity_mut(owner).insert(prepared.simulation);
            // A shared clone keeps the resource available to factory-triggered observers.
            let catalog = world
                .resource::<hollow_engine::blocks::BlockCatalog>()
                .clone();
            for (position, state, inventory) in prepared.entries {
                let entity = hollow_engine::blocks::materialized_block(world, owner, position);
                hollow_engine::blocks::classify(world, entity, state, &catalog);
                if profile(state.kind()).chest {
                    world.entity_mut(entity).insert(inventory);
                }
            }
            let layout = world.resource::<hollow_engine::world::WorldLayout>().0;
            for saved in prepared.spawners {
                let cell =
                    hollow_engine::blocks::local_index(IVec3::from_array(saved.position), layout);
                let entity = world
                    .get::<hollow_engine::blocks::ColumnBlocks>(owner)
                    .and_then(|blocks| blocks.cells.get(&cell))
                    .copied()
                    .expect("Discovered spawner block");
                world
                    .entity_mut(entity)
                    .insert(crate::creatures::CreatureSpawner {
                        species: saved.species,
                        delay: saved.delay,
                    });
            }
            let mut data = world
                .get_mut::<GameplayColumn>(owner)
                .expect("Gameplay column");
            data.revision = prepared.revision + 1;
            data.saved_revision = prepared.revision;
            data.initialized = true;
        }
        // The insert observers see fully restored blocks before queued terrain edits.
        world.entity_mut(terrain).insert((
            LoadedColumn(owner),
            hollow_worldgen::runtime::ChunkRetention,
        ));
    });
}

pub(crate) fn loaded(
    event: On<Insert<LoadedColumn>>,
    terrain: Query<(&crate::voxel::VoxelChunk, &LoadedColumn)>,
    topology: TopologyUpdates,
    simulations: Query<&crate::simulation::ColumnSimulation>,
    mut columns: Query<&mut GameplayColumn>,
    catalog: Res<crate::creatures::CreatureCatalog>,
    spawners: Query<&crate::creatures::CreatureSpawner>,
    mut waits: ResMut<crate::simulation::BoundaryWaits>,
    mut light: super::visuals::LightInvalidation,
    mut commands: Commands,
) {
    let Ok((chunk, owner)) = terrain.get(event.entity) else {
        return;
    };
    if let Ok(work) = simulations.get(owner.0) {
        for task in work.suspended.values() {
            waits.register(IVec2::from_array(task.chunk), owner.0);
        }
    }
    let invalid_spawner = topology.blocks.get(owner.0).is_ok_and(|blocks| {
        blocks
            .iter()
            .filter_map(|entity| spawners.get(entity).ok())
            .any(|spawner| catalog.get(spawner.species).is_none())
    });
    if let Ok(mut column) = columns.get_mut(owner.0) {
        if invalid_spawner
            || column
                .application
                .creatures
                .iter()
                .any(|saved| catalog.get(saved.species).is_none())
        {
            commands.trigger(crate::world::LoadError(
                "Saved creature or spawner has no registered profile.".into(),
            ));
            return;
        }
        for drop in column.application.drops.drain(..) {
            crate::dropped_item::restore_drop(&mut commands, drop);
        }
        for saved in std::mem::take(&mut column.application.creatures) {
            crate::creatures::persistence::restore(&mut commands, saved, &catalog, owner.0);
        }
    }
    if let Ok(blocks) = topology.blocks.get(owner.0) {
        for entity in blocks.iter() {
            commands.entity(entity).try_remove::<Dormant>();
            if topology.chests.contains(entity) {
                commands
                    .entity(entity)
                    .try_insert(super::visuals::PrepareView);
            }
        }
    }
    topology.neighbors(chunk.position, &mut commands);
    light.neighbors(chunk.position);
}

pub(crate) fn changed(
    event: On<crate::voxel::edits::BlockChanged>,
    catalog: Res<hollow_engine::blocks::BlockCatalog>,
    lookup: BlockLookup,
    chests: Query<(), With<Chest>>,
    inventories: Query<&ContainerInventory>,
    index: Res<crate::voxel::streaming::ChunkIndex>,
    mut commands: Commands,
) {
    let position = event.position;
    for offset in [
        IVec3::ZERO,
        IVec3::NEG_Y,
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Z,
        IVec3::NEG_Z,
    ] {
        if let Some(entity) = lookup.entity(position + offset)
            && chests.contains(entity)
        {
            commands.entity(entity).try_insert(TopologyDirty);
        }
    }
    let new = event.new;
    let materialized = catalog.materialized(hollow_core::BlockId(new.kind().index() as u32));
    if !materialized && !catalog.materialized(hollow_core::BlockId(event.old.kind().index() as u32))
    {
        return;
    }
    let container_changed = profile(event.old.kind()).chest != profile(new.kind()).chest;
    if container_changed
        && let Some(entity) = lookup.entity(position)
        && let Ok(inventory) = inventories.get(entity)
    {
        use crate::dropped_item::DroppedItemCommandsExt;
        let mut drops = [None; CONTAINER_SIZE];
        for &stack in inventory.iter().flatten() {
            let remaining = crate::inventory::slots::insert(&mut drops, stack, 0..CONTAINER_SIZE);
            debug_assert!(
                remaining.is_none(),
                "Container drops fit their original slot count"
            );
        }
        for stack in drops.into_iter().flatten() {
            commands.spawn_dropped_item(
                stack,
                position.as_vec3() + Vec3::splat(0.5),
                Vec3::Y * 1.2,
            );
        }
    }
    let coordinate = crate::voxel::split_world_position(position).0;
    let loaded = index.contains_key(&coordinate);
    let catalog = catalog.clone();
    commands.queue(move |world: &mut World| {
        let owner = column(world, coordinate);
        let entity = hollow_engine::blocks::materialized_block(world, owner, position);
        if container_changed {
            world
                .get_mut::<GameplayColumn>(owner)
                .expect("Gameplay column")
                .revision += 1;
            world
                .entity_mut(entity)
                .remove::<(Chest, ContainerInventory)>();
        }
        hollow_engine::blocks::classify(world, entity, new, &catalog);
        if loaded {
            world.entity_mut(entity).remove::<Dormant>();
        }
        if !materialized {
            world.despawn(entity);
        }
    });
}

pub(crate) fn unload(
    event: On<Remove<crate::voxel::VoxelChunk>>,
    terrain: Query<(&crate::voxel::VoxelChunk, &LoadedColumn)>,
    topology: TopologyUpdates,
    mut commands: Commands,
) {
    if let Ok((chunk, owner)) = terrain.get(event.entity) {
        topology.neighbors(chunk.position, &mut commands);
        if let Ok(blocks) = topology.blocks.get(owner.0) {
            for entity in blocks.iter() {
                commands.entity(entity).try_insert(Dormant);
            }
        }
    }
}

/// The registered pairing is canonical; physical rules are checked before every mutation.
#[derive(SystemParam)]
pub(crate) struct ContainerTopology<'w, 's> {
    world: VoxelWorld<'w, 's>,
    lookup: BlockLookup<'w, 's>,
    blocks: Query<
        'w,
        's,
        (
            &'static BlockEntity,
            &'static BlockState,
            Option<&'static PairedWith>,
            Option<&'static ChestPartners>,
            Has<AvailableContainer>,
        ),
        (With<Chest>, Without<Dormant>),
    >,
}
impl ContainerTopology<'_, '_> {
    pub fn owners(&self, canonical: Entity, actor: Vec3) -> Option<[Option<Entity>; 2]> {
        let (_, state, pair, partners, available) = self.blocks.get(canonical).ok()?;
        if pair.is_some() || !available {
            return None;
        }
        let owners = [
            Some(canonical),
            partners.and_then(|partners| partners.first().copied()),
        ];
        if state_behavior(state.0).chest_half == 1 && owners[1].is_none() {
            return None;
        }
        owners
            .iter()
            .flatten()
            .all(|&entity| {
                let Ok((block, state, _, _, _)) = self.blocks.get(entity) else {
                    return false;
                };
                block.accessible(&self.world, state.0, actor, 5.0)
                    && top_clear(&self.world, block.position)
            })
            .then_some(owners)
    }
    fn partner(&self, origin: IVec3, state: VoxelState) -> Option<Entity> {
        let position = chest_partner(origin, state)?;
        let entity = self.lookup.entity(position)?;
        let (_, next, _, _, _) = self.blocks.get(entity).ok()?;
        (next.0.kind() == state.kind()
            && chest_partner(position, next.0) == Some(origin)
            && state_behavior(next.0).facing == state_behavior(state).facing)
            .then_some(entity)
    }
    pub fn canonical(&self, entity: Entity) -> Option<Entity> {
        let (_, _, pair, _, _) = self.blocks.get(entity).ok()?;
        Some(pair.map_or(entity, |pair| pair.0))
    }
}

pub(crate) fn update_topology(
    mut commands: Commands,
    topology: ContainerTopology,
    dirty: Query<
        (Entity, &BlockEntity, &BlockState, Option<&PairedWith>),
        (With<Chest>, With<TopologyDirty>),
    >,
    viewers: Query<&StationViewers>,
) {
    for (entity, block, state, previous) in &dirty {
        let partner = topology.partner(block.position, state.0);
        let next = (state_behavior(state.0).chest_half == 2)
            .then_some(partner)
            .flatten();
        if previous.map(|pair| pair.0) != next {
            for owner in [Some(entity), previous.map(|pair| pair.0), next]
                .into_iter()
                .flatten()
            {
                if let Ok(viewers) = viewers.get(owner) {
                    for viewer in viewers.iter() {
                        commands.entity(viewer).remove::<InventorySession>();
                    }
                }
            }
            commands.entity(entity).remove::<PairedWith>();
            if let Some(owner) = next {
                commands.entity(entity).insert(PairedWith(owner));
            }
        }
        let info = state_behavior(state.0);
        let available = topology.blocks.contains(entity)
            && info.chest_half != 2
            && (info.chest_half == 0 || partner.is_some())
            && top_clear(&topology.world, block.position)
            && partner.is_none_or(|partner| {
                topology
                    .blocks
                    .get(partner)
                    .is_ok_and(|(block, _, _, _, _)| top_clear(&topology.world, block.position))
            });
        commands
            .entity(entity)
            .remove::<(TopologyDirty, AvailableContainer)>();
        if available {
            commands.entity(entity).insert(AvailableContainer);
        }
    }
}

pub(crate) fn pairing_removed(
    event: On<Remove<PairedWith>>,
    pairs: Query<&PairedWith>,
    viewers: Query<&StationViewers>,
    mut commands: Commands,
) {
    if let Ok(pair) = pairs.get(event.entity)
        && let Ok(viewers) = viewers.get(pair.0)
    {
        for viewer in viewers.iter() {
            commands.entity(viewer).remove::<InventorySession>();
        }
    }
}

pub(crate) fn track_inventory_changes(
    inventories: Query<&BlockOf, Changed<ContainerInventory>>,
    mut columns: Query<&mut GameplayColumn>,
) {
    for owner in &inventories {
        if let Ok(mut column) = columns.get_mut(owner.0) {
            column.revision += 1;
        }
    }
}
/// Only columns measured dormant, or changed outside change detection since that
/// measurement, can qualify; the rest are not visited.
pub(crate) fn cleanup_dormant(
    mut commands: Commands,
    columns: Query<(&GameplayColumn, Option<&LoadedTerrain>)>,
    pressure: Res<crate::world::StoragePressure>,
    stored: Res<crate::voxel::streaming::StoredChunks>,
    pending: Res<crate::voxel::streaming::PendingChunks>,
    mut index: ResMut<GameplayColumns>,
    mut candidates: Local<Vec<Entity>>,
) {
    candidates.extend(pressure.dormant.iter().chain(&pressure.touched));
    candidates.sort_unstable();
    candidates.dedup();
    for entity in candidates.drain(..) {
        let Ok((data, terrain)) = columns.get(entity) else {
            continue;
        };
        if terrain.is_none_or(|terrain| terrain.is_empty())
            && data.revision == data.saved_revision
            && !stored.contains_key(&data.position)
            && !pending.contains_key(&data.position)
        {
            index.remove(&data.position);
            commands.entity(entity).despawn();
        }
    }
}
pub(crate) struct ColumnSnapshot {
    pub revision: u64,
    containers: Vec<(IVec3, ContainerInventory)>,
    simulation: crate::simulation::SimulationSnapshot,
    drops: Vec<crate::world::SavedDrop>,
    creatures: Vec<crate::creatures::SavedCreature>,
    spawners: Vec<crate::creatures::SavedSpawner>,
    biomes: std::sync::Arc<[u8]>,
}
impl ColumnSnapshot {
    pub fn encode(self) -> io::Result<Vec<u8>> {
        let records = self
            .containers
            .iter()
            .map(|(position, inventory)| {
                (
                    position.to_array(),
                    std::array::from_fn(|index| {
                        inventory.slot(index).map(crate::world::SavedStack::capture)
                    }),
                )
            })
            .collect();
        serde_json::to_vec(&ColumnRecord {
            containers: records,
            simulation: self.simulation,
            drops: self.drops,
            creatures: self.creatures,
            spawners: self.spawners,
            biomes: self.biomes.to_vec(),
        })
        .map_err(io::Error::other)
    }
}
type SnapshotColumn = (
    &'static GameplayColumn,
    Option<&'static BlockEntities>,
    &'static crate::simulation::ColumnSimulation,
    Option<&'static crate::simulation::SimulatedEntities>,
    Option<&'static crate::creatures::spawning::ColumnBiomes>,
);
#[derive(SystemParam)]
pub(crate) struct ColumnSnapshots<'w, 's> {
    columns: Query<'w, 's, SnapshotColumn>,
    drops: Query<
        'w,
        's,
        (
            &'static crate::dropped_item::DroppedItem,
            &'static crate::dropped_item::DropMotion,
            &'static FixedPosition,
        ),
    >,
    inventories: Query<'w, 's, (&'static BlockEntity, &'static ContainerInventory)>,
    creatures: crate::creatures::CreatureSnapshots<'w, 's>,
    index: Res<'w, GameplayColumns>,
}
impl ColumnSnapshots<'_, '_> {
    pub fn candidates(&self) -> Vec<(IVec2, usize, usize)> {
        self.columns
            .iter()
            .filter(|(data, ..)| data.revision != data.saved_revision)
            .map(|(data, blocks, work, mobiles, _)| {
                let containers = blocks
                    .into_iter()
                    .flat_map(|blocks| blocks.iter())
                    .filter_map(|entity| self.inventories.get(entity).ok())
                    .filter(|(_, inventory)| !inventory.is_empty())
                    .count();
                let drops = data.application.drops.len()
                    + mobiles
                        .into_iter()
                        .flat_map(|entities| entities.iter())
                        .filter(|&entity| self.drops.contains(entity))
                        .count();
                // One populated 27-slot inventory reserves 32 KiB, covering slot
                // records plus worst-case textual contents and compression buffers.
                (
                    data.position,
                    work.capture_reservation()
                        + containers * 32768
                        + drops * 2048
                        + data.application.creatures.len() * 8192
                        + mobiles
                            .into_iter()
                            .flat_map(|m| m.iter())
                            .filter(|&e| self.creatures.contains(e))
                            .count()
                            * 8192
                        + blocks.map_or(0, |b| b.len()) * 128
                        + 8192,
                    // Creature records are captured into exact-capacity vectors, so
                    // a large herd is charged once rather than for doubling growth.
                    work.snapshot_bytes()
                        + containers * 2 * std::mem::size_of::<(IVec3, ContainerInventory)>()
                        + drops * 2 * std::mem::size_of::<crate::world::SavedDrop>()
                        + data.application.retained_bytes() * 2
                        + mobiles
                            .into_iter()
                            .flat_map(|m| m.iter())
                            .map(|e| self.creatures.bytes(e))
                            .sum::<usize>()
                        + blocks.map_or(0, |b| b.len()) * 256
                        + 4096,
                )
            })
            .collect()
    }
    /// Resolve the selected column without rescanning every gameplay owner.
    pub fn capture_at(&self, position: IVec2) -> Option<ColumnSnapshot> {
        self.snapshot(self.columns.get(*self.index.get(&position)?).ok()?)
            .map(|(_, snapshot)| snapshot)
    }
    fn snapshot(
        &self,
        (data, blocks, simulation, mobiles, biomes): (
            &GameplayColumn,
            Option<&BlockEntities>,
            &crate::simulation::ColumnSimulation,
            Option<&crate::simulation::SimulatedEntities>,
            Option<&crate::creatures::spawning::ColumnBiomes>,
        ),
    ) -> Option<(IVec2, ColumnSnapshot)> {
        if data.revision == data.saved_revision {
            return None;
        }
        let live = mobiles.map_or(&[][..], |m| m.as_slice());
        let mut creatures = Vec::with_capacity(
            data.application.creatures.len()
                + live.iter().filter(|&&e| self.creatures.contains(e)).count(),
        );
        creatures.extend(data.application.creatures.iter().cloned());
        creatures.extend(
            live.iter()
                .filter_map(|&entity| self.creatures.capture(entity)),
        );
        Some((
            data.position,
            ColumnSnapshot {
                containers: blocks
                    .into_iter()
                    .flat_map(|blocks| blocks.iter())
                    .filter_map(|entity| {
                        self.inventories
                            .get(entity)
                            .ok()
                            .map(|(block, inventory)| (block.position, inventory.clone()))
                    })
                    .collect(),
                creatures,
                spawners: blocks
                    .into_iter()
                    .flat_map(|b| b.iter())
                    .filter_map(|entity| self.creatures.spawner(entity))
                    .collect(),
                biomes: biomes.map(|b| b.0.clone()).unwrap_or_default(),
                revision: data.revision,
                simulation: simulation.capture(),
                drops: data
                    .application
                    .drops
                    .iter()
                    .cloned()
                    .chain(
                        live.iter()
                            .filter_map(|&entity| self.drops.get(entity).ok())
                            .map(|(drop, motion, position)| drop.capture(motion, position)),
                    )
                    .collect(),
            },
        ))
    }
}
