//! Minecraft movement profiles and camera animation.
use crate::voxel::VoxelState;
use bevy::{math::DVec3, prelude::*};
use bevy_voxel::character::{
    CharacterBody, CharacterShape, CharacterState, CharacterSystems, Pose,
};
use bevy_voxel::character::{
    CharacterPresentation, CollisionCapabilities, MovementProfile, MovementRules, VoxelCharacter,
};
const GRAVITY: f64 = 0.08;
const VERTICAL_DRAG: f64 = 0.98;
const AIR_DRAG: f64 = 0.91;
const DEFAULT_SLIPPERINESS: f64 = 0.6;
const WALK_SPEED: f64 = 0.1;
const SPRINT_FACTOR: f64 = 1.3;
const SNEAK_FACTOR: f64 = 0.3;
const INPUT_SCALE: f64 = 0.98;
const AIR_SPEED: f64 = 0.02;
const AIR_SPRINT_SPEED: f64 = 0.026;
const JUMP_VELOCITY: f64 = 0.42;
const SPRINT_JUMP_BOOST: f64 = 0.2;
const STEP_HEIGHT: f64 = 0.6;
const AUTO_JUMP_HEIGHT: f64 = 1.2;
const AUTO_JUMP_PROBE: f64 = 0.3;
const SNEAK_EDGE_STEP: f64 = 0.05;
const FLUID_SWIM_UP: f64 = 0.04;
const FLUID_JUMP_THRESHOLD: f64 = 0.4;
const SHORE_JUMP: f64 = 0.3;
const CLIMB_SPEED: f64 = 0.15;
const CLIMB_RISE: f64 = 0.2;
const MIN_VELOCITY: f64 = 0.003;
const FLY_SPEED: f64 = 0.05;
const FLY_VERTICAL: f64 = FLY_SPEED * 3.0;
const FLY_VERTICAL_DRAG: f64 = 0.6;

pub static PLAYER_SHAPE: CharacterShape = CharacterShape {
    half_width: 0.3,
    origin_height: 0.9,
    heights: [1.8, 1.5, 0.6],
    eyes: [1.62, 1.27, 0.4],
    step_height: STEP_HEIGHT,
    jump_velocity: JUMP_VELOCITY,
    speed: 1.0,
};

#[derive(Component, Default)]
#[require(VoxelCharacter, CharacterBody = CharacterBody(&PLAYER_SHAPE), CharacterView)]
pub struct GameCharacter;

#[derive(Component, Default, Debug, Clone, Copy)]
pub struct CharacterView {
    /// Camera height above the origin: it eases toward the pose's eyes, as in vanilla.
    eye: f32,
    previous_eye: f32,
    walk_distance: f32,
    walk_delta: f32,
    bob: f32,
    previous_bob: f32,
}
impl CharacterView {
    /// Minecraft's backwards walk interpolation and tick-smoothed bob amplitude.
    pub(crate) fn walking_animation(&self, alpha: f32) -> (f32, f32) {
        (
            -(self.walk_distance + self.walk_delta * alpha),
            self.previous_bob.lerp(self.bob, alpha),
        )
    }

    /// Eased eye height above the entity origin, for the camera.
    pub fn eye_offset(&self, alpha: f32) -> f32 {
        self.previous_eye.lerp(self.eye, alpha)
    }
}

impl CharacterPresentation for CharacterView {
    fn reset(&mut self, body: &CharacterBody, pose: Pose) {
        *self = Self {
            eye: body.eye_offset(pose),
            previous_eye: body.eye_offset(pose),
            ..default()
        };
    }
    fn snapshot(&mut self) {
        self.previous_eye = self.eye;
    }
    fn begin(&mut self, state: &CharacterState, velocity: DVec3, flying: bool) {
        self.previous_bob = self.bob;
        let target = if state.on_ground && !flying && state.pose != Pose::Swimming {
            (velocity.xz().length() as f32).min(0.1)
        } else {
            0.0
        };
        self.bob += (target - self.bob) * 0.4;
    }
    fn pose(&mut self, body: &CharacterBody, pose: Pose) {
        self.eye += (body.eye_offset(pose) - self.eye) * 0.5;
    }
    fn finish(&mut self, distance: f32) {
        self.walk_delta = distance * 0.6;
        self.walk_distance = (self.walk_distance + self.walk_delta).rem_euclid(2.0);
    }
}

pub struct CharacterPlugin;
impl MovementRules for CharacterPlugin {
    const PROFILE: MovementProfile = MovementProfile {
        gravity: GRAVITY,
        vertical_drag: VERTICAL_DRAG,
        air_drag: AIR_DRAG,
        default_slipperiness: DEFAULT_SLIPPERINESS,
        walk_speed: WALK_SPEED,
        sprint_factor: SPRINT_FACTOR,
        sneak_factor: SNEAK_FACTOR,
        input_scale: INPUT_SCALE,
        air_speed: AIR_SPEED,
        air_sprint_speed: AIR_SPRINT_SPEED,
        sprint_jump_boost: SPRINT_JUMP_BOOST,
        auto_jump_height: AUTO_JUMP_HEIGHT,
        auto_jump_probe: AUTO_JUMP_PROBE,
        sneak_edge_step: SNEAK_EDGE_STEP,
        fluid_swim_up: FLUID_SWIM_UP,
        fluid_jump_threshold: FLUID_JUMP_THRESHOLD,
        shore_jump: SHORE_JUMP,
        climb_speed: CLIMB_SPEED,
        climb_rise: CLIMB_RISE,
        min_velocity: MIN_VELOCITY,
        fly_speed: FLY_SPEED,
        fly_vertical: FLY_VERTICAL,
        fly_vertical_drag: FLY_VERTICAL_DRAG,
        fly_sprint_factor: 2.0,
        swim_pitch_threshold: -0.2,
        swim_down_response: 0.085,
        swim_response: 0.06,
        swim_probe_height: 0.9,
        ground_acceleration: 0.216,
        slow_falling_gravity: 0.01,
        fluid_fall_correction: 0.005,
    };
}
impl Plugin for CharacterPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(bevy_voxel::character::CharacterPlugin::<
            VoxelState,
            Self,
            CharacterView,
        >::default())
            .configure_sets(
                FixedUpdate,
                CharacterSystems
                    .in_set(crate::simulation::SimulationSystems::Actors)
                    .run_if(crate::simulation::running),
            )
            .add_systems(
                PreStartup,
                prepare_collision_capabilities
                    .after(bevy_voxel::engine::blocks::FinalizeBlockContent),
            );
    }
}
fn prepare_collision_capabilities(mut commands: Commands) {
    commands.insert_resource(CollisionCapabilities {
        climbable: crate::voxel::VoxelKind::all()
            .map(|kind| {
                crate::block_behavior::catalog::state_behavior(kind.default_state()).climbable
            })
            .collect(),
    });
}
