//! Registration runs during plugin construction. Runtime catalogs are immutable.
use bevy_app::{App, Plugin, PreStartup};
use bevy_derive::Deref;
use bevy_ecs::prelude::*;
pub use hollow_core::content::*;

#[derive(Resource)]
pub struct ContentRegistrations(pub Option<ContentBuilder>);
impl Default for ContentRegistrations {
    fn default() -> Self {
        Self(Some(ContentBuilder::new()))
    }
}
impl ContentRegistrations {
    pub fn builder(&mut self) -> &mut ContentBuilder {
        self.0.as_mut().expect("Content already frozen")
    }
}
#[derive(Resource, Clone, Deref)]
pub struct RegisteredContent(pub ContentCatalog);
#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FreezeContent;
pub struct ContentPlugin;
impl Plugin for ContentPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ContentRegistrations>()
            .add_systems(PreStartup, freeze.in_set(FreezeContent));
    }
}
fn freeze(mut commands: Commands, mut registrations: ResMut<ContentRegistrations>) {
    let builder = registrations.0.take().expect("Content already frozen");
    commands.insert_resource(RegisteredContent(builder.freeze()));
}

#[cfg(feature = "content-ron")]
pub mod import;
