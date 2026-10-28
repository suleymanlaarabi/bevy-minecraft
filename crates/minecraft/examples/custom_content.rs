//! Compile-only example composition. F8 places a source above the player's feet.
use bevy::prelude::*;
use hollow_core::{
    StateId,
    content::{BlockSpec, BlockVisual, PropertySpec, StateDefinition},
};
use hollow_engine::{
    content::ContentRegistrations,
    fluids::{FlowControl, FluidAccess, FluidFog, FluidProfile, FluidRegistrations, FluidRender},
};
use hollow_minecraft::{
    app::HollowPlugin,
    block_behavior::BlockUse,
    inventory::InventoryStation,
    player::Player,
    voxel::{SetVoxel, VoxelState},
};

#[derive(Resource)]
struct ResinSource {
    fluid: StateId,
    block: StateId,
}
#[derive(Component, Default)]
struct ResinInteraction;

struct ResinPlugin;
impl Plugin for ResinPlugin {
    fn build(&self, app: &mut App) {
        let source = app.world_mut().resource_scope(
            |world, mut registrations: Mut<ContentRegistrations>| {
                let content = registrations.builder();
                let still = content.resource("examples/resin/still.png");
                let flowing = content.resource("examples/resin/flow.png");
                let mut profile = FluidProfile::new("example:resin");
                profile.render = Some(FluidRender {
                    still,
                    flowing,
                    translucent: true,
                    emission: 0,
                    effect: 0,
                    ocean_surface: false,
                });
                profile.delay = 12;
                profile.slope_limit = 3;
                profile.contact.swimmable = true;
                profile.movement.horizontal_drag = 0.6;
                profile.fog = Some(FluidFog {
                    color: [0.3, 0.15, 0.02],
                    start: 0.0,
                    end: 12.0,
                    ambient: [1.0, 0.6, 0.2],
                    brightness: 0.5,
                    daylight: true,
                });
                let mut block = BlockSpec::new(
                    "example:resin",
                    StateDefinition {
                        replaceable: true,
                        washable: true,
                        ..default()
                    },
                );
                block.resources = vec![still, flowing];
                let (block, fluid) = world
                    .resource_mut::<FluidRegistrations>()
                    .register_simple(content, block, profile, 8);
                let source = content.blocks[block.index()].default_state;
                let shape = content.shape(vec![hollow_core::shape::BlockBox {
                    min: Vec3::ZERO,
                    max: Vec3::ONE,
                }]);
                let mut solid = BlockSpec::new(
                    "example:resin_block",
                    StateDefinition {
                        collision: shape,
                        selection: shape,
                        full_cube: true,
                        full_collision: true,
                        solid: true,
                        occludes: true,
                        solid_render: true,
                        solid_faces: 63,
                        center_faces: 63,
                        opacity: 15,
                        ..default()
                    },
                );
                solid.properties.push(PropertySpec {
                    name: "polished".into(),
                    values: vec!["false".into(), "true".into()],
                    default: 0,
                });
                let solid = content.register(solid);
                content.set_visual(
                    solid,
                    BlockVisual {
                        faces: [still; 6],
                        translucent: false,
                    },
                );
                // This block composes the same crafting/session capability as a workbench.
                // No block-specific interaction observer, validation, or UI system is needed.
                world
                    .resource_mut::<hollow_engine::blocks::BlockRegistrations>()
                    .register_bundle::<InventoryStation>(solid, true);
                // A second capability can react to the same use without changing
                // the crafting station or its observer.
                world
                    .resource_mut::<hollow_engine::blocks::BlockRegistrations>()
                    .register_bundle::<ResinInteraction>(solid, true);
                let solid_state = content.blocks[solid.index()].default_state;
                let cured_state = StateId(solid_state.0 + 1);
                content.states[cured_state.index()].luminance = 4;
                // The closure captures numeric definitions at registration. No name
                // lookup or special-case branch is added to the engine.
                world
                    .resource_mut::<FluidRegistrations>()
                    .profile_mut(fluid)
                    .before_step = Some(Box::new(move |access: &mut dyn FluidAccess| {
                    let origin = access.origin();
                    let below = access.read(origin - IVec3::Y)?;
                    if access.catalog().content().state(below).block == solid {
                        access.write(origin, cured_state)?;
                        Ok(FlowControl::Stop)
                    } else {
                        Ok(FlowControl::Continue)
                    }
                }));
                ResinSource {
                    fluid: source,
                    block: solid_state,
                }
            },
        );
        app.insert_resource(source)
            .add_observer(resin_used)
            .add_systems(Update, place_resin);
    }
}
fn resin_used(event: On<BlockUse>, resin: Query<(), With<ResinInteraction>>) {
    if resin.contains(event.entity) {
        info!("Resin station used by {:?}", event.actor);
    }
}

fn place_resin(
    keys: Res<ButtonInput<KeyCode>>,
    source: Res<ResinSource>,
    player: Query<&Transform, With<Player>>,
    mut commands: Commands,
) {
    let state = if keys.just_pressed(KeyCode::F8) {
        source.fluid
    } else if keys.just_pressed(KeyCode::F9) {
        source.block
    } else {
        return;
    };
    let Ok(player) = player.single() else {
        return;
    };
    commands.trigger(SetVoxel {
        world_position: player.translation.floor().as_ivec3() + IVec3::new(2, 1, 0),
        state: VoxelState::from_id(state.0).unwrap(),
    });
}
fn main() {
    App::new()
        .add_plugins(HollowPlugin)
        .add_plugins(ResinPlugin)
        .run();
}
