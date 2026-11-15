//! Bounded relighting for model instances owned by materialized voxel blocks.
use bevy::{
    ecs::{entity::EntityHashSet, system::SystemParam},
    mesh::MeshTag,
    prelude::*,
};
use hollow_core::VoxelCell;
use hollow_engine::{
    blocks::{BlockEntities, BlockEntity, ColumnOwners, Dormant},
    world::VoxelWorld,
};

pub trait InstanceView: Component {
    fn meshes(&self, root: Entity) -> impl IntoIterator<Item = Entity>;
}
pub fn register<T: VoxelCell>(app: &mut App) {
    app.init_resource::<LightQueue>()
        .add_observer(changed::<T>)
        .add_observer(light_changed)
        .add_observer(chunk_departed)
        .add_observer(clear_updates);
}
fn clear_updates(_: On<hollow_engine::session::SessionClosed>, mut lights: ResMut<LightQueue>) {
    lights.clear();
}

#[derive(Component)]
#[relationship(relationship_target = Instances)]
pub struct InstanceOf(pub Entity);
#[derive(Component, Default, Deref)]
#[relationship_target(relationship = InstanceOf, linked_spawn)]
pub struct Instances(Vec<Entity>);
#[derive(Component, Default)]
#[component(storage = "SparseSet", on_add = hollow_engine::work::enqueue::<Self>)]
pub struct Relight {
    offset: usize,
    rerun: bool,
}
type LightQueue = hollow_engine::work::EntityWork<Relight>;
#[derive(Component, Default)]
pub struct InstanceLight(u32);

#[derive(SystemParam)]
pub struct LightInvalidation<'w, 's> {
    columns: Res<'w, ColumnOwners>,
    progress: Query<'w, 's, &'static mut Relight>,
    commands: Commands<'w, 's>,
}
impl LightInvalidation<'_, '_> {
    fn column(&mut self, position: IVec2) {
        let Some(&entity) = self.columns.get(&position) else {
            return;
        };
        if let Ok(mut progress) = self.progress.get_mut(entity) {
            if !progress.rerun {
                progress.rerun = true;
            }
        } else {
            // Repeated invalidations before a flush must not replace active progress.
            self.commands
                .entity(entity)
                .try_insert_if_new(Relight::default());
        }
    }
    pub fn neighbors(&mut self, position: IVec2) {
        for offset in [IVec2::ZERO, IVec2::X, IVec2::NEG_X, IVec2::Y, IVec2::NEG_Y] {
            self.column(position + offset);
        }
    }
}
fn changed<T: VoxelCell>(
    event: On<hollow_engine::edits::BlockChanged<T>>,
    mut invalidation: LightInvalidation,
) {
    let positions = [IVec3::ZERO, IVec3::X, IVec3::NEG_X, IVec3::Z, IVec3::NEG_Z]
        .map(|delta| hollow_core::coordinates::split_world_position(event.position + delta).0);
    for (index, &position) in positions.iter().enumerate() {
        if !positions[..index].contains(&position) {
            invalidation.column(position);
        }
    }
}
fn chunk_departed(
    event: On<Despawn<hollow_engine::chunk::VoxelChunk>>,
    chunks: Query<&hollow_engine::chunk::VoxelChunk>,
    mut invalidation: LightInvalidation,
) {
    if let Ok(chunk) = chunks.get(event.entity) {
        invalidation.neighbors(chunk.position);
    }
}
fn light_changed(
    event: On<hollow_engine::lighting::ChunkLightChanged>,
    chunks: Query<&hollow_engine::chunk::VoxelChunk>,
    mut invalidation: LightInvalidation,
) {
    if let Ok(chunk) = chunks.get(event.0) {
        invalidation.neighbors(chunk.position);
    }
}
pub fn instance_light<T: VoxelCell>(world: &VoxelWorld<T>, position: IVec3) -> u32 {
    let mut sky = 0;
    let mut block = 0;
    for delta in [
        IVec3::ZERO,
        IVec3::Y,
        IVec3::X,
        IVec3::NEG_X,
        IVec3::Z,
        IVec3::NEG_Z,
    ] {
        if let Some(value) = world.light(position + delta) {
            sky = sky.max(value >> 4);
            block = block.max(value & 15);
        }
    }
    u32::from((sky << 4) | block)
}
pub fn update_lighting<T: VoxelCell, V: InstanceView, const BUDGET: usize>(
    mut commands: Commands,
    mut queue: ResMut<LightQueue>,
    mut samples: Local<EntityHashSet>,
    mut columns: Query<(Option<&BlockEntities>, &mut Relight)>,
    mut blocks: Query<(&BlockEntity, &Instances, &mut InstanceLight), Without<Dormant>>,
    views: Query<&V>,
    mut tags: Query<&mut MeshTag>,
    world: VoxelWorld<T>,
) {
    samples.clear();
    for _ in 0..BUDGET {
        let Some(column) = queue.pop_front() else {
            break;
        };
        let Ok((blocks_in_column, mut progress)) = columns.get_mut(column) else {
            continue;
        };
        if let Some(entity) = blocks_in_column
            .and_then(|blocks| blocks.get(progress.offset))
            .copied()
        {
            samples.insert(entity);
        }
        progress.offset += 1;
        if progress.offset >= blocks_in_column.map_or(0, |blocks| blocks.len()) {
            if !progress.rerun {
                commands.entity(column).remove::<Relight>();
                continue;
            }
            progress.offset = 0;
            progress.rerun = false;
        }
        queue.push_back(column);
    }
    blocks
        .par_iter_many_unique_mut(&*samples)
        .for_each(|result| {
            if let Ok((block, _, mut sample)) = result {
                sample.0 = instance_light(&world, block.position);
            }
        });
    for &entity in samples.iter() {
        let Ok((_, block_views, sample)) = blocks.get(entity) else {
            continue;
        };
        for root in block_views.iter() {
            if let Ok(view) = views.get(root) {
                for target in view.meshes(root) {
                    if let Ok(mut tag) = tags.get_mut(target) {
                        tag.set_if_neq(MeshTag::new(sample.0));
                    }
                }
            }
        }
    }
}
