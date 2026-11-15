//! Spatial activation and durable ownership of moving entities.
use bevy::{ecs::query::QueryFilter, prelude::*};
use hollow_core::coordinates::chunk_at;
use hollow_engine::{
    blocks::ColumnOwners,
    simulation::{ColumnData, SimulatedIn, Simulating},
};
use hollow_features::motion::FixedPosition;

pub fn update_mobile_owners<A: Send + Sync + 'static, P: Component, F: QueryFilter>(
    mut commands: Commands,
    columns: Res<ColumnOwners>,
    mut owners: Query<&mut ColumnData<A>>,
    active: Query<(), With<Simulating>>,
    mobiles: Query<
        (
            Entity,
            &FixedPosition,
            Option<&SimulatedIn>,
            Has<Simulating>,
            Has<P>,
        ),
        F,
    >,
) {
    for (entity, position, previous, was_active, creature) in &mobiles {
        let chunk = chunk_at(position.current.as_vec3());
        let owner = if let Some(previous) = previous
            && owners
                .get(previous.0)
                .is_ok_and(|column| column.position == chunk)
        {
            Some(previous.0)
        } else {
            columns.get(&chunk).copied()
        };
        let changed_owner = owner != previous.map(|previous| previous.0);
        let is_active = owner.is_some_and(|owner| active.contains(owner));
        if !changed_owner && was_active == is_active {
            continue;
        }
        if changed_owner && creature {
            for owner in [previous.map(|p| p.0), owner].into_iter().flatten() {
                if let Ok(mut column) = owners.get_mut(owner) {
                    column.revision += 1;
                }
            }
        }
        let mut mobile = commands.entity(entity);
        if changed_owner {
            if let Some(owner) = owner {
                mobile.insert(SimulatedIn(owner));
            } else {
                mobile.remove::<SimulatedIn>();
            }
        }
        if is_active {
            mobile.insert(Simulating);
        } else {
            mobile.remove::<Simulating>();
        }
    }
}
