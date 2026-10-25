//! Retain a grounded checkpoint; a streaming failure must not become an endless fall.
use super::Player;
use crate::voxel::VoxelWorld;
use avian3d::prelude::*;
use bevy::prelude::*;

#[derive(Component, Default)]
pub(super) struct PlayerCheckpoint(Option<Vec3>);

pub(super) fn register(app: &mut App) {
    app.add_systems(
        FixedPostUpdate,
        recover_player
            .before(PhysicsSystems::Prepare)
            .run_if(crate::world::playing),
    );
}
fn recover_player(
    world: VoxelWorld,
    mut players: Query<
        (
            &mut Position,
            &mut Transform,
            &mut PlayerCheckpoint,
            &crate::character::CharacterState,
        ),
        With<Player>,
    >,
) {
    for (mut position, mut transform, mut checkpoint, state) in &mut players {
        if !position.0.is_finite() || position.y < -8.0 {
            if let Some(safe) = checkpoint.0 {
                position.0 = safe;
                // The controller sees the jump as a teleport and drops its velocity.
                transform.translation = safe;
            }
        } else if world.collision_ready(position.0) && (state.on_ground || checkpoint.0.is_none()) {
            checkpoint.0 = Some(position.0);
        }
    }
}
