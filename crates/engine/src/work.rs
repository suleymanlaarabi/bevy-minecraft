//! A work component owns state; its add hook queues the entity exactly once.
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::{lifecycle::HookContext, prelude::*, world::DeferredWorld};
use std::{collections::VecDeque, marker::PhantomData};

#[derive(Resource, Deref, DerefMut)]
pub struct EntityWork<T: Component> {
    #[deref]
    pending: VecDeque<Entity>,
    marker: PhantomData<T>,
}
impl<T: Component> Default for EntityWork<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
            marker: PhantomData,
        }
    }
}
pub fn enqueue<T: Component>(mut world: DeferredWorld, context: HookContext) {
    world
        .resource_mut::<EntityWork<T>>()
        .push_back(context.entity);
}
