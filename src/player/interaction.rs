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

use crate::{
    character::{Aabb, CharacterBody, CharacterMotion, CharacterState},
    dropped_item::DroppedItemCommandsExt,
    game::GameState,
    inventory::{InventoryState, ItemKind, ItemStack},
    player::{Player, PlayerCamera},
    spatial::FollowSystems,
    voxel::{VoxelCommandsExt, VoxelKind, VoxelState, VoxelWorld},
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
    kind: VoxelKind,
    state: VoxelState,
    elapsed: f32,
}

#[derive(Component, Default)]
pub(super) struct BreakingState(Option<BreakProgress>);

#[derive(Component)]
struct BreakingOverlay;

#[derive(Resource)]
struct BreakingAssets {
    mesh: Handle<Mesh>,
    materials: [Handle<crate::voxel::render::ModelMaterial>; 10],
}

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(super) struct PlayerInteraction;

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
            .run_if(crate::world::playing)
            .run_if(in_state(InventoryState::Closed))
            .run_if(crate::devtools::panel_closed),
    );
}

fn prepare_breaking_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<crate::voxel::render::ModelMaterial>>,
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
    player: Single<(&Transform, &CharacterMotion), With<Player>>,
    camera: Single<(&PlayerCamera, &mut BlockTargetState)>,
    voxel_world: VoxelWorld,
) {
    let (transform, motion) = *player;
    let (camera, mut target) = camera.into_inner();
    let origin = transform.translation + Vec3::Y * motion.eye_offset();
    let look = camera.look(transform.rotation) * Dir3::NEG_Z;
    target.0 = targeted_state(origin, look, |position| voxel_world.state(position));
}

fn update_breaking(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    mut breaking: Single<&mut BreakingState, With<PlayerCamera>>,
    mut swing: Single<&mut super::skin::ArmSwing, With<PlayerCamera>>,
    voxel_world: VoxelWorld,
) {
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
    let kind = state.kind();
    let Some(duration) = kind.break_duration() else {
        breaking.0 = None;
        return;
    };
    swing.request();

    if breaking
        .0
        .as_ref()
        .is_none_or(|progress| progress.position != target.destroy || progress.state != state)
    {
        breaking.0 = Some(BreakProgress {
            position: target.destroy,
            kind,
            state,
            elapsed: 0.0,
        });
    }
    let progress = breaking.0.as_mut().unwrap();
    progress.elapsed += time.delta_secs();
    if progress.elapsed < duration || voxel_world.state(target.destroy) != Some(state) {
        return;
    }

    commands.set_voxel_state(target.destroy, state.removed());
    if let Some(other) = crate::voxel::rules::paired_position(state, target.destroy)
        && let Some(other_state) = voxel_world.state(other)
        && crate::voxel::rules::paired_states(state, other_state)
    {
        commands.set_voxel_state(other, other_state.removed());
    }
    if let Some(item) = ItemKind::from_voxel(kind) {
        let center = target.destroy.as_vec3() + Vec3::splat(0.5);
        commands.spawn_dropped_item(
            ItemStack::new(item, 1),
            center + target.normal * 0.55,
            target.normal * 0.35 + Vec3::Y * 1.2,
        );
    }
    breaking.0 = None;
}

fn handle_block_placement(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    voxel_world: VoxelWorld,
    players: Query<(&CharacterMotion, &CharacterBody, &CharacterState), With<Player>>,
    mut inventory: Single<&mut crate::inventory::PlayerInventory, With<Player>>,
    mut swing: Single<&mut super::skin::ArmSwing, With<PlayerCamera>>,
) {
    let Some(target) = target.0 else {
        return;
    };
    if !mouse.just_pressed(MouseButton::Right) {
        return;
    }
    let Some(ItemKind::Block(kind)) = inventory.selected_stack().map(|stack| stack.item()) else {
        return;
    };
    let Some(edits) = crate::voxel::rules::placement(
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
    for &(position, state) in &edits {
        if block_contains_player(&players, position, state) {
            return;
        }
    }
    if inventory.consume_selected(1) {
        swing.request();
        for (position, state) in edits {
            commands.set_voxel_state(position, state);
        }
    }
}

fn sync_breaking_overlay(
    breaking: Single<&BreakingState, With<PlayerCamera>>,
    overlay: Single<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<crate::voxel::render::ModelMaterial>,
            &mut Mesh3d,
        ),
        With<BreakingOverlay>,
    >,
    assets: Res<BreakingAssets>,
    mut meshes: ResMut<Assets<Mesh>>,
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
        let geometry =
            crate::voxel::render::model_mesh::state_geometry(progress.state, progress.position)
                .into_dynamic_mesh();
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
    let duration = progress
        .kind
        .break_duration()
        .expect("active breaking state is breakable");
    let stage = ((progress.elapsed / duration.max(0.001) * 10.0).floor() as usize).min(9);
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

fn block_contains_player(
    players: &Query<(&CharacterMotion, &CharacterBody, &CharacterState), With<Player>>,
    position: IVec3,
    state: VoxelState,
) -> bool {
    let origin = position.as_dvec3();
    players.iter().any(|(motion, body, character)| {
        let player = body.aabb(motion.position, character.pose);
        state.collision_boxes(position).any(|shape| {
            player.intersects(&Aabb {
                min: origin + shape.min.as_dvec3(),
                max: origin + shape.max.as_dvec3(),
            })
        })
    })
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
    mut sample: impl FnMut(IVec3) -> Option<VoxelState>,
) -> Option<BlockTarget> {
    let direction = direction.as_vec3();
    let mut cell = origin.floor().as_ivec3();
    let step = direction.signum().as_ivec3();
    let delta = direction.recip().abs();
    let mut next = Vec3::splat(f32::INFINITY);
    for axis in 0..3 {
        if direction[axis] != 0.0 {
            let boundary = cell[axis] as f32 + if step[axis] > 0 { 1.0 } else { 0.0 };
            next[axis] = (boundary - origin[axis]) / direction[axis];
        }
    }
    let mut entered = 0.0;
    loop {
        sample(cell)?; // An unavailable chunk stops traversal.
        let end = next.min_element().min(BLOCK_REACH);
        let mut best: Option<(f32, BlockTarget)> = None;
        // Shapes can overhang horizontally, rise above fences, or shift below plants.
        for dy in -1..=1 {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let owner = cell + IVec3::new(dx, dy, dz);
                    let Some(state) = sample(owner) else {
                        continue;
                    };
                    if state.native().is_air() || state.kind() == VoxelKind::Water {
                        continue;
                    }
                    let mut inspect = |shape: crate::voxel::state::BlockBox| {
                        let Some((distance, normal)) =
                            shape.translated(owner.as_vec3()).ray(origin, direction)
                        else {
                            return;
                        };
                        if distance + 1e-5 < entered
                            || distance > end + 1e-5
                            || distance > BLOCK_REACH
                        {
                            return;
                        }
                        if best.as_ref().is_none_or(|(old, _)| distance < *old) {
                            best = Some((
                                distance,
                                BlockTarget {
                                    destroy: owner,
                                    place: owner + normal.round().as_ivec3(),
                                    normal,
                                    hit: origin + direction * distance,
                                    look: direction,
                                },
                            ));
                        }
                    };
                    if state.kind() == VoxelKind::Lava {
                        inspect(crate::voxel::state::BlockBox {
                            min: Vec3::ZERO,
                            max: Vec3::ONE,
                        });
                    } else {
                        for shape in state.selection_boxes(owner) {
                            inspect(shape);
                        }
                    }
                }
            }
        }
        if let Some((_, target)) = best {
            return Some(target);
        }
        let axis = if next.x <= next.y && next.x <= next.z {
            0
        } else if next.y <= next.z {
            1
        } else {
            2
        };
        if next[axis] > BLOCK_REACH {
            return None;
        }
        entered = next[axis];
        cell[axis] += step[axis];
        next[axis] += delta[axis];
    }
}
#[cfg(test)]
fn targeted_voxel(
    origin: Vec3,
    direction: Dir3,
    mut sample: impl FnMut(IVec3) -> Option<VoxelKind>,
) -> Option<BlockTarget> {
    targeted_state(origin, direction, |p| {
        sample(p).map(VoxelKind::default_state)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voxel_ray_targets_leaves_through_water_and_keeps_the_entry_face() {
        let target = targeted_voxel(Vec3::splat(0.5), Dir3::X, |cell| {
            Some(match cell.x {
                1 => VoxelKind::Water,
                2 => VoxelKind::Leaves,
                _ => VoxelKind::Air,
            })
        })
        .unwrap();
        assert_eq!(target.destroy, IVec3::new(2, 0, 0));
        assert_eq!(target.place, IVec3::new(1, 0, 0));
        assert_eq!(target.normal, Vec3::NEG_X);
    }

    #[test]
    fn voxel_ray_handles_negative_boundary_origins_and_axis_aligned_rays() {
        let target = targeted_voxel(Vec3::new(0.0, 0.5, 0.5), Dir3::NEG_X, |cell| {
            Some(if cell.x == -1 {
                VoxelKind::Stone
            } else {
                VoxelKind::Air
            })
        })
        .unwrap();
        assert_eq!(target.destroy, IVec3::new(-1, 0, 0));
        assert_eq!(target.place, IVec3::ZERO);
        assert_eq!(target.normal, Vec3::X);
        assert!(
            targeted_voxel(Vec3::splat(0.5), Dir3::Y, |cell| {
                Some(if cell.y == 6 {
                    VoxelKind::Stone
                } else {
                    VoxelKind::Air
                })
            })
            .is_none()
        );
    }

    #[test]
    fn voxel_ray_stops_at_unbreakable_blocks() {
        for blocker in [VoxelKind::Bedrock, VoxelKind::Lava] {
            let target = targeted_voxel(Vec3::splat(0.5), Dir3::X, |cell| {
                Some(match cell.x {
                    1 => blocker,
                    2 => VoxelKind::Stone,
                    _ => VoxelKind::Air,
                })
            })
            .unwrap();
            assert_eq!(target.destroy, IVec3::new(1, 0, 0));
        }
    }

    #[test]
    fn voxel_ray_stops_at_unavailable_chunks() {
        let hit = targeted_voxel(Vec3::splat(0.5), Dir3::X, |cell| match cell.x {
            1 => None,
            2 => Some(VoxelKind::Stone),
            _ => Some(VoxelKind::Air),
        });
        assert!(hit.is_none());
    }

    #[test]
    fn overlay_global_transform_is_current_in_the_render_frame() {
        let mut app = App::new();
        app.add_plugins(TransformPlugin);
        app.init_resource::<Assets<Mesh>>();
        app.insert_resource(BreakingAssets {
            mesh: default(),
            materials: default(),
        });
        app.add_systems(
            PostUpdate,
            sync_breaking_overlay.before(TransformSystems::Propagate),
        );
        app.world_mut().spawn((
            PlayerCamera::default(),
            BreakingState(Some(BreakProgress {
                position: IVec3::new(3, 4, 5),
                kind: VoxelKind::Stone,
                state: VoxelKind::Stone.default_state(),
                elapsed: 0.1,
            })),
        ));
        let overlay = app
            .world_mut()
            .spawn((
                BreakingOverlay,
                Transform::default(),
                Visibility::Hidden,
                MeshMaterial3d::<crate::voxel::render::ModelMaterial>::default(),
                Mesh3d::default(),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(overlay)
                .unwrap()
                .translation(),
            Vec3::new(3.0, 4.0, 5.0)
        );
    }
}
