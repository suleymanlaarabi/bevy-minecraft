//! Embedded vanilla structure templates, block metadata, and transformations.

use std::io::Cursor;
use std::sync::OnceLock;

use pumpkin_data::{Block, BlockId, BlockStateId, Mirror, Property, Rotation};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{block_box::BlockBox, vector3::Vector3};

use super::processor::StructureProcessor;
use crate::generation::structure::structures::jigsaw::{JigsawBlock, JigsawJointType};

/// Settings used when placing, transforming, or querying a [`StructureTemplate`].
#[derive(Clone, Debug)]
pub struct StructurePlaceSettings {
    pub mirror: Mirror,
    pub rotation: Rotation,
    pub rotation_pivot: Vector3<i32>,
    pub bounding_box: Option<BlockBox>,
    pub ignore_entities: bool,
    pub apply_waterlogging: bool,
    pub known_shape: bool,
    pub processors: Vec<StructureProcessor>,
    pub finalize_entities: bool,
    pub palette_index: Option<usize>,
}

impl Default for StructurePlaceSettings {
    fn default() -> Self {
        Self {
            mirror: Mirror::None,
            rotation: Rotation::None,
            rotation_pivot: Vector3::new(0, 0, 0),
            bounding_box: None,
            ignore_entities: false,
            apply_waterlogging: false,
            known_shape: false,
            processors: Vec::new(),
            finalize_entities: false,
            palette_index: None,
        }
    }
}

impl StructurePlaceSettings {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn set_mirror(mut self, mirror: Mirror) -> Self {
        self.mirror = mirror;
        self
    }

    #[must_use]
    pub const fn set_rotation(mut self, rotation: Rotation) -> Self {
        self.rotation = rotation;
        self
    }

    #[must_use]
    pub const fn set_rotation_pivot(mut self, pivot: Vector3<i32>) -> Self {
        self.rotation_pivot = pivot;
        self
    }

    #[must_use]
    pub const fn set_bounding_box(mut self, bounding_box: Option<BlockBox>) -> Self {
        self.bounding_box = bounding_box;
        self
    }

    #[must_use]
    pub const fn set_ignore_entities(mut self, ignore_entities: bool) -> Self {
        self.ignore_entities = ignore_entities;
        self
    }

    #[must_use]
    pub const fn set_apply_waterlogging(mut self, apply_waterlogging: bool) -> Self {
        self.apply_waterlogging = apply_waterlogging;
        self
    }

    #[must_use]
    pub const fn set_known_shape(mut self, known_shape: bool) -> Self {
        self.known_shape = known_shape;
        self
    }

    #[must_use]
    pub const fn set_finalize_entities(mut self, finalize_entities: bool) -> Self {
        self.finalize_entities = finalize_entities;
        self
    }

    #[must_use]
    pub fn add_processor(mut self, processor: StructureProcessor) -> Self {
        self.processors.push(processor);
        self
    }

    #[must_use]
    pub const fn get_mirror(&self) -> Mirror {
        self.mirror
    }

    #[must_use]
    pub const fn get_rotation(&self) -> Rotation {
        self.rotation
    }

    #[must_use]
    pub const fn get_rotation_pivot(&self) -> Vector3<i32> {
        self.rotation_pivot
    }

    #[must_use]
    pub const fn get_bounding_box(&self) -> Option<&BlockBox> {
        self.bounding_box.as_ref()
    }

    #[must_use]
    pub const fn is_ignore_entities(&self) -> bool {
        self.ignore_entities
    }

    #[must_use]
    pub const fn should_apply_waterlogging(&self) -> bool {
        self.apply_waterlogging
    }

    #[must_use]
    pub const fn get_known_shape(&self) -> bool {
        self.known_shape
    }

    #[must_use]
    pub fn get_processors(&self) -> &[StructureProcessor] {
        &self.processors
    }

    #[must_use]
    pub const fn should_finalize_entities(&self) -> bool {
        self.finalize_entities
    }

    #[must_use]
    pub fn get_random_palette<'a>(
        &self,
        palettes: &'a [Palette],
        pos: Vector3<i32>,
    ) -> &'a Palette {
        assert!(
            !palettes.is_empty(),
            "Cannot get palette from empty palette list"
        );
        if let Some(idx) = self.palette_index
            && idx < palettes.len()
        {
            return &palettes[idx];
        }
        let index = ((pos.x.wrapping_add(pos.y).wrapping_add(pos.z)).unsigned_abs() as usize)
            % palettes.len();
        &palettes[index]
    }
}

/// Lazily-parsed jigsaw blocks for a template.
///
/// Wrapped so the template's `Debug` derive does not require `JigsawBlock: Debug`.
#[derive(Clone, Default)]
struct JigsawBlockCache(Vec<JigsawBlock>);

impl std::fmt::Debug for JigsawBlockCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("JigsawBlockCache")
            .field(&self.0.len())
            .finish()
    }
}

/// A loaded structure template from an NBT file matching vanilla `StructureTemplate`.
#[derive(Debug, Clone, Default)]
pub struct StructureTemplate {
    pub palettes: Vec<Palette>,
    pub entity_info_list: Vec<StructureEntityInfo>,
    pub size: Vector3<i32>,
    pub author: String,
    pub name: Option<String>,

    // Backward-compatible fields
    // TODO: make these fields private with accessors. They are public now, so a
    // caller can change them after the jigsaw cache is filled. The cache then
    // goes stale. `load()` resets the cache, but a direct mutation does not.
    pub palette: Vec<PaletteEntry>,
    pub blocks: Vec<TemplateBlock>,
    pub entities: Vec<TemplateEntity>,

    /// Jigsaw blocks parsed from the flat `blocks`/`palette` view, computed lazily
    /// on first use so placement never rescans the template's block list.
    jigsaw_blocks_cache: OnceLock<JigsawBlockCache>,
}

/// A template palette entry is a validated native state, without owned names or properties.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PaletteEntry {
    pub id: BlockStateId,
}
impl PaletteEntry {
    pub const fn new(id: BlockStateId) -> Self {
        Self { id }
    }

    pub fn with_properties(name: String, properties: Vec<Property>) -> Self {
        let block = Block::from_registry_key(name.strip_prefix("minecraft:").unwrap_or(&name))
            .expect("Unknown template block");
        Self {
            id: block
                .default_state
                .id
                .with_properties(&properties)
                .expect("Invalid template properties"),
        }
    }

    /// Resolve textual jigsaw states at the import boundary.
    pub fn from_string(value: &str) -> Self {
        let (name, properties) = value
            .split_once('[')
            .map_or((value, None), |(name, properties)| (name, Some(properties)));
        let block = Block::from_registry_key(name.strip_prefix("minecraft:").unwrap_or(name))
            .expect("Unknown template block");
        let mut id = block.default_state.id;
        if let Some(properties) = properties {
            let properties = properties
                .strip_suffix(']')
                .expect("Invalid template property list");
            for property in properties
                .split(',')
                .filter(|property| !property.is_empty())
            {
                let (key, value) = property.split_once('=').expect("Invalid template property");
                let property = Property::from_str_pair(key.trim(), value.trim())
                    .expect("Unknown template property");
                id = id
                    .with_properties(&[property])
                    .expect("Invalid template property value");
            }
        }
        Self { id }
    }

    pub fn rotate(&self, rotation: Rotation) -> Self {
        self.transform(rotation, Mirror::None)
    }
    pub fn mirror(&self, mirror: Mirror) -> Self {
        self.transform(Rotation::None, mirror)
    }
    pub fn transform(&self, rotation: Rotation, mirror: Mirror) -> Self {
        let state = self.id.to_block().mirror(self.id, mirror);
        Self {
            id: state.rotate(rotation).id,
        }
    }

    pub fn to_nbt_compound(&self) -> NbtCompound {
        let mut compound = NbtCompound::new();
        compound.put_string("id", format!("minecraft:{}", self.id.to_block().name));
        let mut properties = NbtCompound::new();
        for property in self.id.properties() {
            properties.put_string(property.key_str(), property.value_str().to_owned());
        }
        if !properties.child_tags.is_empty() {
            compound.put_compound("properties", properties);
        }
        compound
    }
}
const _: () = assert!(std::mem::size_of::<PaletteEntry>() == 2);

/// Block info record within a template.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureBlockInfo {
    pub pos: Vector3<i32>,
    pub state: PaletteEntry,
    pub nbt: Option<NbtCompound>,
}

impl StructureBlockInfo {
    #[must_use]
    pub const fn new(pos: Vector3<i32>, state: PaletteEntry, nbt: Option<NbtCompound>) -> Self {
        Self { pos, state, nbt }
    }
}

/// Entity info record within a template.
#[derive(Debug, Clone, PartialEq)]
pub struct StructureEntityInfo {
    pub pos: Vector3<f64>,
    pub block_pos: Vector3<i32>,
    pub nbt: NbtCompound,
}

impl StructureEntityInfo {
    #[must_use]
    pub const fn new(pos: Vector3<f64>, block_pos: Vector3<i32>, nbt: NbtCompound) -> Self {
        Self {
            pos,
            block_pos,
            nbt,
        }
    }
}

/// Jigsaw metadata extracted from a jigsaw block in a structure template.
#[derive(Debug, Clone, PartialEq)]
pub struct JigsawBlockInfo {
    pub info: StructureBlockInfo,
    pub joint_type: JigsawJointType,
    pub name: String,
    pub pool: String,
    pub target: String,
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl JigsawBlockInfo {
    #[must_use]
    pub fn of(info: StructureBlockInfo) -> Self {
        let nbt = info.nbt.as_ref();
        let joint_type = StructureTemplate::get_joint_type(nbt, &info.state);
        let name = nbt
            .and_then(|n| n.get_string("name"))
            .unwrap_or("minecraft:empty")
            .to_string();
        let pool = nbt
            .and_then(|n| n.get_string("pool"))
            .unwrap_or("minecraft:empty")
            .to_string();
        let target = nbt
            .and_then(|n| n.get_string("target"))
            .unwrap_or("minecraft:empty")
            .to_string();
        let placement_priority = nbt
            .and_then(|n| n.get_int("placement_priority"))
            .unwrap_or(0);
        let selection_priority = nbt
            .and_then(|n| n.get_int("selection_priority"))
            .unwrap_or(0);

        Self {
            info,
            joint_type,
            name,
            pool,
            target,
            placement_priority,
            selection_priority,
        }
    }

    #[must_use]
    pub fn with_info(&self, info: StructureBlockInfo) -> Self {
        Self {
            info,
            joint_type: self.joint_type,
            name: self.name.clone(),
            pool: self.pool.clone(),
            target: self.target.clone(),
            placement_priority: self.placement_priority,
            selection_priority: self.selection_priority,
        }
    }
}

/// Represents a palette of block placements within a template.
#[derive(Debug, Clone)]
pub struct Palette {
    blocks: Vec<StructureBlockInfo>,
    cached_jigsaws: Option<Vec<JigsawBlockInfo>>,
}

impl Palette {
    #[must_use]
    pub const fn new(blocks: Vec<StructureBlockInfo>) -> Self {
        Self {
            blocks,
            cached_jigsaws: None,
        }
    }

    #[must_use]
    pub fn blocks(&self) -> &[StructureBlockInfo] {
        &self.blocks
    }

    #[must_use]
    pub fn blocks_by_id(&self, filter: BlockId) -> Vec<StructureBlockInfo> {
        self.blocks
            .iter()
            .filter(|b| b.state.id.to_block_id() == filter)
            .cloned()
            .collect()
    }

    pub fn jigsaws(&mut self) -> &[JigsawBlockInfo] {
        self.cached_jigsaws.get_or_insert_with(|| {
            self.blocks
                .iter()
                .filter(|b| b.state.id.to_block_id() == BlockId::JIGSAW)
                .map(|b| JigsawBlockInfo::of(b.clone()))
                .collect()
        })
    }
}

/// Helper mapping table between palette entries and integer indices.
#[derive(Debug, Default, Clone)]
pub struct SimplePalette {
    pub entries: Vec<PaletteEntry>,
}

impl SimplePalette {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn id_for(&mut self, state: PaletteEntry) -> u32 {
        if let Some(pos) = self.entries.iter().position(|e| *e == state) {
            pos as u32
        } else {
            let id = self.entries.len() as u32;
            self.entries.push(state);
            id
        }
    }

    #[must_use]
    pub fn state_for(&self, index: usize) -> Option<&PaletteEntry> {
        self.entries.get(index)
    }

    pub fn add_mapping(&mut self, state: PaletteEntry, id: usize) {
        if id >= self.entries.len() {
            self.entries.resize(
                id + 1,
                PaletteEntry::new(pumpkin_data::Block::AIR.default_state.id),
            );
        }
        self.entries[id] = state;
    }

    #[must_use]
    pub const fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// A single block placement in the template (legacy format).
#[derive(Debug, Clone)]
pub struct TemplateBlock {
    pub pos: Vector3<i32>,
    pub state: u32,
    pub nbt: Option<NbtCompound>,
}

/// An entity to spawn when placing the template (legacy format).
#[derive(Debug, Clone)]
pub struct TemplateEntity {
    pub pos: Vector3<f64>,
    pub block_pos: Vector3<i32>,
    pub nbt: NbtCompound,
}

impl StructureTemplate {
    pub const PALETTE_TAG: &'static str = "palette";
    pub const PALETTE_LIST_TAG: &'static str = "palettes";
    pub const ENTITIES_TAG: &'static str = "entities";
    pub const BLOCKS_TAG: &'static str = "blocks";
    pub const BLOCK_TAG_POS: &'static str = "pos";
    pub const BLOCK_TAG_STATE: &'static str = "state";
    pub const BLOCK_TAG_NBT: &'static str = "nbt";
    pub const ENTITY_TAG_POS: &'static str = "pos";
    pub const ENTITY_TAG_BLOCKPOS: &'static str = "blockPos";
    pub const ENTITY_TAG_NBT: &'static str = "nbt";
    pub const SIZE_TAG: &'static str = "size";

    #[must_use]
    pub const fn get_size(&self) -> Vector3<i32> {
        self.size
    }

    pub fn set_author(&mut self, author: String) {
        self.author = author;
    }

    #[must_use]
    pub fn get_author(&self) -> &str {
        &self.author
    }

    #[must_use]
    pub fn palettes(&self) -> &[Palette] {
        &self.palettes
    }

    pub const fn palettes_mut(&mut self) -> &mut Vec<Palette> {
        &mut self.palettes
    }

    /// Returns the template's jigsaw blocks in template-local coordinates.
    ///
    /// Parsed once from the flat `blocks`/`palette` view and cached, rather than
    /// being re-scanned and re-parsed on every placement attempt.
    #[must_use]
    pub fn jigsaw_blocks(&self) -> &[JigsawBlock] {
        &self
            .jigsaw_blocks_cache
            .get_or_init(|| {
                if let Some(name) = &self.name
                    && let Some(meta) =
                        pumpkin_data::structure_metadata::StaticStructureMetadataList::get(name)
                {
                    return JigsawBlockCache(meta.jigsaws.iter().map(JigsawBlock::from).collect());
                }

                JigsawBlockCache(
                    self.blocks
                        .iter()
                        .filter_map(|block| {
                            JigsawBlock::from_template_block(
                                block,
                                &self.palette[block.state as usize],
                            )
                        })
                        .collect(),
                )
            })
            .0
    }

    #[must_use]
    pub fn entity_info_list(&self) -> &[StructureEntityInfo] {
        &self.entity_info_list
    }

    #[must_use]
    pub const fn get_size_with_rotation(&self, rotation: Rotation) -> Vector3<i32> {
        match rotation {
            Rotation::CounterClockwise90 | Rotation::Clockwise90 => {
                Vector3::new(self.size.z, self.size.y, self.size.x)
            }
            _ => self.size,
        }
    }

    /// Transforms a block position by mirror, rotation, and pivot.
    #[must_use]
    pub const fn transform_block_pos(
        pos: Vector3<i32>,
        mirror: Mirror,
        rotation: Rotation,
        pivot: Vector3<i32>,
    ) -> Vector3<i32> {
        let mut x = pos.x;
        let y = pos.y;
        let mut z = pos.z;
        let mut was_mirrored = true;
        match mirror {
            Mirror::LeftRight => z = -z,
            Mirror::FrontBack => x = -x,
            Mirror::None => was_mirrored = false,
        }

        let pivot_x = pivot.x;
        let pivot_z = pivot.z;

        match rotation {
            Rotation::CounterClockwise90 => {
                Vector3::new(pivot_x - pivot_z + z, y, pivot_x + pivot_z - x)
            }
            Rotation::Clockwise90 => Vector3::new(pivot_x + pivot_z - z, y, pivot_z - pivot_x + x),
            Rotation::Rotate180 => Vector3::new(pivot_x + pivot_x - x, y, pivot_z + pivot_z - z),
            Rotation::None => {
                if was_mirrored {
                    Vector3::new(x, y, z)
                } else {
                    pos
                }
            }
        }
    }

    /// Transforms a Vec3 double position by mirror, rotation, and pivot.
    #[must_use]
    pub fn transform_vec3(
        pos: Vector3<f64>,
        mirror: Mirror,
        rotation: Rotation,
        pivot: Vector3<i32>,
    ) -> Vector3<f64> {
        let mut x = pos.x;
        let y = pos.y;
        let mut z = pos.z;
        let mut was_mirrored = true;
        match mirror {
            Mirror::LeftRight => z = 1.0 - z,
            Mirror::FrontBack => x = 1.0 - x,
            Mirror::None => was_mirrored = false,
        }

        let pivot_x = pivot.x as f64;
        let pivot_z = pivot.z as f64;

        match rotation {
            Rotation::CounterClockwise90 => {
                Vector3::new(pivot_x - pivot_z + z, y, pivot_x + pivot_z + 1.0 - x)
            }
            Rotation::Clockwise90 => {
                Vector3::new(pivot_x + pivot_z + 1.0 - z, y, pivot_z - pivot_x + x)
            }
            Rotation::Rotate180 => {
                Vector3::new(pivot_x + pivot_x + 1.0 - x, y, pivot_z + pivot_z + 1.0 - z)
            }
            Rotation::None => {
                if was_mirrored {
                    Vector3::new(x, y, z)
                } else {
                    pos
                }
            }
        }
    }

    #[must_use]
    pub fn get_zero_position_with_transform(
        &self,
        zero_pos: Vector3<i32>,
        mirror: Mirror,
        rotation: Rotation,
    ) -> Vector3<i32> {
        Self::get_zero_position_with_transform_size(
            zero_pos,
            mirror,
            rotation,
            self.size.x,
            self.size.z,
        )
    }

    #[must_use]
    pub fn get_zero_position_with_transform_size(
        zero_pos: Vector3<i32>,
        mirror: Mirror,
        rotation: Rotation,
        mut size_x: i32,
        mut size_z: i32,
    ) -> Vector3<i32> {
        size_x -= 1;
        size_z -= 1;
        let mirror_delta_x = if mirror == Mirror::FrontBack {
            size_x
        } else {
            0
        };
        let mirror_delta_z = if mirror == Mirror::LeftRight {
            size_z
        } else {
            0
        };

        match rotation {
            Rotation::CounterClockwise90 => {
                zero_pos + Vector3::new(mirror_delta_z, 0, size_x - mirror_delta_x)
            }
            Rotation::Clockwise90 => {
                zero_pos + Vector3::new(size_z - mirror_delta_z, 0, mirror_delta_x)
            }
            Rotation::Rotate180 => {
                zero_pos + Vector3::new(size_x - mirror_delta_x, 0, size_z - mirror_delta_z)
            }
            Rotation::None => zero_pos + Vector3::new(mirror_delta_x, 0, mirror_delta_z),
        }
    }

    #[must_use]
    pub const fn calculate_relative_position(
        settings: &StructurePlaceSettings,
        pos: Vector3<i32>,
    ) -> Vector3<i32> {
        Self::transform_block_pos(
            pos,
            settings.get_mirror(),
            settings.get_rotation(),
            settings.get_rotation_pivot(),
        )
    }

    #[must_use]
    pub fn calculate_connected_position(
        settings1: &StructurePlaceSettings,
        connection1: Vector3<i32>,
        settings2: &StructurePlaceSettings,
        connection2: Vector3<i32>,
    ) -> Vector3<i32> {
        let marker_pos1 = Self::calculate_relative_position(settings1, connection1);
        let marker_pos2 = Self::calculate_relative_position(settings2, connection2);
        marker_pos1 - marker_pos2
    }

    #[must_use]
    pub fn get_bounding_box(
        &self,
        settings: &StructurePlaceSettings,
        position: Vector3<i32>,
    ) -> BlockBox {
        self.get_bounding_box_with_pivot(
            position,
            settings.get_rotation(),
            settings.get_rotation_pivot(),
            settings.get_mirror(),
        )
    }

    #[must_use]
    pub fn get_bounding_box_with_pivot(
        &self,
        position: Vector3<i32>,
        rotation: Rotation,
        pivot: Vector3<i32>,
        mirror: Mirror,
    ) -> BlockBox {
        Self::get_bounding_box_static(position, rotation, pivot, mirror, self.size)
    }

    #[must_use]
    pub fn get_bounding_box_static(
        position: Vector3<i32>,
        rotation: Rotation,
        pivot: Vector3<i32>,
        mirror: Mirror,
        size: Vector3<i32>,
    ) -> BlockBox {
        let delta = Vector3::new(size.x - 1, size.y - 1, size.z - 1);
        let corner1 = Self::transform_block_pos(Vector3::new(0, 0, 0), mirror, rotation, pivot);
        let corner2 = Self::transform_block_pos(delta, mirror, rotation, pivot);
        let min_x = corner1.x.min(corner2.x);
        let min_y = corner1.y.min(corner2.y);
        let min_z = corner1.z.min(corner2.z);
        let max_x = corner1.x.max(corner2.x);
        let max_y = corner1.y.max(corner2.y);
        let max_z = corner1.z.max(corner2.z);
        let mut bb = BlockBox::new(min_x, min_y, min_z, max_x, max_y, max_z);
        bb.move_pos(position.x, position.y, position.z);
        bb
    }

    pub fn get_jigsaws(
        &mut self,
        position: Vector3<i32>,
        rotation: Rotation,
    ) -> Vec<JigsawBlockInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }

        let settings = StructurePlaceSettings::new().set_rotation(rotation);
        let jigsaws = {
            let palette = settings.get_random_palette(&self.palettes, position);
            let mut pal_clone = palette.clone();
            pal_clone.jigsaws().to_vec()
        };

        let mut result = Vec::with_capacity(jigsaws.len());
        for jigsaw in jigsaws {
            let block_info = &jigsaw.info;
            let rel_pos = Self::calculate_relative_position(&settings, block_info.pos) + position;
            let rotated_info = StructureBlockInfo::new(
                rel_pos,
                block_info.state.rotate(settings.get_rotation()),
                block_info.nbt.clone(),
            );
            result.push(jigsaw.with_info(rotated_info));
        }

        result
    }

    #[must_use]
    pub fn filter_blocks(
        &self,
        position: Vector3<i32>,
        settings: &StructurePlaceSettings,
        block: BlockId,
    ) -> Vec<StructureBlockInfo> {
        self.filter_blocks_with_absolute(position, settings, block, true)
    }

    #[must_use]
    pub fn filter_blocks_with_absolute(
        &self,
        position: Vector3<i32>,
        settings: &StructurePlaceSettings,
        block: BlockId,
        absolute: bool,
    ) -> Vec<StructureBlockInfo> {
        if self.palettes.is_empty() {
            return Vec::new();
        }

        let mut result = Vec::new();
        let bounding_box = settings.get_bounding_box();
        let palette = settings.get_random_palette(&self.palettes, position);

        for block_info in palette.blocks() {
            if block_info.state.id.to_block_id() == block {
                let block_pos = if absolute {
                    Self::calculate_relative_position(settings, block_info.pos) + position
                } else {
                    block_info.pos
                };
                if bounding_box.is_none_or(|bb| bb.contains_pos(&block_pos)) {
                    result.push(StructureBlockInfo::new(
                        block_pos,
                        block_info.state.rotate(settings.get_rotation()),
                        block_info.nbt.clone(),
                    ));
                }
            }
        }

        result
    }

    #[must_use]
    pub fn get_joint_type(nbt: Option<&NbtCompound>, state: &PaletteEntry) -> JigsawJointType {
        nbt.and_then(|n| n.get_string("joint")).map_or_else(
            || Self::get_default_joint_type(state),
            JigsawJointType::from_str,
        )
    }

    #[must_use]
    pub fn get_default_joint_type(state: &PaletteEntry) -> JigsawJointType {
        use pumpkin_data::block_properties::{Orientation, Property};
        state
            .id
            .properties()
            .find_map(|p| match p {
                Property::Orientation(o) => Some(o),
                _ => None,
            })
            .map_or(JigsawJointType::Rollable, |orientation| match orientation {
                Orientation::NorthUp
                | Orientation::SouthUp
                | Orientation::EastUp
                | Orientation::WestUp => JigsawJointType::Aligned,
                _ => JigsawJointType::Rollable,
            })
    }

    /// Loads a structure template from a pre-parsed static template definition.
    pub fn from_static(
        static_template: &pumpkin_data::structure_template::StaticStructureTemplate,
    ) -> Self {
        let size = Vector3::new(
            static_template.size[0],
            static_template.size[1],
            static_template.size[2],
        );
        let author = static_template.author.to_string();

        let mut entity_info_list = Vec::with_capacity(static_template.entities.len());
        for e in static_template.entities {
            let mut cursor = Cursor::new(e.nbt);
            let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(&mut reader)
                .map(|n| n.root_tag)
                .unwrap_or_default();
            entity_info_list.push(StructureEntityInfo::new(
                Vector3::new(e.pos[0], e.pos[1], e.pos[2]),
                Vector3::new(e.block_pos[0], e.block_pos[1], e.block_pos[2]),
                nbt,
            ));
        }

        let mut block_entities_map =
            std::collections::HashMap::with_capacity(static_template.block_entities.len());
        for be in static_template.block_entities {
            let mut cursor = Cursor::new(be.nbt);
            let mut reader = pumpkin_nbt::deserializer::NbtReadHelperJava::new(&mut cursor);
            let nbt = pumpkin_nbt::Nbt::read_unnamed(&mut reader)
                .map(|n| n.root_tag)
                .unwrap_or_default();
            block_entities_map.insert(be.pos, nbt);
        }

        let block_count = static_template.packed_blocks.len() / 4;
        let mut palettes = Vec::with_capacity(static_template.palettes.len());

        for static_palette in static_template.palettes {
            let palette_entries: Vec<PaletteEntry> = static_palette
                .iter()
                .map(|p| PaletteEntry::with_properties(p.name.to_string(), p.properties.to_vec()))
                .collect();

            let mut block_info_list = Vec::with_capacity(block_count);
            for chunk in static_template.packed_blocks.as_chunks::<4>().0 {
                let pos = Vector3::new(chunk[0] as i32, chunk[1] as i32, chunk[2] as i32);
                let state_idx = chunk[3] as usize;
                let state = if state_idx < palette_entries.len() {
                    palette_entries[state_idx].clone()
                } else {
                    PaletteEntry::new(pumpkin_data::Block::AIR.default_state.id)
                };
                let nbt = block_entities_map.get(&[pos.x, pos.y, pos.z]).cloned();
                block_info_list.push(StructureBlockInfo::new(pos, state, nbt));
            }
            palettes.push(Palette::new(block_info_list));
        }

        let mut template = Self {
            palettes,
            entity_info_list,
            size,
            author,
            name: None,
            palette: Vec::new(),
            blocks: Vec::new(),
            entities: Vec::new(),
            jigsaw_blocks_cache: OnceLock::new(),
        };

        template.sync_legacy_fields();
        template
    }

    fn sync_legacy_fields(&mut self) {
        if let Some(main_palette) = self.palettes.first() {
            let mut simple_palette = SimplePalette::new();
            let mut blocks = Vec::with_capacity(main_palette.blocks().len());
            for b in main_palette.blocks() {
                let state_id = simple_palette.id_for(b.state.clone());
                blocks.push(TemplateBlock {
                    pos: b.pos,
                    state: state_id,
                    nbt: b.nbt.clone(),
                });
            }
            self.palette = simple_palette.entries;
            self.blocks = blocks;
        } else {
            self.palette.clear();
            self.blocks.clear();
        }

        self.entities = self
            .entity_info_list
            .iter()
            .map(|e| TemplateEntity {
                pos: e.pos,
                block_pos: e.block_pos,
                nbt: e.nbt.clone(),
            })
            .collect();
    }

    /// Saves this structure template to an NBT compound matching vanilla 1:1.
    #[must_use]
    pub fn save(&self) -> NbtCompound {
        let mut tag = NbtCompound::new();

        if self.palettes.is_empty() {
            tag.put_list(Self::BLOCKS_TAG, Vec::new());
            tag.put_list(Self::PALETTE_TAG, Vec::new());
        } else {
            let mut palettes = Vec::with_capacity(self.palettes.len());
            let main_palette = SimplePalette::new();
            palettes.push(main_palette);
            for _ in 1..self.palettes.len() {
                palettes.push(SimplePalette::new());
            }

            let mut block_list = Vec::new();
            let main_palette_blocks = self.palettes[0].blocks();

            for (i, block_info) in main_palette_blocks.iter().enumerate() {
                let mut block_tag = NbtCompound::new();
                let pos_tags = vec![
                    NbtTag::Int(block_info.pos.x),
                    NbtTag::Int(block_info.pos.y),
                    NbtTag::Int(block_info.pos.z),
                ];
                block_tag.put_list(Self::BLOCK_TAG_POS, pos_tags);

                let id = palettes[0].id_for(block_info.state.clone());
                block_tag.put_int(Self::BLOCK_TAG_STATE, id as i32);

                if let Some(nbt) = &block_info.nbt {
                    block_tag.put_compound(Self::BLOCK_TAG_NBT, nbt.clone());
                }

                block_list.push(NbtTag::Compound(block_tag));

                for p in 1..self.palettes.len() {
                    let state = self.palettes[p].blocks()[i].state.clone();
                    palettes[p].add_mapping(state, id as usize);
                }
            }

            tag.put_list(Self::BLOCKS_TAG, block_list);

            if palettes.len() == 1 {
                let palette_tags: Vec<NbtTag> = palettes[0]
                    .entries
                    .iter()
                    .map(|state| NbtTag::Compound(state.to_nbt_compound()))
                    .collect();
                tag.put_list(Self::PALETTE_TAG, palette_tags);
            } else {
                let palette_list_list: Vec<NbtTag> = palettes
                    .into_iter()
                    .map(|pal| {
                        let inner_tags: Vec<NbtTag> = pal
                            .entries
                            .into_iter()
                            .map(|state| NbtTag::Compound(state.to_nbt_compound()))
                            .collect();
                        NbtTag::List(inner_tags)
                    })
                    .collect();
                tag.put_list(Self::PALETTE_LIST_TAG, palette_list_list);
            }
        }

        let mut entity_list = Vec::new();
        for entity_info in &self.entity_info_list {
            let mut entity_tag = NbtCompound::new();
            let pos_tags = vec![
                NbtTag::Double(entity_info.pos.x),
                NbtTag::Double(entity_info.pos.y),
                NbtTag::Double(entity_info.pos.z),
            ];
            entity_tag.put_list(Self::ENTITY_TAG_POS, pos_tags);

            let block_pos_tags = vec![
                NbtTag::Int(entity_info.block_pos.x),
                NbtTag::Int(entity_info.block_pos.y),
                NbtTag::Int(entity_info.block_pos.z),
            ];
            entity_tag.put_list(Self::ENTITY_TAG_BLOCKPOS, block_pos_tags);

            entity_tag.put_compound(Self::ENTITY_TAG_NBT, entity_info.nbt.clone());
            entity_list.push(NbtTag::Compound(entity_tag));
        }
        tag.put_list(Self::ENTITIES_TAG, entity_list);

        let size_tags = vec![
            NbtTag::Int(self.size.x),
            NbtTag::Int(self.size.y),
            NbtTag::Int(self.size.z),
        ];
        tag.put_list(Self::SIZE_TAG, size_tags);

        tag.put_int("DataVersion", 4189);

        tag
    }

    #[must_use]
    pub const fn block_count(&self) -> usize {
        self.blocks.len()
    }

    #[must_use]
    pub fn has_block_entities(&self) -> bool {
        self.blocks.iter().any(|b| b.nbt.is_some())
    }

    #[must_use]
    pub const fn has_entities(&self) -> bool {
        !self.entities.is_empty()
    }
}
