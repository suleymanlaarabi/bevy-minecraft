//! Bounded snapshot admission and streamed encoding for transactional column saves.
use super::{
    compress_record,
    sqlite::{EncodedColumn, PreparedCheckpoint, SqliteStore},
};
use crate::{
    chunk::{ChunkVoxels, VoxelChunk},
    edits::StoredChunks,
    tasks::{VoxelTaskBudget, WorkClass},
    world::ChunkIndex,
};
use bevy_ecs::entity::EntityHashSet;
use bevy_ecs::{prelude::*, system::SystemParam};
use bevy_math::IVec2;
use hollow_core::VoxelCell;
use std::{
    collections::{HashSet, VecDeque},
    io,
};

pub trait ApplicationSnapshot: Send + 'static {
    fn revision(&self) -> u64;
    fn encode(self) -> io::Result<Vec<u8>>;
}
pub trait ApplicationSnapshots {
    type Snapshot: ApplicationSnapshot;
    fn dirty_positions(&self) -> impl Iterator<Item = IVec2> + '_;
    /// Encoding reserve and retained snapshot bytes, priced before cloning.
    fn reservation(&self, position: IVec2) -> Option<(usize, usize)>;
    fn capture_at(&self, position: IVec2) -> Option<Self::Snapshot>;
}
#[derive(SystemParam)]
pub struct TerrainSnapshots<'w, 's, T: VoxelCell> {
    chunks: Query<'w, 's, (&'static VoxelChunk, &'static ChunkVoxels<T>)>,
    index: Res<'w, ChunkIndex>,
    stored: Res<'w, StoredChunks<T>>,
}
impl<T: VoxelCell> TerrainSnapshots<'_, '_, T> {
    fn at(&self, position: &IVec2) -> Option<&ChunkVoxels<T>> {
        self.index
            .get(position)
            .and_then(|&entity| self.chunks.get(entity).ok())
            .map(|(_, voxels)| voxels)
            .filter(|voxels| voxels.save_revision != voxels.saved_revision)
            .or_else(|| {
                self.stored
                    .get(position)
                    .filter(|voxels| voxels.save_revision != voxels.saved_revision)
            })
    }
}
pub struct CaptureLimits {
    pub columns: usize,
    pub bytes: usize,
    pub encoding_batch_bytes: usize,
}
#[derive(Default)]
pub struct CaptureQueue(VecDeque<IVec2>);
impl CaptureQueue {
    pub fn clear(&mut self) {
        self.0.clear();
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
    pub fn capture<T: VoxelCell, A: ApplicationSnapshots>(
        &mut self,
        terrain: &TerrainSnapshots<T>,
        application: &A,
        limits: CaptureLimits,
        rebuild: bool,
    ) -> io::Result<CapturedColumns<T, A::Snapshot>> {
        if rebuild {
            self.clear();
        }
        if self.is_empty() {
            // Loaded terrain can also be retained in StoredChunks; queue it once.
            let mut dirty: HashSet<IVec2> = application.dirty_positions().collect();
            dirty.extend(
                terrain
                    .stored
                    .iter()
                    .map(|(p, v)| (*p, v))
                    .chain(terrain.chunks.iter().map(|(p, v)| (p.position, v)))
                    .filter(|(_, voxels)| voxels.save_revision != voxels.saved_revision)
                    .map(|(position, _)| position),
            );
            self.0.extend(dirty);
        }
        let mut result = CapturedColumns {
            columns: Vec::new(),
            retained_bytes: 0,
            encoding_batch_bytes: limits.encoding_batch_bytes,
        };
        let mut snapshot_bytes = 0usize;
        let mut codec_bytes = limits.encoding_batch_bytes;
        while let Some(&position) = self.0.front() {
            let voxels = terrain.at(&position);
            let application_cost = application.reservation(position);
            if voxels.is_none() && application_cost.is_none() {
                self.0.pop_front();
                continue;
            }
            let (mut codec, mut snapshot) = application_cost.unwrap_or_default();
            if let Some(voxels) = voxels {
                codec += voxels.retained_bytes() * 8 + 4096;
                snapshot += std::mem::size_of::<ChunkVoxels<T>>();
            }
            let next_snapshot_bytes = snapshot_bytes.saturating_add(snapshot);
            let next_codec_bytes = codec_bytes.max(codec);
            let next_bytes = next_snapshot_bytes.saturating_add(next_codec_bytes);
            if !result.is_empty()
                && (result.columns.len() >= limits.columns || next_bytes > limits.bytes)
            {
                break;
            }
            if next_bytes > limits.bytes {
                return Err(io::Error::other(
                    "A world column exceeds the bounded save capture budget.",
                ));
            }
            self.0.pop_front();
            snapshot_bytes = next_snapshot_bytes;
            codec_bytes = next_codec_bytes;
            result.columns.push((
                position,
                (
                    voxels.map(ChunkVoxels::save_snapshot),
                    application.capture_at(position),
                    codec,
                ),
            ));
        }
        result.retained_bytes = snapshot_bytes.saturating_add(codec_bytes);
        Ok(result)
    }
}
type CapturedColumn<T, A> = (IVec2, (Option<ChunkVoxels<T>>, Option<A>, usize));
pub struct CapturedColumns<T: VoxelCell, A: ApplicationSnapshot> {
    columns: Vec<CapturedColumn<T, A>>,
    pub retained_bytes: usize,
    encoding_batch_bytes: usize,
}
pub struct SaveRevisions {
    pub terrain: Vec<(IVec2, u64)>,
    pub application: Vec<(IVec2, u64)>,
}
impl<T: VoxelCell, A: ApplicationSnapshot> CapturedColumns<T, A> {
    pub fn is_empty(&self) -> bool {
        self.columns.is_empty()
    }
    /// Call from an I/O worker after flushing the terrain provider. SQL remains
    /// on that worker; only bounded encoding batches enter the shared CPU budget.
    pub async fn commit<R: Send + 'static>(
        self,
        database: &SqliteStore,
        budget: VoxelTaskBudget,
        class: WorkClass,
        prepare: impl FnOnce(Vec<((i32, i32), bool, bool)>) -> io::Result<(PreparedCheckpoint, R)>
        + Send
        + 'static,
    ) -> io::Result<(SaveRevisions, R)> {
        let revisions = SaveRevisions {
            terrain: self
                .columns
                .iter()
                .filter_map(|(p, (v, _, _))| v.as_ref().map(|v| (*p, v.save_revision)))
                .collect(),
            application: self
                .columns
                .iter()
                .filter_map(|(p, (_, a, _))| a.as_ref().map(|a| (*p, a.revision())))
                .collect(),
        };
        let positions = self
            .columns
            .iter()
            .map(|(p, (v, a, _))| ((p.x, p.y), v.is_some(), a.is_some()))
            .collect();
        let (prepared, records) = budget
            .spawn(class, 0, async move { prepare(positions) })
            .await?;
        let mut captures = self.columns.into_iter().peekable();
        let batches = std::iter::from_fn(move || {
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            while let Some((_, (_, _, cost))) = captures.peek() {
                if !batch.is_empty() && bytes.saturating_add(*cost) > self.encoding_batch_bytes {
                    break;
                }
                bytes += *cost;
                batch.push(captures.next().expect("Peeked column"));
            }
            if batch.is_empty() {
                return None;
            }
            Some(bevy_tasks::block_on(budget.spawn(class, 0, async move {
                batch
                    .into_iter()
                    .map(|(position, (terrain, application, _))| {
                        let terrain = terrain.map(|v| compress_record(&v.encode_saved()));
                        let application = application
                            .map(|snapshot| {
                                let bytes = snapshot.encode()?;
                                if bytes.len() > super::MAX_RECORD_BYTES {
                                    return Err(io::Error::other(
                                        "Application snapshot exceeds the record size limit.",
                                    ));
                                }
                                Ok(compress_record(&bytes))
                            })
                            .transpose()?;
                        Ok(EncodedColumn {
                            position: (position.x, position.y),
                            terrain,
                            application,
                        })
                    })
                    .collect::<io::Result<Vec<_>>>()
            })))
        });
        database.checkpoint(prepared, batches)?;
        Ok((revisions, records))
    }
}
#[derive(SystemParam)]
pub struct TerrainAcknowledgements<'w, 's, T: VoxelCell> {
    chunks: Query<'w, 's, &'static mut ChunkVoxels<T>>,
    index: Res<'w, ChunkIndex>,
    stored: ResMut<'w, StoredChunks<T>>,
}
impl<T: VoxelCell> TerrainAcknowledgements<'_, '_, T> {
    pub fn acknowledge(&mut self, revisions: Vec<(IVec2, u64)>) {
        for (position, revision) in revisions {
            if let Some(&entity) = self.index.get(&position)
                && let Ok(mut voxels) = self.chunks.get_mut(entity)
                && revision <= voxels.save_revision
            {
                voxels.bypass_change_detection().saved_revision = revision;
            }
            if self
                .stored
                .get(&position)
                .is_some_and(|voxels| revision == voxels.save_revision)
            {
                self.stored.remove(&position);
            }
        }
    }
}

/// One gameplay column's share of save pressure, re-measured only when its inputs change.
#[derive(Component, Default, Clone, Copy)]
pub struct ColumnPressure {
    pub ready: usize,
    pub suspended: usize,
    pub retired_bytes: usize,
    pub oversized: bool,
    /// Live mobiles are charged from creature state outside the column's change ticks.
    pub mobiles: bool,
    /// Retired and saved: despawned once no stored terrain or load keeps it alive.
    pub dormant: bool,
}

/// Exact sums of every live [`ColumnPressure`], so per-frame cost follows changes
/// rather than the number of retained columns.
#[derive(Resource, Default)]
pub struct StoragePressure {
    pub ready: usize,
    pub suspended: usize,
    pub retired_bytes: usize,
    pub oversized: usize,
    pub stream_revision: u64,
    /// Re-measured every frame while they charge live mobiles.
    pub mobiles: EntityHashSet,
    pub dormant: EntityHashSet,
    /// Columns changed outside change detection since the last measurement.
    pub touched: Vec<Entity>,
}
impl StoragePressure {
    pub fn set(&mut self, entity: Entity, current: &mut ColumnPressure, next: ColumnPressure) {
        self.ready = self.ready - current.ready + next.ready;
        self.suspended = self.suspended - current.suspended + next.suspended;
        self.retired_bytes = self.retired_bytes - current.retired_bytes + next.retired_bytes;
        self.oversized =
            self.oversized - usize::from(current.oversized) + usize::from(next.oversized);
        for (set, was, is) in [
            (&mut self.mobiles, current.mobiles, next.mobiles),
            (&mut self.dormant, current.dormant, next.dormant),
        ] {
            if was != is {
                if is {
                    set.insert(entity);
                } else {
                    set.remove(&entity);
                }
            }
        }
        *current = next;
    }
}

pub fn forget_pressure(
    event: On<Remove<ColumnPressure>>,
    columns: Query<&ColumnPressure>,
    mut pressure: ResMut<StoragePressure>,
) {
    if let Ok(&current) = columns.get(event.entity) {
        pressure.set(event.entity, &mut { current }, ColumnPressure::default());
    }
}
