//! RON models compile once into shared meshes, numeric part indices and joint clips.
use crate::{
    RenderSource, catalog::CatalogRenderSource, model_mesh::ModelGeometry, models::ModelQuad,
};
use bevy::prelude::*;
use hollow_core::shape::BlockBox;
use hollow_features::articulation::{
    AnimatingJoint, ArticulationPlugin, Easing, JointAnimation, JointClip, JointMotion,
};
use serde::Deserialize;
use std::io;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelManifest {
    pub parts: Vec<PartDeclaration>,
    #[serde(default)]
    pub animations: Vec<AnimationDeclaration>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PartDeclaration {
    pub name: String,
    /// Parents precede children. Pivots are relative to the parent pivot.
    pub parent: Option<String>,
    pub pivot: [f32; 3],
    pub cuboids: Vec<ModelCuboid>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelCuboid {
    /// Geometry is local to this part's pivot, in blocks.
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub texture: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnimationDeclaration {
    pub name: String,
    pub seconds: f32,
    #[serde(default)]
    pub cubic_out: bool,
    pub tracks: Vec<TrackDeclaration>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrackDeclaration {
    pub part: String,
    pub motions: Vec<MotionDeclaration>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub enum MotionDeclaration {
    Rotation { axis: [f32; 3], radians: f32 },
    Translation { start: [f32; 3], end: [f32; 3] },
    Scale { start: [f32; 3], end: [f32; 3] },
}
struct ModelPart {
    parent: Option<u16>,
    transform: Transform,
    mesh: Handle<Mesh>,
}
#[derive(Asset, TypePath)]
pub struct VoxelModel {
    parts: Box<[ModelPart]>,
    // Names are an import/UI boundary. Playback receives only asset handles.
    animations: Box<[(String, Handle<ModelAnimation>)]>,
}
impl VoxelModel {
    pub fn animation(&self, name: &str) -> Option<Handle<ModelAnimation>> {
        self.animations
            .iter()
            .find(|(candidate, _)| candidate == name)
            .map(|(_, handle)| handle.clone())
    }
}
#[derive(Asset, TypePath)]
pub struct ModelAnimation {
    tracks: Box<[(u16, Handle<JointClip>)]>,
    model: bevy::asset::AssetId<VoxelModel>,
}
#[derive(Component)]
pub struct ModelInstance {
    pub model: Handle<VoxelModel>,
    // Direct indices eliminate hierarchy scans and name lookups on playback changes.
    parts: Box<[Entity]>,
}
#[derive(EntityEvent)]
pub struct PlayModelAnimation {
    pub entity: Entity,
    pub animation: Handle<ModelAnimation>,
    pub looping: bool,
    pub target: f32,
}
pub struct VoxelModelPlugin;
impl Plugin for VoxelModelPlugin {
    fn build(&self, app: &mut App) {
        if !app.is_plugin_added::<ArticulationPlugin>() {
            app.add_plugins(ArticulationPlugin);
        }
        app.init_asset::<VoxelModel>()
            .init_asset::<ModelAnimation>()
            .add_observer(play);
    }
}
fn play(
    event: On<PlayModelAnimation>,
    mut commands: Commands,
    instances: Query<&ModelInstance>,
    animations: Res<Assets<ModelAnimation>>,
    joints: Query<&JointAnimation>,
) {
    if !event.target.is_finite() {
        return;
    }
    let (Ok(instance), Some(animation)) = (
        instances.get(event.entity),
        animations.get(&event.animation),
    ) else {
        return;
    };
    if animation.model != instance.model.id() {
        return;
    }
    for (index, clip) in &animation.tracks {
        let entity = instance.parts[usize::from(*index)];
        commands.entity(entity).insert((
            JointAnimation {
                clip: clip.clone(),
                progress: joints
                    .get(entity)
                    .ok()
                    .filter(|joint| joint.clip == *clip)
                    .map_or(0.0, |joint| joint.progress),
                target: event.target.clamp(0.0, 1.0),
                looping: event.looping,
            },
            AnimatingJoint,
        ));
    }
}
impl ModelManifest {
    pub fn parse(source: &str, bytes: &[u8]) -> io::Result<Self> {
        ron::de::from_bytes(bytes).map_err(|error| io::Error::other(format!("{source}: {error}")))
    }
    pub fn bake(
        self,
        source_name: &str,
        source: &CatalogRenderSource,
        meshes: &mut Assets<Mesh>,
        clips: &mut Assets<JointClip>,
        animations: &mut Assets<ModelAnimation>,
        models: &mut Assets<VoxelModel>,
    ) -> io::Result<Handle<VoxelModel>> {
        let invalid = |reason: &str| io::Error::other(format!("{source_name}: {reason}"));
        if self.parts.is_empty() || self.parts.len() > u16::MAX as usize {
            return Err(invalid("Model requires 1..=65535 parts"));
        }
        let handle = models.reserve_handle();
        let mut names: Vec<String> = Vec::with_capacity(self.parts.len());
        let mut parts = Vec::with_capacity(self.parts.len());
        for part in self.parts {
            if part.name.is_empty()
                || names.contains(&part.name)
                || !Vec3::from_array(part.pivot).is_finite()
            {
                return Err(invalid("Invalid part name or pivot"));
            }
            let parent = part
                .parent
                .map(|parent| {
                    names
                        .iter()
                        .position(|name| *name == parent)
                        .map(|index| index as u16)
                        .ok_or_else(|| invalid("Parent must precede its child"))
                })
                .transpose()?;
            let mut geometry = ModelGeometry::default();
            for cuboid in part.cuboids {
                let bounds = BlockBox {
                    min: Vec3::from_array(cuboid.min),
                    max: Vec3::from_array(cuboid.max),
                };
                if !bounds.min.is_finite()
                    || !bounds.max.is_finite()
                    || !bounds.min.cmplt(bounds.max).all()
                    || !bounds.min.cmpge(Vec3::splat(-2.0)).all()
                    || !bounds.max.cmplt(Vec3::splat(30.0)).all()
                {
                    return Err(invalid("Cuboid exceeds packed model bounds [-2, 30)"));
                }
                let texture = source
                    .content()
                    .resolve_resource(&cuboid.texture)
                    .and_then(|resource| source.texture_slot(resource))
                    .ok_or_else(|| invalid("Unregistered model texture"))?;
                for face in bounds.faces() {
                    geometry.quad(
                        &ModelQuad {
                            positions: face.positions.map(|position| position.to_array()),
                            normal: face.normal.to_array(),
                            uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                            texture,
                            tinted: false,
                            cull: [0; 3],
                            translucent: false,
                            effect: 0,
                            light_emission: 0,
                            shade_up: false,
                        },
                        Vec3::ZERO,
                        [60; 4],
                        0,
                        None,
                    );
                }
            }
            names.push(part.name);
            parts.push(ModelPart {
                parent,
                transform: Transform::from_translation(Vec3::from_array(part.pivot)),
                mesh: meshes.add(geometry.into_mesh()),
            });
        }
        let mut compiled = Vec::with_capacity(self.animations.len());
        for animation in self.animations {
            if animation.name.is_empty()
                || compiled.iter().any(|(name, _)| *name == animation.name)
                || !animation.seconds.is_finite()
                || animation.seconds <= 0.0
            {
                return Err(invalid("Invalid animation name or duration"));
            }
            let mut tracks = Vec::with_capacity(animation.tracks.len());
            for track in animation.tracks {
                let index = names
                    .iter()
                    .position(|name| *name == track.part)
                    .ok_or_else(|| invalid("Unknown animated part"))?
                    as u16;
                if tracks.iter().any(|(part, _)| *part == index) {
                    return Err(invalid("Duplicate animated part"));
                }
                let mut channels = 0u8;
                let mut motions = Vec::with_capacity(track.motions.len());
                for motion in track.motions {
                    let (channel, motion) = match motion {
                        MotionDeclaration::Rotation { axis, radians } => {
                            if !radians.is_finite() {
                                return Err(invalid("Invalid rotation"));
                            }
                            (
                                1,
                                JointMotion::Rotation {
                                    axis: Dir3::new(Vec3::from_array(axis))
                                        .map_err(|_| invalid("Invalid rotation axis"))?,
                                    radians,
                                },
                            )
                        }
                        MotionDeclaration::Translation { start, end } => (
                            2,
                            JointMotion::Translation {
                                start: Vec3::from_array(start),
                                end: Vec3::from_array(end),
                            },
                        ),
                        MotionDeclaration::Scale { start, end } => (
                            4,
                            JointMotion::Scale {
                                start: Vec3::from_array(start),
                                end: Vec3::from_array(end),
                            },
                        ),
                    };
                    if channels & channel != 0 {
                        return Err(invalid("Duplicate motion channel"));
                    }
                    if let JointMotion::Translation { start, end }
                    | JointMotion::Scale { start, end } = motion
                    {
                        if !start.is_finite() || !end.is_finite() {
                            return Err(invalid("Invalid animation vector"));
                        }
                    }
                    channels |= channel;
                    motions.push(motion);
                }
                tracks.push((
                    index,
                    clips.add(JointClip {
                        motions: motions.into(),
                        duration_seconds: animation.seconds,
                        easing: if animation.cubic_out {
                            Easing::CubicOut
                        } else {
                            Easing::Linear
                        },
                    }),
                ));
            }
            compiled.push((
                animation.name,
                animations.add(ModelAnimation {
                    tracks: tracks.into(),
                    model: handle.id(),
                }),
            ));
        }
        models
            .insert(
                handle.id(),
                VoxelModel {
                    parts: parts.into(),
                    animations: compiled.into(),
                },
            )
            .map_err(|error| invalid(&error.to_string()))?;
        Ok(handle)
    }
}
/// Parts share baked mesh/material assets. The root owns the complete hierarchy.
pub fn spawn_model(
    world: &mut World,
    root: Entity,
    model: Handle<VoxelModel>,
    material: Handle<crate::material::ModelMaterial>,
) -> io::Result<()> {
    let Some(definition) = world.resource::<Assets<VoxelModel>>().get(&model) else {
        return Err(io::Error::other("Model asset is not loaded"));
    };
    let definitions: Vec<_> = definition
        .parts
        .iter()
        .map(|part| (part.parent, part.transform, part.mesh.clone()))
        .collect();
    if let Some(previous) = world.entity_mut(root).take::<ModelInstance>() {
        for part in previous.parts {
            if world
                .get::<ChildOf>(part)
                .is_some_and(|parent| parent.parent() == root)
            {
                world.despawn(part);
            }
        }
    }
    let mut parts: Vec<Entity> = Vec::with_capacity(definitions.len());
    for (parent, transform, mesh) in definitions {
        let parent = parent.map_or(root, |parent| parts[usize::from(parent)]);
        parts.push(
            world
                .spawn((
                    Mesh3d(mesh),
                    MeshMaterial3d(material.clone()),
                    transform,
                    ChildOf(parent),
                ))
                .id(),
        );
    }
    world.entity_mut(root).insert(ModelInstance {
        model,
        parts: parts.into(),
    });
    Ok(())
}
