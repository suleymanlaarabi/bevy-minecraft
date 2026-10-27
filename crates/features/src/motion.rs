//! Controller-independent fixed-step translation and render interpolation.
use bevy_app::{App, Plugin, RunFixedMainLoop, RunFixedMainLoopSystems};
use bevy_ecs::prelude::*;
use bevy_math::{DVec3, Vec3};
use bevy_time::{Fixed, Time};
use bevy_transform::components::Transform;

/// Keep the simulation samples together; controllers retain their own velocity units.
/// Gameplay reads `current`, while rendering alone writes `Transform`.
#[derive(Component, Default, Debug, Clone, Copy)]
#[require(Transform)]
pub struct FixedPosition {
    pub current: DVec3,
    pub previous: DVec3,
    rendered: Option<Vec3>,
}

impl FixedPosition {
    pub fn new(position: Vec3) -> Self {
        Self {
            current: position.as_dvec3(),
            previous: position.as_dvec3(),
            rendered: Some(position),
        }
    }

    /// Reset both samples and let the controller reinitialize its own state next tick.
    pub fn teleport(&mut self, position: Vec3) {
        *self = Self::new(position);
        self.rendered = None;
    }

    /// An external transform write is a teleport; interpolation must not overwrite it.
    pub fn externally_moved(&self, transform: &Transform) -> bool {
        self.rendered
            .is_none_or(|written| written.distance_squared(transform.translation) > 1e-6)
    }

    pub fn acknowledge(&mut self, transform: &Transform) {
        self.rendered = Some(transform.translation);
    }
}

/// Shared render sample; suspending this set also freezes all controller pose interpolation.
#[derive(Resource, Default, PartialEq)]
pub struct InterpolationAlpha(pub f32);

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct InterpolatePosition;

pub struct FixedPositionPlugin;
impl Plugin for FixedPositionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InterpolationAlpha>().add_systems(
            RunFixedMainLoop,
            interpolate
                .in_set(InterpolatePosition)
                .in_set(RunFixedMainLoopSystems::AfterFixedMainLoop),
        );
    }
}

fn interpolate(
    time: Res<Time<Fixed>>,
    mut interpolation: ResMut<InterpolationAlpha>,
    mut bodies: Query<(&mut FixedPosition, &mut Transform)>,
) {
    let alpha = time.overstep_fraction();
    interpolation.set_if_neq(InterpolationAlpha(alpha));
    for (mut position, mut transform) in &mut bodies {
        if position.externally_moved(&transform) {
            continue;
        }
        let translation = position
            .previous
            .lerp(position.current, alpha as f64)
            .as_vec3();
        if transform.translation != translation {
            transform.translation = translation;
        }
        // Presentation must not dirty gameplay snapshots or spatial ownership queries.
        position.bypass_change_detection().rendered = Some(translation);
    }
}
