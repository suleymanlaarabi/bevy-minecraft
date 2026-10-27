//! Shared skin assets, articulated player rigs, and the inventory portrait.
mod mesh;
mod texture;

use crate::inventory::InventoryState;
use bevy::prelude::*;
use std::collections::HashMap;

mod animation;
mod assets;
mod held_item;
mod rig;
mod view_model;
use animation::{animate_rigs, animate_world};
use assets::{load_skin, setup};
use held_item::{FirstPersonRoot, HeldItem, HeldItemAssets, HeldItemPart};
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

#[derive(Resource, Default)]
pub(crate) struct SkinAssets {
    pub preview: Handle<Image>,
    arm_view: Handle<Image>,
    source: Handle<Image>,
    loaded: bool,
    texture: Handle<Image>,
    base: Handle<StandardMaterial>,
    outer: Handle<StandardMaterial>,
    arm_base: Handle<StandardMaterial>,
    arm_outer: Handle<StandardMaterial>,
    classic: [[Handle<Mesh>; 6]; 2],
    armor_meshes: [Handle<Mesh>; 6],
    leggings_meshes: [Handle<Mesh>; 6],
    armor: HashMap<String, Handle<StandardMaterial>>,
}

#[derive(Component, Clone, Default)]
pub(crate) struct SkinPortrait;

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
#[derive(Component)]
struct HeldItemLight;

/// Linear brightness of the first-person arm in the middle of the night.
const ARM_NIGHT_LEVEL: f32 = 0.25;

/// The arm is unlit and drawn by its own camera at a fixed exposure: dim it with the
/// daylight so it does not glow against the night.
fn dim_arm_with_daylight(
    sky: Res<crate::sky::SkyState>,
    assets: Res<SkinAssets>,
    held: Query<Ref<MeshMaterial3d<StandardMaterial>>, With<HeldItemPart>>,
    damage: Single<Ref<crate::survival::DamageState>, With<crate::player::Player>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut cameras: Query<&mut AmbientLight, With<ArmCamera>>,
    mut lights: Query<&mut DirectionalLight, With<HeldItemLight>>,
) {
    if !sky.is_changed() && !damage.is_changed() && !held.iter().any(|material| material.is_added())
    {
        return;
    }
    let level = ARM_NIGHT_LEVEL + (1.0 - ARM_NIGHT_LEVEL) * (1.0 - sky.night);
    let tint = if damage.flash_ticks > 0 { 0.4 } else { 1.0 };
    let color = Color::linear_rgb(level, level * tint, level * tint);
    for mut ambient in &mut cameras {
        if ambient.brightness != 500. * level {
            ambient.brightness = 500. * level;
        }
    }
    for mut light in &mut lights {
        if light.illuminance != 6000. * level {
            light.illuminance = 6000. * level;
        }
    }
    let body_color = Color::linear_rgb(1.0, tint, tint);
    for (handle, color) in held
        .iter()
        .map(|material| (&material.into_inner().0, color))
        .chain([
            (&assets.arm_base, color),
            (&assets.arm_outer, color),
            (&assets.base, body_color),
            (&assets.outer, body_color),
        ])
    {
        // Reading first avoids notifying the renderer when the material is unchanged.
        if materials
            .get(handle)
            .is_some_and(|material| material.base_color != color)
            && let Some(mut material) = materials.get_mut(handle)
        {
            material.base_color = color;
        }
    }
}

pub(super) fn register(app: &mut App) {
    app.init_resource::<PlayerView>()
        .init_resource::<HeldItemAssets>()
        .init_asset::<held_item::HeldManifest>()
        .init_asset_loader::<held_item::HeldManifestLoader>()
        .init_resource::<SkinAssets>()
        .add_systems(Startup, (setup, held_item::setup))
        .add_systems(Update, (load_skin, sync_parts).chain_weak())
        .add_systems(OnEnter(InventoryState::Open), spawn_preview)
        .add_systems(OnEnter(InventoryState::Open), cancel_swing)
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
            animate_rigs
                .run_if(in_state(InventoryState::Open))
                .run_if(any_with_component::<SkinPortrait>),
        )
        .add_systems(PostUpdate, resize_arm_target.run_if(crate::world::playing))
        .add_systems(
            PostUpdate,
            animate_world
                .run_if(crate::world::playing)
                .before(bevy::transform::TransformSystems::Propagate),
        )
        .add_systems(
            PostUpdate,
            (
                advance_swing,
                held_item::asset_events,
                held_item::select,
                held_item::advance_equipment,
                held_item::build_visual,
                dim_arm_with_daylight,
                animate_first_person,
            )
                .chain_weak()
                .after(crate::block_behavior::GameplaySystems::Derived)
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(crate::world::playing),
        );
}
