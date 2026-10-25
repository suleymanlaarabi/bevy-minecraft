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
use avian3d::prelude::*;
use bevy::{
    prelude::*,
    settings::SaveSettingsSync,
    tasks::{IoTaskPool, Task, futures::check_ready},
    window::{CursorGrabMode, CursorOptions, WindowCloseRequested},
};
pub(crate) use model::{Checkpoint, SavedDrop, SavedPlayer, SavedStack};
pub use model::{WorldMetadata, WorldQuality};
use std::{collections::HashMap, io, path::PathBuf, sync::Arc};
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
struct SaveState {
    task: Option<Task<io::Result<SaveAck>>>,
    elapsed: f32,
    destination: Destination,
    pub(super) error: Option<String>,
    exit_requested: bool,
    /// Autosave on the next frame, even when nothing seems to have changed.
    force: bool,
    // Give inventory OnExit and deferred commands one complete frame to settle.
    settle_frames: u8,
}
struct SaveAck {
    revisions: Vec<(IVec2, u64)>,
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

pub(crate) fn report_load_error(commands: &mut Commands, error: String) {
    commands.queue(move |world: &mut World| {
        world.resource_mut::<SaveState>().error = Some(error);
        world
            .resource_mut::<NextState<WorldSessionPhase>>()
            .set(WorldSessionPhase::SaveError);
    });
}

pub struct WorldPlugin;
impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.add_sub_state::<WorldSessionPhase>()
            .init_resource::<Browser>()
            .init_resource::<SaveState>()
            .add_observer(exit_application)
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
            .add_systems(OnEnter(WorldSessionPhase::Playing), resume_physics)
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
                    checkpoint,
                    poll_save,
                    hold_save_error,
                    ui::refresh_save_status,
                )
                    .chain()
                    .run_if(in_state(GameState::Game)),
            );
    }
}

fn start_catalog(mut browser: ResMut<Browser>) {
    browser.error = None;
    browser.pending_delete = None;
    browser.rescan();
    if let Some(path) = browser.reopen.take() {
        open_world(&mut browser, Some(path), None);
    }
}

fn open_world(
    browser: &mut Browser,
    path: Option<PathBuf>,
    creation: Option<(String, i64, WorldQuality)>,
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
            WorldGenerator::persistent(metadata.seed, metadata.quality, database.clone());
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

/// Public navigation entrypoint; direct Game -> Menu transitions would lose unsaved entities.
pub(crate) fn request_leave(mut commands: Commands) {
    commands.queue(|world: &mut World| {
        let mut save = world.resource_mut::<SaveState>();
        save.exit_requested = true;
    });
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
    settings: Res<VoxelSettings>,
    session: Option<Res<WorldSession>>,
    mut physics: ResMut<Time<Physics>>,
    mut virtual_time: ResMut<Time<Virtual>>,
) {
    physics.pause();
    virtual_time.pause();
    // Preserve the existing developer autoplay path as an explicitly ephemeral session.
    if generator.is_none() {
        commands.insert_resource(WorldGenerator::new(settings.seed));
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
    mut physics: ResMut<Time<Physics>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    mut commands: Commands,
    mut cursors: Query<&mut CursorOptions>,
) {
    physics.pause();
    virtual_time.pause();
    for mut cursor in &mut cursors {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
    ui::saving(&mut commands);
}
fn resume_physics(
    mut physics: ResMut<Time<Physics>>,
    mut virtual_time: ResMut<Time<Virtual>>,
    inventory: Res<State<InventoryState>>,
    mut cursors: Query<&mut CursorOptions>,
    mut save: ResMut<SaveState>,
    mut commands: Commands,
    overlays: Query<Entity, With<ui::SaveOverlay>>,
) {
    physics.unpause();
    virtual_time.unpause();
    for mut cursor in &mut cursors {
        cursor.visible = *inventory.get() == InventoryState::Open;
        cursor.grab_mode = if cursor.visible {
            CursorGrabMode::None
        } else {
            CursorGrabMode::Locked
        };
    }
    for entity in &overlays {
        commands.entity(entity).despawn();
    }
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
    mut physics: ResMut<Time<Physics>>,
) {
    let errors = generator.take_errors();
    if errors.is_empty() {
        return;
    }
    save.error = Some(errors.join("\n"));
    physics.pause();
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
    generator: Res<WorldGenerator>,
    players: Query<(&Transform, &PlayerInventory), With<Player>>,
    cameras: Query<&PlayerCamera>,
    cursor: Res<InventoryCursor>,
    drops: Query<(
        &crate::dropped_item::DroppedItem,
        &Transform,
        &LinearVelocity,
    )>,
    chunks: Query<(&VoxelChunk, &ChunkVoxels)>,
    stored: Res<StoredChunks>,
    pending: Res<PendingEdits>,
    game_time: Res<Time>,
    time_of_day: Res<crate::sky::TimeOfDay>,
) {
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
    let final_save = *phase.get() == WorldSessionPhase::Saving;
    let forced = final_save || save.force;
    if !forced && save.elapsed < 30.0 {
        return;
    }
    // Only gameplay changes the world; `settle_exit` leaves without saving otherwise.
    let Ok((transform, inventory)) = players.single() else {
        return;
    };
    save.elapsed = 0.0;
    save.force = false;
    let mut player = SavedPlayer {
        position: transform.translation.to_array(),
        rotation: transform.rotation.to_array(),
        pitch: cameras.single().map_or(0.0, |c| c.pitch),
        slots: (0..crate::inventory::SLOT_COUNT)
            .map(|i| inventory.slot(i).map(SavedStack::capture))
            .collect(),
        selected: inventory.selected_hotbar(),
        cursor: cursor.stack.map(SavedStack::capture),
    };
    let previous = session.last_checkpoint.as_ref().unwrap_or(&session.resume);
    player.sanitize(previous.player.as_ref().or(session.resume.player.as_ref()));
    let checkpoint = Checkpoint {
        player: Some(player),
        drops: drops
            .iter()
            .map(|(drop, transform, velocity)| {
                drop.capture(transform.translation, velocity.0, game_time.elapsed())
            })
            .collect(),
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
    // Layers are copy-on-write: these clones only bump reference counts.
    let mut snapshots = HashMap::new();
    for (position, voxels) in stored
        .iter()
        .map(|(p, v)| (*p, v))
        .chain(chunks.iter().map(|(p, v)| (p.position, v)))
    {
        if voxels.save_revision != voxels.saved_revision {
            snapshots.insert(position, voxels.clone());
        }
    }
    if !forced
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
    save.task = Some(IoTaskPool::get().spawn(async move {
        // Cache bases reach a durable transaction before their game checkpoint.
        generator.flush()?;
        let revisions = snapshots
            .iter()
            .map(|(p, v)| (*p, v.save_revision))
            .collect();
        let records = snapshots
            .into_iter()
            .map(|(p, v)| ((p.x, p.y), v.encode_saved()))
            .collect();
        database.checkpoint(&metadata, &checkpoint, records)?;
        Ok(SaveAck {
            revisions,
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
            save.error = Some(e.to_string());
            next_phase.set(WorldSessionPhase::SaveError);
        }
    }
}

fn end_session(mut commands: Commands, mut virtual_time: ResMut<Time<Virtual>>) {
    virtual_time.unpause();
    commands.queue(|world: &mut World| {
        if let Some(generator) = world.remove_resource::<WorldGenerator>() {
            generator.cancel_requests();
            IoTaskPool::get()
                .spawn(async move {
                    drop(generator);
                })
                .detach();
        }
        if let Some(session) = world.remove_resource::<WorldSession>() {
            IoTaskPool::get()
                .spawn(async move {
                    drop(session);
                })
                .detach();
        }
    });
}
