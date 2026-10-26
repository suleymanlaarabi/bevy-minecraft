//! Compose articulated rigs from shared skin and equipment assets.
use super::*;
use crate::{
    game::GameState,
    inventory::{ARMOR_START, PlayerInventory},
    player::{Player, PlayerCamera},
};
use bevy::{
    camera::{RenderTarget, visibility::RenderLayers},
    light::NotShadowCaster,
    picking::Pickable,
};

fn spawn_rig(commands: &mut Commands, assets: &SkinAssets, preview: bool, parent: Option<Entity>) {
    let rig = commands
        .spawn((
            SkinRig { preview, phase: 0. },
            Transform::default(),
            Visibility::default(),
        ))
        .id();
    if let Some(parent) = parent {
        commands.entity(rig).insert(ChildOf(parent));
    } else {
        commands
            .entity(rig)
            .insert(DespawnOnExit(InventoryState::Open));
    }
    let render_layer = if preview { PREVIEW_LAYER } else { 0 };
    for part in 0..6 {
        for layer in 0..2 {
            commands.spawn((
                SkinPart {
                    rig,
                    part,
                    layer,
                    armor: false,
                },
                Mesh3d(assets.classic[layer][part].clone()),
                MeshMaterial3d(if layer == 0 {
                    assets.base.clone()
                } else {
                    assets.outer.clone()
                }),
                Transform::default(),
                ChildOf(rig),
                RenderLayers::layer(render_layer),
            ));
        }
        // Leggings need their own shell, underneath boots and chest armor.
        for layer in 0..2 {
            commands.spawn((
                SkinPart {
                    rig,
                    part,
                    layer,
                    armor: true,
                },
                Mesh3d(if layer == 0 {
                    assets.armor_meshes[part].clone()
                } else {
                    assets.leggings_meshes[part].clone()
                }),
                MeshMaterial3d(assets.base.clone()),
                Visibility::Hidden,
                Transform::default(),
                ChildOf(rig),
                RenderLayers::layer(render_layer),
            ));
        }
    }
}

pub(super) fn spawn_preview(
    mut commands: Commands,
    assets: Res<SkinAssets>,
    sessions: Query<(), With<crate::block_behavior::containers::ViewingContainer>>,
) {
    if !sessions.is_empty() {
        return;
    }
    spawn_rig(&mut commands, &assets, true, None);
    commands.spawn((
        Camera3d::default(),
        Camera {
            order: -1,
            clear_color: ClearColorConfig::Custom(Color::srgb(0.045, 0.045, 0.055)),
            ..default()
        },
        RenderTarget::Image(assets.preview.clone().into()),
        Projection::Orthographic(OrthographicProjection {
            scaling_mode: bevy::camera::ScalingMode::FixedVertical {
                viewport_height: 2.15,
            },
            ..OrthographicProjection::default_3d()
        }),
        Transform::from_xyz(0., 0.95, 5.).looking_at(Vec3::new(0., 0.95, 0.), Vec3::Y),
        AmbientLight {
            brightness: 500.,
            ..default()
        },
        RenderLayers::layer(PREVIEW_LAYER),
        DespawnOnExit(InventoryState::Open),
    ));
    commands.spawn((
        DirectionalLight {
            illuminance: 6000.,
            ..default()
        },
        Transform::from_xyz(-3., 5., 4.).looking_at(Vec3::Y, Vec3::Y),
        RenderLayers::layer(PREVIEW_LAYER),
        DespawnOnExit(InventoryState::Open),
    ));
}

pub(super) fn spawn_world_rig(
    mut commands: Commands,
    assets: Res<SkinAssets>,
    players: Query<Entity, Added<Player>>,
    cameras: Query<Entity, Added<PlayerCamera>>,
    settings: Res<SkinSettings>,
    sky: Res<crate::sky::SkyState>,
) {
    let level = ARM_NIGHT_LEVEL + (1. - ARM_NIGHT_LEVEL) * (1. - sky.night);
    for player in &players {
        spawn_rig(&mut commands, &assets, false, Some(player));
    }
    for camera in &cameras {
        commands.spawn((
            Camera3d::default(),
            Camera {
                order: -1,
                is_active: false,
                clear_color: ClearColorConfig::Custom(Color::NONE),
                ..default()
            },
            RenderTarget::Image(assets.arm_view.clone().into()),
            Msaa::Off,
            Projection::from(PerspectiveProjection {
                fov: 70_f32.to_radians(),
                near: 0.01,
                ..default()
            }),
            RenderLayers::layer(ARM_LAYER),
            ArmCamera,
            AmbientLight {
                brightness: 500. * level,
                ..default()
            },
            Transform::default(),
            ChildOf(camera),
        ));
        commands.spawn((
            HeldItemLight,
            DirectionalLight {
                illuminance: 6000. * level,
                shadow_maps_enabled: false,
                ..default()
            },
            Transform::from_xyz(-3., 5., 4.).looking_at(Vec3::ZERO, Vec3::Y),
            RenderLayers::layer(ARM_LAYER),
            ChildOf(camera),
        ));
        commands.spawn((
            FirstPersonRoot,
            HeldItem::default(),
            Transform::default(),
            Visibility::Hidden,
            ChildOf(camera),
        ));
        let arm = commands
            .spawn((
                FirstPersonArm,
                FirstPersonRoot,
                first_person_pose(0., settings.model == SkinModel::Slim),
                Visibility::Hidden,
                ChildOf(camera),
            ))
            .id();
        for layer in 0..2 {
            commands.spawn((
                SkinPart {
                    rig: arm,
                    part: 2,
                    layer,
                    armor: false,
                },
                Mesh3d(assets.classic[layer][2].clone()),
                MeshMaterial3d(if layer == 0 {
                    assets.arm_base.clone()
                } else {
                    assets.arm_outer.clone()
                }),
                Transform::default(),
                ChildOf(arm),
                RenderLayers::layer(ARM_LAYER),
                NotShadowCaster,
            ));
        }
        // Composite the transparent hand before every HUD root on the world UI
        // camera. No window color or MSAA buffers are shared with the hand pass.
        commands.spawn((
            Node {
                position_type: PositionType::Absolute,
                width: percent(100),
                height: percent(100),
                ..default()
            },
            ImageNode::new(assets.arm_view.clone()),
            ArmView,
            Visibility::Hidden,
            GlobalZIndex(-1),
            Pickable::IGNORE,
            DespawnOnExit(GameState::Game),
        ));
    }
}

pub(super) fn sync_parts(
    settings: Res<SkinSettings>,
    assets: Res<SkinAssets>,
    inventory: Query<Ref<PlayerInventory>, With<Player>>,
    new_parts: Query<(), Added<SkinPart>>,
    mut parts: Query<(
        &SkinPart,
        &mut Mesh3d,
        &mut MeshMaterial3d<StandardMaterial>,
        &mut Visibility,
    )>,
) {
    let inventory = inventory.single().ok();
    let changed = settings.is_changed()
        || assets.is_changed()
        || inventory
            .as_ref()
            .is_some_and(|inventory| inventory.is_changed());
    if !changed && new_parts.is_empty() {
        return;
    }
    // Leggings (slot 2) use their own texture layer; the other pieces share `humanoid`.
    let armor: [Option<&Handle<StandardMaterial>>; 4] = std::array::from_fn(|slot| {
        let stack = inventory.as_ref()?.slot(ARMOR_START + slot)?;
        let name = stack
            .item()
            .component::<pumpkin_data::data_component_impl::EquippableImpl>()?
            .asset_id
            .as_ref()?;
        let layer = if slot == 2 {
            "humanoid_leggings"
        } else {
            "humanoid"
        };
        assets.armor.get(&format!(
            "{layer}/{}",
            name.strip_prefix("minecraft:").unwrap_or(name)
        ))
    });
    for (part, mut mesh, mut material, mut visibility) in &mut parts {
        if !changed && !mesh.is_added() {
            continue;
        }
        if part.armor {
            let slot = if part.layer == 1 {
                if matches!(part.part, 1 | 4 | 5) {
                    Some(2)
                } else {
                    None
                }
            } else {
                match part.part {
                    0 => Some(0),
                    1..=3 => Some(1),
                    4 | 5 => Some(3),
                    _ => None,
                }
            };
            if let Some(handle) = slot.and_then(|slot| armor[slot]) {
                if material.0 != *handle {
                    material.0 = handle.clone();
                }
                visibility.set_if_neq(Visibility::Inherited);
            } else {
                visibility.set_if_neq(Visibility::Hidden);
            }
        } else {
            let handle = if settings.model == SkinModel::Slim {
                &assets.slim[part.layer][part.part]
            } else {
                &assets.classic[part.layer][part.part]
            };
            if mesh.0 != *handle {
                mesh.0 = handle.clone();
            }
            visibility.set_if_neq(if part.layer == 0 || settings.outer_layer {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            });
        }
    }
}
