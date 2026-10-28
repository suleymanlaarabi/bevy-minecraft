//! Always-visible frame rate counter in the top-left corner.
//!
//! Development builds use Bevy's richer FPS overlay at the same place instead.

use bevy::{picking::prelude::Pickable, prelude::*};
use core::time::Duration;

const REFRESH: Duration = Duration::from_millis(250);

#[derive(Component)]
struct FpsText;

pub(super) struct FpsCounterPlugin;

impl Plugin for FpsCounterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn)
            .add_systems(Update, update);
    }
}

fn spawn(mut commands: Commands) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            top: px(6),
            left: px(8),
            ..default()
        },
        Text::new("FPS"),
        TextColor(Color::WHITE),
        TextShadow::default(),
        GlobalZIndex(1000),
        Pickable::IGNORE,
        FpsText,
    ));
}

/// Averages frames over each refresh window and rewrites the text only then.
fn update(
    time: Res<Time<Real>>,
    mut window: Local<(Duration, u32)>,
    mut text: Single<&mut Text, With<FpsText>>,
) {
    let (elapsed, frames) = &mut *window;
    *elapsed += time.delta();
    *frames += 1;
    if *elapsed < REFRESH {
        return;
    }
    let fps = *frames as f32 / elapsed.as_secs_f32();
    *window = default();
    text.0.clear();
    use std::fmt::Write;
    let _ = write!(text.0, "{fps:.0} FPS");
}
