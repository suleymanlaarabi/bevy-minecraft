use bevy::prelude::*;

use crate::game::GameState;

pub struct GameMenuPlugin;

impl Plugin for GameMenuPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(GameState::Menu), menu.spawn())
            .add_systems(
                Update,
                (handle_play_button, handle_exit_button, button_hover_system)
                    .run_if(in_state(GameState::Menu)),
            );
    }
}

fn handle_play_button(
    mut next_state: ResMut<NextState<GameState>>,
    query: Query<&Interaction, (Changed<Interaction>, With<PlayButton>)>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            next_state.set(GameState::Game);
        }
    }
}

fn handle_exit_button(
    mut app_exit: MessageWriter<AppExit>,
    query: Query<&Interaction, (Changed<Interaction>, With<ExitButton>)>,
) {
    for interaction in &query {
        if *interaction == Interaction::Pressed {
            app_exit.write(AppExit::Success);
        }
    }
}

fn button<M: Component + Clone + Default + Unpin>() -> impl Scene {
    bsn! {
        Button
        Interaction::default()
        M::default()
        Node {
            width: px(250),
            height: px(80),
            display: Display::Flex,
            justify_content: JustifyContent::Center,
            align_items: AlignItems::Center
        }
        BackgroundColor(Color::srgb_u8(30, 30, 30))
    }
}

#[derive(Component, Clone, Copy, Default)]
struct PlayButton;

#[derive(Component, Clone, Copy, Default)]
struct ExitButton;

fn button_hover_system(
    mut query: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<Button>)>,
) {
    for (interaction, mut bg_color) in &mut query {
        match *interaction {
            Interaction::Pressed => {
                *bg_color = BackgroundColor(Color::srgb_u8(70, 70, 70));
            }
            Interaction::Hovered => {
                *bg_color = BackgroundColor(Color::srgb_u8(50, 50, 50));
            }
            Interaction::None => {
                *bg_color = BackgroundColor(Color::srgb_u8(30, 30, 30));
            }
        }
    }
}

fn menu() -> impl SceneList {
    bsn_list! [
        Node {
            width: percent(100),
            height: percent(100),
            display: Display::Flex,
            flex_direction: FlexDirection::Column,
            align_items: AlignItems::Center,
            justify_content: JustifyContent::Center,
            row_gap: px(20),
        }
        BackgroundColor(Color::srgb_u8(15, 15, 18))
        DespawnOnExit::<GameState>(GameState::Menu)
        Children [
            button::<PlayButton>() Children [
                Text::new("Play")
                TextFont {
                    font_size: px(32)
                }
                TextColor(Color::WHITE)
            ],
            button::<ExitButton>() Children [
                Text::new("Exit")
                TextFont {
                    font_size: px(32)
                }
                TextColor(Color::WHITE)
            ]
        ],
        Camera2d
        DespawnOnExit::<GameState>(GameState::Menu)
    ]
}
