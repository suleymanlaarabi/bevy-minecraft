use avian3d::{
    collision::collider::Collider,
    spatial_query::{ShapeCaster, ShapeHits},
};
use bevy::prelude::*;

use crate::voxel::VoxelWorld;

pub const CHARACTER_GRAVITY_SCALE: f32 = 2.8;
pub const CHARACTER_WATER_GRAVITY_SCALE: f32 = 0.15;

pub struct CharacterPlugin;

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
pub struct OnGround;

#[derive(Component, Default)]
#[require(ShapeCaster = caster())]
pub struct OnGroundSensor;

#[derive(Component)]
pub struct InWater;

#[derive(Component, Default)]
pub struct InWaterSensor;

#[derive(Component, Default)]
#[require(OnGroundSensor, InWaterSensor)]
pub struct GameCharacter;

impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                add_grounded_state,
                remove_grounded_state,
                add_in_water_state,
                remove_in_water_state,
            ),
        );
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

fn add_in_water_state(
    mut query: Query<(Entity, &Transform), (With<InWaterSensor>, Without<InWater>)>,
    voxel_world: VoxelWorld,

    mut commands: Commands,
) {
    for (entity, transform) in &mut query {
        if voxel_world
            .get_at(transform.translation)
            .is_some_and(|kind| kind.is_liquid())
        {
            commands.entity(entity).insert(InWater);
        }
    }
}

fn remove_in_water_state(
    mut query: Query<(Entity, &Transform), (With<InWaterSensor>, With<InWater>)>,
    voxel_world: VoxelWorld,

    mut commands: Commands,
) {
    for (entity, transform) in &mut query {
        if !voxel_world
            .get_at(transform.translation)
            .is_some_and(|kind| kind.is_liquid())
        {
            commands.entity(entity).remove::<InWater>();
        }
    }
}
