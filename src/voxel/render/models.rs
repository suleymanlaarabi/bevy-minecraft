//! Baked vanilla models. All JSON, property matching and rotations happen once;
//! mesh workers only index shared, immutable quad lists.
use crate::voxel::{VoxelKind, VoxelState};
use bevy::prelude::*;
use serde::Deserialize;
use serde_json::Value;
use std::{
    collections::HashMap,
    sync::{Arc, OnceLock},
};

#[derive(Clone, Debug, Deserialize)]
pub(crate) struct ModelQuad {
    pub positions: [[f32; 3]; 4],
    pub normal: [f32; 3],
    pub uv: [[f32; 2]; 4],
    pub texture: u32,
    pub tinted: bool,
    pub cull: [i32; 3],
    pub translucent: bool,
    /// Vanilla's `shade_direction_override: "up"`, resolved once instead of per quad.
    #[serde(default, rename = "shade", deserialize_with = "shade_up")]
    pub shade_up: bool,
}
fn shade_up<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<bool, D::Error> {
    Ok(String::deserialize(deserializer)? == "up")
}
#[derive(Deserialize)]
struct Imported {
    version: u32,
    minecraft: String,
    textures: Vec<String>,
    /// Textures without transparent texels; older imports lack it (all cutout).
    #[serde(default)]
    opaque_textures: Vec<bool>,
    models: HashMap<String, Vec<ModelQuad>>,
    blocks: HashMap<String, Value>,
}
#[derive(Clone)]
struct Weighted {
    quads: Arc<[ModelQuad]>,
    weight: u32,
}
#[derive(Default)]
struct Recipe {
    groups: Vec<Vec<Weighted>>,
}
struct Library {
    recipes: Box<[Recipe]>,
    textures: Vec<String>,
    opaque_textures: Vec<bool>,
    /// Mesher classification of every state, read once per cell.
    cells: Box<[CellInfo]>,
    /// Face slots of greedy cubes, indexed by [`CellInfo::appearance`].
    appearances: Vec<Appearance>,
}

/// Per-state mesher flags (bits of [`CellInfo::flags`]).
pub(crate) mod cell {
    pub(crate) const AIR: u8 = 1;
    pub(crate) const WATER: u8 = 1 << 1;
    /// Opaque full cube: hides neighbor faces and darkens corners.
    pub(crate) const OCCLUDES: u8 = 1 << 2;
    /// Opaque cube drawn by the greedy terrain mesher.
    pub(crate) const CUBE: u8 = 1 << 3;
    /// Alpha-tested cube (leaves) drawn by the greedy mesher on its own mesh.
    pub(crate) const CUTOUT: u8 = 1 << 4;
    /// Explicit model quads.
    pub(crate) const MODEL: u8 = 1 << 5;
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct CellInfo {
    /// Greedy cube appearance (0 when not a cube), packed in 14 vertex bits.
    pub(crate) appearance: u16,
    pub(crate) flags: u8,
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
        if q.translucent || q.shade_up {
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
fn legacy_faces(kind: VoxelKind, textures: usize) -> [CubeFace; 6] {
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

fn base_flags(state: VoxelState) -> u8 {
    let mut flags = 0;
    if state.native().is_air() {
        flags |= cell::AIR;
    }
    if state.has_water() {
        flags |= cell::WATER;
    }
    if state.is_opaque_cube() {
        flags |= cell::OCCLUDES;
    }
    flags
}

/// Cells without a greedy cube and with something to draw use explicit models.
fn finish(mut info: CellInfo, state: VoxelState) -> CellInfo {
    if info.flags & (cell::AIR | cell::CUBE | cell::CUTOUT) == 0 && state.kind() != VoxelKind::Water
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
fn matches(condition: &Value, props: &[(&str, &str)]) -> bool {
    let Some(object) = condition.as_object() else {
        return false;
    };
    object.iter().all(|(key, value)| match key.as_str() {
        "OR" => value
            .as_array()
            .is_some_and(|a| a.iter().any(|v| matches(v, props))),
        "AND" => value
            .as_array()
            .is_some_and(|a| a.iter().all(|v| matches(v, props))),
        key => value.as_str().is_some_and(|values| {
            props
                .iter()
                .any(|&(k, v)| k == key && values.split('|').any(|choice| choice == v))
        }),
    })
}
fn variant_matches(key: &str, props: &[(&str, &str)]) -> bool {
    key.is_empty()
        || key.split(',').all(|condition| {
            condition.split_once('=').is_some_and(|(key, value)| {
                props
                    .iter()
                    .any(|&(k, v)| k == key && value.split('|').any(|candidate| candidate == v))
            })
        })
}
fn library() -> &'static Library {
    static LIBRARY: OnceLock<Library> = OnceLock::new();
    LIBRARY.get_or_init(|| {
        let imported = std::fs::read(crate::asset_directory().join("voxel/packs/vanilla/models.json"))
            .ok().and_then(|data| serde_json::from_slice::<Imported>(&data).ok()).filter(|data| data.version == 1 && data.minecraft == "26.3");
        let mut recipes = (0..=u16::MAX).map(|_| Recipe::default()).collect::<Vec<_>>();
        let mut cells = vec![CellInfo::default(); 65536];
        let mut appearances = Appearances::default();
        let legacy = legacy_keys();
        let Some(imported) = imported else {
            warn!("Vanilla models unavailable: using shape-based models; run scripts/import_vanilla.py");
            for raw in 0..=u16::MAX {
                let Some(state) = VoxelState::from_id(raw) else { continue; };
                let mut info = CellInfo { appearance: 0, flags: base_flags(state) };
                if state.is_opaque_cube() && legacy_cube(&legacy, state.kind()) {
                    info.appearance = appearances.id(legacy_faces(state.kind(), 0), state.kind());
                    info.flags |= cell::CUBE;
                }
                cells[raw as usize] = finish(info, state);
            }
            return Library { recipes: recipes.into(), cells: cells.into(), appearances: appearances.list, textures: Vec::new(), opaque_textures: Vec::new() };
        };
        let mut rotations = HashMap::<(String, i32, i32, bool), Arc<[ModelQuad]>>::new();
        let mut missing = std::collections::HashSet::new();
        for raw in 0..=u16::MAX {
            let Some(state) = VoxelState::from_id(raw) else { continue; };
            let kind = state.kind();
            let Some(block) = imported.blocks.get(kind.name()) else { continue; };
            let properties = state.properties();
            let props = properties.iter().map(|p| (p.key_str(), p.value_str())).collect::<Vec<_>>();
            let mut applications = Vec::new();
            if let Some(variants) = block.get("variants").and_then(Value::as_object) {
                if let Some((_, apply)) = variants.iter().find(|(key,_)| variant_matches(key, &props)) { applications.push(apply); }
            }
            if let Some(parts) = block.get("multipart").and_then(Value::as_array) {
                for part in parts {
                    if part.get("when").is_none_or(|when| matches(when, &props)) && let Some(apply) = part.get("apply") { applications.push(apply); }
                }
            }
            let recipe = &mut recipes[raw as usize];
            for apply in applications {
                let mut group = Vec::new();
                let values = if let Some(a) = apply.as_array() { a.as_slice() } else { std::slice::from_ref(apply) };
                for variant in values {
                    let Some(name) = variant.get("model").and_then(Value::as_str) else { continue; };
                    let x = variant.get("x").and_then(Value::as_i64).unwrap_or(0) as i32;
                    let y = variant.get("y").and_then(Value::as_i64).unwrap_or(0) as i32;
                    let lock = variant.get("uvlock").and_then(Value::as_bool).unwrap_or(false);
                    let Some(source) = imported.models.get(name) else { missing.insert(kind.name()); continue; };
                    let quads = rotations.entry((name.to_owned(),x,y,lock)).or_insert_with(|| rotated(source, x as f32, y as f32, lock)).clone();
                    if !quads.is_empty() {
                        group.push(Weighted { quads, weight: variant.get("weight").and_then(Value::as_u64).unwrap_or(1).clamp(1, u32::MAX as u64) as u32 });
                    }
                }
                if !group.is_empty() { recipe.groups.push(group); }
            }
            // Default legacy cubes keep the pack's per-block faces; any other state
            // whose single model is a plain cube shares a deduplicated appearance.
            let mut info = CellInfo { appearance: 0, flags: base_flags(state) };
            let model_faces = match recipe.groups.as_slice() {
                [group] => {
                    let mut faces = group.iter().map(|model| cube_faces(&model.quads));
                    faces.next().flatten().filter(|first| faces.all(|other| other == Some(*first)))
                }
                _ => None,
            };
            if state == kind.default_state() && kind.is_opaque() && legacy_cube(&legacy, kind)
                && !recipe.groups.is_empty()
            {
                info.appearance = appearances.id(legacy_faces(kind, imported.textures.len()), kind);
                info.flags |= cell::CUBE;
            } else if let Some(faces) = model_faces {
                let class = if state.is_opaque_cube() {
                    cell::CUBE
                } else if state.native().is_full_cube() {
                    cell::CUTOUT
                } else {
                    0
                };
                if class != 0 {
                    info.appearance = appearances.id(faces, kind);
                    info.flags |= class;
                }
            }
            cells[raw as usize] = finish(info, state);
        }
        if !missing.is_empty() { warn!("{} vanilla model families unavailable", missing.len()); }
        Library { recipes: recipes.into(), textures: imported.textures, opaque_textures: imported.opaque_textures, cells: cells.into(), appearances: appearances.list }
    })
}
/// Block keys of the bundled vanilla manifest.
fn legacy_keys() -> std::collections::HashSet<String> {
    super::texture_pack::manifest_keys(super::texture_pack::TexturePackId::Vanilla)
}
fn legacy_cube(keys: &std::collections::HashSet<String>, kind: VoxelKind) -> bool {
    // These keys already have the exact top/side/bottom mapping in all packs.
    // Logs with an explicit axis use the model path, as do slabs and farmland.
    !matches!(kind.name(), "dirt_path" | "farmland" | "snow")
        && !kind.name().ends_with("_stairs")
        && !kind.name().ends_with("_slab")
        && keys.contains(kind.key())
}
/// Mesher classification of a state.
#[inline]
pub(crate) fn cell_info(state: VoxelState) -> CellInfo {
    library().cells[state.id() as usize]
}
/// Faces of every greedy cube appearance, indexed by [`CellInfo::appearance`].
pub(crate) fn appearances() -> &'static [Appearance] {
    &library().appearances
}

/// Whether a model quad's texture is fully opaque, so it needs no alpha test.
/// Unknown slots (fallback faces, older imports) stay alpha-tested.
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
pub(crate) fn visit_quads(state: VoxelState, world: IVec3, mut visit: impl FnMut(&ModelQuad)) {
    let recipe = &library().recipes[state.id() as usize];
    if recipe.groups.is_empty() {
        fallback(state, world, visit);
        return;
    }
    let mut seed = (world.x as u32).wrapping_mul(0x8da6b343)
        ^ (world.z as u32).wrapping_mul(0xcb1ab31f)
        ^ (world.y as u32).wrapping_mul(0xd8163841);
    for group in &recipe.groups {
        seed ^= seed >> 16;
        seed = seed.wrapping_mul(0x7feb352d);
        let total: u64 = group.iter().map(|model| u64::from(model.weight)).sum();
        let mut choice = u64::from(seed) % total;
        let model = group
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
    textures as u32
        + kind.index() as u32 * 3
        + if axis == 1 {
            if positive { 0 } else { 2 }
        } else {
            1
        }
}
fn fallback(state: VoxelState, world: IVec3, mut visit: impl FnMut(&ModelQuad)) {
    if state.native().is_air()
        || state.kind() == VoxelKind::Water
        || state.kind().name() == "bubble_column"
    {
        return;
    }
    let kind = state.kind();
    let translucent = kind.name().contains("glass") || kind.name() == "ice";
    let mut any = false;
    for b in state
        .selection_boxes(world)
        .chain(
            (kind == VoxelKind::Lava).then_some(crate::voxel::state::BlockBox {
                min: Vec3::ZERO,
                max: Vec3::ONE,
            }),
        )
    {
        any = true;
        for axis in 0..3 {
            let u = (axis + 1) % 3;
            let v = (axis + 2) % 3;
            for positive in [false, true] {
                let mut p = b.min;
                p[axis] = if positive { b.max[axis] } else { b.min[axis] };
                let mut du = Vec3::ZERO;
                du[u] = b.size()[u];
                let mut dv = Vec3::ZERO;
                dv[v] = b.size()[v];
                let positions = if positive {
                    [p, p + du, p + du + dv, p + dv]
                } else {
                    [p, p + dv, p + du + dv, p + du]
                };
                let mut normal = Vec3::ZERO;
                normal[axis] = if positive { 1.0 } else { -1.0 };
                visit(&ModelQuad {
                    positions: positions.map(|p| p.to_array()),
                    normal: normal.to_array(),
                    uv: [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]],
                    texture: fallback_slot(kind, axis, positive),
                    tinted: false,
                    cull: [0; 3],
                    translucent,
                    shade_up: false,
                });
            }
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
            .map(|kind| match kind.name() {
                "spruce_leaves" => 2,
                "birch_leaves" => 3,
                name if name.ends_with("_leaves") || name == "vine" => 4,
                "lily_pad" => 5,
                _ => 1,
            })
            .collect()
    });
    INDICES[kind.index()]
}

pub(crate) fn tint(kind: VoxelKind) -> [f32; 3] {
    TINTS[tint_index(kind) as usize]
}
