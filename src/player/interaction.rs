use avian3d::prelude::*;
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
    dropped_item::DroppedItemCommandsExt,
    game::GameState,
    inventory::{InventoryState, ItemKind, ItemStack},
    player::{Player, PlayerCamera},
    spatial::FollowSystems,
    voxel::{VoxelCommandsExt, VoxelKind, VoxelWorld},
};

const BLOCK_REACH: f32 = 5.0;
const BLOCK_OUTLINE_SIZE: f32 = 1.01;

#[derive(Debug, Clone, Copy)]
struct BlockTarget {
    destroy: IVec3,
    place: IVec3,
    normal: Vec3,
}

#[derive(Component, Default)]
pub(super) struct BlockTargetState(Option<BlockTarget>);

#[derive(Debug, Clone, Copy)]
struct BreakProgress {
    position: IVec3,
    kind: VoxelKind,
    elapsed: f32,
}

#[derive(Component, Default)]
pub(super) struct BreakingState(Option<BreakProgress>);

#[derive(Component)]
struct BreakingOverlay;

#[derive(Resource)]
struct BreakingAssets {
    mesh: Handle<Mesh>,
    materials: [Handle<StandardMaterial>; 10],
}

pub(super) fn register(app: &mut App) {
    app.add_systems(PreStartup, prepare_breaking_assets)
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
                .after(FollowSystems)
                .before(TransformSystems::Propagate)
                .run_if(in_state(GameState::Game))
                .run_if(in_state(InventoryState::Closed)),
        );
}

fn prepare_breaking_assets(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let mesh = meshes.add(Cuboid::new(1.006, 1.006, 1.006));
    let stages = core::array::from_fn(|stage| {
        materials.add(StandardMaterial {
            base_color: Color::srgb(0.65, 0.65, 0.65),
            base_color_texture: Some(
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
                    .load(format!("voxel/breaking/stage_{stage}.png")),
            ),
            alpha_mode: AlphaMode::Mask(0.05),
            unlit: true,
            fog_enabled: false,
            depth_bias: 1.0,
            cull_mode: None,
            ..default()
        })
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

fn update_block_target(
    transforms: TransformHelper,
    camera: Single<(Entity, &mut BlockTargetState), With<PlayerCamera>>,
    voxel_world: VoxelWorld,
) {
    let (entity, mut target) = camera.into_inner();
    target.0 = transforms
        .compute_global_transform(entity)
        .ok()
        .and_then(|transform| {
            targeted_voxel(transform.translation(), transform.forward(), |position| {
                voxel_world.get(position)
            })
        });
}

fn update_breaking(
    mut commands: Commands,
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    target: Single<&BlockTargetState, With<PlayerCamera>>,
    mut breaking: Single<&mut BreakingState, With<PlayerCamera>>,
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
    let Some(kind) = voxel_world.get(target.destroy) else {
        breaking.0 = None;
        return;
    };
    let Some(duration) = kind.break_duration() else {
        breaking.0 = None;
        return;
    };

    let progress = match breaking.0.as_mut() {
        Some(progress) if progress.position == target.destroy && progress.kind == kind => progress,
        _ => {
            breaking.0 = Some(BreakProgress {
                position: target.destroy,
                kind,
                elapsed: 0.0,
            });
            return;
        }
    };
    progress.elapsed += time.delta_secs();
    if progress.elapsed < duration || voxel_world.get(target.destroy) != Some(kind) {
        return;
    }

    commands.set_voxel(target.destroy, VoxelKind::Air);
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
    spatial_query: SpatialQuery,
    players: Query<(), With<Player>>,
    mut inventory: Single<&mut crate::inventory::PlayerInventory, With<Player>>,
) {
    let Some(target) = target.0 else {
        return;
    };
    if !mouse.just_pressed(MouseButton::Right)
        || voxel_world.get(target.place) != Some(VoxelKind::Air)
    {
        return;
    }
    let Some(ItemKind::Block(kind)) = inventory.selected_stack().map(|stack| stack.item()) else {
        return;
    };
    if block_contains_player(&spatial_query, &players, target.place) {
        return;
    }
    if inventory.consume_selected(1) {
        commands.set_voxel(target.place, kind);
    }
}

fn sync_breaking_overlay(
    breaking: Single<&BreakingState, With<PlayerCamera>>,
    overlay: Single<
        (
            &mut Transform,
            &mut Visibility,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<BreakingOverlay>,
    >,
    assets: Res<BreakingAssets>,
) {
    let (mut transform, mut visibility, mut material) = overlay.into_inner();
    let Some(progress) = breaking.0 else {
        visibility.set_if_neq(Visibility::Hidden);
        return;
    };
    visibility.set_if_neq(Visibility::Visible);
    let translation = progress.position.as_vec3() + Vec3::splat(0.5);
    if transform.translation != translation {
        transform.translation = translation;
    }
    let duration = progress
        .kind
        .break_duration()
        .expect("active breaking state is breakable");
    let stage = ((progress.elapsed / duration * 10.0).floor() as usize).min(9);
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
    spatial_query: &SpatialQuery,
    players: &Query<(), With<Player>>,
    position: IVec3,
) -> bool {
    let mut contains_player = false;
    spatial_query.shape_intersections_callback(
        &Collider::cuboid(0.999, 0.999, 0.999),
        position.as_vec3() + Vec3::splat(0.5),
        Quat::IDENTITY,
        &SpatialQueryFilter::DEFAULT,
        |entity| {
            contains_player = players.contains(entity);
            !contains_player
        },
    );
    contains_player
}

fn draw_targeted_block(mut gizmos: Gizmos, target: Single<&BlockTargetState, With<PlayerCamera>>) {
    let Some(target) = target.0 else { return };
    gizmos.cube(
        Transform::from_translation(target.destroy.as_vec3() + Vec3::splat(0.5))
            .with_scale(Vec3::splat(BLOCK_OUTLINE_SIZE)),
        BLACK,
    );
}

fn targeted_voxel(
    origin: Vec3,
    direction: Dir3,
    mut sample: impl FnMut(IVec3) -> Option<VoxelKind>,
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
    let mut normal = IVec3::ZERO;
    loop {
        if sample(cell)?.break_duration().is_some() {
            return Some(BlockTarget {
                destroy: cell,
                place: cell + normal,
                normal: normal.as_vec3(),
            });
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
        cell[axis] += step[axis];
        next[axis] += delta[axis];
        normal = IVec3::ZERO;
        normal[axis] = -step[axis];
    }
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
                elapsed: 0.1,
            })),
        ));
        let overlay = app
            .world_mut()
            .spawn((
                BreakingOverlay,
                Transform::default(),
                Visibility::Hidden,
                MeshMaterial3d::<StandardMaterial>::default(),
            ))
            .id();
        app.update();
        assert_eq!(
            app.world()
                .get::<GlobalTransform>(overlay)
                .unwrap()
                .translation(),
            Vec3::new(3.5, 4.5, 5.5)
        );
    }
}
