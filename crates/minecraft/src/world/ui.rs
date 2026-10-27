use super::*;
use bevy::{
    feathers::{
        controls::{
            ButtonVariant, FeathersButton, FeathersListRow, FeathersListView, FeathersTextInput,
            FeathersTextInputContainer,
        },
        theme::{ThemeBackgroundColor, ThemedText},
        tokens,
    },
    input_focus::{InputFocus, tab_navigation::TabIndex},
    ui::InteractionDisabled,
    ui_widgets::{Activate, ValueChange},
};

#[derive(Component, Clone, Default)]
struct WorldPage;
#[derive(Component, Clone, Default)]
struct WorldRow(usize);
#[derive(Component, Clone, Default)]
struct DeleteWorld(usize);
#[derive(Component, Clone, Default)]
struct CancelDelete;
#[derive(Component, Clone, Copy, Default)]
pub(super) enum Field {
    #[default]
    Name,
    Seed,
}
#[derive(Component, Clone, Default)]
pub(super) struct FormStatus;
#[derive(Component, Clone, Default)]
pub(super) struct FormSubmit;
#[derive(Component, Clone, Default)]
pub(super) struct BrowserStatus;
#[derive(Component, Clone, Default)]
#[require(hollow_features::cursor::FreeCursor)]
pub(super) struct SaveOverlay;
#[derive(Component, Clone, Default)]
pub(super) struct SaveStatus;
#[derive(Component, Clone, Default)]
pub(super) struct SaveIndicator;

fn button(label: impl Into<String>, variant: ButtonVariant) -> impl Scene {
    let label = label.into();
    bsn! {
        @FeathersButton { @caption: bsn! { Text(label) ThemedText }, @variant: variant }
        Node { min_width: px(180), min_height: px(38) }
    }
}

fn trash_icon() -> impl Scene {
    let color = Color::srgb_u8(240, 116, 116);
    bsn! {
        Node { width: px(20), height: px(22), flex_shrink: 0.0 }
        Pickable::IGNORE
        Children [
            Node { position_type: PositionType::Absolute, left: px(6), top: px(1), width: px(8), height: px(4), border: UiRect::all(px(2)) }
            BorderColor::all(color)
            Pickable::IGNORE
            --
            Node { position_type: PositionType::Absolute, left: px(1), top: px(5), width: px(18), height: px(2) }
            BackgroundColor(color)
            Pickable::IGNORE
            --
            Node { position_type: PositionType::Absolute, left: px(3), top: px(8), width: px(14), height: px(13), border: UiRect::all(px(2)), justify_content: JustifyContent::SpaceEvenly, padding: UiRect::vertical(px(2)) }
            BorderColor::all(color)
            Pickable::IGNORE
            Children [
                Node { width: px(2), height: percent(100) } BackgroundColor(color) Pickable::IGNORE
                --
                Node { width: px(2), height: percent(100) } BackgroundColor(color) Pickable::IGNORE
            ]
        ]
    }
}

fn page(state: GameState) -> impl Scene {
    bsn! {
        Node { width: percent(100), height: percent(100), padding: UiRect::all(px(32)), align_items: AlignItems::Center, justify_content: JustifyContent::Center, flex_direction: FlexDirection::Column, row_gap: px(18) }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        WorldPage
        DespawnOnExit::<GameState>(state)
    }
}

pub(super) fn world_list(mut commands: Commands, browser: Res<Browser>) {
    spawn_world_list(&mut commands, &browser);
}

pub(super) fn replace_world_list(commands: &mut Commands, browser: &Browser) {
    commands.queue(|world: &mut World| {
        let entities: Vec<_> = world
            .query_filtered::<Entity, With<WorldPage>>()
            .iter(world)
            .collect();
        for entity in entities {
            world.despawn(entity);
        }
    });
    spawn_world_list(commands, browser);
}

fn spawn_world_list(commands: &mut Commands, browser: &Browser) {
    if let Some(path) = &browser.pending_delete {
        let name = browser
            .entries
            .iter()
            .find(|entry| &entry.path == path)
            .and_then(|entry| entry.metadata.as_ref())
            .map_or("Unreadable world", |metadata| metadata.name.as_str());
        // Both buttons lock while the deletion runs.
        let busy = browser.busy();
        let disabled = || busy.then_some(bsn! { InteractionDisabled });
        let (cancel_disabled, delete_disabled) = (disabled(), disabled());
        commands.spawn_scene(bsn! {
            @page(GameState::WorldSelect)
            Children [
                Text("Delete world?") ThemedText
                --
                Text({format!("Delete \"{name}\" and all its saved progress? This cannot be undone.")}) ThemedText
                --
                Text("") ThemedText BrowserStatus
                --
                Node { flex_direction: FlexDirection::Row, column_gap: px(12) }
                Children [
                    @button("Cancel", ButtonVariant::Normal) @cancel_disabled CancelDelete on(cancel_delete)
                    --
                    @button("Delete World", ButtonVariant::Primary) @delete_disabled on(confirm_delete)
                ]
            ]
        });
        commands.queue(|world: &mut World| {
            if let Some(entity) = world
                .query_filtered::<Entity, With<CancelDelete>>()
                .iter(world)
                .next()
            {
                world
                    .resource_mut::<InputFocus>()
                    .set(entity, bevy::input_focus::FocusCause::Navigated);
            }
        });
        return;
    }
    let rows: Vec<_> = browser.entries.iter().enumerate().map(|(index, entry)| {
        let caption = match &entry.metadata {
            Some(m) => format!("{}\nSeed: {}  ·  {}  ·  Last played: {}", m.name, m.seed, m.quality.label(), date(m.last_played)),
            None => "Unreadable world".into(),
        };
        let error = entry.error.clone();
        let disabled = error.is_some().then_some(bsn! { InteractionDisabled });
        bsn! {
            @FeathersListRow
            Node { flex_direction: FlexDirection::Row, align_items: AlignItems::Center, column_gap: px(12), padding: UiRect::all(px(12)) }
            WorldRow(index)
            @disabled
            Children [
                Node { flex_direction: FlexDirection::Column, flex_grow: 1.0, flex_basis: px(0), min_width: px(0), row_gap: px(4) }
                Children [
                    Text(caption) ThemedText
                    --
                    Text({error.unwrap_or_default()}) ThemedText
                ]
                --
                @FeathersButton { @caption: bsn! { @trash_icon() }, @variant: ButtonVariant::Plain }
                Node { width: px(38), height: px(38), flex_shrink: 0.0, align_items: AlignItems::Center, justify_content: JustifyContent::Center }
                Name("Delete world")
                AccessibleLabel::new("Delete world")
                DeleteWorld(index)
                on(request_delete)
            ]
        }
    }).collect();
    let message = if browser.catalog_task.is_some() {
        "Loading worlds…"
    } else if browser.entries.is_empty() {
        "No worlds yet. Create your first world."
    } else {
        "Select a world to continue playing."
    };
    commands.spawn_scene(bsn! {
        @page(GameState::WorldSelect)
        Children [
            Text("Worlds") ThemedText
            --
            Text(message) ThemedText
            --
            @FeathersListView { @rows: {Box::new(rows) as Box<dyn SceneList>} }
            on(select_world)
            Node { width: percent(85), max_width: px(900), height: percent(60) }
            --
            Text({browser.error.clone().unwrap_or_default()}) ThemedText BrowserStatus
            --
            Node { flex_direction: FlexDirection::Row, column_gap: px(12) }
            Children [
                @button("Create World", ButtonVariant::Primary) on(create_world)
                --
                @button("Back", ButtonVariant::Plain) on(back_to_menu)
            ]
        ]
    });
}

fn request_delete(
    event: On<Activate>,
    buttons: Query<&DeleteWorld>,
    mut browser: ResMut<Browser>,
    mut commands: Commands,
) {
    if !browser.idle() || browser.catalog_task.is_some() {
        return;
    }
    let Ok(button) = buttons.get(event.entity) else {
        return;
    };
    let Some(entry) = browser.entries.get(button.0) else {
        return;
    };
    browser.pending_delete = Some(entry.path.clone());
    browser.error = None;
    replace_world_list(&mut commands, &browser);
}

fn confirm_delete(_: On<Activate>, mut browser: ResMut<Browser>, mut commands: Commands) {
    if browser.busy() {
        return;
    }
    let Some(path) = browser.pending_delete.clone() else {
        return;
    };
    browser.error = None;
    browser.delete_task =
        Some(IoTaskPool::get().spawn(async move { storage::delete_world(&path) }));
    replace_world_list(&mut commands, &browser);
}

fn cancel_delete(_: On<Activate>, mut browser: ResMut<Browser>, mut commands: Commands) {
    if browser.busy() {
        return;
    }
    browser.pending_delete = None;
    replace_world_list(&mut commands, &browser);
}

pub(super) fn create_page(mut commands: Commands, mut browser: ResMut<Browser>) {
    browser.error = None;
    browser.quality = WorldQuality::Fast;
    commands.spawn_scene(bsn! {
        @page(GameState::WorldCreate)
        Children [
            Text("Create World") ThemedText
            --
            Node { width: px(480), max_width: percent(95), flex_direction: FlexDirection::Column, row_gap: px(12) }
            Children [
                Text("World name") ThemedText
                --
                @FeathersTextInputContainer
                Children [ @FeathersTextInput { @max_characters: {Some(64)} } Text("New World") Field::Name TabIndex(0) ]
                --
                Text("Seed (signed integer; leave empty for a random seed)") ThemedText
                --
                @FeathersTextInputContainer
                Children [ @FeathersTextInput { @max_characters: {Some(20)} } Text("") Field::Seed TabIndex(1) ]
                --
                Text("Quality: Fast prioritizes performance; Vanilla preserves Pumpkin terrain calculations.") ThemedText
                --
                @button("Quality: Fast", ButtonVariant::Normal) on(toggle_quality)
                --
                Text("") ThemedText FormStatus
                --
                Text("") ThemedText BrowserStatus
                --
                @button("Create and Play", ButtonVariant::Primary) FormSubmit on(submit)
                --
                @button("Back", ButtonVariant::Plain) on(back_to_worlds)
            ]
        ]
    });
    // Focus once the scene commands have materialized the input entities.
    commands.queue(|world: &mut World| {
        let entity = world
            .query::<(Entity, &Field)>()
            .iter(world)
            .find_map(|(e, f)| matches!(f, Field::Name).then_some(e));
        if let Some(entity) = entity {
            world
                .resource_mut::<InputFocus>()
                .set(entity, bevy::input_focus::FocusCause::Navigated);
        }
    });
}

fn create_world(_: On<Activate>, browser: Res<Browser>, mut next: ResMut<NextState<GameState>>) {
    if !browser.busy() {
        next.set(GameState::WorldCreate);
    }
}
fn back_to_menu(_: On<Activate>, browser: Res<Browser>, mut next: ResMut<NextState<GameState>>) {
    if !browser.busy() {
        next.set(GameState::Menu);
    }
}
fn back_to_worlds(_: On<Activate>, browser: Res<Browser>, mut next: ResMut<NextState<GameState>>) {
    if !browser.busy() {
        next.set(GameState::WorldSelect);
    }
}
fn select_world(
    event: On<ValueChange<Entity>>,
    rows: Query<&WorldRow>,
    mut browser: ResMut<Browser>,
    blocks: Res<hollow_engine::blocks::BlockCatalog>,
) {
    let Ok(row) = rows.get(event.value) else {
        return;
    };
    let Some(entry) = browser.entries.get(row.0) else {
        return;
    };
    if entry.error.is_some() {
        return;
    }
    let path = entry.path.clone();
    open_world(&mut browser, Some(path), None, blocks.clone());
}
fn toggle_quality(
    event: On<Activate>,
    mut browser: ResMut<Browser>,
    children: Query<&Children>,
    mut texts: Query<&mut Text>,
) {
    if browser.busy() {
        return;
    }
    browser.quality = match browser.quality {
        WorldQuality::Fast => WorldQuality::Vanilla,
        WorldQuality::Vanilla => WorldQuality::Fast,
    };
    // Feathers captions are descendants of the button's scene root.
    for entity in children.iter_descendants(event.entity) {
        if let Ok(mut text) = texts.get_mut(entity) {
            text.0 = format!("Quality: {}", browser.quality.label());
        }
    }
}

fn form(inputs: &Query<(&Field, &Text)>) -> Result<(String, Option<i64>), String> {
    let mut name = String::new();
    let mut seed = String::new();
    for (field, text) in inputs {
        match field {
            Field::Name => name = text.0.trim().into(),
            Field::Seed => seed = text.0.trim().into(),
        }
    }
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Enter a name between 1 and 64 characters.".into());
    }
    let seed = if seed.is_empty() {
        None
    } else {
        Some(
            seed.parse::<i64>()
                .map_err(|_| "The seed must be a signed 64-bit integer.".to_string())?,
        )
    };
    Ok((name, seed))
}
fn submit(
    _: On<Activate>,
    inputs: Query<(&Field, &Text)>,
    mut browser: ResMut<Browser>,
    blocks: Res<hollow_engine::blocks::BlockCatalog>,
) {
    let (name, seed) = match form(&inputs) {
        Ok(v) => v,
        Err(e) => {
            browser.error = Some(e);
            return;
        }
    };
    let seed = seed.unwrap_or_else(|| {
        i64::from_le_bytes(uuid::Uuid::new_v4().as_bytes()[..8].try_into().unwrap())
    });
    let quality = browser.quality;
    open_world(
        &mut browser,
        None,
        Some((name, seed, quality)),
        blocks.clone(),
    );
}

pub(super) fn validate_form(
    inputs: Query<(&Field, &Text)>,
    mut statuses: Query<&mut Text, (With<FormStatus>, Without<Field>)>,
    buttons: Query<(Entity, Has<InteractionDisabled>), With<FormSubmit>>,
    browser: Res<Browser>,
    mut commands: Commands,
) {
    if inputs.is_empty() {
        return;
    }
    let error = form(&inputs).err();
    for mut text in &mut statuses {
        let value = error.as_deref().unwrap_or("");
        if text.0 != value {
            text.0 = value.into();
        }
    }
    let disabled = error.is_some() || browser.busy();
    for (entity, was_disabled) in &buttons {
        if disabled != was_disabled {
            if disabled {
                commands.entity(entity).insert(InteractionDisabled);
            } else {
                commands.entity(entity).remove::<InteractionDisabled>();
            }
        }
    }
}
pub(super) fn refresh_status(
    browser: Res<Browser>,
    mut texts: Query<&mut Text, With<BrowserStatus>>,
) {
    if !browser.is_changed() {
        return;
    }
    let message = if browser.delete_task.is_some() {
        "Deleting world…"
    } else if browser.open_task.is_some() {
        "Opening world…"
    } else {
        browser.error.as_deref().unwrap_or("")
    };
    for mut text in &mut texts {
        if text.0 != message {
            text.0 = message.into();
        }
    }
}
pub(super) fn navigate_back(
    keyboard: Res<ButtonInput<KeyCode>>,
    state: Res<State<GameState>>,
    mut browser: ResMut<Browser>,
    mut next: ResMut<NextState<GameState>>,
    mut commands: Commands,
) {
    if !keyboard.just_pressed(KeyCode::Escape) || browser.busy() {
        return;
    }
    if *state.get() == GameState::WorldSelect && browser.pending_delete.take().is_some() {
        replace_world_list(&mut commands, &browser);
        return;
    }
    match state.get() {
        GameState::WorldCreate => next.set(GameState::WorldSelect),
        GameState::WorldSelect => next.set(GameState::Menu),
        _ => {}
    }
}

pub(super) fn saving(commands: &mut Commands) {
    commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), position_type: PositionType::Absolute, align_items: AlignItems::Center, justify_content: JustifyContent::Center }
        BackgroundColor(Color::srgba(0.03,0.03,0.04,0.95))
        GlobalZIndex(100)
        SaveOverlay
        DespawnOnExit::<GameState>(GameState::Game)
        Children [ Text("Saving world…") ThemedText ]
    });
}

pub(super) fn save_error(
    mut commands: Commands,
    save: Res<SaveState>,
    mut virtual_time: ResMut<Time<Virtual>>,
    players: Query<(), With<Player>>,
) {
    virtual_time.pause();
    commands.despawn_all::<With<SaveOverlay>>();
    let can_resume = !players.is_empty();
    let resume_disabled = (!can_resume).then_some(bsn! { InteractionDisabled });
    commands.spawn_scene(bsn! {
        Node { width: percent(100), height: percent(100), position_type: PositionType::Absolute, flex_direction: FlexDirection::Column, row_gap: px(16), padding: UiRect::all(px(32)), align_items: AlignItems::Center, justify_content: JustifyContent::Center }
        BackgroundColor(Color::srgb_u8(15,15,18))
        GlobalZIndex(100)
        SaveOverlay
        DespawnOnExit::<GameState>(GameState::Game)
        Children [
            Text("The world could not be saved or loaded") ThemedText
            --
            Text({save.error.clone().unwrap_or_default()}) ThemedText SaveStatus
            --
            @button("Retry", ButtonVariant::Primary) on(retry)
            --
            @button("Return to Game", ButtonVariant::Normal) on(resume)
            @resume_disabled
            --
            @button("Leave Without Saving", ButtonVariant::Plain) on(abort_loading)
        ]
    });
}
fn retry(
    _: On<Activate>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
    mut save: ResMut<SaveState>,
    mut commands: Commands,
    players: Query<(), With<Player>>,
    session: Option<Res<WorldSession>>,
    mut browser: ResMut<Browser>,
    mut game: ResMut<NextState<GameState>>,
) {
    if save.task.is_some() {
        return;
    }
    if players.is_empty() {
        if let Some(session) = session {
            browser.reopen = Some(session.path.clone());
            game.set(GameState::WorldSelect);
        }
        return;
    }
    commands.despawn_all::<With<SaveOverlay>>();
    save.error = None;
    if save.exit_requested {
        next.set(WorldSessionPhase::Saving);
    } else {
        // An autosave failed: retry it while playing instead of leaving the world.
        save.force = true;
        next.set(WorldSessionPhase::Playing);
    }
}
fn resume(
    _: On<Activate>,
    players: Query<(), With<Player>>,
    mut next: ResMut<NextState<WorldSessionPhase>>,
    mut save: ResMut<SaveState>,
) {
    if !players.is_empty() {
        save.exit_requested = false;
        save.settle_frames = 0;
        save.destination = Destination::Worlds;
        next.set(WorldSessionPhase::Playing);
    }
}
fn abort_loading(_: On<Activate>, mut next: ResMut<NextState<GameState>>) {
    // Keep the last committed checkpoint: storage keeps failing or nothing was played.
    next.set(GameState::WorldSelect);
}
pub(super) fn spawn_indicator(commands: &mut Commands) {
    commands.spawn_scene(bsn! {
        Node { position_type: PositionType::Absolute, right: px(16), top: px(16) }
        DespawnOnExit::<GameState>(GameState::Game)
        Children [ Text("") ThemedText SaveIndicator ]
    });
}
pub(super) fn refresh_save_status(
    save: Res<SaveState>,
    mut texts: Query<&mut Text, With<SaveIndicator>>,
    mut errors: Query<&mut Text, (With<SaveStatus>, Without<SaveIndicator>)>,
) {
    for mut text in &mut texts {
        let message = if save.task.is_some() { "Saving…" } else { "" };
        if text.0 != message {
            text.0 = message.into();
        }
    }
    for mut text in &mut errors {
        if let Some(error) = &save.error {
            if text.0 != *error {
                text.0.clone_from(error);
            }
        }
    }
}

fn date(seconds: u64) -> String {
    // Gregorian civil date from Unix days, independent of the host's locale.
    let z = (seconds / 86400) as i64 + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = mp + if mp < 10 { 3 } else { -9 };
    let y = y + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02} UTC",
        seconds / 3600 % 24,
        seconds / 60 % 60
    )
}
