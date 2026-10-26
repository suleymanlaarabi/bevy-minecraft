//! A work component owns state; its add hook queues the entity exactly once.
use bevy::{
    ecs::{lifecycle::HookContext, world::DeferredWorld},
    prelude::*,
};
use std::{collections::VecDeque, marker::PhantomData};

#[derive(Resource, Deref, DerefMut)]
pub(crate) struct EntityWork<T: Component> {
    #[deref]
    pending: VecDeque<Entity>,
    marker: PhantomData<T>,
}
impl<T: Component> Default for EntityWork<T> {
    fn default() -> Self {
        Self {
            pending: default(),
            marker: PhantomData,
        }
    }
}
pub(crate) fn enqueue<T: Component>(mut world: DeferredWorld, context: HookContext) {
    world
        .resource_mut::<EntityWork<T>>()
        .push_back(context.entity);
}
