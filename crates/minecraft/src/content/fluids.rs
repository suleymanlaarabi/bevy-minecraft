//! Water and lava are content providers using exactly the public fluid API.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::{app::PluginGroupBuilder, prelude::*};
use hollow_core::{BlockId, FluidId, StateId, content::FluidState};
use hollow_engine::{content::ContentRegistrations, fluids::*};
use pumpkin_data::{Property, PropertyKey};

pub struct MinecraftFluidsPlugins;
impl PluginGroup for MinecraftFluidsPlugins {
    fn build(self) -> PluginGroupBuilder {
        PluginGroupBuilder::start::<Self>()
            .add(FluidPlugin)
            .add(WaterPlugin)
            .add(LavaPlugin)
            .add(MinecraftFluidReactionsPlugin)
    }
}
#[derive(Resource)]
pub struct Water(pub FluidId);
#[derive(Resource)]
pub struct Lava(pub FluidId);
pub struct WaterPlugin;
pub struct LavaPlugin;
fn profile(
    app: &mut App,
    name: &str,
    kind: VoxelKind,
    delay: u64,
    drop_off: u8,
    slope_limit: u8,
    translucent: bool,
    effect: u8,
) -> FluidProfile {
    let resources = {
        let mut registrations = app.world_mut().resource_mut::<ContentRegistrations>();
        let builder = registrations.builder();
        ["still", "flow"].map(|suffix| builder.resource(format!("minecraft:block/{name}_{suffix}")))
    };
    let state = |amount: u8, falling: bool| {
        StateId(if amount == 0 {
            0
        } else {
            kind.default_state()
                .with(&[Property::Level(if falling { 8 } else { 8 - amount })])
                .id()
        })
    };
    let mut profile = FluidProfile::new(format!("minecraft:{name}"));
    profile.render = Some(FluidRender {
        still: resources[0],
        flowing: resources[1],
        translucent,
        emission: if translucent { 0 } else { 15 },
        effect,
        ocean_surface: translucent,
    });
    profile.delay = delay;
    profile.drop_off = drop_off;
    profile.slope_limit = slope_limit;
    profile.standing = (0..=8).map(|amount| state(amount, false)).collect();
    profile.falling = (0..=8).map(|amount| state(amount, true)).collect();
    profile.movement.sprint_drag = 0.9;
    profile.movement.sprint_cancels_gravity = true;
    profile.fog = Some(FluidFog {
        color: [24.0 / 255.0, 62.0 / 255.0, 138.0 / 255.0],
        start: 2.0,
        end: 40.0,
        ambient: [95.0 / 255.0, 140.0 / 255.0, 215.0 / 255.0],
        brightness: 0.55,
        daylight: true,
    });
    profile
}
impl Plugin for WaterPlugin {
    fn build(&self, app: &mut App) {
        let mut profile = profile(app, "water", VoxelKind::Water, 5, 1, 4, true, 1);
        profile.renewable_sources = Some(2);
        profile.drops_displaced = true;
        profile.contact = FluidContact {
            swimmable: true,
            extinguishes: true,
            ..default()
        };
        let id = app
            .world_mut()
            .resource_mut::<FluidRegistrations>()
            .register(profile);
        let mut registrations = app.world_mut().resource_mut::<ContentRegistrations>();
        let builder = registrations.builder();
        for raw in 0..pumpkin_data::BlockStateId::COUNT {
            let state = VoxelState::from_id(raw).unwrap();
            let definition = &mut builder.states[usize::from(raw)];
            let native = pumpkin_data::BlockStateId::new(raw).unwrap();
            if matches!(
                native.to_block_id(),
                pumpkin_data::BlockId::WATER | pumpkin_data::BlockId::BUBBLE_COLUMN
            ) {
                definition.selection = hollow_core::ShapeId(0);
            }
            if matches!(
                native.to_block_id(),
                pumpkin_data::BlockId::WATER
                    | pumpkin_data::BlockId::BUBBLE_COLUMN
                    | pumpkin_data::BlockId::SEAGRASS
                    | pumpkin_data::BlockId::TALL_SEAGRASS
                    | pumpkin_data::BlockId::KELP
                    | pumpkin_data::BlockId::KELP_PLANT
            ) || native.is_waterlogged()
            {
                let level = match state.property(PropertyKey::Level) {
                    Some(Property::Level(level)) => level,
                    _ => 0,
                };
                definition.fluid = FluidState {
                    id,
                    amount: if level >= 8 { 8 } else { 8 - level },
                    source: level == 0,
                    falling: level >= 8,
                    embedded: !matches!(
                        native.to_block_id(),
                        pumpkin_data::BlockId::WATER | pumpkin_data::BlockId::BUBBLE_COLUMN
                    ),
                };
            }
            if state.property(PropertyKey::Waterlogged).is_some()
                && state.property(PropertyKey::Type)
                    != Some(Property::SlabType(
                        pumpkin_data::block_properties::SlabType::Double,
                    ))
            {
                builder.accept_fluid(
                    StateId(u32::from(raw)),
                    id,
                    StateId(state.with(&[Property::Waterlogged(true)]).id()),
                );
            }
        }
        app.insert_resource(Water(id));
    }
}
fn product(kind: pumpkin_data::BlockId) -> StateId {
    StateId(VoxelKind::from_native(kind).default_state().id())
}
impl Plugin for LavaPlugin {
    fn build(&self, app: &mut App) {
        let mut profile = profile(app, "lava", VoxelKind::Lava, 30, 2, 2, false, 8);
        profile.movement = FluidMovement {
            acceleration: 0.02,
            horizontal_drag: 0.5,
            vertical_drag: 0.8,
            sprint_drag: 0.5,
            deep_vertical_drag: 0.5,
            gravity: 0.02,
            sprint_cancels_gravity: false,
        };
        profile.fog = Some(FluidFog {
            color: [210.0 / 255.0, 80.0 / 255.0, 10.0 / 255.0],
            start: 0.0,
            end: 1.5,
            ambient: [1.0, 140.0 / 255.0, 60.0 / 255.0],
            brightness: 0.28,
            daylight: false,
        });
        profile.rising_delay_chance = 4;
        profile.rising_delay_multiplier = 4;
        let damage_resource = app
            .world_mut()
            .resource_mut::<ContentRegistrations>()
            .builder()
            .resource("minecraft:damage_type/lava");
        profile.contact = FluidContact {
            damage_resource: Some(damage_resource),
            damage: 4.0,
            ignites: true,
            ..default()
        };
        use pumpkin_data::BlockId as Native;
        profile.reactions = vec![FluidReaction {
            contact: ContactFilter::Block(BlockId(u32::from(Native::BLUE_ICE.as_u16()))),
            support: Some(BlockId(u32::from(Native::SOUL_SOIL.as_u16()))),
            directions: 31,
            target: ReactionTarget::Origin,
            source_product: product(Native::BASALT),
            flowing_product: product(Native::BASALT),
            minimum_amount: 0,
            free_fluid_only: false,
        }];
        let id = app
            .world_mut()
            .resource_mut::<FluidRegistrations>()
            .register(profile);
        let mut registrations = app.world_mut().resource_mut::<ContentRegistrations>();
        let builder = registrations.builder();
        let selection = builder.shape(vec![hollow_core::shape::BlockBox {
            min: Vec3::ZERO,
            max: Vec3::ONE,
        }]);
        for state in VoxelKind::Lava.block().states {
            builder.states[usize::from(state.id.as_u16())].selection = selection;
            let level = match state.id.property(PropertyKey::Level) {
                Some(Property::Level(level)) => level,
                _ => 0,
            };
            builder.states[usize::from(state.id.as_u16())].fluid = FluidState {
                id,
                amount: if level >= 8 { 8 } else { 8 - level },
                source: level == 0,
                falling: level >= 8,
                embedded: false,
            };
        }
        app.insert_resource(Lava(id));
    }
}

/// Adds interactions when both providers are installed. Neither fluid provider
/// depends on the other, and other games can supply their own reaction plugin.
pub struct MinecraftFluidReactionsPlugin;
impl Plugin for MinecraftFluidReactionsPlugin {
    fn build(&self, app: &mut App) {
        let Some(water) = app.world().get_resource::<Water>().map(|water| water.0) else {
            return;
        };
        let Some(lava) = app.world().get_resource::<Lava>().map(|lava| lava.0) else {
            return;
        };
        use pumpkin_data::BlockId as Native;
        let mut fluids = app.world_mut().resource_mut::<FluidRegistrations>();
        fluids.profile_mut(lava).reactions.insert(
            0,
            FluidReaction {
                contact: ContactFilter::Fluid(water),
                support: None,
                directions: 31,
                target: ReactionTarget::Origin,
                source_product: product(Native::OBSIDIAN),
                flowing_product: product(Native::COBBLESTONE),
                minimum_amount: 0,
                free_fluid_only: false,
            },
        );
        fluids.profile_mut(lava).reactions.push(FluidReaction {
            contact: ContactFilter::Fluid(water),
            support: None,
            directions: 32,
            target: ReactionTarget::Neighbor,
            source_product: product(Native::STONE),
            flowing_product: product(Native::STONE),
            minimum_amount: 0,
            free_fluid_only: true,
        });
        fluids.profile_mut(water).reactions.push(FluidReaction {
            contact: ContactFilter::Fluid(lava),
            support: None,
            directions: 63,
            target: ReactionTarget::Neighbor,
            source_product: product(Native::OBSIDIAN),
            flowing_product: product(Native::COBBLESTONE),
            minimum_amount: 0,
            free_fluid_only: true,
        });
    }
}
