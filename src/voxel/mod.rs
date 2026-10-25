mod data;
#[cfg(feature = "dev")]
mod diagnostics;
pub(crate) mod generation;
mod lighting;
mod material;
mod meshing;
mod rebuild;
mod streaming;
use crate::{game::GameState, settings::GraphicsSettings};
use bevy::{
    ecs::system::SystemParam,
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};
pub(crate) use data::{CHUNK_SIZE, ChunkVoxels, VoxelChunk, split_world_position};
pub use data::{SetVoxel, VoxelKind, VoxelViewer, chunk_at};
use generation::WorldGenerator;
pub use material::TexturePackId;
use material::WaterMaterial;
pub(crate) use material::{TexturePack, VoxelMaterial};
pub(crate) use meshing::build_block_item_mesh;

/// Where the sun's `DirectionalLight` sits, looking at the origin. Shadow meshes keep
/// only the faces turned toward it: remesh every chunk if this ever changes.
pub(crate) const SUN_POSITION: Vec3 = Vec3::new(100.0, 200.0, 100.0);
/// Render layer of depth-only terrain: drawn into the sun's shadow maps, never by cameras.
pub(crate) const SHADOW_LAYER: usize = 1;
pub(crate) use rebuild::VoxelAssets;
use rebuild::{
    apply_texture_pack, cleanup_removed_chunk, mark_changed_chunks, poll_builds,
    poll_chunk_colliders, prepare_assets, request_chunk_colliders, set_voxel, start_changed_builds,
};
use streaming::{
    ChunkIndex, PendingChunks, PendingEdits, StoredChunks, StreamOffsets, StreamState,
    poll_chunk_generations, refresh_neighbors_of_spawned, release_ready_chunks, stream_chunks,
};

#[derive(SystemParam)]
pub struct VoxelWorld<'w, 's> {
    index: Res<'w, ChunkIndex>,
    chunks: Query<'w, 's, &'static ChunkVoxels>,
    colliders: Query<'w, 's, (), With<avian3d::prelude::Collider>>,
}

impl VoxelWorld<'_, '_> {
    /// Collision is installed after asynchronous terrain/mesh generation.
    pub fn collision_ready(&self, position: Vec3) -> bool {
        self.index
            .get(&chunk_at(position))
            .is_some_and(|entity| self.colliders.contains(*entity))
    }

    /// Returns the voxel at an integer world position, or `None` when it is unavailable.
    pub fn get(&self, world_position: IVec3) -> Option<VoxelKind> {
        let (chunk_position, local_position) = split_world_position(world_position);
        self.index
            .get(&chunk_position)
            .and_then(|entity| self.chunks.get(*entity).ok())
            .and_then(|voxels| voxels.get(local_position))
    }

    /// Returns the voxel containing a continuous world position.
    pub fn get_at(&self, world_position: Vec3) -> Option<VoxelKind> {
        self.get(world_position.floor().as_ivec3())
    }
}

pub trait VoxelCommandsExt {
    /// Queues a world-space voxel edit, generating an unloaded chunk when necessary.
    fn set_voxel(&mut self, world_position: IVec3, kind: VoxelKind) -> &mut Self;
}

impl VoxelCommandsExt for Commands<'_, '_> {
    fn set_voxel(&mut self, world_position: IVec3, kind: VoxelKind) -> &mut Self {
        self.trigger(SetVoxel::new(world_position, kind));
        self
    }
}

pub struct VoxelPlugin {
    settings: VoxelSettings,
}

impl VoxelPlugin {
    pub const fn new(settings: VoxelSettings) -> Self {
        Self { settings }
    }
}

impl Default for VoxelPlugin {
    fn default() -> Self {
        Self::new(VoxelSettings::default())
    }
}

impl Plugin for VoxelPlugin {
    fn build(&self, app: &mut App) {
        let view_distance = app.world().get_resource::<GraphicsSettings>().map_or_else(
            || GraphicsSettings::default().effective_view_distance(),
            GraphicsSettings::effective_view_distance,
        );

        app.add_plugins((
            MaterialPlugin::<VoxelMaterial>::default(),
            MaterialPlugin::<WaterMaterial>::default(),
        ))
        .insert_resource(self.settings.clone())
        .insert_resource(WorldGenerator::new(self.settings.seed))
        .insert_resource(StreamOffsets::new(view_distance))
        .init_resource::<ChunkIndex>()
        .init_resource::<StoredChunks>()
        .init_resource::<PendingChunks>()
        .init_resource::<PendingEdits>()
        .init_resource::<StreamState>()
        .init_resource::<lighting::LightingState>()
        .add_observer(apply_view_distance)
        .add_observer(apply_texture_pack)
        .add_observer(set_voxel)
        .add_observer(cleanup_removed_chunk)
        .add_systems(PreStartup, prepare_assets)
        .add_systems(Update, stream_chunks.run_if(in_state(GameState::Game)))
        .add_systems(
            Update,
            material::generate_block_mipmaps.run_if(
                on_message::<AssetEvent<Image>>
                    .or_else(on_message::<AssetEvent<VoxelMaterial>>)
                    .or_else(resource_exists::<material::MipmapJob>),
            ),
        )
        .add_systems(
            PostUpdate,
            (
                poll_chunk_generations,
                refresh_neighbors_of_spawned,
                release_ready_chunks,
                lighting::update_lighting,
                mark_changed_chunks,
                start_changed_builds,
                poll_builds,
                request_chunk_colliders,
                poll_chunk_colliders,
            )
                .chain()
                .before(bevy::transform::TransformSystems::Propagate)
                .run_if(in_state(GameState::Game)),
        )
        .add_systems(
            PostUpdate,
            update_underwater_effect.run_if(in_state(GameState::Game)),
        )
        .add_systems(OnExit(GameState::Game), cleanup_voxel_world);

        #[cfg(feature = "dev")]
        diagnostics::register(app);
    }
}

fn apply_view_distance(
    _insert: On<Insert<GraphicsSettings>>,
    settings: Res<GraphicsSettings>,
    mut offsets: ResMut<StreamOffsets>,
    mut state: ResMut<StreamState>,
) {
    let view_distance = settings.effective_view_distance();
    if offsets.render_radius == view_distance {
        return;
    }
    *offsets = StreamOffsets::new(view_distance);
    state.center = None;
}

fn update_underwater_effect(
    voxel_world: VoxelWorld,
    mut camera: Single<(&Transform, &mut DistanceFog, &mut AmbientLight), With<VoxelViewer>>,
    mut clear_color: ResMut<ClearColor>,
    mut previous: Local<Option<Option<VoxelKind>>>,
) {
    let liquid = voxel_world
        .get_at(camera.0.translation)
        .filter(|kind| kind.is_liquid());

    if *previous == Some(liquid) {
        return;
    }
    *previous = Some(liquid);

    let (_, fog, ambient) = &mut *camera;
    if liquid == Some(VoxelKind::Lava) {
        fog.color = Color::srgb_u8(210, 80, 10);
        fog.falloff = FogFalloff::Linear {
            start: 0.0,
            end: 1.5,
        };
        ambient.color = Color::srgb_u8(255, 140, 60);
        ambient.brightness = 400.0;
        clear_color.0 = Color::srgb_u8(210, 80, 10);
    } else if liquid.is_some() {
        // A darkened vanilla water tint (#3F76E4).
        fog.color = Color::srgb_u8(24, 62, 138);
        fog.falloff = FogFalloff::Linear {
            start: 2.0,
            end: 40.0,
        };
        ambient.color = Color::srgb_u8(95, 140, 215);
        ambient.brightness = 280.0;
        clear_color.0 = Color::srgb_u8(24, 62, 138);
    } else {
        fog.color = Color::srgb_u8(195, 222, 255);
        fog.falloff = FogFalloff::Linear {
            start: 160.0,
            end: 280.0,
        };
        ambient.color = Color::srgb_u8(215, 235, 255);
        ambient.brightness = 500.0;
        clear_color.0 = Color::srgb_u8(148, 195, 255);
    }
}

fn cleanup_voxel_world(
    mut commands: Commands,
    generator: Res<WorldGenerator>,
    chunks: Query<Entity, With<VoxelChunk>>,
    generations: Query<Entity, With<streaming::ChunkGeneration>>,
) {
    for entity in &chunks {
        commands.entity(entity).despawn();
    }
    for entity in &generations {
        commands.entity(entity).despawn();
    }
    // Let Pumpkin unload the area; the next session sets its own view.
    generator.clear_view();
    // Despawn observers write back into these resources, so reset them afterwards.
    commands.queue(|world: &mut World| {
        world.resource_mut::<ChunkIndex>().clear();
        world.resource_mut::<PendingChunks>().clear();
        world.resource_mut::<PendingEdits>().clear();
        world.resource_mut::<StoredChunks>().clear();
        // Drop the context on its own worker: Pumpkin shutdown must not stall ECS.
        if let Some(state) = world.remove_resource::<lighting::LightingState>() {
            bevy::tasks::IoTaskPool::get()
                .spawn(async move {
                    drop(state);
                })
                .detach();
        }
        world.init_resource::<lighting::LightingState>();
        *world.resource_mut::<StreamState>() = StreamState::default();
    });
}

#[derive(Resource, Clone, Debug)]
pub struct VoxelSettings {
    pub base_height: f32,
    pub seed: u32,
    pub spawn_budget_per_frame: usize,
    pub despawn_budget_per_frame: usize,
}

impl Default for VoxelSettings {
    fn default() -> Self {
        Self {
            base_height: 127.0,
            seed: 22,
            spawn_budget_per_frame: 8,
            // Counted in section meshes: despawning a chunk frees all of them.
            despawn_budget_per_frame: 96,
        }
    }
}

impl VoxelSettings {
    pub fn chunk_center(&self, chunk: IVec2) -> Vec3 {
        let origin = (chunk * CHUNK_SIZE).as_vec2();
        let half = CHUNK_SIZE as f32 * 0.5;
        Vec3::new(origin.x + half, self.base_height, origin.y + half)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use avian3d::prelude::Collider;
    use bevy::ecs::system::RunSystemOnce;

    #[test]
    fn collision_readiness_waits_for_the_loaded_chunk_collider() {
        let mut world = World::new();
        world.init_resource::<ChunkIndex>();
        let chunk = world.spawn_empty().id();
        world
            .resource_mut::<ChunkIndex>()
            .insert(IVec2::ZERO, chunk);
        let ready = |voxels: VoxelWorld| voxels.collision_ready(Vec3::new(8.0, 130.0, 8.0));
        assert!(!world.run_system_once(ready).unwrap());
        world
            .entity_mut(chunk)
            .insert(Collider::cuboid(16.0, 1.0, 16.0));
        assert!(world.run_system_once(ready).unwrap());
        world.despawn(chunk);
        assert!(!world.run_system_once(ready).unwrap());
    }
}
