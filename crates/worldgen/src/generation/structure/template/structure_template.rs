//! Embedded vanilla structure templates, block metadata, and transformations.

use std::io::Cursor;
use std::sync::OnceLock;

use pumpkin_data::{Block, BlockStateId, Mirror, Property, Rotation};
use pumpkin_nbt::{compound::NbtCompound, tag::NbtTag};
use pumpkin_util::math::{block_box::BlockBox, vector3::Vector3};

use super::processor::StructureProcessor;
use crate::generation::structure::structures::jigsaw::JigsawBlock;

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

/// Embedded geometry stays in its four-byte records; only palettes and sparse NBT are owned.
#[derive(Debug, Clone, Default)]
pub struct StructureTemplate {
    palettes: Box<[Palette]>,
    packed_blocks: &'static [[u8; 4]],
    block_entities: Box<[(usize, NbtCompound)]>,
    pub entities: Box<[StructureEntityInfo]>,
    pub size: Vector3<i32>,
    pub author: String,
    pub name: Option<String>,

    /// Parsed lazily so placement never rescans the template's block list.
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

    pub fn with_properties(name: &str, properties: &[Property]) -> Self {
        let block = Block::from_registry_key(name.strip_prefix("minecraft:").unwrap_or(name))
            .expect("Unknown template block");
        Self {
            id: block
                .default_state
                .id
                .with_properties(properties)
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

/// Native states resolved once at import, shared by every placement of a palette.
#[derive(Debug, Clone, Default)]
pub struct Palette {
    entries: Box<[PaletteEntry]>,
}

/// A decoded view into embedded geometry, with borrowed metadata.
#[derive(Debug, Clone, Copy)]
pub struct TemplateBlock<'a> {
    pub pos: Vector3<i32>,
    pub state: PaletteEntry,
    pub nbt: Option<&'a NbtCompound>,
}

impl StructureTemplate {
    #[cfg(feature = "dev")]
    pub(crate) fn owned_bytes(&self) -> usize {
        size_of_val(self.palettes.as_ref())
            + self
                .palettes
                .iter()
                .map(|p| size_of_val(p.entries.as_ref()))
                .sum::<usize>()
            + size_of_val(self.block_entities.as_ref())
            + self
                .block_entities
                .iter()
                .map(|(_, nbt)| nbt_bytes(nbt))
                .sum::<usize>()
            + size_of_val(self.entities.as_ref())
            + self
                .entities
                .iter()
                .map(|e| nbt_bytes(&e.nbt))
                .sum::<usize>()
            + self.author.capacity()
            + self.name.as_ref().map_or(0, String::capacity)
            + self.jigsaw_blocks_cache.get().map_or(0, |cache| {
                cache.0.capacity() * size_of::<JigsawBlock>()
                    + cache
                        .0
                        .iter()
                        .map(|b| {
                            b.name.capacity()
                                + b.target.capacity()
                                + b.pool.capacity()
                                + b.final_state.capacity()
                        })
                        .sum::<usize>()
            })
    }

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

    pub fn blocks(&self) -> impl Iterator<Item = TemplateBlock<'_>> {
        self.palettes
            .first()
            .into_iter()
            .flat_map(|palette| self.blocks_in(palette))
    }

    pub fn blocks_in<'a>(
        &'a self,
        palette: &'a Palette,
    ) -> impl ExactSizeIterator<Item = TemplateBlock<'a>> {
        // NBT is sparse and ordered by block ordinal: a merge walk avoids a hash
        // lookup or a metadata pointer stored for every ordinary template block.
        let mut entities = self.block_entities.iter().peekable();
        self.packed_blocks.iter().enumerate().map(move |(i, b)| {
            let nbt = if entities.peek().is_some_and(|(index, _)| *index == i) {
                Some(&entities.next().unwrap().1)
            } else {
                None
            };
            TemplateBlock {
                pos: Vector3::new(i32::from(b[0]), i32::from(b[1]), i32::from(b[2])),
                state: palette.entries[usize::from(b[3])],
                nbt,
            }
        })
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
                    self.blocks()
                        .filter_map(|block| JigsawBlock::from_template_block(&block, &block.state))
                        .collect(),
                )
            })
            .0
    }

    #[must_use]
    pub fn entity_info_list(&self) -> &[StructureEntityInfo] {
        &self.entities
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

        let packed_blocks = static_template.packed_blocks.as_chunks::<4>().0;
        let palette_len = packed_blocks
            .iter()
            .map(|block| usize::from(block[3]) + 1)
            .max()
            .unwrap_or(0);
        let palettes = static_template
            .palettes
            .iter()
            .map(|states| {
                let mut entries: Vec<_> = states
                    .iter()
                    .map(|p| PaletteEntry::with_properties(p.name, p.properties))
                    .collect();
                if entries.len() < palette_len {
                    entries.resize(palette_len, PaletteEntry::new(BlockStateId::AIR));
                }
                Palette {
                    entries: entries.into_boxed_slice(),
                }
            })
            .collect();
        let block_entities = packed_blocks
            .iter()
            .enumerate()
            .filter_map(|(i, b)| {
                block_entities_map
                    .get(&[i32::from(b[0]), i32::from(b[1]), i32::from(b[2])])
                    .map(|nbt| (i, nbt.clone()))
            })
            .collect();
        Self {
            palettes,
            packed_blocks,
            block_entities,
            entities: entity_info_list.into_boxed_slice(),
            size,
            author,
            name: None,
            jigsaw_blocks_cache: OnceLock::new(),
        }
    }

    /// Saves this structure template to an NBT compound matching vanilla 1:1.
    #[must_use]
    pub fn save(&self) -> NbtCompound {
        let mut tag = NbtCompound::new();

        let blocks = self
            .blocks()
            .zip(self.packed_blocks)
            .map(|(block, packed)| {
                let mut tag = NbtCompound::new();
                tag.put_list(
                    Self::BLOCK_TAG_POS,
                    vec![
                        NbtTag::Int(block.pos.x),
                        NbtTag::Int(block.pos.y),
                        NbtTag::Int(block.pos.z),
                    ],
                );
                tag.put_int(Self::BLOCK_TAG_STATE, i32::from(packed[3]));
                if let Some(nbt) = block.nbt {
                    tag.put_compound(Self::BLOCK_TAG_NBT, nbt.clone());
                }
                NbtTag::Compound(tag)
            })
            .collect();
        tag.put_list(Self::BLOCKS_TAG, blocks);
        let mut palettes = self.palettes.iter().map(|palette| {
            palette
                .entries
                .iter()
                .map(|state| NbtTag::Compound(state.to_nbt_compound()))
                .collect()
        });
        if self.palettes.len() <= 1 {
            tag.put_list(Self::PALETTE_TAG, palettes.next().unwrap_or_default());
        } else {
            tag.put_list(Self::PALETTE_LIST_TAG, palettes.map(NbtTag::List).collect());
        }

        let mut entity_list = Vec::new();
        for entity_info in &self.entities {
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
        if self.palettes.is_empty() {
            0
        } else {
            self.packed_blocks.len()
        }
    }

    #[must_use]
    pub fn has_block_entities(&self) -> bool {
        !self.palettes.is_empty() && !self.block_entities.is_empty()
    }

    #[must_use]
    pub const fn has_entities(&self) -> bool {
        !self.entities.is_empty()
    }
}

/// Heap payload only. Hash-map capacity is a lower bound, excluding bucket slack.
#[cfg(feature = "dev")]
pub(crate) fn nbt_bytes(nbt: &NbtCompound) -> usize {
    fn tag_bytes(tag: &NbtTag) -> usize {
        match tag {
            NbtTag::ByteArray(values) => values.len(),
            NbtTag::String(value) => value.len(),
            NbtTag::List(values) => {
                values.capacity() * size_of::<NbtTag>()
                    + values.iter().map(tag_bytes).sum::<usize>()
            }
            NbtTag::Compound(value) => nbt_bytes(value),
            NbtTag::IntArray(values) => values.capacity() * size_of::<i32>(),
            NbtTag::LongArray(values) => values.capacity() * size_of::<i64>(),
            _ => 0,
        }
    }
    nbt.child_tags.capacity() * size_of::<(Box<str>, NbtTag)>()
        + nbt
            .child_tags
            .iter()
            .map(|(key, value)| key.len() + tag_bytes(value))
            .sum::<usize>()
}
