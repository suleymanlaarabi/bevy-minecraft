//! Session ownership prevents asynchronous results crossing world lifetimes.
use bevy_ecs::prelude::*;
#[derive(Resource, Clone, Copy)]
pub struct ActiveSession(pub Entity);
#[derive(Component)]
#[relationship(relationship_target = SessionMembers)]
pub struct SessionMember(pub Entity);
#[derive(Component, Default)]
#[relationship_target(relationship = SessionMember, linked_spawn)]
pub struct SessionMembers(Vec<Entity>);

#[derive(Event)]
pub struct SessionClosed;

pub fn open_session(mut commands: Commands, current: Option<Res<ActiveSession>>) {
    assert!(
        current.is_none(),
        "Close the active world before opening another session"
    );
    let entity = commands.spawn_empty().id();
    commands.insert_resource(ActiveSession(entity));
}
pub fn close_session(mut commands: Commands, session: Option<Res<ActiveSession>>) {
    if let Some(session) = session {
        commands.entity(session.0).despawn();
    }
    commands.trigger(SessionClosed);
    commands.remove_resource::<ActiveSession>();
}
