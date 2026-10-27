//! Modal roots release cursor capture for their lifetime, independently of game states.
use bevy_app::{App, Plugin, PreUpdate};
use bevy_ecs::prelude::*;
use bevy_window::{CursorGrabMode, CursorOptions};

/// Add to a live modal root. Capture resumes when the last such root disappears.
#[derive(Component, Default, Clone, Copy)]
pub struct FreeCursor;

/// Opt-in policy: capture window cursors unless a modal root requests otherwise.
/// Run after state transitions, before input consumers in the fixed main loop.
pub struct CursorCapturePlugin;
impl Plugin for CursorCapturePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(PreUpdate, sync_cursor);
    }
}

fn sync_cursor(modals: Query<(), With<FreeCursor>>, mut cursors: Query<&mut CursorOptions>) {
    let visible = !modals.is_empty();
    let grab = if visible {
        CursorGrabMode::None
    } else {
        CursorGrabMode::Locked
    };
    for mut cursor in &mut cursors {
        if cursor.visible != visible || cursor.grab_mode != grab {
            cursor.visible = visible;
            cursor.grab_mode = grab;
        }
    }
}
