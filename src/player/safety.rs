//! Retain a grounded checkpoint for invalid coordinate recovery.
use super::Player;
use crate::voxel::VoxelWorld;
use bevy::prelude::*;

#[derive(Component, Default)]
pub(super) struct PlayerCheckpoint(Option<Vec3>);

pub(super) fn register(app: &mut App) {
    app.add_systems(
        FixedUpdate,
        recover_player
            .before(crate::character::CharacterSystems)
            .run_if(crate::world::playing),
    );
}

fn recover_player(
    world: VoxelWorld,
    mut players: Query<
        (
            &mut Transform,
            &mut PlayerCheckpoint,
            &crate::character::CharacterState,
        ),
        With<Player>,
    >,
) {
    for (mut transform, mut checkpoint, state) in &mut players {
        if !transform.translation.is_finite() {
            if let Some(safe) = checkpoint.0 {
                // The controller sees the jump as a teleport and drops its velocity.
                transform.translation = safe;
            }
        } else if world.collision_ready(transform.translation)
            && (state.on_ground || checkpoint.0.is_none())
        {
            checkpoint.0 = Some(transform.translation);
        }
    }
}
