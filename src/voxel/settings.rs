use super::CHUNK_SIZE;
use bevy::prelude::*;

#[derive(Resource, Clone, Debug)]
pub struct VoxelSettings {
    pub base_height: f32,
    pub seed: i64,
    pub spawn_budget_per_frame: usize,
    pub despawn_budget_per_frame: usize,
}

impl Default for VoxelSettings {
    fn default() -> Self {
        Self {
            base_height: 127.0,
            seed: 12,
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
