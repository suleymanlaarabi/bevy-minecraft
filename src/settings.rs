use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
};

#[derive(Resource, SettingsGroup, Reflect, Default)]
#[reflect(Resource, SettingsGroup, Default)]
pub struct GraphicsSettings {}

pub struct GameSettingsPlugin {}
