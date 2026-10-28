use bevy::{
    color::palettes::css::BLACK,
    image::{
        ImageAddressMode, ImageFilterMode, ImageLoaderSettings, ImageSampler,
        ImageSamplerDescriptor,
    },
    light::{NotShadowCaster, NotShadowReceiver},
    prelude::*,
    transform::TransformSystems,
};
use hollow_features::{motion::InterpolationAlpha, spatial::FollowSystems};

use crate::{
    character::CharacterMotion,
    game::GameState,
    inventory::{InventoryState, ItemKind, ItemStack},
    player::{Player, PlayerCamera},
    voxel::{VoxelState, VoxelWorld},
};

const BLOCK_REACH: f32 = 5.0;
const BLOCK_OUTLINE_SIZE: f32 = 1.01;

#[derive(Debug, Clone, Copy)]
struct BlockTarget {
    destroy: IVec3,
    place: IVec3,
    normal: Vec3,
    hit: Vec3,
    /// Eye look direction, independent of the third-person camera.
    look: Vec3,
}

#[derive(Component, Default)]
pub(super) struct BlockTargetState(Option<BlockTarget>);

#[derive(Debug, Clone, Copy)]
struct BreakProgress {
    position: IVec3,
    state: VoxelState,
    slot: usize,
    tool: Option<ItemStack>,
    progress: f32,
    hit_elapsed: f32,
}

#[derive(Component, Default)]
pub(super) struct BreakingState(Option<BreakProgress>, u64);

#[derive(Component)]
struct BreakingOverlay;

#[derive(Resource)]
struct BreakingAssets {
    mesh: Handle<Mesh>,
    materials: [Handle<hollow_render::material::ModelMaterial>; 10],
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PlayerInteraction;

pub(super) fn register(app: &mut App) {
    app.add_systems(
        PreStartup,
        prepare_breaking_assets.after(crate::voxel::prepare_assets),
    )
    .add_systems(OnEnter(GameState::Game), spawn_breaking_overlay)
    .add_systems(OnEnter(InventoryState::Open), cancel_breaking)
    .add_systems(
        PostUpdate,
        (
            update_block_target,
            update_breaking,
            handle_block_placement,
            sync_breaking_overlay,
            draw_targeted_block,
        )
            .chain()
            .in_set(PlayerInteraction)
            .after(FollowSystems)
            .before(TransformSystems::Propagate)
            .run_if(crate::simulation::running)
            .run_if(crate::player::alive)
            .run_if(in_state(InventoryState::Closed))
            .run_if(crate::devtools::panel_closed),
    );
}

fn prepare_breaking_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<hollow_render::material::ModelMaterial>>,
    voxel_assets: Res<crate::voxel::VoxelAssets>,
) {
    // Keep a single reusable overlay; changing stages only swaps its material handle.
    let mesh = meshes.add(Cuboid::new(1.002, 1.002, 1.002));
    let vanilla = (0..10).all(|stage| {
        crate::asset_directory()
            .join(format!("voxel/breaking/vanilla/destroy_stage_{stage}.png"))
            .is_file()
    });
    let stages = core::array::from_fn(|stage| {
        let texture = if vanilla {
            format!("voxel/breaking/vanilla/destroy_stage_{stage}.png")
        } else {
            format!("voxel/breaking/stage_{stage}.png")
        };
        let mut material = materials
            .get(&voxel_assets.models)
            .expect("shared block material")
            .clone();
        material.cracks = Some(
            asset_server
                .load_builder()
                .with_settings(|settings: &mut ImageLoaderSettings| {
                    settings.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
                        address_mode_u: ImageAddressMode::ClampToEdge,
                        address_mode_v: ImageAddressMode::ClampToEdge,
                        mag_filter: ImageFilterMode::Nearest,
                        min_filter: ImageFilterMode::Nearest,
                        mipmap_filter: ImageFilterMode::Nearest,
                        ..default()
                    });
                })
                .load(texture),
        );
        materials.add(material)
    });
    commands.insert_resource(BreakingAssets {
        mesh,
        materials: stages,
    });
}

fn spawn_breaking_overlay(mut commands: Commands, assets: Res<BreakingAssets>) {
    commands.spawn((
        BreakingOverlay,
        Mesh3d(assets.mesh.clone()),
        MeshMaterial3d(assets.materials[0].clone()),
        Transform::default(),
        Visibility::Hidden,
        NotShadowCaster,
        NotShadowReceiver,
        DespawnOnExit(GameState::Game),
    ));
}

/// Rays start at the player's eyes: third-person cameras sit behind or in front of them.
fn update_block_target(
    interpolation: Res<InterpolationAlpha>,
    player: Single<(&Transform, &CharacterMotion), With<Player>>,
    camera: Single<(&PlayerCamera, &mut BlockTargetState)>,
    voxel_world: VoxelWorld,
) {
    let (transform, motion) = *player;
    let (camera, mut target) = camera.into_inner();
    let origin = transform.translation + Vec3::Y * motion.eye_offset(interpolation.0);
    let look = camera.look(transform.rotation) * Dir3::NEG_Z;
    target.0 = targeted_state(origin, look, |position| voxel_world.state(position));
}

fn update_breaking(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    clock: Res<crate::simulation::GameplayClock>,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    mut breaking: Single<&mut BreakingState, With<PlayerCamera>>,
    mut swing: Single<&mut super::skin::ArmSwing, With<PlayerCamera>>,
    voxel_world: VoxelWorld,
    player: Single<
        (
            Entity,
            &crate::inventory::PlayerInventory,
            &crate::inventory::HotbarSelection,
            &crate::character::CharacterState,
            &crate::survival::UsingItem,
            &crate::survival::ActiveEffects,
        ),
        With<Player>,
    >,
) {
    let elapsed = clock.0.saturating_sub(breaking.1).min(2) as f32 * 0.05;
    breaking.1 = clock.0;
    if !mouse.pressed(MouseButton::Left) {
        breaking.0 = None;
        return;
    }
    let Some(target) = target.0 else {
        breaking.0 = None;
        return;
    };
    let Some(state) = voxel_world.state(target.destroy) else {
        breaking.0 = None;
        return;
    };
    let (actor, inventory, selection, player_state, using, effects) = *player;
    if using.0.is_some() {
        breaking.0 = None;
        return;
    }
    let tool = selection.stack(inventory);
    let Some(mut outcome) = crate::inventory::mining::outcome(state, tool, player_state) else {
        breaking.0 = None;
        return;
    };
    outcome.duration /= crate::survival::mining_speed(effects);
    swing.request_mining();

    if breaking.0.as_ref().is_none_or(|progress| {
        progress.position != target.destroy
            || progress.state != state
            || progress.slot != selection.0
            || progress.tool != tool
    }) {
        breaking.0 = Some(BreakProgress {
            position: target.destroy,
            state,
            slot: selection.0,
            tool,
            progress: 0.0,
            hit_elapsed: 0.25,
        });
    }
    let progress = breaking.0.as_mut().unwrap();
    progress.progress = if outcome.duration == 0.0 {
        1.0
    } else {
        progress.progress + elapsed / outcome.duration
    };
    progress.hit_elapsed += elapsed;
    if progress.progress < 1.0 && progress.hit_elapsed >= 0.25 {
        progress.hit_elapsed %= 0.25;
        commands.trigger(crate::audio::Sound::block(
            state.kind(),
            crate::audio::BlockSound::Hit,
            target.destroy,
        ));
    }
    if progress.progress < 1.0 {
        return;
    }

    let mut edits = vec![crate::voxel::edits::CheckedEdit {
        position: target.destroy,
        expected: state,
        state: crate::content::fluid_catalog().residual(state),
    }];
    if let Some(other) = crate::voxel::rules::paired_position(state, target.destroy) {
        let Some(other_state) = voxel_world.state(other) else {
            return;
        };
        if crate::voxel::rules::paired_states(state, other_state) {
            edits.push(crate::voxel::edits::CheckedEdit {
                position: other,
                expected: other_state,
                state: crate::content::fluid_catalog().residual(other_state),
            });
        }
    }
    commands.trigger(crate::voxel::edits::EditBatch {
        edits,
        consume: None,
        harvest: Some(crate::block_behavior::loot::Harvest {
            position: target.destroy,
            state,
            offset: target.normal * 0.55,
            velocity: target.normal * 0.35 + Vec3::Y * 1.2,
        }),
        tool: Some(crate::voxel::edits::MiningAction {
            actor,
            slot: selection.0,
            expected: tool,
        }),
    });
    breaking.0 = None;
}

fn handle_block_placement(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    voxel_world: VoxelWorld,
    player: Single<
        (
            Entity,
            &crate::inventory::PlayerInventory,
            &crate::inventory::HotbarSelection,
            &crate::character::CharacterMovement,
            &mut crate::survival::UseInput,
        ),
        With<Player>,
    >,
    collisions: crate::block_behavior::interaction::BlockCollisions,
    targets: crate::block_behavior::interaction::BlockTargets,
    mut swing: Single<&mut super::skin::ArmSwing, With<PlayerCamera>>,
) {
    use crate::{
        block_behavior::interaction::BlockUse,
        voxel::edits::{CheckedEdit, EditBatch},
    };
    let (actor, inventory, selection, movement, mut use_input) = player.into_inner();
    use_input.0 = None;
    if !mouse.pressed(MouseButton::Right) {
        return;
    }
    let selected = selection.stack(inventory).map(|stack| stack.item());
    let bypass = movement.sneak && selected.is_some_and(|item| item.block_kind().is_some());
    if !bypass
        && let Some(target) = target.0
        && let Some(entity) = targets.target(target.destroy)
        && let Some(state) = voxel_world.state(target.destroy)
    {
        if !mouse.just_pressed(MouseButton::Right) {
            return;
        }
        commands.trigger(BlockUse {
            entity,
            actor,
            state,
            look: target.look,
        });
        swing.request();
        return;
    }
    if (!selected.is_some_and(|item| item.block_kind().is_some()) || target.0.is_none())
        && let Some(slot) = crate::survival::choose_use_slot(inventory, selection.0)
    {
        use_input.0 = Some(slot);
        return;
    }
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(target) = target.0 else {
        return;
    };
    let Some(kind) = selected.and_then(ItemKind::block_kind) else {
        return;
    };
    let Some(changes) = crate::voxel::rules::placement(
        &voxel_world,
        kind,
        target.destroy,
        target.place,
        target.hit,
        target.normal,
        target.look,
    ) else {
        return;
    };
    let mut edits = Vec::with_capacity(changes.len());
    for (position, state) in changes {
        if collisions.intersects(position, state) {
            return;
        }
        let Some(expected) = voxel_world.state(position) else {
            return;
        };
        edits.push(CheckedEdit {
            position,
            expected,
            state,
        });
    }
    commands.trigger(EditBatch {
        edits,
        consume: Some((actor, ItemKind::for_block(kind))),
        harvest: None,
        tool: None,
    });
    swing.request();
}

fn sync_breaking_overlay(
    breaking: Single<&BreakingState, With<PlayerCamera>>,
    overlay: Single<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<hollow_render::material::ModelMaterial>,
            &mut Mesh3d,
        ),
        With<BreakingOverlay>,
    >,
    assets: Res<BreakingAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
    render_source: Res<crate::voxel::render::MinecraftRenderSource>,
    mut previous: Local<Option<(IVec3, VoxelState)>>,
) {
    let (mut transform, mut visibility, mut material, mut mesh) = overlay.into_inner();
    let Some(progress) = breaking.0 else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Visible);
    let key = (progress.position, progress.state);
    if *previous != Some(key) {
        let geometry = hollow_render::model_mesh::state_geometry(
            &*render_source,
            hollow_core::StateId(progress.state.id()),
            progress.position,
        )
        .into_mesh();
        if let Some(mut target) = meshes.get_mut(&mesh.0) {
            *target = geometry;
        } else {
            mesh.0 = meshes.add(geometry);
        }
        *previous = Some(key);
    }
    let translation = progress.position.as_vec3();
    if transform.translation != translation {
        transform.translation = translation;
    }
    let stage = ((progress.progress * 10.0).floor() as usize).min(9);
    if material.0 != assets.materials[stage] {
        material.0 = assets.materials[stage].clone();
    }
}

fn cancel_breaking(
    mut breaking: Query<&mut BreakingState, With<PlayerCamera>>,
    mut overlays: Query<&mut Visibility, With<BreakingOverlay>>,
) {
    for mut state in &mut breaking {
        state.0 = None;
    }
    for mut visibility in &mut overlays {
        *visibility = Visibility::Hidden;
    }
}

fn draw_targeted_block(
    mut gizmos: Gizmos,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    world: VoxelWorld,
) {
    let Some(target) = target.0 else {
        return;
    };
    let Some(state) = world.state(target.destroy) else {
        return;
    };
    for shape in state.selection_boxes(target.destroy) {
        gizmos.cube(
            Transform::from_translation(target.destroy.as_vec3() + shape.center())
                .with_scale(shape.size() * BLOCK_OUTLINE_SIZE),
            BLACK,
        );
    }
}

fn targeted_state(
    origin: Vec3,
    direction: Dir3,
    sample: impl FnMut(IVec3) -> Option<VoxelState>,
) -> Option<BlockTarget> {
    let target = hollow_engine::raycast::raycast(
        origin,
        direction,
        BLOCK_REACH,
        1,
        sample,
        |owner, state| (!state.is_air()).then(|| state.selection_boxes(owner)),
    )?;
    Some(BlockTarget {
        destroy: target.owner,
        place: target.owner + target.normal.round().as_ivec3(),
        normal: target.normal,
        hit: target.location,
        look: direction.as_vec3(),
    })
}
