//! Block factories are resolved once; runtime topology only indexes numeric IDs.
use crate::world::VoxelWorld;
use bevy_app::{App, Plugin, PreStartup};
use bevy_derive::Deref;
use bevy_ecs::{lifecycle::HookContext, prelude::*, world::DeferredWorld};
use bevy_math::{IVec2, IVec3, Vec3};
use hollow_core::VoxelCell;
use hollow_core::{BlockId, StateId};
use std::sync::Arc;

/// Factories run on structural changes, never from rendering or meshing loops.
#[derive(Clone, Copy)]
pub struct BlockFactory {
    pub attach: fn(&mut EntityWorldMut, StateId),
    pub detach: fn(&mut EntityWorldMut),
    pub interactive: bool,
    pub refresh_on_state_change: bool,
}

// Per-block vectors exist only while plugins register unknown bundle counts.
// Finalization drops them and retains a single contiguous callback allocation.
#[derive(Resource)]
pub struct BlockRegistrations(Option<Vec<Vec<BlockFactory>>>);
impl Default for BlockRegistrations {
    fn default() -> Self {
        Self(Some(Vec::new()))
    }
}
impl BlockRegistrations {
    /// Compose ordinary components without writing structural callback plumbing.
    /// Multiple interactive capabilities can observe the same block use.
    pub fn register_bundle<B: Bundle + Default>(&mut self, block: BlockId, interactive: bool) {
        self.register(
            block,
            BlockFactory {
                attach: |entity, _| {
                    entity.insert(B::default());
                },
                detach: |entity| {
                    entity.remove::<B>();
                },
                interactive,
                refresh_on_state_change: false,
            },
        );
    }
    pub fn register(&mut self, block: BlockId, factory: BlockFactory) {
        let entries = self
            .0
            .as_mut()
            .expect("Block registrations are already frozen");
        entries.resize_with((block.index() + 1).max(entries.len()), Vec::new);
        entries[block.index()].push(factory);
    }
}

#[derive(Clone, Copy, Default)]
struct Entry {
    start: u32,
    end: u32,
    interactive: bool,
}

/// Dense block-domain index and one contiguous list of structural callbacks.
struct CatalogData {
    entries: Box<[Entry]>,
    factories: Box<[BlockFactory]>,
}
/// An immutable catalog is shared with chunk-discovery tasks; cells hold no Arc.
#[derive(Resource, Clone)]
pub struct BlockCatalog(Arc<CatalogData>);
impl BlockCatalog {
    fn entry(&self, block: BlockId) -> Entry {
        self.0
            .entries
            .get(block.index())
            .copied()
            .unwrap_or_default()
    }
    pub fn materialized(&self, block: BlockId) -> bool {
        let entry = self.entry(block);
        entry.start != entry.end
    }
    pub fn interactive(&self, block: BlockId) -> bool {
        self.entry(block).interactive
    }
    pub fn attach(
        &self,
        block: BlockId,
        entity: &mut EntityWorldMut,
        state: StateId,
        kind_changed: bool,
    ) {
        let entry = self.entry(block);
        for factory in &self.0.factories[entry.start as usize..entry.end as usize] {
            if kind_changed || factory.refresh_on_state_change {
                (factory.attach)(entity, state);
            }
        }
    }
    pub fn detach(&self, block: BlockId, entity: &mut EntityWorldMut) {
        let entry = self.entry(block);
        for factory in &self.0.factories[entry.start as usize..entry.end as usize] {
            (factory.detach)(entity);
        }
    }
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FinalizeBlockContent;

pub struct BlockRegistryPlugin;
impl Plugin for BlockRegistryPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ColumnOwners>()
            .add_observer(reset_columns)
            .init_resource::<BlockRegistrations>()
            .add_systems(PreStartup, freeze.in_set(FinalizeBlockContent));
    }
}
fn freeze(mut commands: Commands, mut registrations: ResMut<BlockRegistrations>) {
    let registrations = registrations
        .0
        .take()
        .expect("Block registrations already finalized");
    let mut factories = Vec::with_capacity(registrations.iter().map(Vec::len).sum());
    let entries = registrations
        .into_iter()
        .map(|block| {
            let start = factories.len();
            let interactive = block.iter().any(|factory| factory.interactive);
            factories.extend(block);
            Entry {
                start: u32::try_from(start).expect("Too many block factories"),
                end: u32::try_from(factories.len()).expect("Too many block factories"),
                interactive,
            }
        })
        .collect();
    commands.insert_resource(BlockCatalog(Arc::new(CatalogData {
        entries,
        factories: factories.into(),
    })));
}

#[derive(Component)]
#[component(storage = "SparseSet")]
pub struct Dormant;
#[derive(Component, Clone, Copy)]
pub struct BlockState<T: VoxelCell>(pub T);

#[derive(Component, Clone, Copy)]
#[component(immutable, on_add = index_block, on_remove = unindex_block)]
pub struct BlockEntity {
    pub position: IVec3,
}
impl BlockEntity {
    /// Loaded identity and reach are the common precondition for block interactions.
    pub fn accessible<T: VoxelCell>(
        &self,
        world: &VoxelWorld<T>,
        state: T,
        actor: Vec3,
        reach: f32,
    ) -> bool {
        world.state(self.position) == Some(state)
            && actor.distance_squared(self.position.as_vec3() + Vec3::splat(0.5)) <= reach * reach
    }
}

#[derive(Component)]
#[relationship(relationship_target = BlockEntities)]
pub struct BlockOf(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = BlockOf, linked_spawn)]
pub struct BlockEntities(Vec<Entity>);

#[derive(Component)]
#[relationship(relationship_target = LoadedTerrain)]
pub struct LoadedColumn(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = LoadedColumn)]
pub struct LoadedTerrain(Vec<Entity>);

/// Only stateful cells have entities; a direct table would reserve a slot for every voxel.
#[derive(Component, Default)]
pub struct ColumnBlocks {
    pub cells: bevy_platform::collections::HashMap<u32, Entity>,
}
/// Durable owners can outlive loaded terrain while saves drain, so this sparse index
/// is separate from ChunkIndex and shares ownership through LoadedColumn.
#[derive(Resource, Default, bevy_derive::Deref, bevy_derive::DerefMut)]
pub struct ColumnOwners(pub bevy_platform::collections::HashMap<IVec2, Entity>);

pub fn local_index(position: IVec3, layout: hollow_core::coordinates::ColumnLayout) -> u32 {
    hollow_core::coordinates::column_cell_index(layout.split(position).1, layout.section_count())
        .expect("Block position inside world") as u32
}
fn index_block(mut world: DeferredWorld, context: HookContext) {
    let block = world
        .get::<BlockEntity>(context.entity)
        .expect("Block address")
        .position;
    let owner = world.get::<BlockOf>(context.entity).expect("Block owner").0;
    let index = local_index(block, world.resource::<crate::world::WorldLayout>().0);
    world
        .get_mut::<ColumnBlocks>(owner)
        .expect("Gameplay column")
        .cells
        .insert(index, context.entity);
}
fn unindex_block(mut world: DeferredWorld, context: HookContext) {
    let position = world
        .get::<BlockEntity>(context.entity)
        .expect("Block address")
        .position;
    let index = local_index(position, world.resource::<crate::world::WorldLayout>().0);
    if let Some(owner) = world.get::<BlockOf>(context.entity).map(|owner| owner.0)
        && let Some(mut column) = world.get_mut::<ColumnBlocks>(owner)
    {
        column.cells.remove(&index);
    }
}

pub fn column(world: &mut World, position: IVec2) -> Entity {
    if let Some(&entity) = world.resource::<ColumnOwners>().get(&position) {
        return entity;
    }
    let session = world.resource::<crate::session::ActiveSession>().0;
    let entity = world
        .spawn((
            ColumnBlocks::default(),
            crate::session::SessionMember(session),
        ))
        .id();
    world
        .resource_mut::<ColumnOwners>()
        .insert(position, entity);
    entity
}
pub fn materialized_block(world: &mut World, owner: Entity, position: IVec3) -> Entity {
    let layout = world.resource::<crate::world::WorldLayout>().0;
    if let Some(&entity) = world
        .get::<ColumnBlocks>(owner)
        .expect("Column block index")
        .cells
        .get(&local_index(position, layout))
    {
        return entity;
    }
    world
        .spawn((BlockOf(owner), BlockEntity { position }, Dormant))
        .id()
}
#[derive(bevy_ecs::system::SystemParam)]
pub struct BlockLookup<'w, 's> {
    layout: Res<'w, crate::world::WorldLayout>,
    columns: Res<'w, ColumnOwners>,
    data: Query<'w, 's, &'static ColumnBlocks>,
}
impl BlockLookup<'_, '_> {
    pub fn entity(&self, position: IVec3) -> Option<Entity> {
        if !self.layout.contains(position.y) {
            return None;
        }
        let (column, _) = self.layout.split(position);
        self.data
            .get(*self.columns.get(&column)?)
            .ok()?
            .cells
            .get(&local_index(position, self.layout.0))
            .copied()
    }
}
pub fn classify<T: VoxelCell>(world: &mut World, entity: Entity, state: T, catalog: &BlockCatalog) {
    let content = world.resource::<crate::content::RegisteredContent>();
    let kind = BlockId(content.state(StateId(state.id())).block.0);
    let previous = world
        .get::<BlockState<T>>(entity)
        .map(|previous| content.state(StateId(previous.0.id())).block);
    let kind_changed = previous != Some(kind);
    let mut entity = world.entity_mut(entity);
    if kind_changed && let Some(previous) = previous {
        catalog.detach(previous, &mut entity);
    }
    entity.insert(BlockState(state));
    catalog.attach(kind, &mut entity, StateId(state.id()), kind_changed);
}

fn reset_columns(_: On<crate::session::SessionClosed>, mut owners: ResMut<ColumnOwners>) {
    owners.clear();
}
