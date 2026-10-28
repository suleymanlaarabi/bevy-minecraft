//! Baked vanilla models. All JSON, property matching and rotations happen once;
//! mesh workers only index shared, immutable quad lists.
use super::texture_pack::{TexturePackId, manifest_keys};
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use hollow_render::models::{CellInfo, ModelQuad, cell};
use pumpkin_data::{BlockId, Property};
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

#[derive(Deserialize)]
struct Imported {
    chest_pivots: [[f32; 3]; 3],
    textures: Vec<String>,
    /// Textures without transparent texels.
    opaque_textures: Vec<bool>,
    models: HashMap<String, Vec<ModelQuad>>,
    blocks: HashMap<String, Value>,
    texture_layers: Vec<u32>,
    special_layers: u32,
    animations: Vec<Animation>,
}
#[derive(Deserialize)]
struct Animation {
    layer: u32,
    frames: Vec<[u32; 2]>,
    interpolate: bool,
}
#[derive(Clone)]
struct Weighted {
    quads: Arc<[ModelQuad]>,
    weight: u32,
}
struct Group {
    choices: Vec<Weighted>,
    total: u64,
}
#[derive(Default)]
struct Recipe {
    groups: Vec<Group>,
    /// Common deterministic recipes bypass hashing and weighted selection.
    fixed: Option<Arc<[ModelQuad]>>,
    /// A resolved empty model is deliberately invisible, never a fallback cube.
    resolved: bool,
}
pub(crate) struct ChestParts {
    pub body: Arc<[ModelQuad]>,
    pub lid: Arc<[ModelQuad]>,
    pub pivot: Vec3,
}
struct Library {
    chests: Option<[ChestParts; 3]>,
    recipes: Box<[Recipe]>,
    textures: Vec<String>,
    opaque_textures: Vec<bool>,
    /// Mesher classification of every state, read once per cell.
    cells: Box<[CellInfo]>,
    /// Face slots of greedy cubes, indexed by [`CellInfo::appearance`].
    appearances: Vec<Appearance>,
    texture_layers: Vec<u32>,
    external: Vec<(hollow_core::ResourceId, u32)>,
    block_slots: Box<[[u32; 6]]>,
    special_layers: u32,
    animations: Vec<Animation>,
    animated_layers: Box<[bool; 2048]>,
}

/// One face of a greedy cube: a model face-table slot, whether the block tint
/// applies, and the dihedral transform from block UVs to the model's UVs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub(crate) struct CubeFace {
    pub(crate) slot: u32,
    pub(crate) tinted: bool,
    pub(crate) transform: u8,
}

/// Six faces indexed by `axis * 2 + positive`; `tint` is the block whose tint
/// tinted faces take (Air when none is).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Appearance {
    pub(crate) faces: [CubeFace; 6],
    pub(crate) tint: VoxelKind,
}

/// `transform` bit 0 swaps u and v, bits 1 and 2 negate them. Integer-valued
/// maps keep tiled UVs continuous, so merged quads keep their derivatives.
pub(crate) fn transform_uv(uv: Vec2, transform: u8) -> Vec2 {
    let swapped = if transform & 1 != 0 { uv.yx() } else { uv };
    Vec2::new(
        if transform & 2 != 0 {
            -swapped.x
        } else {
            swapped.x
        },
        if transform & 4 != 0 {
            -swapped.y
        } else {
            swapped.y
        },
    )
}

/// Block UVs the terrain shader derives from a position (`voxel_vertex.wesl`).
fn block_uv(position: Vec3, axis: usize) -> Vec2 {
    match axis {
        0 => Vec2::new(position.z, -position.y),
        1 => Vec2::new(position.z, position.x),
        _ => Vec2::new(position.x, -position.y),
    }
}

/// Six full, culled, axis-aligned faces whose UVs are a whole-texture dihedral
/// map of block UVs: exactly what greedy terrain draws.
fn cube_faces(quads: &[ModelQuad]) -> Option<[CubeFace; 6]> {
    if quads.len() != 6 {
        return None;
    }
    let mut faces = [None; 6];
    for q in quads {
        if q.translucent || q.shade_up || q.effect != 0 || q.light_emission != 0 {
            return None;
        }
        // Rotated variants carry float noise (0.99999994): compare with a tolerance.
        let near = |a: f32, b: f32| (a - b).abs() < 1e-4;
        let normal = IVec3::from_array(q.normal.map(|v| v.round() as i32));
        if normal.abs().element_sum() != 1
            || !(0..3).all(|i| near(q.normal[i], normal[i] as f32))
            || IVec3::from_array(q.cull) != normal
        {
            return None;
        }
        let axis = normal.abs().max_position();
        let positive = normal[axis] > 0;
        let plane = if positive { 1.0 } else { 0.0 };
        let positions = q.positions.map(|p| Vec3::from_array(p).round());
        if q.positions.iter().zip(&positions).any(|(raw, snapped)| {
            !(0..3).all(|i| near(raw[i], snapped[i]))
                || snapped[axis] != plane
                || snapped.cmplt(Vec3::ZERO).any()
                || snapped.cmpgt(Vec3::ONE).any()
        }) {
            return None;
        }
        let uv = q.uv.map(Vec2::from_array);
        if !near(uv[0].x, uv[0].x.round()) || !near(uv[0].y, uv[0].y.round()) {
            return None;
        }
        let canonical = positions.map(|p| block_uv(p, axis));
        let transform = (0..8u8).find(|&t| {
            (1..4).all(|i| {
                (transform_uv(canonical[i] - canonical[0], t) - (uv[i] - uv[0]))
                    .abs()
                    .max_element()
                    < 1e-4
            })
        })?;
        let direction = axis * 2 + usize::from(positive);
        if faces[direction].is_some() {
            return None;
        }
        faces[direction] = Some(CubeFace {
            slot: q.texture,
            tinted: q.tinted,
            transform,
        });
    }
    Some(faces.map(Option::unwrap))
}

/// Legacy cubes keep the pack's own per-block top/side/bottom configuration.
fn pack_faces(kind: VoxelKind, textures: usize) -> [CubeFace; 6] {
    std::array::from_fn(|direction| CubeFace {
        slot: fallback_slot_after(textures, kind, direction / 2, direction % 2 == 1),
        tinted: false,
        transform: 0,
    })
}

#[derive(Default)]
struct Appearances {
    list: Vec<Appearance>,
    ids: HashMap<Appearance, u16>,
}
impl Appearances {
    fn id(&mut self, faces: [CubeFace; 6], kind: VoxelKind) -> u16 {
        let appearance = Appearance {
            faces,
            tint: if faces.iter().any(|face| face.tinted) {
                kind
            } else {
                VoxelKind::Air
            },
        };
        if self.list.is_empty() {
            // Id 0 is never drawn: it marks non-cube cells.
            self.list.push(appearance);
        }
        *self.ids.entry(appearance).or_insert_with(|| {
            self.list.push(appearance);
            u16::try_from(self.list.len() - 1)
                .ok()
                .filter(|&id| id < 1 << 14)
                .expect("cube appearances must fit in 14 vertex bits")
        })
    }
}

pub(crate) fn translucent_kind(kind: VoxelKind) -> bool {
    hollow_worldgen::content::catalog()
        .visual(hollow_core::BlockId(kind.0))
        .is_some_and(|v| v.translucent)
        || kind
            .native_id()
            .is_some_and(|id| matches!(id, BlockId::ICE | BlockId::FROSTED_ICE))
        || kind.tagged(pumpkin_data::tag::Block::C_GLASS_BLOCKS)
        || kind.tagged(pumpkin_data::tag::Block::C_GLASS_PANES)
}

fn base_flags(state: VoxelState) -> u8 {
    let mut flags = 0;
    if state.definition().is_air() {
        flags |= cell::AIR;
    }
    let fluid = state.definition().fluid.id;
    if fluid != hollow_core::FluidId::NONE
        && fluid.0 < (1 << 14)
        && crate::content::fluid_catalog()
            .profile(fluid)
            .render
            .is_some_and(|render| render.ocean_surface && render.translucent)
    {
        flags |= cell::OCEAN_SURFACE;
    }
    if state.is_opaque_cube() {
        flags |= cell::OCCLUDES;
    }
    if state.definition().is_full_cube() {
        // Full cubes enclose adjacent water regardless of texture transparency.
        // In particular, dry leaves and roots cover the water below them: drawing
        // a lowered free surface there creates a visible blue ledge underneath.
        // Keep this separate from visual occlusion so cutouts still show holes
        // and transmit light. Waterlogged neighbors also share the fluid family.
        flags |= cell::FLUID_OCCLUDES;
    }
    if state.definition().can_occlude()
        && !state.definition().is_full_cube()
        && !state.selection_empty()
    {
        flags |= cell::FLUID_OCCLUDER;
    }
    flags
}

fn state_info(state: VoxelState) -> CellInfo {
    let fluid = state.definition().fluid;
    let visible = fluid.id != hollow_core::FluidId::NONE
        && crate::content::fluid_catalog()
            .profile(fluid.id)
            .render
            .is_some();
    CellInfo {
        appearance: 0,
        flags: base_flags(state),
        fluid: if visible {
            u32::from(fluid.id.0) << 8 | u32::from(fluid.amount) | (u32::from(fluid.falling) << 24)
        } else {
            0
        },
    }
}

/// Cells without a greedy cube and with something to draw use explicit models.
fn finish(mut info: CellInfo, state: VoxelState) -> CellInfo {
    if info.flags & (cell::AIR | cell::CUBE | cell::CUTOUT) == 0
        && (state.definition().fluid.id == hollow_core::FluidId::NONE
            || state.definition().fluid.embedded)
    {
        info.flags |= cell::MODEL;
    }
    info
}

fn canonical_uv(p: Vec3, n: Vec3) -> Vec2 {
    let axis = n.abs().max_element();
    if n.y.abs() == axis {
        if n.y > 0.0 {
            Vec2::new(p.x, p.z)
        } else {
            Vec2::new(p.x, 1.0 - p.z)
        }
    } else if n.x.abs() == axis {
        if n.x > 0.0 {
            Vec2::new(1.0 - p.z, 1.0 - p.y)
        } else {
            Vec2::new(p.z, 1.0 - p.y)
        }
    } else if n.z > 0.0 {
        Vec2::new(p.x, 1.0 - p.y)
    } else {
        Vec2::new(1.0 - p.x, 1.0 - p.y)
    }
}
fn rotated(source: &[ModelQuad], x: f32, y: f32, uvlock: bool) -> Arc<[ModelQuad]> {
    let rotation = Quat::from_rotation_y(-y.to_radians()) * Quat::from_rotation_x(-x.to_radians());
    source
        .iter()
        .map(|q| {
            let mut out = q.clone();
            let normal = rotation * Vec3::from_array(q.normal);
            out.normal = normal.to_array();
            out.cull = (rotation * IVec3::from_array(q.cull).as_vec3())
                .round()
                .as_ivec3()
                .to_array();
            out.positions = q.positions.map(|p| {
                (rotation * (Vec3::from_array(p) - Vec3::splat(0.5)) + Vec3::splat(0.5)).to_array()
            });
            if uvlock {
                let old = q
                    .positions
                    .map(|p| canonical_uv(Vec3::from_array(p), Vec3::from_array(q.normal)));
                let du = old[1] - old[0];
                let dv = old[3] - old[0];
                let determinant = du.x * dv.y - dv.x * du.y;
                if determinant.abs() > 1e-6 {
                    out.uv = out.positions.map(|p| {
                        let p = canonical_uv(Vec3::from_array(p), normal) - old[0];
                        let u = (p.x * dv.y - p.y * dv.x) / determinant;
                        let v = (du.x * p.y - du.y * p.x) / determinant;
                        (Vec2::from_array(q.uv[0])
                            + (Vec2::from_array(q.uv[1]) - Vec2::from_array(q.uv[0])) * u
                            + (Vec2::from_array(q.uv[3]) - Vec2::from_array(q.uv[0])) * v)
                            .to_array()
                    });
                }
            }
            out
        })
        .collect()
}
/// Text conditions are compiled once at the model import boundary.
enum Condition {
    All(Vec<Self>),
    Any(Vec<Self>),
    Property(Vec<Property>),
}
impl Condition {
    fn property(key: &str, choices: &str) -> Self {
        Self::Property(
            choices
                .split('|')
                .map(|value| {
                    Property::from_str_pair(key, value).expect("Invalid imported model property")
                })
                .collect(),
        )
    }
    fn multipart(value: &Value) -> Self {
        let object = value.as_object().expect("Invalid multipart condition");
        Self::All(
            object
                .iter()
                .map(|(key, value)| match key.as_str() {
                    "OR" => Self::Any(
                        value
                            .as_array()
                            .expect("Invalid OR condition")
                            .iter()
                            .map(Self::multipart)
                            .collect(),
                    ),
                    "AND" => Self::All(
                        value
                            .as_array()
                            .expect("Invalid AND condition")
                            .iter()
                            .map(Self::multipart)
                            .collect(),
                    ),
                    key => Self::property(key, value.as_str().expect("Invalid property condition")),
                })
                .collect(),
        )
    }
    fn variant(key: &str) -> Self {
        Self::All(
            key.split(',')
                .filter(|key| !key.is_empty())
                .map(|condition| {
                    let (key, value) = condition.split_once('=').expect("Invalid model variant");
                    Self::property(key, value)
                })
                .collect(),
        )
    }
    fn matches(&self, state: VoxelState) -> bool {
        match self {
            Self::All(conditions) => conditions.iter().all(|condition| condition.matches(state)),
            Self::Any(conditions) => conditions.iter().any(|condition| condition.matches(state)),
            Self::Property(choices) => choices.iter().any(|&choice| {
                state
                    .property(choice.key())
                    .is_some_and(|actual| actual.matches_property(choice))
            }),
        }
    }
}
struct ModelApplication {
    model: Option<usize>,
    x: i32,
    y: i32,
    uvlock: bool,
    weight: u32,
}
struct ModelPart {
    condition: Condition,
    applications: Vec<ModelApplication>,
}
struct BlockModel {
    variants: Vec<ModelPart>,
    multipart: Vec<ModelPart>,
}
impl BlockModel {
    fn compile(value: Value, models: &HashMap<String, usize>) -> Self {
        let applications = |value: &Value| {
            value
                .as_array()
                .map(Vec::as_slice)
                .unwrap_or_else(|| std::slice::from_ref(value))
                .iter()
                .map(|value| ModelApplication {
                    model: models
                        .get(
                            value
                                .get("model")
                                .and_then(Value::as_str)
                                .expect("Missing model name"),
                        )
                        .copied(),
                    x: value.get("x").and_then(Value::as_i64).unwrap_or(0) as i32,
                    y: value.get("y").and_then(Value::as_i64).unwrap_or(0) as i32,
                    uvlock: value
                        .get("uvlock")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    weight: value
                        .get("weight")
                        .and_then(Value::as_u64)
                        .unwrap_or(1)
                        .clamp(1, u32::MAX as u64) as u32,
                })
                .collect()
        };
        let variants = value
            .get("variants")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(key, value)| ModelPart {
                condition: Condition::variant(key),
                applications: applications(value),
            })
            .collect();
        let multipart = value
            .get("multipart")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .map(|part| ModelPart {
                condition: part
                    .get("when")
                    .map(Condition::multipart)
                    .unwrap_or(Condition::All(Vec::new())),
                applications: applications(
                    part.get("apply").expect("Missing multipart application"),
                ),
            })
            .collect();
        Self {
            variants,
            multipart,
        }
    }
}
fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let imported = match std::fs::read(crate::asset_directory().join("voxel/packs/vanilla/models.json")) {
            Ok(bytes) => {
                let data: Imported = serde_json::from_slice(&bytes).expect("Invalid vanilla model cache; regenerate it with scripts/import_vanilla.py");
                Some(data)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => panic!("Cannot read vanilla model cache: {error}"),
        };
        let mut recipes = (0..VoxelState::count() as u32).map(|_| Recipe::default()).collect::<Vec<_>>();
        let mut cells = vec![CellInfo::default(); VoxelState::count()];
        let mut appearances = Appearances::default();
        let pack_keys = manifest_keys(TexturePackId::Vanilla);
        let Some(imported) = imported else {
            warn!("Vanilla models unavailable: using shape-based models; run scripts/import_vanilla.py");
            for raw in 0..VoxelState::count() as u32 {
                let Some(state) = VoxelState::from_id(raw) else { continue; };
                let mut info = state_info(state);
                if state.is_opaque_cube() && pack_cube(&pack_keys, state.kind()) {
                    info.appearance = appearances.id(pack_faces(state.kind(), 0), state.kind());
                    info.flags |= cell::CUBE;
                }
                cells[raw as usize] = finish(info, state);
            }
            return augment_content(Library { chests: None, recipes: recipes.into(), cells: cells.into(), appearances: appearances.list, textures: Vec::new(), opaque_textures: Vec::new(), texture_layers: Vec::new(), special_layers: 0, animations: Vec::new(), animated_layers: Box::new([false; 2048]), external: Vec::new(), block_slots: Box::new([]) });
        };
        let model_ids: HashMap<_, _> = imported.models.keys().enumerate().map(|(index, name)| (name.clone(), index)).collect();
        let models: Vec<_> = {
            let mut models = vec![Vec::new(); model_ids.len()];
            for (name, quads) in imported.models { models[model_ids[&name]] = quads; }
            models
        };
        let mut blocks: Vec<Option<BlockModel>> = (0..VoxelKind::count()).map(|_| None).collect();
        for (name, value) in imported.blocks {
            // Vanilla also ships blockstate files for these two entities.
            if matches!(name.as_str(), "item_frame" | "glow_item_frame") { continue; }
            let kind = VoxelKind::from_key(&name).unwrap_or_else(|| panic!("Unknown imported model block: {name}"));
            let slot = &mut blocks[kind.index()];
            assert!(slot.is_none(), "Duplicate imported block identity");
            *slot = Some(BlockModel::compile(value, &model_ids));
        }
        let chest_models = ["single", "left", "right"].map(|half| {
            ["body", "lid"].map(|part| model_ids[&format!("hollow:chest/{half}/{part}")])
        });
        drop(model_ids);
        let mut rotations = HashMap::<(usize, i32, i32, bool), Arc<[ModelQuad]>>::new();
        let mut assemblies = HashMap::<Vec<usize>, Arc<[ModelQuad]>>::new();
        let mut missing = std::collections::HashSet::new();
        for raw in 0..VoxelState::count() as u32 {
            let Some(state) = VoxelState::from_id(raw) else { continue; };
            let kind = state.kind();
            let Some(block) = blocks[kind.index()].as_ref() else {
                missing.insert(kind);
                cells[raw as usize] = finish(state_info(state), state);
                continue;
            };
            let applications = block.variants.iter().find(|part| part.condition.matches(state))
                .into_iter().chain(block.multipart.iter().filter(|part| part.condition.matches(state)));
            let recipe = &mut recipes[raw as usize];
            let mut selected_model = false;
            let mut missing_model = false;
            for part in applications {
                let mut group = Vec::new();
                for application in &part.applications {
                    let Some(model) = application.model else { missing.insert(kind); missing_model = true; continue; };
                    selected_model = true;
                    let quads = rotations.entry((model, application.x, application.y, application.uvlock)).or_insert_with(|| rotated(&models[model], application.x as f32, application.y as f32, application.uvlock)).clone();
                    if !quads.is_empty() {
                        group.push(Weighted { quads, weight: application.weight });
                    }
                }
                if !group.is_empty() {
                    recipe.groups.push(Group { total: group.iter().map(|m| u64::from(m.weight)).sum(), choices: group });
                }
            }
            recipe.resolved = selected_model && !missing_model;
            if !recipe.resolved {
                missing.insert(kind);
                recipe.groups.clear();
            }
            // Default pack cubes keep the pack's per-block faces; any other state
            // whose single model is a plain cube shares a deduplicated appearance.
            let mut info = state_info(state);
            let model_faces = match recipe.groups.as_slice() {
                [group] => {
                    let mut faces = group.choices.iter().map(|model| cube_faces(&model.quads));
                    faces.next().flatten().filter(|first| faces.all(|other| other == Some(*first)))
                }
                _ => None,
            };
            if state == kind.default_state() && kind.is_opaque() && pack_cube(&pack_keys, kind)
                && !recipe.groups.is_empty()
            {
                info.appearance = appearances.id(pack_faces(kind, imported.textures.len()), kind);
                info.flags |= cell::CUBE;
            } else if let Some(faces) = model_faces {
                let class = if state.is_opaque_cube() {
                    cell::CUBE
                } else if state.definition().is_full_cube() {
                    cell::CUTOUT
                } else {
                    0
                };
                if class != 0 {
                    info.appearance = appearances.id(faces, kind);
                    info.flags |= class;
                }
            }
            cells[raw as usize] = if recipe.resolved && recipe.groups.is_empty() {
                // Invisible technical blocks still retain collision and native
                // lighting behavior, but have no model-meshing work.
                info
            } else {
                finish(info, state)
            };
            if !recipe.groups.is_empty() && recipe.groups.iter().all(|group| group.choices.len() == 1) {
                let key = recipe.groups.iter().map(|group| Arc::as_ptr(&group.choices[0].quads) as *const ModelQuad as usize).collect::<Vec<_>>();
                recipe.fixed = Some(assemblies.entry(key).or_insert_with(|| {
                    if recipe.groups.len() == 1 { recipe.groups[0].choices[0].quads.clone() }
                    else { recipe.groups.iter().flat_map(|group| group.choices[0].quads.iter().cloned()).collect() }
                }).clone());
                recipe.groups.clear();
                recipe.groups.shrink_to_fit();
            }
        }
        if !missing.is_empty() { warn!("{} vanilla model families unavailable", missing.len()); }
        let mut animated_layers = Box::new([false; 2048]);
        for animation in &imported.animations { animated_layers[animation.layer as usize] = true; }
        let chests = ["single", "left", "right"].map(|half| {
            let i = match half { "left" => 1, "right" => 2, _ => 0 };
            ChestParts {
                body: models[chest_models[i][0]].clone().into(),
                lid: models[chest_models[i][1]].clone().into(),
                pivot: Vec3::from_array(imported.chest_pivots[i]),
            }
        });
        augment_content(Library { chests: Some(chests), recipes: recipes.into(), textures: imported.textures, opaque_textures: imported.opaque_textures, cells: cells.into(), appearances: appearances.list, texture_layers: imported.texture_layers, special_layers: imported.special_layers, animations: imported.animations, animated_layers, external: Vec::new(), block_slots: Box::new([]) })
    })
}
/// Resolve custom asset names once, before any mesh task is scheduled.
fn augment_content(mut library: Library) -> Library {
    let content = hollow_worldgen::content::catalog();
    let original_texture_count = library.textures.len();
    let mut referenced = vec![false; content.resource_count()];
    for (_, profile) in crate::content::fluid_catalog().profiles() {
        if let Some(render) = profile.render {
            referenced[render.still.index()] = true;
            referenced[render.flowing.index()] = true;
        }
    }
    for kind in VoxelKind::all() {
        if let Some(visual) = content.visual(hollow_core::BlockId(kind.0)) {
            for resource in visual.faces {
                referenced[resource.index()] = true;
            }
        }
    }
    let mut slots = Vec::with_capacity(content.resource_count());
    for index in 0..content.resource_count() {
        if !referenced[index] {
            slots.push(None);
            continue;
        }
        let id = hollow_core::ResourceId(index as u32);
        let path = content.resource(id);
        let slot = if path.ends_with(".png") {
            let slot = library.textures.len() as u32;
            library.textures.push(path.to_owned());
            library.external.push((id, slot));
            Some(slot)
        } else {
            let name = path.strip_prefix("minecraft:block/").unwrap_or(path);
            library
                .textures
                .iter()
                .position(|name_in_pack| name_in_pack == name)
                .map(|i| i as u32)
        };
        slots.push(slot);
    }
    let added = library.textures.len() - original_texture_count;
    for appearance in &mut library.appearances {
        for face in &mut appearance.faces {
            if face.slot as usize >= original_texture_count {
                face.slot += added as u32;
            }
        }
    }
    library.block_slots = VoxelKind::all()
        .filter(|kind| !kind.is_native())
        .map(|kind| {
            let visual = content.visual(hollow_core::BlockId(kind.0));
            std::array::from_fn(|face| {
                visual
                    .and_then(|v| slots[v.faces[face].index()])
                    .unwrap_or_else(|| {
                        // A missing visual is explicit; resource lists have no implied material layout.
                        library.textures.len() as u32
                            + VoxelKind::Stone.index() as u32 * 3
                            + if face / 2 == 1 {
                                if face % 2 == 1 { 0 } else { 2 }
                            } else {
                                1
                            }
                    })
            })
        })
        .collect();
    for kind in VoxelKind::all().filter(|kind| !kind.is_native()) {
        let resources = library.block_slots[kind.index() - VoxelKind::ALL.len()];
        let faces = resources.map(|slot| CubeFace { slot, ..default() });
        let appearance = u16::try_from(library.appearances.len())
            .ok()
            .filter(|id| *id > 0 && *id < 1 << 14);
        if let Some(appearance) = appearance {
            let mut used = false;
            for raw in content.block(hollow_core::BlockId(kind.0)).states.clone() {
                let state = content.state(hollow_core::StateId(raw));
                if state.full_cube
                    && state.opacity == 15
                    && state.fluid.id == hollow_core::FluidId::NONE
                    && content
                        .visual(hollow_core::BlockId(kind.0))
                        .is_some_and(|v| !v.translucent)
                {
                    library.cells[raw as usize].appearance = appearance;
                    library.cells[raw as usize].flags =
                        (library.cells[raw as usize].flags & !cell::MODEL) | cell::CUBE;
                    used = true;
                }
            }
            if used {
                library.appearances.push(Appearance {
                    faces,
                    tint: VoxelKind::Air,
                });
            }
        }
    }
    library
}
pub(crate) fn external_textures() -> &'static [(hollow_core::ResourceId, u32)] {
    &library().external
}
fn pack_cube(keys: &[bool], kind: VoxelKind) -> bool {
    // These keys already have the exact top/side/bottom mapping in all packs.
    // Logs with an explicit axis use the model path, as do slabs and farmland.
    kind.is_native()
        && !matches!(
            kind.native_id().unwrap(),
            BlockId::DIRT_PATH | BlockId::FARMLAND | BlockId::SNOW
        )
        && !kind.tagged(pumpkin_data::tag::Block::MINECRAFT_STAIRS)
        && !kind.tagged(pumpkin_data::tag::Block::MINECRAFT_SLABS)
        && keys.get(kind.index()).copied().unwrap_or(false)
}
/// Mesher classification of a state.
#[inline]
pub(crate) fn cells() -> &'static [CellInfo] {
    &library().cells
}
/// Faces of every greedy cube appearance, indexed by [`CellInfo::appearance`].
pub(crate) fn appearances() -> &'static [Appearance] {
    &library().appearances
}

/// Whether a model quad's texture is fully opaque, so it needs no alpha test.
/// Unknown slots (shape-based texture faces) stay alpha-tested.
#[inline]
pub(crate) fn opaque_texture(slot: u32) -> bool {
    library()
        .opaque_textures
        .get(slot as usize)
        .copied()
        .unwrap_or(false)
}
pub(crate) fn texture_names() -> &'static [String] {
    &library().textures
}
pub(crate) fn texture_layer(slot: usize) -> u32 {
    animated_layer(library().texture_layers[slot])
}
/// Bit 11 marks a flipbook. Static vertices bypass the GPU timeline buffer.
pub(crate) fn animated_layer(layer: u32) -> u32 {
    layer
        | if library()
            .animated_layers
            .get(layer as usize)
            .copied()
            .unwrap_or(false)
        {
            2048
        } else {
            0
        }
}
pub(crate) fn special_layers() -> u32 {
    library().special_layers
}

pub(crate) fn fluid_texture(family: hollow_core::FluidId, flowing: bool) -> u32 {
    static SLOTS: OnceLock<Box<[[u32; 2]]>> = OnceLock::new();
    let slots = SLOTS.get_or_init(|| {
        let content = hollow_worldgen::content::catalog();
        crate::content::fluid_catalog()
            .profiles()
            .map(|(_, profile)| {
                let Some(render) = profile.render else {
                    return [0; 2];
                };
                [render.still, render.flowing].map(|resource| {
                    let name = content
                        .resource(resource)
                        .strip_prefix("minecraft:block/")
                        .unwrap_or(content.resource(resource));
                    texture_names()
                        .iter()
                        .position(|texture| texture == name)
                        .map_or_else(
                            || {
                                fallback_slot(
                                    VoxelKind(
                                        content
                                            .state(profile.standing[profile.max_amount() as usize])
                                            .block
                                            .0,
                                    ),
                                    1,
                                    true,
                                )
                            },
                            |index| index as u32,
                        )
                })
            })
            .collect()
    });
    slots[family.index() - 1][usize::from(flowing)]
}

/// Fixed descriptor prefix, followed by cumulative animation timeline entries.
/// Layers are physical array indices; static surfaces bypass this buffer entirely.
pub(crate) fn animation_table() -> Vec<[u32; 4]> {
    let mut table = vec![[0; 4]; 2048];
    for animation in &library().animations {
        let offset = table.len() as u32;
        let mut end = 0;
        for &[layer, ticks] in &animation.frames {
            end += ticks;
            table.push([layer, end, 0, 0]);
        }
        table[animation.layer as usize] = [
            offset,
            animation.frames.len() as u32,
            end,
            u32::from(animation.interpolate),
        ];
    }
    table
}
pub(crate) fn visit_quads(state: VoxelState, world: IVec3, mut visit: impl FnMut(&ModelQuad)) {
    let recipe = &library().recipes[state.id() as usize];
    if let Some(quads) = &recipe.fixed {
        for q in quads.iter() {
            visit(q);
        }
        return;
    }
    if recipe.groups.is_empty() {
        if !recipe.resolved {
            fallback(state, world, visit);
        }
        return;
    }
    let mut seed = (world.x as u32).wrapping_mul(0x8da6b343)
        ^ (world.z as u32).wrapping_mul(0xcb1ab31f)
        ^ (world.y as u32).wrapping_mul(0xd8163841);
    for group in &recipe.groups {
        seed ^= seed >> 16;
        seed = seed.wrapping_mul(0x7feb352d);
        let mut choice = u64::from(seed) % group.total;
        let model = group
            .choices
            .iter()
            .find(|m| {
                if choice < u64::from(m.weight) {
                    true
                } else {
                    choice -= u64::from(m.weight);
                    false
                }
            })
            .unwrap();
        for q in model.quads.iter() {
            visit(q);
        }
    }
}
/// Fallback texture slots follow the cube face table, after all imported textures.
pub(crate) fn fallback_slot(kind: VoxelKind, axis: usize, positive: bool) -> u32 {
    fallback_slot_after(texture_names().len(), kind, axis, positive)
}
fn fallback_slot_after(textures: usize, kind: VoxelKind, axis: usize, positive: bool) -> u32 {
    if !kind.is_native() {
        return library().block_slots[kind.index() - VoxelKind::ALL.len()]
            [axis * 2 + usize::from(positive)];
    }
    textures as u32
        + kind.index() as u32 * 3
        + if axis == 1 {
            if positive { 0 } else { 2 }
        } else {
            1
        }
}
fn fallback(state: VoxelState, world: IVec3, mut visit: impl FnMut(&ModelQuad)) {
    if state.is_air() || (state.is_liquid() && !state.definition().fluid.embedded) {
        return;
    }
    let kind = state.kind();
    let translucent = translucent_kind(kind);
    let mut any = false;
    for b in state.selection_boxes(world) {
        any = true;
        let full_cube = b.min == Vec3::ZERO && b.max == Vec3::ONE;
        for face in b.faces() {
            visit(&ModelQuad {
                positions: face.positions.map(|position| position.to_array()),
                normal: face.normal.to_array(),
                uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                texture: fallback_slot(kind, face.axis, face.positive),
                tinted: false,
                // Full fallback cubes follow the imported models' neighbor culling.
                cull: if full_cube {
                    face.normal.as_ivec3().to_array()
                } else {
                    [0; 3]
                },
                translucent,
                shade_up: false,
                effect: 0,
                light_emission: 0,
            });
        }
    }

    if !any && !kind.is_liquid() {
        // Unsupported entity-rendered blocks remain visible rather than disappearing.
        let stone = VoxelKind::Stone.default_state();
        fallback(stone, world, visit);
    }
}

/// Linear albedo multipliers of tinted faces, indexed by [`tint_index`]; the
/// official biome colors are encoded in sRGB. Index 0 leaves a face untinted.
/// `model_vertex.wesl` reads this palette from the end of the model face table.
pub(crate) const TINTS: [[f32; 3]; 8] = [
    [1.0; 3],
    [0.2835549, 0.5086107, 0.0998871],
    [0.1192802, 0.3185468, 0.1192802],
    [0.2158970, 0.3865575, 0.0906547],
    [0.1847580, 0.4077951, 0.0283367],
    [0.0143499, 0.2158970, 0.0294883],
    [1.0; 3],
    [1.0; 3],
];

/// Tint only faces marked tintindex by the model. Fixed plains colors match the
/// current world lighting/texture convention; biome tinting remains separate.
pub(crate) fn tint_index(kind: VoxelKind) -> u8 {
    static INDICES: std::sync::LazyLock<Box<[u8]>> = std::sync::LazyLock::new(|| {
        VoxelKind::ALL
            .iter()
            .map(|kind| match kind.native_id().unwrap() {
                BlockId::SPRUCE_LEAVES => 2,
                BlockId::BIRCH_LEAVES => 3,
                id if kind.tagged(pumpkin_data::tag::Block::MINECRAFT_LEAVES)
                    || id == BlockId::VINE =>
                {
                    4
                }
                BlockId::LILY_PAD => 5,
                _ => 1,
            })
            .collect()
    });
    INDICES.get(kind.index()).copied().unwrap_or(0)
}

pub(crate) fn tint(kind: VoxelKind) -> [f32; 3] {
    TINTS[tint_index(kind) as usize]
}

pub(crate) fn chest_parts(half: usize) -> Option<&'static ChestParts> {
    library().chests.as_ref().map(|parts| &parts[half])
}
