//! Minecraft import providers and translation of game presentation policy.
pub(super) mod assets;
pub(crate) mod models;
mod texture_pack;
use super::{VoxelKind, VoxelState};
use bevy::prelude::*;
use hollow_core::{BlockId, FluidId, StateId};
use hollow_render::{
    RenderSource,
    environment::{EnvironmentSettings, SkyLighting, ViewEffects},
};
pub(crate) use texture_pack::TexturePack;
pub use texture_pack::TexturePackId;
#[derive(Resource, Clone)]
pub struct MinecraftRenderSource {
    content: hollow_core::content::ContentCatalog,
    fluids: hollow_engine::fluids::FluidCatalog,
    cells: &'static [hollow_render::models::CellInfo],
}
pub fn initialize_source(
    mut commands: Commands,
    content: Res<hollow_engine::content::RegisteredContent>,
    fluids: Res<hollow_engine::fluids::FluidCatalog>,
) {
    commands.insert_resource(MinecraftRenderSource {
        content: content.0.clone(),
        fluids: fluids.clone(),
        cells: models::cells(),
    });
}
impl RenderSource for MinecraftRenderSource {
    type Cell = VoxelState;
    fn content(&self) -> &hollow_core::content::ContentCatalog {
        &self.content
    }
    fn fluids(&self) -> &hollow_engine::fluids::FluidCatalog {
        &self.fluids
    }
    fn cell_info(&self, state: StateId) -> hollow_render::models::CellInfo {
        self.cells[state.index()]
    }
    fn visit_quads(
        &self,
        state: StateId,
        world: IVec3,
        visit: impl FnMut(&hollow_render::models::ModelQuad),
    ) {
        models::visit_quads(
            VoxelState::from_id(state.0).expect("Registered model state"),
            world,
            visit,
        );
    }
    fn fluid_texture(&self, fluid: FluidId, flowing: bool) -> u32 {
        models::fluid_texture(fluid, flowing)
    }
    fn tint_index(&self, block: BlockId) -> u8 {
        models::tint_index(VoxelKind(block.0))
    }
    fn opaque_texture(&self, slot: u32) -> bool {
        models::opaque_texture(slot)
    }
    fn visual_offset(&self, state: StateId, world: IVec3) -> Vec3 {
        VoxelState::from_id(state.0).unwrap().visual_offset(world)
    }
    fn has_imported_models(&self) -> bool {
        !models::texture_names().is_empty()
    }
    fn entity_rendered(&self, state: StateId) -> bool {
        crate::block_behavior::catalog::profile(VoxelState::from_id(state.0).unwrap().kind()).chest
    }
    fn shadow_directions(&self) -> [bool; 3] {
        crate::sky::LIGHT_NEVER_FROM
    }
}
pub fn sync_environment(
    settings: Res<crate::settings::GraphicsSettings>,
    sky: Res<crate::sky::SkyState>,
    effects: Query<&crate::survival::ActiveEffects, With<crate::player::Player>>,
    mut environment: ResMut<EnvironmentSettings>,
    mut lighting: ResMut<SkyLighting>,
    mut vision: ResMut<ViewEffects>,
) {
    use pumpkin_data::effect::StatusEffect as E;
    let radius = terrain_radius(&settings);
    environment.set_if_neq(EnvironmentSettings {
        terrain_radius: radius,
        horizon_radius: if settings.clouds {
            crate::sky::cloud_reach(&settings)
        } else {
            radius
        },
    });
    lighting.set_if_neq(SkyLighting {
        night: sky.night,
        ev100: sky.ev100,
        ambient_color: sky.ambient_color,
        ambient_brightness: sky.ambient_brightness,
        horizon_color: sky.horizon_color,
    });
    let effects = effects.single().ok();
    vision.set_if_neq(ViewEffects {
        minimum_ambient: if effects.is_some_and(|effects| effects.level(E::NIGHT_VISION.id) > 0) {
            0.65
        } else {
            0.0
        },
        fog_distance: effects
            .is_some_and(|effects| effects.level(E::BLINDNESS.id) > 0)
            .then_some(5.0),
    });
}
pub(crate) fn terrain_radius(settings: &crate::settings::GraphicsSettings) -> f32 {
    (settings.effective_view_distance() * hollow_core::coordinates::CHUNK_SIZE as u32) as f32
}
