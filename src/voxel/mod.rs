mod data;
mod generation;
mod material;
mod meshing;
mod rebuild;
mod streaming;
use crate::game::GameState;
use bevy::{
    pbr::{DistanceFog, FogFalloff},
    prelude::*,
};
pub use data::{ChunkVoxels, SetVoxel, VoxelChunk, VoxelKind, VoxelViewer};
use generation::WorldGenerator;
use material::{VoxelMaterial, WaterMaterial};
use rebuild::{
    cleanup_removed_chunk, poll_builds, prepare_assets, set_voxel, start_changed_builds,
};
use streaming::{ChunkIndex, StoredChunks, StreamOffsets, StreamState, stream_chunks};

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
        app.add_plugins((
            MaterialPlugin::<VoxelMaterial>::default(),
            MaterialPlugin::<WaterMaterial>::default(),
        ))
        .insert_resource(self.settings.clone())
        .insert_resource(WorldGenerator::new(self.settings.seed))
        .insert_resource(StreamOffsets::new(self.settings.view_distance))
        .init_resource::<ChunkIndex>()
        .init_resource::<StoredChunks>()
        .init_resource::<StreamState>()
        .add_observer(set_voxel)
        .add_observer(cleanup_removed_chunk)
        .add_systems(PreStartup, prepare_assets)
        .add_systems(Update, stream_chunks.run_if(in_state(GameState::Game)))
        .add_systems(
            PostUpdate,
            (start_changed_builds, poll_builds)
                .chain()
                .run_if(in_state(GameState::Game)),
        )
        .add_systems(
            PostUpdate,
            update_underwater_effect.run_if(in_state(GameState::Game)),
        )
        .add_systems(OnExit(GameState::Game), cleanup_voxel_world);
    }
}

fn update_underwater_effect(
    settings: Res<VoxelSettings>,
    index: Res<ChunkIndex>,
    chunks: Query<&ChunkVoxels>,
    mut camera: Single<(&Transform, &mut DistanceFog, &mut AmbientLight), With<VoxelViewer>>,
    mut clear_color: ResMut<ClearColor>,
    mut previous: Local<Option<bool>>,
) {
    let world_position = camera.0.translation.floor().as_ivec3();
    let (chunk_position, local_position) = settings.split_world_position(world_position);
    let underwater = index
        .get(&chunk_position)
        .and_then(|entity| chunks.get(*entity).ok())
        .and_then(|voxels| voxels.get(local_position))
        == Some(VoxelKind::Water);

    if *previous == Some(underwater) {
        return;
    }
    *previous = Some(underwater);

    let (_, fog, ambient) = &mut *camera;
    if underwater {
        fog.color = Color::srgb_u8(18, 72, 98);
        fog.falloff = FogFalloff::Linear {
            start: 2.0,
            end: 35.0,
        };
        ambient.color = Color::srgb_u8(80, 145, 170);
        ambient.brightness = 260.0;
        clear_color.0 = Color::srgb_u8(18, 72, 98);
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
    chunks: Query<Entity, With<VoxelChunk>>,
    mut chunk_index: ResMut<ChunkIndex>,
    mut stored_chunks: ResMut<StoredChunks>,
    mut stream_state: ResMut<StreamState>,
) {
    for entity in &chunks {
        commands.entity(entity).despawn();
    }
    chunk_index.clear();
    stored_chunks.clear();
    *stream_state = StreamState::default();
}

#[derive(Resource, Clone, Debug)]
pub struct VoxelSettings {
    pub chunk_size: i32,
    pub base_height: f32,
    pub max_height: i32,
    pub seed: u32,
    pub view_distance: u32,
    pub spawn_budget_per_frame: usize,
    pub despawn_budget_per_frame: usize,
}

impl Default for VoxelSettings {
    fn default() -> Self {
        Self {
            chunk_size: 16,
            base_height: 8.0,
            max_height: 40,
            seed: 42,
            view_distance: 8,
            spawn_budget_per_frame: 4,
            despawn_budget_per_frame: 8,
        }
    }
}

impl VoxelSettings {
    pub fn chunk_center(&self, chunk: IVec2) -> Vec3 {
        let origin = (chunk * self.chunk_size).as_vec2();
        let half = self.chunk_size as f32 * 0.5;
        Vec3::new(origin.x + half, self.base_height, origin.y + half)
    }

    pub fn chunk_at(&self, world: Vec3) -> IVec2 {
        IVec2::new(
            (world.x.floor() as i32).div_euclid(self.chunk_size),
            (world.z.floor() as i32).div_euclid(self.chunk_size),
        )
    }

    pub fn split_world_position(&self, world: IVec3) -> (IVec2, IVec3) {
        (
            IVec2::new(
                world.x.div_euclid(self.chunk_size),
                world.z.div_euclid(self.chunk_size),
            ),
            IVec3::new(
                world.x.rem_euclid(self.chunk_size),
                world.y,
                world.z.rem_euclid(self.chunk_size),
            ),
        )
    }
}
