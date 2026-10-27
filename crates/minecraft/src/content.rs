//! Content providers compose through the engine registration API.
pub mod fluids;
use bevy::prelude::*;
use hollow_engine::content::{
    ContentPlugin, ContentRegistrations, FreezeContent, RegisteredContent,
};

pub struct MinecraftContentPlugin;
impl Plugin for MinecraftContentPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ContentRegistrations(Some(
            hollow_worldgen::content::import(),
        )))
        .add_plugins(ContentPlugin)
        .add_systems(
            PreStartup,
            bind.after(FreezeContent)
                .before(hollow_engine::blocks::FinalizeBlockContent),
        )
        .add_systems(
            PreStartup,
            bind_fluids
                .after(hollow_engine::fluids::FreezeFluids)
                .before(hollow_engine::blocks::FinalizeBlockContent),
        );
    }
}
fn bind(content: Res<RegisteredContent>) {
    hollow_worldgen::content::install(content.0.clone());
}

struct FluidAdapter {
    catalog: hollow_engine::fluids::FluidCatalog,
    // DamageRequest retains static definitions; this process-lived import adapter
    // owns one resolved definition per fluid, never one per actor or contact.
    sources: Box<[pumpkin_data::damage::DamageType]>,
}
static FLUIDS: std::sync::OnceLock<FluidAdapter> = std::sync::OnceLock::new();
pub fn fluid_catalog() -> &'static hollow_engine::fluids::FluidCatalog {
    &FLUIDS.get().expect("Fluids not finalized").catalog
}
pub(crate) fn fluid_damage_source(
    fluid: hollow_core::FluidId,
) -> &'static pumpkin_data::damage::DamageType {
    &FLUIDS.get().expect("Fluids not finalized").sources[fluid.index() - 1]
}
fn bind_fluids(fluids: Res<hollow_engine::fluids::FluidCatalog>, content: Res<RegisteredContent>) {
    use pumpkin_data::damage::DamageType;
    let sources = fluids
        .profiles()
        .map(|(_, profile)| {
            profile
                .contact
                .damage_resource
                .map_or(DamageType::GENERIC, |resource| {
                    let name = content.resource(resource);
                    let name = name.strip_prefix("minecraft:damage_type/").unwrap_or(name);
                    DamageType::from_name(name)
                        .unwrap_or_else(|| panic!("Unknown fluid damage resource: {name}"))
                })
        })
        .collect();
    assert!(
        FLUIDS
            .set(FluidAdapter {
                catalog: fluids.clone(),
                sources
            })
            .is_ok(),
        "Fluid adapter already initialized"
    );
}
