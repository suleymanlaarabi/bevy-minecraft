//! Explicit camera presentation capabilities, independent of application settings.
mod bloom;
mod composite;
mod present;
mod resolution;
mod targets;
use bevy::prelude::*;
pub use present::DirectPresent;
pub use resolution::SceneResolution;
pub use targets::{CompactHdr, UnsampledDepthPrepass};
pub struct PresentationPlugin;
impl Plugin for PresentationPlugin {
    fn build(&self, app: &mut App) {
        resolution::register(app);
        composite::register(app);
        present::register(app);
        targets::register(app);
    }
}
