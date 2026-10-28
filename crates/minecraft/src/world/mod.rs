//! Local world lifecycle. Session entities survive until their final commit succeeds.
mod model;
pub(crate) mod storage;
mod ui;

use crate::{
    game::GameState,
    inventory::{InventoryCursor, InventoryState, PlayerInventory},
    player::{Player, PlayerCamera},
    voxel::{
        ChunkVoxels, VoxelChunk, VoxelSettings,
        generation::WorldGenerator,
        streaming::{PendingEdits, StoredChunks},
    },
};
use bevy::{
    prelude::*,
    settings::SaveSettingsSync,
    tasks::{IoTaskPool, Task, futures::check_ready},
    window::WindowCloseRequested,
};
pub(crate) use model::{Checkpoint, SavedDrop, SavedPlayer, SavedStack};
pub use model::{WorldMetadata, WorldQuality};
use std::{
    collections::{HashMap, VecDeque},
    io,
    path::PathBuf,
    sync::Arc,
};
use storage::{CatalogEntry, Database};

#[derive(SubStates, Clone, Copy, Debug, Default, Eq, PartialEq, Hash)]
#[source(GameState = GameState::Game)]
pub enum WorldSessionPhase {
    #[default]
    Loading,
    Playing,
    Saving,
    SaveError,
}

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PrepareSession;

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct WorldSaveCapture;

pub(crate) fn playing(phase: Option<Res<State<WorldSessionPhase>>>) -> bool {
    phase.is_some_and(|p| *p.get() == WorldSessionPhase::Playing)
}
pub(crate) fn streaming(phase: Option<Res<State<WorldSessionPhase>>>) -> bool {
    phase.is_some_and(|p| {
        matches!(
            p.get(),
            WorldSessionPhase::Loading | WorldSessionPhase::Playing
        )
    })
}

#[derive(Resource)]
pub(crate) struct WorldSession {
    pub(crate) metadata: WorldMetadata,
    pub(crate) database: Arc<Database>,
    pub(crate) resume: Checkpoint,
    path: PathBuf,
    last_checkpoint: Option<Checkpoint>,
}

#[derive(Default)]
enum Destination {
    #[default]
    Worlds,
    Exit,
}

#[derive(Resource, Default)]
pub(crate) struct SaveState {
    in_flight_bytes: usize,
    remaining: VecDeque<IVec2>,
    task: Option<Task<io::Result<SaveAck>>>,
    elapsed: f32,
    destination: Destination,
    pub(super) error: Option<String>,
    /// Freeze and save before leaving; direct GameState transitions would lose entities.
    pub(crate) exit_requested: bool,
    /// Autosave on the next frame, even when nothing seems to have changed.
    force: bool,
    // Give inventory OnExit and deferred commands one complete frame to settle.
    settle_frames: u8,
}
struct SaveAck {
    revisions: Vec<(IVec2, u64)>,
    gameplay_revisions: Vec<(IVec2, u64)>,
    checkpoint: Checkpoint,
    metadata: WorldMetadata,
    final_save: bool,
}

#[derive(Resource, Default)]
struct Browser {
    entries: Vec<CatalogEntry>,
    catalog_task: Option<Task<io::Result<Vec<CatalogEntry>>>>,
    open_task: Option<Task<io::Result<OpenedWorld>>>,
    delete_task: Option<Task<io::Result<()>>>,
    pending_delete: Option<PathBuf>,
    error: Option<String>,
    quality: WorldQuality,
    close_after_operation: bool,
    reopen: Option<PathBuf>,
}
impl Browser {
    fn busy(&self) -> bool {
        self.open_task.is_some() || self.delete_task.is_some()
    }
    /// No task runs and no deletion awaits confirmation: a new operation may start.
    fn idle(&self) -> bool {
        !self.busy() && self.pending_delete.is_none()
    }
    fn rescan(&mut self) {
        self.entries.clear();
        self.catalog_task = Some(IoTaskPool::get().spawn(async { storage::catalog() }));
    }
}
struct OpenedWorld {
    session: WorldSession,
    generator: WorldGenerator,
}

#[derive(Event)]
struct RequestExit;

pub(crate) fn request_exit(commands: &mut Commands) {
    commands.trigger(RequestExit);
}

pub(crate) fn spawn_save_indicator(commands: &mut Commands) {
    ui::spawn_indicator(commands);
}

#[derive(Event)]
pub(crate) struct LoadError(pub String);

fn load_error(
    mut event: On<LoadError>,
    mut save: ResMut<SaveState>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
) {
    save.error = Some(std::mem::take(&mut event.0));
    next.set(WorldSessionPhase::SaveError);
}

pub struct WorldPlugin;
impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<WorldSessionPhase>()
            .init_resource::<Browser>()
            .init_resource::<SaveState>()
            .add_observer(exit_application)
            .add_observer(load_error)
            .add_observer(ui::focus_new_control)
            .add_systems(
                OnEnter(GameState::Game),
                prepare_session.in_set(PrepareSession),
            )
            .add_systems(
                OnEnter(GameState::WorldSelect),
                (start_catalog, ui::world_list).chain(),
            )
            .add_systems(OnEnter(GameState::WorldCreate), ui::create_page)
            .add_systems(OnEnter(WorldSessionPhase::Saving), begin_saving)
            .add_systems(OnEnter(WorldSessionPhase::SaveError), ui::save_error)
            .add_systems(OnEnter(WorldSessionPhase::Playing), resume_session)
            .add_systems(OnExit(GameState::Game), end_session)
            .add_systems(
                Update,
                (poll_browser, ui::refresh_status)
                    .chain()
                    .run_if(not(in_state(GameState::Game))),
            )
            .add_systems(
                Update,
                ui::validate_form.run_if(in_state(GameState::WorldCreate)),
            )
            .add_systems(Update, handle_window_close)
            .add_systems(Update, ui::navigate_back)
            .add_systems(
                PreUpdate,
                poll_storage_errors.run_if(in_state(GameState::Game)),
            )
            .add_systems(
                Last,
                (
                    settle_exit,
                    crate::block_behavior::containers::track_inventory_changes,
                    storage_pressure,
                    checkpoint.in_set(WorldSaveCapture),
                    poll_save,
                    crate::block_behavior::containers::cleanup_dormant,
                    hold_save_error,
                    ui::refresh_save_status,
                )
                    .chain()
                    .run_if(in_state(GameState::Game)),
            );
    }
}

fn start_catalog(mut browser: ResMut<Browser>, blocks: Res<hollow_engine::blocks::BlockCatalog>) {
    browser.error = None;
    browser.pending_delete = None;
    browser.rescan();
    if let Some(path) = browser.reopen.take() {
        open_world(&mut browser, Some(path), None, blocks.clone());
    }
}

fn open_world(
    browser: &mut Browser,
    path: Option<PathBuf>,
    creation: Option<(String, i64, WorldQuality)>,
    blocks: hollow_engine::blocks::BlockCatalog,
) {
    if !browser.idle() {
        return;
    }
    browser.error = None;
    browser.open_task = Some(IoTaskPool::get().spawn(async move {
        let source_path = path.clone();
        let (database, metadata, resume) = if let Some(path) = path {
            Database::open(&path)?
        } else {
            let (name, seed, quality) =
                creation.ok_or_else(|| io::Error::other("Invalid world creation request."))?;
            let metadata = WorldMetadata::new(name, seed, quality);
            (
                Database::create(&metadata)?,
                metadata,
                Checkpoint::default(),
            )
        };
        let generator =
            WorldGenerator::persistent(metadata.seed, metadata.quality, database.clone(), blocks);
        let path = source_path.map_or_else(
            || storage::root().map(|root| root.join(&metadata.id).join("world.sqlite3")),
            Ok,
        )?;
        Ok(OpenedWorld {
            session: WorldSession {
                metadata,
                database,
                last_checkpoint: Some(resume.clone()),
                resume,
                path,
            },
            generator,
        })
    }));
}

#[cfg(feature = "dev")]
pub(crate) fn register_flight_benchmark(app: &mut App) {
    app.add_systems(
        Startup,
        |mut browser: ResMut<Browser>, blocks: Res<hollow_engine::blocks::BlockCatalog>| {
            open_world(
                &mut browser,
                None,
                Some(("Flight benchmark".into(), 12, WorldQuality::Fast)),
                blocks.clone(),
            );
        },
    );
}

fn poll_browser(
    mut commands: Commands,
    mut browser: ResMut<Browser>,
    mut next: ResMut<NextState<GameState>>,
    mut settings: ResMut<VoxelSettings>,
    mut save: ResMut<SaveState>,
    state: Res<State<GameState>>,
) {
    if let Some(task) = &mut browser.catalog_task {
        if let Some(result) = check_ready(task) {
            browser.catalog_task = None;
            match result {
                Ok(entries) => browser.entries = entries,
                Err(e) => browser.error = Some(e.to_string()),
            }
            if *state.get() == GameState::WorldSelect {
                ui::replace_world_list(&mut commands, &browser);
            }
        }
    }
    if let Some(task) = &mut browser.delete_task {
        if let Some(result) = check_ready(task) {
            browser.delete_task = None;
            browser.pending_delete = None;
            if std::mem::take(&mut browser.close_after_operation) {
                quit(&mut commands);
                return;
            }
            if let Err(error) = result {
                browser.error = Some(format!("Could not delete world: {error}"));
            }
            // Rescan even after an error: a filesystem failure can leave a partial deletion.
            browser.rescan();
            if *state.get() == GameState::WorldSelect {
                ui::replace_world_list(&mut commands, &browser);
            }
        }
    }
    let Some(task) = &mut browser.open_task else {
        return;
    };
    let Some(result) = check_ready(task) else {
        return;
    };
    browser.open_task = None;
    let close = std::mem::take(&mut browser.close_after_operation);
    match result {
        // Nothing was played: entering the game only to save it again would delay the exit.
        Ok(opened) if close => {
            opened.generator.cancel_requests();
            IoTaskPool::get()
                .spawn(async move {
                    drop(opened);
                })
                .detach();
            quit(&mut commands);
        }
        Ok(opened) => {
            settings.seed = opened.session.metadata.seed;
            commands.insert_resource(opened.session);
            commands.insert_resource(opened.generator.runtime.clone());
            commands.insert_resource(opened.generator);
            *save = SaveState::default();
            next.set(GameState::Game);
        }
        Err(_) if close => quit(&mut commands),
        Err(e) => browser.error = Some(e.to_string()),
    }
}

fn quit(commands: &mut Commands) {
    commands.queue(SaveSettingsSync::IfChanged);
    commands.write_message(AppExit::Success);
}

fn leave(destination: &Destination, game: &mut NextState<GameState>, commands: &mut Commands) {
    match destination {
        Destination::Worlds => game.set(GameState::WorldSelect),
        Destination::Exit => quit(commands),
    }
}

fn handle_window_close(mut events: MessageReader<WindowCloseRequested>, mut commands: Commands) {
    if events.read().next().is_some() {
        request_exit(&mut commands);
    }
}

fn exit_application(
    _: On<RequestExit>,
    state: Res<State<GameState>>,
    phase: Option<Res<State<WorldSessionPhase>>>,
    mut save: ResMut<SaveState>,
    mut browser: ResMut<Browser>,
    mut commands: Commands,
) {
    // Closing the window over a save error must not trap the user in endless retries.
    let failed = phase.is_some_and(|p| *p.get() == WorldSessionPhase::SaveError);
    if *state.get() == GameState::Game && !failed {
        save.destination = Destination::Exit;
        save.exit_requested = true;
    } else if browser.busy() {
        browser.close_after_operation = true;
    } else {
        quit(&mut commands);
    }
}

#[allow(clippy::too_many_arguments)]
fn settle_exit(
    mut save: ResMut<SaveState>,
    phase: Res<State<WorldSessionPhase>>,
    inventory: Res<State<InventoryState>>,
    mut next_inventory: ResMut<NextState<InventoryState>>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
    session: Option<Res<WorldSession>>,
    players: Query<(), With<Player>>,
    mut game: ResMut<NextState<GameState>>,
    mut commands: Commands,
) {
    if !save.exit_requested
        || matches!(
            phase.get(),
            WorldSessionPhase::Saving | WorldSessionPhase::SaveError
        )
    {
        return;
    }
    if *inventory.get() == InventoryState::Open {
        next_inventory.set(InventoryState::Closed);
        return;
    }
    // Without a player nothing was played: the committed checkpoint stays as it is.
    // Saving here would store replayed pending edits both in chunks and in the list.
    if session.is_none() || players.is_empty() {
        leave(&save.destination, &mut game, &mut commands);
        return;
    }
    save.settle_frames += 1;
    if save.settle_frames >= 2 {
        next.set(WorldSessionPhase::Saving);
    }
}

fn prepare_session(
    mut commands: Commands,
    generator: Option<Res<WorldGenerator>>,
    blocks: Res<hollow_engine::blocks::BlockCatalog>,
    settings: Res<VoxelSettings>,
    session: Option<Res<WorldSession>>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    virtual_time.pause();
    // Preserve the existing developer autoplay path as an explicitly ephemeral session.
    if generator.is_none() {
        let generator = WorldGenerator::new(settings.seed, blocks.clone());
        commands.insert_resource(generator.runtime.clone());
        commands.insert_resource(generator);
    }
    if let Some(session) = session {
        for &(position, state) in &session.resume.pending {
            if let Some(state) = crate::voxel::VoxelState::from_id(state) {
                commands.trigger(crate::voxel::SetVoxel::with_state(
                    IVec3::from_array(position),
                    state,
                ));
            }
        }
    }
}

fn begin_saving(
    generator: Res<WorldGenerator>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut commands: Commands,
) {
    generator.runtime.pause();
    virtual_time.pause();
    ui::saving(&mut commands);
}
fn resume_session(
    generator: Res<WorldGenerator>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut save: ResMut<SaveState>,
    mut commands: Commands,
) {
    generator.runtime.resume();
    virtual_time.unpause();
    commands.despawn_all::<With<ui::SaveOverlay>>();
    save.error = None;
}

/// `NextState` keeps the last write of a frame: run last so no transition requested
/// this frame (loading finished, exit settled) can hide an error.
fn hold_save_error(
    save: Res<SaveState>,
    phase: Res<State<WorldSessionPhase>>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
) {
    if save.error.is_some() && *phase.get() != WorldSessionPhase::SaveError {
        next.set(WorldSessionPhase::SaveError);
    }
}

fn poll_storage_errors(
    generator: Res<WorldGenerator>,
    mut save: ResMut<SaveState>,
    phase: Res<State<WorldSessionPhase>>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
    session: Res<hollow_engine::session::ActiveSession>,
    mut terrain_errors: MessageReader<hollow_engine::streaming::TerrainFailure>,
) {
    let mut errors = generator.take_errors();
    errors.extend(
        terrain_errors
            .read()
            .filter(|event| event.session == session.0)
            .map(|event| format!("Cannot load column {:?}: {}", event.position, event.reason)),
    );
    if errors.is_empty() {
        return;
    }
    save.error = Some(errors.join("\n"));
    if *phase.get() != WorldSessionPhase::SaveError {
        next.set(WorldSessionPhase::SaveError);
    }
}

#[allow(clippy::too_many_arguments)]
fn checkpoint(
    mut save: ResMut<SaveState>,
    phase: Res<State<WorldSessionPhase>>,
    time: Res<Time<Real>>,
    session: Option<Res<WorldSession>>,
    generation: (
        Res<WorldGenerator>,
        Res<hollow_engine::tasks::VoxelTaskBudget>,
    ),
    players: Query<
        (
            &Transform,
            &hollow_features::motion::FixedPosition,
            &PlayerInventory,
            &crate::inventory::CraftingGrid,
            &crate::inventory::HotbarSelection,
            &crate::survival::Health,
            &crate::survival::Nutrition,
            &crate::survival::ActiveEffects,
            &crate::survival::DamageState,
            &crate::survival::Hazards,
            &crate::survival::RespawnPoint,
        ),
        With<Player>,
    >,
    cameras: Query<&PlayerCamera>,
    cursor: Res<InventoryCursor>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels)>,
    chunk_index: Res<crate::voxel::streaming::ChunkIndex>,
    stored: Res<StoredChunks>,
    pending: Res<PendingEdits>,
    gameplay: crate::block_behavior::containers::ColumnSnapshots,
    clock: Res<crate::simulation::GameplayClock>,
    time_of_day: Res<crate::sky::TimeOfDay>,
) {
    let (generator, budget) = generation;
    let Some(session) = session else {
        return;
    };
    if save.task.is_some()
        || matches!(
            phase.get(),
            WorldSessionPhase::Loading | WorldSessionPhase::SaveError
        )
    {
        return;
    }
    save.elapsed += time.delta_secs();
    let saving = *phase.get() == WorldSessionPhase::Saving;
    let forced = saving || save.force;
    if !forced && save.elapsed < 30.0 {
        return;
    }
    // Only gameplay changes the world; `settle_exit` leaves without saving otherwise.
    let Ok((
        transform,
        motion,
        inventory,
        grid,
        selection,
        health,
        nutrition,
        effects,
        damage,
        hazards,
        respawn,
    )) = players.single()
    else {
        return;
    };
    save.elapsed = 0.0;
    save.force = false;
    let mut player = SavedPlayer {
        survival: crate::survival::SurvivalSnapshot {
            health: health.clone(),
            nutrition: nutrition.clone(),
            effects: effects.clone(),
            damage: damage.clone(),
            hazards: hazards.clone(),
            respawn: respawn.clone(),
        },
        position: motion.current.as_vec3().to_array(),
        rotation: transform.rotation.to_array(),
        pitch: cameras.single().map_or(0.0, |c| c.pitch),
        slots: (0..crate::inventory::SLOT_COUNT)
            .map(|i| inventory.slot(i).map(SavedStack::capture))
            .collect(),
        selected: selection.0,
        cursor: cursor.stack.map(SavedStack::capture),
        crafting: grid.slots.map(|stack| stack.map(SavedStack::capture)),
    };
    let previous = session.last_checkpoint.as_ref().unwrap_or(&session.resume);
    player.sanitize(previous.player.as_ref().or(session.resume.player.as_ref()));
    let checkpoint = Checkpoint {
        gameplay_tick: clock.0,
        player: Some(player),
        pending: pending
            .values()
            .flat_map(|edits| {
                edits
                    .iter()
                    .map(|e| (e.world_position.to_array(), e.state.id()))
            })
            .collect(),
        time_of_day: Some(time_of_day.days()),
    };
    // Estimate before cloning: only the selected columns retain snapshots while
    // the worker encodes them. Terrain and gameplay from a column share a capture.
    let mut reservations: HashMap<_, _> = gameplay
        .candidates()
        .into_iter()
        .map(|(position, codec, snapshot)| (position, (codec, snapshot)))
        .collect();
    let terrain_at = |position: &IVec2| {
        chunk_index
            .get(position)
            .and_then(|&entity| chunks.get(entity).ok())
            .map(|(_, voxels)| voxels)
            .filter(|voxels| voxels.save_revision != voxels.saved_revision)
            .or_else(|| {
                stored
                    .get(position)
                    .filter(|voxels| voxels.save_revision != voxels.saved_revision)
            })
    };
    for (position, voxels) in stored
        .iter()
        .map(|(p, v)| (*p, v))
        .chain(chunks.iter().map(|(p, v)| (p.position, v)))
    {
        if voxels.save_revision != voxels.saved_revision {
            // A loaded column can also be retained in StoredChunks. Resolve its
            // current owner once below rather than charging both copies.
            reservations.entry(position).or_default();
        }
    }
    for (position, (codec, snapshot)) in &mut reservations {
        if let Some(voxels) = terrain_at(position) {
            *codec += voxels.retained_bytes() * 8 + 4096;
            *snapshot += std::mem::size_of::<ChunkVoxels>();
        }
    }
    // On exit, rebuild from all dirty revisions after the previous write was
    // acknowledged, including edits made during the last background capture.
    if saving {
        save.remaining.clear();
    }
    if save.remaining.is_empty() {
        save.remaining.extend(reservations.keys().copied());
    }
    let mut snapshots = HashMap::new();
    let mut gameplay_snapshots = Vec::new();
    let mut snapshot_bytes = 0usize;
    let mut codec_bytes = crate::simulation::ENCODING_BATCH_BYTES;
    let mut selected = 0usize;
    while let Some(&position) = save.remaining.front() {
        let Some(&(codec, snapshot)) = reservations.get(&position) else {
            save.remaining.pop_front();
            continue;
        };
        let next_snapshot_bytes = snapshot_bytes.saturating_add(snapshot);
        let next_codec_bytes = codec_bytes.max(codec);
        let next_bytes = next_snapshot_bytes.saturating_add(next_codec_bytes);
        if selected > 0 && (selected >= 128 || next_bytes > crate::simulation::MAX_CAPTURE_BYTES) {
            break;
        }
        if next_bytes > crate::simulation::MAX_CAPTURE_BYTES {
            let error = io::Error::other("A world column exceeds the bounded save capture budget.");
            save.task = Some(IoTaskPool::get().spawn(async move { Err(error) }));
            return;
        }
        save.remaining.pop_front();
        snapshot_bytes = next_snapshot_bytes;
        codec_bytes = next_codec_bytes;
        selected += 1;
        if let Some(voxels) = terrain_at(&position) {
            snapshots.insert(position, voxels.save_snapshot());
        }
        if let Some(snapshot) = gameplay.capture_at(position) {
            gameplay_snapshots.push((position, snapshot));
        }
    }
    let captured_bytes = snapshot_bytes.saturating_add(codec_bytes);
    save.force = !save.remaining.is_empty();
    let final_save = saving && save.remaining.is_empty();
    if !forced
        && gameplay_snapshots.is_empty()
        && snapshots.is_empty()
        && session
            .last_checkpoint
            .as_ref()
            .is_some_and(|last| last.same_state(&checkpoint))
    {
        return;
    }
    let mut metadata = session.metadata.clone();
    metadata.last_played = model::now();
    let database = session.database.clone();
    let generator = generator.clone();
    let budget = budget.clone();
    save.in_flight_bytes = captured_bytes;
    save.task = Some(IoTaskPool::get().spawn(async move {
        generator.flush().await?;
        let revisions = snapshots
            .iter()
            .map(|(p, v)| (*p, v.save_revision))
            .collect();
        let gameplay_revisions = gameplay_snapshots
            .iter()
            .map(|(p, data)| (*p, data.revision))
            .collect();
        let mut captures: HashMap<
            _,
            (
                Option<ChunkVoxels>,
                Option<crate::block_behavior::containers::ColumnSnapshot>,
                usize,
            ),
        > = snapshots
            .into_iter()
            .map(|(position, voxels)| (position, (Some(voxels), None, reservations[&position].0)))
            .collect();
        for (position, gameplay) in gameplay_snapshots {
            captures
                .entry(position)
                .or_insert_with(|| (None, None, reservations[&position].0))
                .1 = Some(gameplay);
        }
        let positions = captures
            .iter()
            .map(|(p, (terrain, gameplay, _))| ((p.x, p.y), terrain.is_some(), gameplay.is_some()))
            .collect();
        let (prepared, checkpoint, metadata) = budget
            .spawn(hollow_engine::tasks::WorkClass::Encoding, 0, async move {
                prepare_checkpoint(&metadata, &checkpoint, positions)
                    .map(|prepared| (prepared, checkpoint, metadata))
            })
            .await?;
        let mut captures = captures.into_iter().peekable();
        let batches = std::iter::from_fn(move || {
            let mut batch = Vec::new();
            let mut bytes = 0usize;
            while let Some((_, (_, _, cost))) = captures.peek() {
                if !batch.is_empty()
                    && bytes.saturating_add(*cost) > crate::simulation::ENCODING_BATCH_BYTES
                {
                    break;
                }
                bytes += *cost;
                batch.push(captures.next().expect("Peeked column"));
            }
            if batch.is_empty() {
                return None;
            }
            // SQL stays on this I/O worker, with a single transaction. Encoding
            // waits on the shared compute budget; no extra writer thread/channel
            // or whole-world vector of compressed records is retained.
            Some(bevy::tasks::block_on(budget.spawn(
                hollow_engine::tasks::WorkClass::Encoding,
                0,
                async move {
                    batch
                        .into_iter()
                        .map(|(position, (terrain, gameplay, _))| {
                            let terrain = terrain.map(|voxels| {
                                hollow_worldgen::codec::compress_record(&voxels.encode_saved())
                            });
                            let gameplay = gameplay
                                .map(|snapshot| {
                                    let bytes = snapshot.encode()?;
                                    if bytes.len() > hollow_worldgen::codec::MAX_RECORD_BYTES {
                                        return Err(io::Error::other(
                                            "Gameplay snapshot exceeds the record size limit.",
                                        ));
                                    }
                                    Ok(hollow_worldgen::codec::compress_record(&bytes))
                                })
                                .transpose()?;
                            Ok(storage::EncodedColumn {
                                position: (position.x, position.y),
                                terrain,
                                application: gameplay,
                            })
                        })
                        .collect::<io::Result<Vec<_>>>()
                },
            )))
        });
        database.checkpoint(prepared, batches)?;
        Ok(SaveAck {
            revisions,
            gameplay_revisions,
            checkpoint,
            metadata,
            final_save,
        })
    }));
}

fn poll_save(
    mut save: ResMut<SaveState>,
    mut session: Option<ResMut<WorldSession>>,
    mut chunks: Query<(&VoxelChunk, &mut ChunkVoxels)>,
    mut stored: ResMut<StoredChunks>,
    mut gameplay_columns: Query<&mut crate::block_behavior::containers::GameplayColumn>,
    phase: Res<State<WorldSessionPhase>>,
    mut next_phase: ResMut<NextState<WorldSessionPhase>>,
    mut next: ResMut<NextState<GameState>>,
    mut commands: Commands,
) {
    let Some(task) = &mut save.task else {
        return;
    };
    let Some(result) = check_ready(task) else {
        return;
    };
    save.task = None;
    save.in_flight_bytes = 0;
    match result {
        Ok(ack) => {
            if session
                .as_ref()
                .is_none_or(|session| session.metadata.id != ack.metadata.id)
            {
                return;
            }
            let revisions: HashMap<_, _> = ack.revisions.into_iter().collect();
            for (chunk, mut voxels) in &mut chunks {
                if let Some(&revision) = revisions.get(&chunk.position) {
                    if revision <= voxels.save_revision {
                        voxels.bypass_change_detection().saved_revision = revision;
                    }
                }
            }
            stored.retain(|position, voxels| {
                !revisions
                    .get(position)
                    .is_some_and(|&r| r == voxels.save_revision)
            });
            let gameplay_revisions: HashMap<_, _> = ack.gameplay_revisions.into_iter().collect();
            for mut data in &mut gameplay_columns {
                if let Some(&revision) = gameplay_revisions.get(&data.position) {
                    if revision <= data.revision {
                        data.bypass_change_detection().saved_revision = revision;
                    }
                }
            }
            if let Some(session) = &mut session {
                session.metadata = ack.metadata;
                session.last_checkpoint = Some(ack.checkpoint);
            }
            if *phase.get() == WorldSessionPhase::Saving {
                // A previous autosave may have finished after the exit was requested.
                // Only leave after a checkpoint captured while frozen.
                if ack.final_save {
                    leave(&save.destination, &mut next, &mut commands);
                }
            }
        }
        Err(e) => {
            // Rebuild from dirty revisions on retry, including the failed batch.
            save.remaining.clear();
            save.error = Some(e.to_string());
            next_phase.set(WorldSessionPhase::SaveError);
        }
    }
}

fn end_session(
    mut commands: Commands,
    mut virtual_time: ResMut<Time<Virtual>>,
    generator: Option<Res<WorldGenerator>>,
) {
    virtual_time.unpause();
    if let Some(generator) = generator {
        generator.cancel_requests();
    }
    commands.remove_resource::<hollow_worldgen::GenerationSession>();
    commands.remove_resource::<hollow_worldgen::GenerationSettings>();
    // Taking ownership for asynchronous destruction requires structural access.
    commands.queue(|world: &mut World| {
        let retired = (
            world.remove_resource::<WorldGenerator>(),
            world.remove_resource::<WorldSession>(),
        );
        IoTaskPool::get()
            .spawn(async move {
                drop(retired);
            })
            .detach();
    });
}

fn storage_pressure(
    mut save: ResMut<SaveState>,
    stored: Res<StoredChunks>,
    columns: Query<(
        &crate::block_behavior::containers::GameplayColumn,
        &crate::simulation::ColumnSimulation,
        Option<&crate::block_behavior::containers::LoadedTerrain>,
        Option<&crate::block_behavior::containers::BlockEntities>,
        Option<&crate::simulation::SimulatedEntities>,
        Option<&crate::creatures::spawning::ColumnBiomes>,
    )>,
    stream: Res<crate::voxel::streaming::StreamState>,
    creatures: crate::creatures::CreatureSnapshots,
    mut diagnostics: ResMut<crate::simulation::SimulationDiagnostics>,
) {
    let mut bytes = stored
        .values()
        .map(ChunkVoxels::retained_bytes)
        .sum::<usize>();
    let (mut ready, mut suspended) = (0, 0);
    let mut oversized_retirement = false;
    for (column, work, loaded, blocks, mobiles, biomes) in &columns {
        ready += work.ready_count();
        suspended += work.suspended.len();
        let retired = loaded.is_none_or(|loaded| loaded.is_empty());
        // Streamed columns are the common case; skip them before resolving creatures.
        if !retired && stream.data.contains(&column.position) {
            continue;
        }
        let mobile_bytes = creatures.retirement_bytes(mobiles);
        if column.revision == column.saved_revision && mobile_bytes == 0 {
            continue;
        }
        let retained = mobile_bytes
            + column.application.retained_bytes()
            + biomes.map_or(0, |b| b.0.len())
            + work.estimated_bytes()
            + blocks.map_or(0, |blocks| blocks.len())
                * crate::block_behavior::containers::RETAINED_BLOCK_BYTES;
        if retired {
            bytes += retained;
        } else if retained >= crate::simulation::MAX_PENDING_WRITE_BYTES / 2 {
            oversized_retirement = true;
        }
    }
    diagnostics.ready_work = ready;
    diagnostics.suspended_work = suspended;
    diagnostics.pending_write_bytes = bytes.saturating_add(save.in_flight_bytes);
    if oversized_retirement || bytes >= crate::simulation::MAX_RETIRED_BYTES / 2 {
        save.force = true;
    }
}

fn prepare_checkpoint(
    metadata: &WorldMetadata,
    checkpoint: &Checkpoint,
    positions: Vec<((i32, i32), bool, bool)>,
) -> std::io::Result<storage::PreparedCheckpoint> {
    Ok(storage::PreparedCheckpoint::new(
        serde_json::to_vec(metadata)?,
        serde_json::to_vec(checkpoint)?,
        positions,
    ))
}
