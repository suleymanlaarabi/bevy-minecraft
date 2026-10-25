//! Shared skin assets, articulated player rigs, and the inventory portrait.
mod mesh;
mod texture;

use crate::inventory::InventoryState;
use bevy::{
    prelude::*,
    settings::{ReflectSettingsGroup, SettingsGroup},
    tasks::Task,
};
use std::collections::HashMap;

mod animation;
mod appearance;
mod assets;
mod rig;
mod view_model;
use animation::{animate_rigs, animate_world};
use appearance::{finish_import, import_skin, sync_labels};
pub(crate) use appearance::{reset_skin, toggle_layers, toggle_model};
use assets::{load_skin, setup};
use rig::{spawn_preview, spawn_world_rig, sync_parts};
pub(crate) use view_model::ArmSwing;
use view_model::{
    advance_swing, animate_first_person, cancel_swing, first_person_pose, resize_arm_target,
};

const PREVIEW_LAYER: usize = 2;
const ARM_LAYER: usize = 3;

#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlayerView {
    #[default]
    FirstPerson,
    Back,
    Front,
}

fn cycle_view(keyboard: Res<ButtonInput<KeyCode>>, mut view: ResMut<PlayerView>) {
    if keyboard.just_pressed(KeyCode::F5) {
        *view = match *view {
            PlayerView::FirstPerson => PlayerView::Back,
            PlayerView::Back => PlayerView::Front,
            PlayerView::Front => PlayerView::FirstPerson,
        };
    }
}

#[derive(Reflect, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[type_path = "hollow::player"]
pub enum SkinModel {
    #[default]
    Classic,
    Slim,
}

#[derive(Resource, SettingsGroup, Reflect, Clone, Debug, PartialEq)]
#[component(immutable)]
#[reflect(Resource, SettingsGroup, Default)]
#[settings_group(group = "skin")]
#[type_path = "hollow::player"]
pub struct SkinSettings {
    pub texture: String,
    pub model: SkinModel,
    pub outer_layer: bool,
}
impl Default for SkinSettings {
    fn default() -> Self {
        Self {
            texture: "skin.png".into(),
            model: SkinModel::Classic,
            outer_layer: true,
        }
    }
}

#[derive(Resource, Default)]
pub(crate) struct SkinAssets {
    pub preview: Handle<Image>,
    arm_view: Handle<Image>,
    source: Handle<Image>,
    requested: String,
    loaded: bool,
    fallback: bool,
    texture: Handle<Image>,
    base: Handle<StandardMaterial>,
    outer: Handle<StandardMaterial>,
    arm_base: Handle<StandardMaterial>,
    arm_outer: Handle<StandardMaterial>,
    classic: [[Handle<Mesh>; 6]; 2],
    slim: [[Handle<Mesh>; 6]; 2],
    armor_meshes: [Handle<Mesh>; 6],
    leggings_meshes: [Handle<Mesh>; 6],
    armor: HashMap<String, Handle<StandardMaterial>>,
    pub status: String,
}

#[derive(Component, Clone, Default)]
pub(crate) struct SkinPortrait;

#[derive(Resource, Default)]
pub(crate) struct SkinImport(Option<Task<Result<String, String>>>);

#[derive(Component)]
struct SkinRig {
    preview: bool,
    phase: f32,
}
#[derive(Component)]
struct SkinPart {
    rig: Entity,
    part: usize,
    layer: usize,
    armor: bool,
}
#[derive(Component)]
struct FirstPersonArm;
#[derive(Component)]
struct ArmCamera;
#[derive(Component)]
struct ArmView;
#[derive(Component, Clone, Default)]
pub(crate) struct SkinModelLabel;
#[derive(Component, Clone, Default)]
pub(crate) struct SkinLayersLabel;
#[derive(Component, Clone, Default)]
pub(crate) struct SkinStatusLabel;

pub(super) fn register(app: &mut App) {
    app.init_resource::<PlayerView>()
        .init_resource::<SkinAssets>()
        .init_resource::<SkinImport>()
        .add_systems(Startup, setup)
        .add_systems(
            Update,
            (import_skin, finish_import, load_skin, sync_parts).chain(),
        )
        .add_systems(OnEnter(InventoryState::Open), spawn_preview)
        .add_systems(OnEnter(InventoryState::Open), cancel_swing)
        .add_systems(Update, resize_arm_target.run_if(crate::world::playing))
        .add_systems(
            Update,
            (
                spawn_world_rig,
                cycle_view.run_if(in_state(InventoryState::Closed)),
            )
                .run_if(crate::world::playing),
        )
        .add_systems(
            Update,
            (animate_rigs, sync_labels).run_if(in_state(InventoryState::Open)),
        )
        .add_systems(
            PostUpdate,
            animate_world
                .run_if(crate::world::playing)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            (advance_swing, animate_first_person)
                .chain()
                .after(crate::player::interaction::PlayerInteraction)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(crate::world::playing),
        );
}
