use super::*;
use crate::{
    character::{CharacterActivity, CharacterMovement, CharacterState},
    simulation::Simulating,
    survival::DamageState,
};
use bevy::{
    camera::primitives::{Aabb, MeshAabb},
    mesh::MeshTag,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::AsBindGroup,
    shader::ShaderRef,
    transform::TransformSystems,
};
use hollow_features::{actors::LocomotionAnimation, motion::FixedPosition};

type QuadrupedMaterial = ExtendedMaterial<StandardMaterial, QuadrupedExtension>;

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
struct QuadrupedExtension {}

impl MaterialExtension for QuadrupedExtension {
    fn vertex_shader() -> ShaderRef {
        "embedded://hollow_minecraft/creatures/quadruped.wesl".into()
    }
    fn prepass_vertex_shader() -> ShaderRef {
        "embedded://hollow_minecraft/creatures/quadruped_prepass.wesl".into()
    }
    fn deferred_vertex_shader() -> ShaderRef {
        Self::prepass_vertex_shader()
    }
}

#[derive(Resource)]
struct RigAssets {
    materials: [[Handle<QuadrupedMaterial>; 2]; 2],
    meshes: [Handle<Mesh>; 2],
    bounds: [Aabb; 2],
}

#[derive(Component, Default)]
pub(super) struct CreatureAudio {
    step_distance: f32,
    step_cooldown: u8,
    ambient_cooldown: u16,
    ambient_initialized: bool,
}

impl Plugin for CreaturePresentationPlugin {
    fn build(&self, app: &mut App) {
        bevy::shader::load_shader_library!(app, "quadruped.wesl");
        bevy::shader::load_shader_library!(app, "quadruped_prepass.wesl");
        app.add_plugins(MaterialPlugin::<QuadrupedMaterial>::default())
            .add_systems(Startup, assets)
            .add_systems(
                FixedUpdate,
                (sample_animation, sample_audio)
                    .after(crate::character::CharacterSystems)
                    .in_set(crate::simulation::SimulationSystems::Actors),
            )
            .add_systems(
                PostUpdate,
                (create, orient, animate)
                    .chain()
                    .before(TransformSystems::Propagate)
                    .run_if(crate::world::streaming),
            );
    }
}
fn assets(
    mut commands: Commands,
    server: Res<AssetServer>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<QuadrupedMaterial>>,
) {
    // All animals of a species share one mesh. Vertex colors carry joint pivots;
    // the material restores white vertex color before standard PBR shading.
    let mut models: [Handle<Mesh>; 2] = default();
    let mut bounds: [Aabb; 2] = default();
    for species in 0..2 {
        let mut model: Option<Mesh> = None;
        let cow = species == 0;
        let origin_height = if cow {
            COW.body.origin_height
        } else {
            PIG.body.origin_height
        };
        let leg_height = if cow { 12.0 } else { 6.0 };
        let scale = 1.0 / 16.0;
        let mesh_scale = scale / crate::player::skin::mesh::PIXEL;
        let mut part = |position: Vec3, size: Vec3, center: Vec3, uv: Vec2, leg: f32| {
            let mut mesh =
                crate::player::skin::mesh::cuboid(size, center, uv, Vec2::splat(64.0), 0.0)
                    .scaled_by(Vec3::splat(mesh_scale))
                    // Skin cuboids face +Z; Minecraft quadrupeds face -Z.
                    .rotated_by(Quat::from_rotation_y(std::f32::consts::PI));
            // Minecraft model coordinates point down on Y. Converting to Bevy's
            // upward Y reverses the torso rotation; otherwise the udder texture
            // appears on the back of the animal.
            if uv == Vec2::new(18.0, 4.0)
                || uv == Vec2::new(28.0, 8.0)
                || uv == Vec2::new(52.0, 0.0)
            {
                mesh = mesh.rotated_by(Quat::from_rotation_x(-std::f32::consts::FRAC_PI_2));
            }
            let pivot = position * scale - Vec3::Y * origin_height;
            mesh = mesh.translated_by(pivot);
            mesh.final_aabb = None;
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_COLOR,
                vec![[pivot.x, pivot.y, pivot.z, leg]; mesh.count_vertices()],
            );
            mesh.insert_attribute(
                Mesh::ATTRIBUTE_UV_1,
                vec![[origin_height, 0.0]; mesh.count_vertices()],
            );
            if let Some(model) = model.as_mut() {
                model
                    .merge(&mesh)
                    .expect("Quadruped meshes have identical vertex layouts");
            } else {
                model = Some(mesh);
            }
        };
        if cow {
            part(
                Vec3::new(0.0, 17.0, 1.0),
                Vec3::new(12.0, 18.0, 10.0),
                Vec3::ZERO,
                Vec2::new(18.0, 4.0),
                0.0,
            );
            part(
                Vec3::new(0.0, 20.0, -11.0),
                Vec3::new(8.0, 8.0, 6.0),
                Vec3::ZERO,
                Vec2::ZERO,
                0.0,
            );
            part(
                Vec3::new(0.0, 17.5, -14.5),
                Vec3::new(6.0, 3.0, 1.0),
                Vec3::ZERO,
                Vec2::new(1.0, 33.0),
                0.0,
            );
            part(
                Vec3::new(0.0, 11.5, 7.0),
                Vec3::new(4.0, 6.0, 1.0),
                Vec3::ZERO,
                Vec2::new(52.0, 0.0),
                0.0,
            );
            for x in [-4.5, 4.5] {
                part(
                    Vec3::new(x, 22.5, -12.5),
                    Vec3::new(1.0, 3.0, 1.0),
                    Vec3::ZERO,
                    Vec2::new(22.0, 0.0),
                    0.0,
                );
            }
        } else {
            part(
                Vec3::new(0.0, 10.0, 0.0),
                Vec3::new(10.0, 16.0, 8.0),
                Vec3::ZERO,
                Vec2::new(28.0, 8.0),
                0.0,
            );
            part(
                Vec3::new(0.0, 12.0, -10.0),
                Vec3::splat(8.0),
                Vec3::ZERO,
                Vec2::ZERO,
                0.0,
            );
            part(
                Vec3::new(0.0, 10.5, -14.5),
                Vec3::new(4.0, 3.0, 1.0),
                Vec3::ZERO,
                Vec2::new(16.0, 16.0),
                0.0,
            );
        }
        for x in [-1.0, 1.0] {
            for z in [-1.0, 1.0] {
                let leg_x = if cow { 4.0 } else { 3.0 };
                let leg_z = if z < 0.0 { -5.0 } else { 7.0 };
                part(
                    Vec3::new(x * leg_x, leg_height, leg_z),
                    Vec3::new(4.0, leg_height, 4.0),
                    Vec3::new(0.0, -leg_height * 0.5, 0.0),
                    Vec2::new(0.0, 16.0),
                    x * z,
                );
            }
        }
        drop(part);
        let mut model = model.expect("Quadruped model must have parts");
        model.final_aabb = None;
        let mut aabb = model
            .get_aabb()
            .expect("Quadruped model must have vertices");
        // Include leg rotations and the small swimming bob in frustum culling.
        aabb.half_extents += bevy::math::Vec3A::splat(1.6);
        bounds[species] = aabb;
        models[species] = meshes.add(model);
    }
    let materials = ["cow", "pig"].map(|name| {
        let texture = crate::load_nearest(&server, format!("creatures/vanilla/{name}.png"));
        [Color::WHITE, Color::srgb(1.0, 0.35, 0.35)].map(|base_color| {
            materials.add(QuadrupedMaterial {
                base: StandardMaterial {
                    base_color,
                    base_color_texture: Some(texture.clone()),
                    perceptual_roughness: 1.0,
                    ..default()
                },
                extension: QuadrupedExtension::default(),
            })
        })
    });
    commands.insert_resource(RigAssets {
        materials,
        meshes: models,
        bounds,
    });
}

fn create(
    mut commands: Commands,
    actors: Query<(Entity, &Creature), Added<Creature>>,
    catalog: Res<CreatureCatalog>,
    assets: Res<RigAssets>,
) {
    for (actor, creature) in &actors {
        let Some(profile) = catalog.get(creature.0) else {
            continue;
        };
        let rig = profile.rig as usize;
        commands.entity(actor).insert((
            Mesh3d(assets.meshes[rig].clone()),
            MeshMaterial3d(assets.materials[rig][0].clone()),
            MeshTag::new(0x0080_8000),
            assets.bounds[rig],
        ));
    }
}
fn sample_animation(
    mut actors: Query<
        (&CharacterActivity, &mut LocomotionAnimation),
        (
            With<Creature>,
            With<Simulating>,
            Without<super::combat::Dying>,
        ),
    >,
) {
    let update = |(activity, mut animation): (&CharacterActivity, Mut<LocomotionAnimation>)| {
        animation.previous_stride = animation.stride;
        animation.stride += (activity.distance - animation.stride) * 0.4;
        animation.phase_delta = activity.distance * 5.0;
        animation.phase =
            (animation.phase + animation.phase_delta).rem_euclid(std::f32::consts::TAU);
    };
    if actors.iter().len() < 128 {
        actors.iter_mut().for_each(update);
    } else {
        actors
            .par_iter_mut()
            .batching_strategy(bevy::ecs::batching::BatchingStrategy::new().min_batch_size(64))
            .for_each(update);
    }
}

fn sample_audio(
    mut actors: Query<
        (
            Entity,
            &Creature,
            &FixedPosition,
            &CharacterActivity,
            &CharacterState,
            &mut CreatureAudio,
        ),
        (With<Simulating>, Without<super::combat::Dying>),
    >,
    clock: Res<crate::simulation::GameplayClock>,
    catalog: Res<CreatureCatalog>,
    session: Res<hollow_worldgen::GenerationSession>,
    commands: bevy::ecs::system::ParallelCommands,
    listener: Query<&GlobalTransform, With<crate::player::PlayerCamera>>,
) {
    use crate::audio::Sound;
    let listener = listener.single().ok().map(GlobalTransform::translation);
    let seed = session.settings().seed as u64;
    let update = |(entity, creature, position, activity, state, mut audio): (
        Entity,
        &Creature,
        &FixedPosition,
        &CharacterActivity,
        &CharacterState,
        Mut<CreatureAudio>,
    )| {
        let audible = listener.is_some_and(|listener| {
            listener.distance_squared(position.current.as_vec3()) <= 32.0 * 32.0
        });
        audio.step_cooldown = audio.step_cooldown.saturating_sub(1);
        audio.ambient_cooldown = audio.ambient_cooldown.saturating_sub(1);
        let Some(sounds) = catalog.get(creature.0).map(|profile| profile.sounds) else {
            return;
        };
        if state.on_ground
            && !state.in_swimmable_fluid
            && activity.simulated
            && activity.distance <= 4.0
        {
            audio.step_distance += activity.distance;
            if audio.step_distance >= 1.0 / 0.6 && audio.step_cooldown == 0 {
                audio.step_distance = 0.0;
                audio.step_cooldown = 8;
                if audible {
                    commands.command_scope(|mut commands| {
                        commands.trigger(
                            Sound::at(sounds.step, position.current.as_vec3()).with_volume(0.15),
                        );
                    });
                }
            }
        } else {
            audio.step_distance = 0.0;
        }
        if !audio.ambient_initialized {
            audio.ambient_initialized = true;
            let mut random = CreatureRandom(
                (seed ^ entity.to_bits() ^ clock.0.wrapping_mul(0x9e3779b97f4a7c15)).max(1),
            );
            audio.ambient_cooldown = 200 + random.range(1001) as u16;
        } else if audio.ambient_cooldown == 0 {
            let mut random = CreatureRandom(
                (seed ^ entity.to_bits() ^ clock.0.wrapping_mul(0x9e3779b97f4a7c15)).max(1),
            );
            audio.ambient_cooldown = 600 + random.range(601) as u16;
            if audible {
                commands.command_scope(|mut commands| {
                    commands.trigger(Sound::at(sounds.ambient, position.current.as_vec3()));
                });
            }
        }
    };
    if actors.iter().len() < 128 {
        actors.iter_mut().for_each(update);
    } else {
        actors
            .par_iter_mut()
            .batching_strategy(bevy::ecs::batching::BatchingStrategy::new().min_batch_size(64))
            .for_each(update);
    }
}

/// Render-only facing follows safe movement intent between fixed simulation ticks.
fn orient(
    time: Res<Time>,
    mut actors: Query<(&CharacterMovement, &mut Transform), (With<Creature>, With<Simulating>)>,
) {
    let blend = 1.0 - (-8.0 * time.delta_secs()).exp();
    let update = |(movement, mut transform): (&CharacterMovement, Mut<Transform>)| {
        if movement.look_direction.length_squared() < 0.01 {
            return;
        }
        let direction = movement.look_direction;
        let target = Quat::from_rotation_y((-direction.x).atan2(-direction.z));
        let next = transform.rotation.slerp(target, blend);
        if transform.rotation != next {
            transform.rotation = next;
        }
    };
    if actors.iter().len() < 128 {
        actors.iter_mut().for_each(update);
    } else {
        actors
            .par_iter_mut()
            .batching_strategy(bevy::ecs::batching::BatchingStrategy::new().min_batch_size(64))
            .for_each(update);
    }
}

fn animate(
    assets: Res<RigAssets>,
    catalog: Res<CreatureCatalog>,
    time: Res<Time>,
    interpolation: Res<hollow_features::motion::InterpolationAlpha>,
    mut actors: Query<
        (
            &Creature,
            &CharacterState,
            &LocomotionAnimation,
            &DamageState,
            Option<&super::combat::Dying>,
            &ViewVisibility,
            &mut MeshTag,
            &mut MeshMaterial3d<QuadrupedMaterial>,
        ),
        With<Simulating>,
    >,
) {
    let swim_swing = (time.elapsed_secs() * 7.0).sin() * 0.3;
    let swim_bob = (time.elapsed_secs() * 3.0).sin() * 0.025;
    let update = |(creature, state, animation, damage, dying, visible, mut tag, mut material): (
        &Creature,
        &CharacterState,
        &LocomotionAnimation,
        &DamageState,
        Option<&super::combat::Dying>,
        &ViewVisibility,
        Mut<MeshTag>,
        Mut<MeshMaterial3d<QuadrupedMaterial>>,
    )| {
        let Some(profile) = catalog.get(creature.0) else {
            return;
        };
        if !visible.get() {
            return;
        }
        let phase = animation.phase + animation.phase_delta * (interpolation.0 - 1.0);
        let stride = animation
            .previous_stride
            .lerp(animation.stride, interpolation.0);
        let swing = if state.in_swimmable_fluid {
            swim_swing
        } else {
            phase.sin() * (stride * 6.0).min(0.8)
        };
        let bob = if state.in_swimmable_fluid {
            swim_bob
        } else {
            0.0
        };
        let angle_bits = ((swing.clamp(-1.0, 1.0) + 1.0) * 32767.5).round() as u32;
        let bob_bits = ((bob / 0.025 + 1.0) * 127.5).round() as u32;
        let roll_bits = dying.map_or(0, |dying| {
            ((1.0 - dying.0 as f32 / 10.0).clamp(0.0, 1.0) * 255.0).round() as u32
        });
        let value = angle_bits | (bob_bits << 16) | (roll_bits << 24);
        if tag.value != value {
            tag.value = value;
        }
        let handle = &assets.materials[profile.rig as usize][usize::from(damage.flash_ticks > 0)];
        if material.0 != *handle {
            material.0 = handle.clone();
        }
    };
    if actors.iter().len() < 128 {
        actors.iter_mut().for_each(update);
    } else {
        actors
            .par_iter_mut()
            .batching_strategy(bevy::ecs::batching::BatchingStrategy::new().min_batch_size(64))
            .for_each(update);
    }
}
