use avian3d::{
    collision::collider::Collider,
    spatial_query::{ShapeCaster, ShapeHits},
};
use bevy::prelude::*;

pub struct CharacterPlugin;

#[derive(Component)]
pub struct OnGround;

fn caster() -> ShapeCaster {
    ShapeCaster::new(
        Collider::sphere(0.25),
        Vec3::ZERO,
        Quat::IDENTITY,
        Dir3::NEG_Y,
    )
    .with_max_distance(0.75)
    .with_ignore_self(true)
}

#[derive(Component)]
#[require(ShapeCaster = caster())]
pub struct OnGroundSensor;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (add_grounded_state, remove_grounded_state));
    }
}

fn add_grounded_state(
    mut query: Query<(Entity, &ShapeHits), (With<OnGroundSensor>, Without<OnGround>)>,
    mut commands: Commands,
) {
    for (entity, hits) in &mut query {
        if hits.iter().any(|hit| hit.normal1.y > 0.5) {
            commands.entity(entity).insert(OnGround);
        }
    }
}

fn remove_grounded_state(
    mut query: Query<(Entity, &ShapeHits), (With<OnGroundSensor>, With<OnGround>)>,
    mut commands: Commands,
) {
    for (entity, hits) in &mut query {
        if !hits.iter().any(|hit| hit.normal1.y > 0.5) {
            commands.entity(entity).remove::<OnGround>();
        }
    }
}
