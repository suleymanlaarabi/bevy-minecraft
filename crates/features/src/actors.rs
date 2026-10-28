//! Small controller-independent actor capabilities. Content plugins supply policy.
use bevy_ecs::prelude::*;
use bevy_math::Vec3;

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum Behavior {
    #[default]
    Idle,
    Wander,
    Flee,
    EscapeWater,
}

#[derive(Component, Default)]
pub struct Decision {
    pub behavior: Behavior,
    pub next_tick: u64,
}

#[derive(Component, Default)]
pub struct ThreatMemory {
    pub target: Option<Entity>,
    pub position: Vec3,
    pub expires: u64,
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq)]
pub enum NavigationStatus {
    #[default]
    Idle,
    Pending,
    Following,
    Failed,
}

#[derive(Component, Default)]
pub struct Navigation {
    pub destination: Option<Vec3>,
    pub status: NavigationStatus,
    pub retry_tick: u64,
    /// Opaque slot owned by the navigation provider.
    pub path: Option<PathHandle>,
    pub cursor: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PathHandle(pub u32);

impl Navigation {
    /// Providers release any previous path when they observe the new status.
    pub fn set_destination(&mut self, destination: Option<Vec3>) {
        if self.destination == destination {
            return;
        }
        self.destination = destination;
        self.cursor = 0;
        self.retry_tick = 0;
        self.status = if destination.is_some() {
            NavigationStatus::Pending
        } else {
            NavigationStatus::Idle
        };
    }
}

#[derive(Component, Default)]
pub struct LocomotionAnimation {
    pub phase: f32,
    pub phase_delta: f32,
    pub stride: f32,
    pub previous_stride: f32,
}
