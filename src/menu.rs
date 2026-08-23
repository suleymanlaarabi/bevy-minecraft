use bevy::{
    feathers::{
        controls::{ButtonVariant, FeathersButton},
        theme::{ThemeBackgroundColor, ThemedText},
        tokens,
    },
    prelude::*,
    settings::SaveSettingsSync,
    ui_widgets::Activate,
};

use crate::game::GameState;

pub struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Menu), menu.spawn());
    }
}

fn menu() -> impl SceneList {
    bsn_list! [
        Node {
            width: percent(100),
            height: percent(100),
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(12),
        }
        ThemeBackgroundColor(tokens::WINDOW_BG)
        DespawnOnExit::<GameState>(GameState::Menu)
        Children [
            @FeathersButton {
                @caption: bsn! { Text("Play") ThemedText },
                @variant: ButtonVariant::Primary,
            }
            Node { width: px(250), height: px(52) }
            on(play),
            @FeathersButton {
                @caption: bsn! { Text("Settings") ThemedText }
            }
            Node { width: px(250), height: px(52) }
            on(open_settings),
            @FeathersButton {
                @caption: bsn! { Text("Exit") ThemedText },
                @variant: ButtonVariant::Plain,
            }
            Node { width: px(250), height: px(52) }
            on(exit),
        ]
    ]
}

fn play(_activate: On<Activate>, mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Game);
}

fn open_settings(_activate: On<Activate>, mut next_state: ResMut<NextState<GameState>>) {
    next_state.set(GameState::Settings);
}

fn exit(_activate: On<Activate>, mut commands: Commands, mut app_exit: MessageWriter<AppExit>) {
    commands.queue(SaveSettingsSync::IfChanged);
    app_exit.write(AppExit::Success);
}
