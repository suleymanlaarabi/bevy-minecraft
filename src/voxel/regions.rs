use super::{
    VoxelSettings,
    meshing::{ChunkMeshes, VoxelGeometry},
    rebuild::VoxelAssets,
};
use crate::game::GameState;
use bevy::{
    light::{NotShadowCaster, NotShadowReceiver},
    platform::collections::HashMap,
    prelude::*,
};

const REGION_SIZE: i32 = 4;
const SETTLE_FRAMES: u8 = 4;

#[derive(Component)]
pub(crate) struct VoxelRenderRegion;

struct RegionEntities {
    terrain: Entity,
    water: Option<Entity>,
}

#[derive(Resource, Default)]
pub(crate) struct RenderRegions {
    pieces: HashMap<IVec2, HashMap<IVec2, ChunkMeshes>>,
    entities: HashMap<IVec2, RegionEntities>,
    dirty: HashMap<IVec2, u8>,
}

impl RenderRegions {
    pub(crate) fn insert(&mut self, chunk: IVec2, meshes: ChunkMeshes) {
        let region = region_of(chunk);
        let replaced = self
            .pieces
            .entry(region)
            .or_default()
            .insert(chunk, meshes)
            .is_some();
        self.dirty
            .insert(region, if replaced { SETTLE_FRAMES } else { 0 });
    }

    pub(crate) fn remove(&mut self, chunk: IVec2) {
        let region = region_of(chunk);
        let Some(pieces) = self.pieces.get_mut(&region) else {
            return;
        };
        if pieces.remove(&chunk).is_none() {
            return;
        }
        if pieces.is_empty() {
            self.pieces.remove(&region);
        }
        self.dirty.insert(region, SETTLE_FRAMES);
    }

    #[cfg(feature = "dev")]
    pub(crate) fn is_settled(&self) -> bool {
        self.dirty.is_empty()
    }

    #[cfg(feature = "dev")]
    pub(crate) fn dirty_len(&self) -> usize {
        self.dirty.len()
    }

    pub(crate) fn clear(&mut self, commands: &mut Commands) {
        for entities in self.entities.values() {
            commands.entity(entities.terrain).despawn();
        }
        *self = Self::default();
    }
}

pub(crate) fn rebuild_render_regions(
    mut commands: Commands,
    settings: Res<VoxelSettings>,
    assets: Res<VoxelAssets>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut regions: ResMut<RenderRegions>,
) {
    for age in regions.dirty.values_mut() {
        *age = age.saturating_add(1);
    }
    let ready: Vec<_> = regions
        .dirty
        .iter()
        .filter(|(_, age)| **age >= SETTLE_FRAMES)
        .map(|(&region, _)| region)
        .take(settings.spawn_budget_per_frame)
        .collect();

    for region in ready {
        regions.dirty.remove(&region);
        let Some(pieces) = regions.pieces.get(&region) else {
            if let Some(entities) = regions.entities.remove(&region) {
                commands.entity(entities.terrain).despawn();
            }
            continue;
        };

        let mut chunks: Vec<_> = pieces.keys().copied().collect();
        chunks.sort_unstable_by_key(|chunk| (chunk.y, chunk.x));
        let region_chunk_origin = region * REGION_SIZE;
        let mut terrain = VoxelGeometry::default();
        let mut water = VoxelGeometry::default();
        for chunk in chunks {
            let meshes = &pieces[&chunk];
            let local = (chunk - region_chunk_origin) * settings.chunk_size;
            let offset = Vec3::new(local.x as f32, 0.0, local.y as f32);
            terrain.append(&meshes.0, offset);
            if let Some(chunk_water) = &meshes.1 {
                water.append(chunk_water, offset);
            }
        }

        let terrain_mesh = mesh_assets.add(terrain.into_mesh());
        let region_world_origin = region_chunk_origin * settings.chunk_size;
        if let Some(mut entities) = regions.entities.remove(&region) {
            commands
                .entity(entities.terrain)
                .insert(Mesh3d(terrain_mesh));
            update_water(
                &mut commands,
                &mut mesh_assets,
                &assets,
                entities.terrain,
                &mut entities.water,
                water,
            );
            regions.entities.insert(region, entities);
        } else {
            let terrain_entity = commands
                .spawn((
                    VoxelRenderRegion,
                    Mesh3d(terrain_mesh),
                    MeshMaterial3d(assets.terrain.clone()),
                    Transform::from_xyz(
                        region_world_origin.x as f32,
                        0.0,
                        region_world_origin.y as f32,
                    ),
                    DespawnOnExit(GameState::Game),
                ))
                .id();
            let mut entities = RegionEntities {
                terrain: terrain_entity,
                water: None,
            };
            update_water(
                &mut commands,
                &mut mesh_assets,
                &assets,
                terrain_entity,
                &mut entities.water,
                water,
            );
            regions.entities.insert(region, entities);
        }
    }
}

fn update_water(
    commands: &mut Commands,
    mesh_assets: &mut Assets<Mesh>,
    assets: &VoxelAssets,
    terrain: Entity,
    water_entity: &mut Option<Entity>,
    water: VoxelGeometry,
) {
    match (water.is_empty(), *water_entity) {
        (false, Some(entity)) => {
            commands
                .entity(entity)
                .insert(Mesh3d(mesh_assets.add(water.into_mesh())));
        }
        (false, None) => {
            let entity = commands
                .spawn((
                    Mesh3d(mesh_assets.add(water.into_mesh())),
                    MeshMaterial3d(assets.water.clone()),
                    ChildOf(terrain),
                    NotShadowCaster,
                    NotShadowReceiver,
                ))
                .id();
            *water_entity = Some(entity);
        }
        (true, Some(entity)) => {
            commands.entity(entity).despawn();
            *water_entity = None;
        }
        (true, None) => {}
    }
}

pub(crate) fn region_of(chunk: IVec2) -> IVec2 {
    IVec2::new(
        chunk.x.div_euclid(REGION_SIZE),
        chunk.y.div_euclid(REGION_SIZE),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn region_coordinates_handle_negative_chunks() {
        assert_eq!(region_of(IVec2::new(0, 0)), IVec2::ZERO);
        assert_eq!(region_of(IVec2::new(3, 3)), IVec2::ZERO);
        assert_eq!(region_of(IVec2::new(4, 4)), IVec2::ONE);
        assert_eq!(region_of(IVec2::new(-1, -1)), IVec2::NEG_ONE);
        assert_eq!(region_of(IVec2::new(-4, -4)), IVec2::NEG_ONE);
        assert_eq!(region_of(IVec2::new(-5, -5)), IVec2::splat(-2));
    }

    #[test]
    fn removals_drop_empty_region_storage() {
        let mut regions = RenderRegions::default();
        regions.insert(IVec2::ZERO, ChunkMeshes(VoxelGeometry::default(), None));
        regions.remove(IVec2::ZERO);

        assert!(regions.pieces.is_empty());
        assert_eq!(regions.dirty.get(&IVec2::ZERO), Some(&SETTLE_FRAMES));
    }

    #[test]
    fn sixteen_chunks_share_one_render_region_and_edits_skip_debounce() {
        let mut regions = RenderRegions::default();
        for z in 0..REGION_SIZE {
            for x in 0..REGION_SIZE {
                regions.insert(
                    IVec2::new(x, z),
                    ChunkMeshes(VoxelGeometry::default(), None),
                );
            }
        }

        assert_eq!(regions.pieces.len(), 1);
        assert_eq!(regions.pieces[&IVec2::ZERO].len(), 16);
        assert_eq!(regions.dirty.get(&IVec2::ZERO), Some(&0));

        regions.insert(
            IVec2::new(2, 2),
            ChunkMeshes(VoxelGeometry::default(), None),
        );
        assert_eq!(regions.dirty.get(&IVec2::ZERO), Some(&SETTLE_FRAMES));
    }
}
