//! Headless terrain composition using neither Minecraft data nor its generator.
use bevy_app::{App, Startup, TaskPoolPlugin, Update};
use bevy_ecs::prelude::*;
use bevy_math::prelude::*;
use hollow_core::{BlockId, VoxelCell, coordinates::SECTION_COUNT, sections::Layer};
use hollow_engine::{
    VoxelWorldPlugin,
    chunk::{ChunkVoxels, VoxelChunk},
    edits::{CheckedEdit, VoxelEditWorld},
    world::ChunkIndex,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Cell(u32);
impl VoxelCell for Cell {
    type Kind = BlockId;
    fn id(self) -> u32 {
        self.0
    }
    fn from_id(id: u32) -> Option<Self> {
        matches!(id, 0 | 70_000 | 70_001).then_some(Self(id))
    }
    fn kind(self) -> BlockId {
        BlockId(u32::from(self.0 != 0))
    }
    fn default_state(kind: BlockId) -> Self {
        match kind.0 {
            0 => Self(0),
            1 => Self(70_000),
            _ => panic!("Unregistered example block"),
        }
    }
    fn air() -> Self {
        Self(0)
    }
    fn is_liquid(self) -> bool {
        false
    }
    fn is_air(self) -> bool {
        self.0 == 0
    }
    fn is_opaque(self) -> bool {
        self.0 != 0
    }
}
fn main() {
    App::new()
        .add_plugins((
            TaskPoolPlugin::default(),
            VoxelWorldPlugin::<Cell>::default(),
        ))
        .add_systems(Startup, create_column)
        .add_systems(Update, paint_cell)
        .run();
}
fn create_column(mut commands: Commands, mut index: ResMut<ChunkIndex>) {
    let entity = commands
        .spawn((
            VoxelChunk {
                position: IVec2::ZERO,
            },
            ChunkVoxels::from_layers(
                Layer::filled(Cell(70_000), SECTION_COUNT),
                Layer::filled(0xf0, SECTION_COUNT),
            ),
        ))
        .id();
    index.insert(IVec2::ZERO, entity);
}
fn paint_cell(mut world: VoxelEditWorld<Cell>, mut commands: Commands) {
    let edits = [CheckedEdit {
        position: IVec3::ZERO,
        expected: Cell(70_000),
        state: Cell(70_001),
    }];
    // Stale input leaves the terrain untouched, just as it does in gameplay.
    let _ = world.commit(&edits, &mut commands);
}
