//! ECS owns gameplay data through unloads; transfer records are immutable snapshots.
use super::catalog::{Opening, SupportRule, chest_partner, profile, state_behavior};
use crate::{
    inventory::slots::ItemSlots,
    voxel::{ChunkVoxels, VoxelState, VoxelWorld},
};
use bevy::{
    ecs::{lifecycle::HookContext, system::SystemParam, world::DeferredWorld},
    platform::collections::HashMap,
    prelude::*,
};
use std::io;

pub(crate) const CONTAINER_SIZE: usize = 27;
pub(crate) type ContainerInventory = ItemSlots<CONTAINER_SIZE>;

#[derive(Component)]
#[require(ContainerInventory, super::visuals::InstanceLight)]
pub(crate) struct Chest;
#[derive(Component)]
pub(crate) struct ManualOpen;
#[derive(Component)]
pub(crate) struct Door;
#[derive(Component)]
pub(crate) struct Gate;
#[derive(Component)]
pub(crate) struct WallSupport;
#[derive(Component)]
pub(crate) struct Vine;
#[derive(Component)]
#[component(storage = "SparseSet")]
pub(crate) struct Dormant;
#[derive(Component, Clone, Copy)]
pub(crate) struct BlockState(pub VoxelState);

#[derive(Component, Clone, Copy)]
#[component(immutable, on_add = index_block, on_remove = unindex_block)]
pub(crate) struct BlockEntity {
    pub position: IVec3,
}
#[derive(Component)]
#[relationship(relationship_target = BlockEntities)]
pub(crate) struct BlockOf(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = BlockOf, linked_spawn)]
pub(crate) struct BlockEntities(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = LoadedTerrain)]
pub(crate) struct LoadedColumn(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = LoadedColumn)]
pub(crate) struct LoadedTerrain(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = ContainerViewers)]
pub(crate) struct ViewingContainer(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = ViewingContainer)]
pub(crate) struct ContainerViewers(Vec<Entity>);
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

#[derive(Resource, Default, Deref, DerefMut)]
pub(crate) struct GameplayColumns(HashMap<IVec2, Entity>);
#[derive(Component)]
pub(crate) struct GameplayColumn {
    pub position: IVec2,
    pub cells: HashMap<u32, Entity>,
    pub revision: u64,
    pub saved_revision: u64,
    pub initialized: bool,
}
pub(crate) fn local_index(position: IVec3) -> u32 {
    hollow_worldgen::coordinates::cell_index(crate::voxel::split_world_position(position).1)
        .expect("Block entity position is inside the world") as u32
}
fn index_block(mut world: DeferredWorld, context: HookContext) {
    let block = world
        .get::<BlockEntity>(context.entity)
        .expect("Block address")
        .position;
    let owner = world.get::<BlockOf>(context.entity).expect("Block owner").0;
    world
        .get_mut::<GameplayColumn>(owner)
        .expect("Gameplay column")
        .cells
        .insert(local_index(block), context.entity);
}
fn unindex_block(mut world: DeferredWorld, context: HookContext) {
    let position = world
        .get::<BlockEntity>(context.entity)
        .expect("Block address")
        .position;
    if let Some(owner) = world.get::<BlockOf>(context.entity).map(|owner| owner.0)
        && let Some(mut column) = world.get_mut::<GameplayColumn>(owner)
    {
        column.cells.remove(&local_index(position));
    }
}
pub(crate) fn column(world: &mut World, position: IVec2) -> Entity {
    if let Some(&entity) = world.resource::<GameplayColumns>().get(&position) {
        return entity;
    }
    let entity = world
        .spawn((
            GameplayColumn {
                position,
                cells: default(),
                revision: 0,
                saved_revision: 0,
                initialized: false,
            },
            DespawnOnExit(crate::game::GameState::Game),
        ))
        .id();
    world
        .resource_mut::<GameplayColumns>()
        .insert(position, entity);
    entity
}
pub(crate) fn block(world: &mut World, position: IVec3) -> Entity {
    let owner = column(world, crate::voxel::split_world_position(position).0);
    if let Some(&entity) = world
        .get::<GameplayColumn>(owner)
        .expect("Gameplay column")
        .cells
        .get(&local_index(position))
    {
        return entity;
    }
    world
        .spawn((BlockOf(owner), BlockEntity { position }, Dormant))
        .id()
}
pub(crate) fn find_block(world: &World, position: IVec3) -> Option<Entity> {
    let owner = *world
        .resource::<GameplayColumns>()
        .get(&crate::voxel::split_world_position(position).0)?;
    world
        .get::<GameplayColumn>(owner)?
        .cells
        .get(&local_index(position))
        .copied()
}
#[derive(SystemParam)]
pub(crate) struct BlockLookup<'w, 's> {
    columns: Res<'w, GameplayColumns>,
    data: Query<'w, 's, &'static GameplayColumn>,
}
impl BlockLookup<'_, '_> {
    pub fn entity(&self, position: IVec3) -> Option<Entity> {
        let owner = *self
            .columns
            .get(&crate::voxel::split_world_position(position).0)?;
        self.data
            .get(owner)
            .ok()?
            .cells
            .get(&local_index(position))
            .copied()
    }
}
pub(crate) fn materialized(kind: crate::voxel::VoxelKind) -> bool {
    let p = profile(kind);
    p.manual
        || p.chest
        || kind.0 == pumpkin_data::BlockId::CRAFTING_TABLE
        || p.support != SupportRule::None
}
fn classify(world: &mut World, entity: Entity, state: VoxelState) {
    let p = profile(state.kind());
    let kind_changed = world
        .get::<BlockState>(entity)
        .is_none_or(|previous| previous.0.kind() != state.kind());
    let mut entity = world.entity_mut(entity);
    entity.insert(BlockState(state));
    if kind_changed {
        entity.remove::<(
            ManualOpen,
            Door,
            Gate,
            WallSupport,
            Vine,
            super::workbench::CraftingTable,
        )>();
        if state.kind().0 == pumpkin_data::BlockId::CRAFTING_TABLE {
            entity.insert(super::workbench::CraftingTable);
        }
        if p.manual {
            entity.insert(ManualOpen);
        }
        match p.opening {
            Opening::Door => {
                entity.insert(Door);
            }
            Opening::Gate => {
                entity.insert(Gate);
            }
            _ => {}
        }
        match p.support {
            SupportRule::WallMounted => {
                entity.insert(WallSupport);
            }
            SupportRule::Vine => {
                entity.insert(Vine);
            }
            _ => {}
        }
    }
    if p.chest {
        entity.insert((Chest, TopologyDirty, super::visuals::PrepareView));
    }
}
pub(crate) fn top_clear(world: &VoxelWorld, position: IVec3) -> bool {
    position.y + 1 >= hollow_worldgen::coordinates::CHUNK_HEIGHT
        || world.state(position + IVec3::Y).is_some_and(|state| {
            !state
                .native()
                .is_side_solid(pumpkin_data::BlockDirection::Down)
        })
}

#[derive(Clone, Default)]
pub(crate) struct PreparedColumn {
    pub entries: Vec<(IVec3, VoxelState, ContainerInventory)>,
    pub revision: u64,
}
type ContainerRecord = ([i32; 3], [Option<crate::world::SavedStack>; CONTAINER_SIZE]);
impl PreparedColumn {
    /// Runs only on a chunk-copy worker. Uniform sections are classified once.
    pub fn discover(voxels: &ChunkVoxels, chunk: IVec2) -> Self {
        use hollow_worldgen::coordinates::{CHUNK_SIZE, SECTION_COUNT, SECTION_HEIGHT};
        let mut result = Self::default();
        let origin = IVec3::new(chunk.x * CHUNK_SIZE, 0, chunk.y * CHUNK_SIZE);
        for section in 0..SECTION_COUNT {
            if !voxels
                .states
                .section(section)
                .any(|state| materialized(state.kind()))
            {
                continue;
            }
            for y in section as i32 * SECTION_HEIGHT..(section as i32 + 1) * SECTION_HEIGHT {
                for z in 0..CHUNK_SIZE {
                    for x in 0..CHUNK_SIZE {
                        let local = IVec3::new(x, y, z);
                        if voxels.get(local).is_some_and(|kind| materialized(kind)) {
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

    pub fn encode(&self) -> io::Result<Option<Vec<u8>>> {
        let records: Vec<ContainerRecord> = self
            .entries
            .iter()
            .filter(|(_, _, inventory)| !inventory.is_empty())
            .map(|(position, _, inventory)| {
                (
                    position.to_array(),
                    std::array::from_fn(|index| {
                        inventory.slot(index).map(crate::world::SavedStack::capture)
                    }),
                )
            })
            .collect();
        if records.is_empty() {
            Ok(None)
        } else {
            serde_json::to_vec(&records)
                .map(Some)
                .map_err(io::Error::other)
        }
    }

    pub fn restore(&mut self, bytes: &[u8]) -> io::Result<()> {
        let records: Vec<ContainerRecord> =
            serde_json::from_slice(bytes).map_err(io::Error::other)?;
        self.entries
            .sort_unstable_by_key(|(p, _, _)| (p.y, p.z, p.x));
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

fn refresh_topology(world: &mut World, position: IVec2) {
    for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
        let Some(&owner) = world
            .resource::<GameplayColumns>()
            .get(&(position + offset))
        else {
            continue;
        };
        let blocks: Vec<_> = world
            .get::<BlockEntities>(owner)
            .map(|blocks| blocks.iter().collect())
            .unwrap_or_default();
        for entity in blocks {
            if world.get::<Chest>(entity).is_some() {
                world.entity_mut(entity).insert(TopologyDirty);
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
    commands.queue(move |world: &mut World| {
        let owner = column(world, position);
        let initialized = world
            .get::<GameplayColumn>(owner)
            .expect("Gameplay column")
            .initialized;
        if !initialized {
            for (position, state, inventory) in prepared.entries {
                let entity = block(world, position);
                classify(world, entity, state);
                if profile(state.kind()).chest {
                    world.entity_mut(entity).insert(inventory);
                }
            }
            let mut data = world
                .get_mut::<GameplayColumn>(owner)
                .expect("Gameplay column");
            data.revision = prepared.revision;
            data.saved_revision = prepared.revision;
            data.initialized = true;
        }
        world.entity_mut(terrain).insert(LoadedColumn(owner));
        let entities: Vec<_> = world
            .get::<BlockEntities>(owner)
            .map(|blocks| blocks.iter().collect())
            .unwrap_or_default();
        for entity in entities {
            world.entity_mut(entity).remove::<Dormant>();
            if world.get::<Chest>(entity).is_some() {
                world
                    .entity_mut(entity)
                    .insert((TopologyDirty, super::visuals::PrepareView));
            }
        }
        refresh_topology(world, position);
        super::visuals::invalidate_neighbors(world, position);
    });
}

pub(crate) fn changed(event: On<crate::voxel::edits::BlockChanged>, mut commands: Commands) {
    let position = event.position;
    let new = event.new;
    let old = event.old;
    commands.queue(move |world: &mut World| {
        let coordinate = crate::voxel::split_world_position(position).0;
        if materialized(new.kind()) || materialized(old.kind()) {
            let entity = block(world, position);
            if profile(old.kind()).chest != profile(new.kind()).chest {
                if let Some(inventory) = world.get::<ContainerInventory>(entity).cloned() {
                    use crate::dropped_item::DroppedItemCommandsExt;
                    let mut drops = [None; CONTAINER_SIZE];
                    for &stack in inventory.iter().flatten() {
                        let remaining =
                            crate::inventory::slots::insert(&mut drops, stack, 0..CONTAINER_SIZE);
                        debug_assert!(
                            remaining.is_none(),
                            "Container drops fit their original slot count"
                        );
                    }
                    for stack in drops.into_iter().flatten() {
                        world.commands().spawn_dropped_item(
                            stack,
                            position.as_vec3() + Vec3::splat(0.5),
                            Vec3::Y * 1.2,
                        );
                    }
                }
                world
                    .entity_mut(entity)
                    .remove::<(Chest, ContainerInventory)>();
                let owner = world.get::<BlockOf>(entity).expect("Block owner").0;
                world
                    .get_mut::<GameplayColumn>(owner)
                    .expect("Gameplay column")
                    .revision += 1;
            }
            classify(world, entity, new);
            if world
                .resource::<crate::voxel::streaming::ChunkIndex>()
                .contains_key(&coordinate)
            {
                world.entity_mut(entity).remove::<Dormant>();
            }
            if !materialized(new.kind())
                && world.get::<super::supports::SupportCheck>(entity).is_none()
            {
                world.despawn(entity);
            }
        }
        for offset in [
            IVec3::ZERO,
            IVec3::NEG_Y,
            IVec3::X,
            IVec3::NEG_X,
            IVec3::Z,
            IVec3::NEG_Z,
        ] {
            let p = position + offset;
            if let Some(entity) = find_block(world, p)
                && world.get::<Chest>(entity).is_some()
            {
                world.entity_mut(entity).insert(TopologyDirty);
            }
        }
    });
}

pub(crate) fn unload(
    event: On<Remove<crate::voxel::VoxelChunk>>,
    terrain: Query<&LoadedColumn>,
    blocks: Query<&BlockEntities>,
    columns: Query<&GameplayColumn>,
    mut commands: Commands,
) {
    if let Ok(owner) = terrain.get(event.entity) {
        if let Ok(data) = columns.get(owner.0) {
            let position = data.position;
            commands.queue(move |world: &mut World| refresh_topology(world, position));
        }
        if let Ok(blocks) = blocks.get(owner.0) {
            for entity in blocks.iter() {
                commands.entity(entity).try_insert((Dormant, TopologyDirty));
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
    pub fn entity(&self, position: IVec3) -> Option<Entity> {
        let entity = self.lookup.entity(position)?;
        self.blocks.contains(entity).then_some(entity)
    }
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
                actor.distance_squared(block.position.as_vec3() + Vec3::splat(0.5)) <= 25.0
                    && self.world.state(block.position) == Some(state.0)
                    && top_clear(&self.world, block.position)
            })
            .then_some(owners)
    }
    fn partner(&self, origin: IVec3, state: VoxelState) -> Option<Entity> {
        let position = chest_partner(origin, state)?;
        let entity = self.entity(position)?;
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
    viewers: Query<&ContainerViewers>,
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
                        commands.entity(viewer).remove::<ViewingContainer>();
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
    viewers: Query<&ContainerViewers>,
    mut commands: Commands,
) {
    if let Ok(pair) = pairs.get(event.entity)
        && let Ok(viewers) = viewers.get(pair.0)
    {
        for viewer in viewers.iter() {
            commands.entity(viewer).remove::<ViewingContainer>();
        }
    }
}

pub(crate) fn validate_sessions(
    mut commands: Commands,
    sessions: Query<(
        Entity,
        &ViewingContainer,
        &crate::character::CharacterMotion,
    )>,
    topology: ContainerTopology,
) {
    for (entity, viewing, motion) in &sessions {
        if topology
            .owners(viewing.0, motion.position.as_vec3())
            .is_none()
        {
            commands.entity(entity).remove::<ViewingContainer>();
        }
    }
}
pub(crate) fn viewing_closed(
    _: On<Remove<ViewingContainer>>,
    next: Option<ResMut<NextState<crate::inventory::InventoryState>>>,
    mut actions: ResMut<crate::inventory::access::InventoryActions>,
) {
    if let Some(mut next) = next {
        next.set(crate::inventory::InventoryState::Closed);
    }
    actions.clear();
}
pub(crate) fn close_sessions(
    mut commands: Commands,
    sessions: Query<Entity, With<ViewingContainer>>,
) {
    for entity in &sessions {
        commands.entity(entity).remove::<ViewingContainer>();
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
pub(crate) fn cleanup_dormant(
    mut commands: Commands,
    columns: Query<(
        Entity,
        &GameplayColumn,
        Option<&LoadedTerrain>,
        Option<&BlockEntities>,
    )>,
    work: Query<(), With<super::supports::SupportCheck>>,
    stored: Res<crate::voxel::streaming::StoredChunks>,
    pending: Res<crate::voxel::streaming::PendingChunks>,
    mut index: ResMut<GameplayColumns>,
) {
    for (entity, data, terrain, blocks) in &columns {
        if terrain.is_none_or(|terrain| terrain.is_empty())
            && data.revision == data.saved_revision
            && !stored.contains_key(&data.position)
            && !pending.contains_key(&data.position)
            && blocks.is_none_or(|blocks| blocks.iter().all(|entity| !work.contains(entity)))
        {
            index.remove(&data.position);
            commands.entity(entity).despawn();
        }
    }
}
#[derive(SystemParam)]
pub(crate) struct ContainerSnapshots<'w, 's> {
    columns: Query<'w, 's, (&'static GameplayColumn, Option<&'static BlockEntities>)>,
    inventories: Query<
        'w,
        's,
        (
            &'static BlockEntity,
            &'static BlockState,
            &'static ContainerInventory,
        ),
    >,
}
impl ContainerSnapshots<'_, '_> {
    pub fn capture(&self) -> Vec<(IVec2, PreparedColumn)> {
        self.columns
            .iter()
            .filter(|(data, _)| data.revision != data.saved_revision)
            .map(|(data, blocks)| {
                (
                    data.position,
                    PreparedColumn {
                        entries: blocks
                            .into_iter()
                            .flat_map(|blocks| blocks.iter())
                            .filter_map(|entity| {
                                self.inventories.get(entity).ok().map(
                                    |(block, state, inventory)| {
                                        (block.position, state.0, inventory.clone())
                                    },
                                )
                            })
                            .collect(),
                        revision: data.revision,
                    },
                )
            })
            .collect()
    }
}
