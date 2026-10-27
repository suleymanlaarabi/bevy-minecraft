//! Block factories are resolved once; runtime topology only indexes numeric IDs.
use bevy_app::{App, Plugin, PreStartup};
use bevy_ecs::prelude::*;
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
        assert!(
            !factory.interactive || !entries[block.index()].iter().any(|entry| entry.interactive),
            "A block can only register one interaction owner"
        );
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
        app.init_resource::<BlockRegistrations>()
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
