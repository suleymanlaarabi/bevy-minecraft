use super::jigsaw_placement::{
    DimensionPadding, JigsawPlacement, LiquidSettings, MaxDistance, PoolAliasLookup,
};
use crate::generation::structure::structures::{
    StructureGenerator, StructureGeneratorContext, StructurePieceBase, StructurePosition,
};
use crate::generation::structure::template::{
    BlockMirror, BlockPlacer, BlockRotation, PaletteEntry, StructureTemplate,
};
use pumpkin_util::math::block_box::BlockBox;
use pumpkin_util::math::position::BlockPos;
use pumpkin_util::math::vector3::Vector3;
use pumpkin_util::random::{RandomDeriverImpl, RandomImpl};
use std::sync::Arc;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JigsawProjection {
    Rigid,
    TerrainMatching,
}

pub struct TemplatePool {
    pub id: String,
    pub fallback: String,
    pub elements: Vec<Arc<PoolElement>>,
}

pub struct PoolElement {
    pub weight: u32,
    pub projection: JigsawProjection,
    pub kind: PoolElementKind,
}

pub enum PoolElementKind {
    Empty,
    Single {
        template: String,
        /// Resolved template, loaded once when the pool element is created.
        resolved_template: Option<Arc<StructureTemplate>>,
        processors: ProcessorListRef,
        legacy: bool,
    },
    List(Vec<Self>),
    Feature(pumpkin_data::placed_feature::PlacedFeature),
}

#[derive(Clone, Default)]
pub enum ProcessorListRef {
    Named(String),
    #[default]
    Empty,
}

impl PoolElement {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self.kind, PoolElementKind::Empty)
    }

    #[must_use]
    pub fn first_template(&self) -> Option<Arc<StructureTemplate>> {
        fn find(kind: &PoolElementKind) -> Option<Arc<StructureTemplate>> {
            match kind {
                PoolElementKind::Single {
                    template,
                    resolved_template,
                    ..
                } => resolved_template
                    .clone()
                    .or_else(|| crate::generation::structure::template::get_template(template)),
                PoolElementKind::List(elements) => elements.iter().find_map(find),
                PoolElementKind::Empty | PoolElementKind::Feature(_) => None,
            }
        }

        find(&self.kind)
    }

    pub fn for_each_template(
        &self,
        mut consumer: impl FnMut(&str, &ProcessorListRef, bool, Arc<StructureTemplate>),
    ) {
        fn visit(
            kind: &PoolElementKind,
            consumer: &mut impl FnMut(&str, &ProcessorListRef, bool, Arc<StructureTemplate>),
        ) {
            match kind {
                PoolElementKind::Single {
                    template,
                    resolved_template,
                    processors,
                    legacy,
                } => {
                    let st = resolved_template.as_ref().map_or_else(
                        || crate::generation::structure::template::get_template(template),
                        |structure_template| Some(Arc::clone(structure_template)),
                    );
                    if let Some(structure_template) = st {
                        consumer(template, processors, *legacy, structure_template);
                    }
                }
                PoolElementKind::List(elements) => {
                    for element in elements {
                        visit(element, consumer);
                    }
                }
                PoolElementKind::Empty | PoolElementKind::Feature(_) => {}
            }
        }

        visit(&self.kind, &mut consumer);
    }
}

impl PoolElementKind {
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty)
    }

    #[must_use]
    pub fn get_ground_level_delta(&self) -> i32 {
        match self {
            Self::Single { .. } => 1,
            Self::List(elements) => elements.first().map_or(1, Self::get_ground_level_delta),
            Self::Feature(_) | Self::Empty => 0,
        }
    }

    #[must_use]
    pub fn get_y_size(&self) -> Option<i32> {
        match self {
            Self::Single {
                template,
                resolved_template,
                ..
            } => resolved_template.as_ref().map_or_else(
                || {
                    pumpkin_data::structure_metadata::StaticStructureMetadataList::get(template)
                        .map_or_else(
                            || {
                                crate::generation::structure::template::get_template(template)
                                    .map(|t| t.size.y)
                            },
                            |meta| Some(meta.size[1]),
                        )
                },
                |t| Some(t.size.y),
            ),
            Self::List(elements) => elements.iter().filter_map(Self::get_y_size).max(),
            Self::Feature(_) => Some(1),
            Self::Empty => None,
        }
    }

    #[must_use]
    pub fn get_bounding_box(&self, offset: BlockPos, rotation: pumpkin_data::Rotation) -> BlockBox {
        match self {
            Self::Single {
                template,
                resolved_template,
                ..
            } => {
                let size = resolved_template.as_ref().map_or_else(
                    || {
                        pumpkin_data::structure_metadata::StaticStructureMetadataList::get(template)
                            .map_or_else(
                                || {
                                    crate::generation::structure::template::get_template(template)
                                        .map(|t| t.size)
                                },
                                |meta| Some(Vector3::new(meta.size[0], meta.size[1], meta.size[2])),
                            )
                    },
                    |t| Some(t.size),
                );

                size.map_or_else(
                    || {
                        BlockBox::new(
                            offset.0.x, offset.0.y, offset.0.z, offset.0.x, offset.0.y, offset.0.z,
                        )
                    },
                    |s| super::jigsaw_placement::rotated_box(offset, s, rotation),
                )
            }
            Self::List(elements) => {
                let mut bbox: Option<BlockBox> = None;
                for element in elements {
                    if element.is_empty() {
                        continue;
                    }
                    let b = element.get_bounding_box(offset, rotation);
                    if let Some(existing) = &mut bbox {
                        existing.encompass(&b);
                    } else {
                        bbox = Some(b);
                    }
                }
                bbox.unwrap_or_else(|| {
                    BlockBox::new(
                        offset.0.x, offset.0.y, offset.0.z, offset.0.x, offset.0.y, offset.0.z,
                    )
                })
            }
            Self::Feature(_) | Self::Empty => BlockBox::new(
                offset.0.x, offset.0.y, offset.0.z, offset.0.x, offset.0.y, offset.0.z,
            ),
        }
    }

    #[must_use]
    pub fn get_shuffled_jigsaw_blocks(
        &self,
        offset: BlockPos,
        rotation: pumpkin_data::Rotation,
        random: &mut pumpkin_util::random::RandomGenerator,
    ) -> Vec<JigsawBlock> {
        match self {
            Self::Single {
                template,
                resolved_template,
                ..
            } => {
                let mut jigsaws: Vec<JigsawBlock> = if let Some(meta) =
                    pumpkin_data::structure_metadata::StaticStructureMetadataList::get(template)
                {
                    meta.jigsaws.iter().map(JigsawBlock::from).collect()
                } else if let Some(template) = resolved_template {
                    template.jigsaw_blocks().to_vec()
                } else if let Some(template) =
                    crate::generation::structure::template::get_template(template)
                {
                    template.jigsaw_blocks().to_vec()
                } else {
                    return Vec::new();
                };

                for i in (1..jigsaws.len()).rev() {
                    let j = random.next_bounded_i32(i as i32 + 1) as usize;
                    jigsaws.swap(i, j);
                }
                jigsaws.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
                for jigsaw in &mut jigsaws {
                    let rotated_pos = super::jigsaw_placement::rotate_pos(jigsaw.pos.0, rotation);
                    jigsaw.pos = offset.add(rotated_pos.x, rotated_pos.y, rotated_pos.z);
                    jigsaw.facing =
                        super::jigsaw_placement::rotate_direction(jigsaw.facing, rotation);
                    jigsaw.up = super::jigsaw_placement::rotate_direction(jigsaw.up, rotation);
                }
                jigsaws
            }
            Self::List(elements) => elements.first().map_or_else(Vec::new, |e| {
                e.get_shuffled_jigsaw_blocks(offset, rotation, random)
            }),
            Self::Feature(_) => vec![JigsawBlock {
                pos: offset,
                name: "minecraft:bottom".to_string(),
                target: "minecraft:empty".to_string(),
                pool: "minecraft:empty".to_string(),
                final_state: "minecraft:air".to_string(),
                joint: JigsawJointType::Rollable,
                facing: pumpkin_util::BlockDirection::Down,
                up: pumpkin_util::BlockDirection::South,
                selection_priority: 0,
                placement_priority: 0,
            }],
            Self::Empty => Vec::new(),
        }
    }
}

impl PoolElement {
    #[must_use]
    pub const fn feature(&self) -> Option<pumpkin_data::placed_feature::PlacedFeature> {
        match self.kind {
            PoolElementKind::Feature(feature) => Some(feature),
            _ => None,
        }
    }

    #[must_use]
    pub fn get_ground_level_delta(&self) -> i32 {
        self.kind.get_ground_level_delta()
    }

    #[must_use]
    pub fn get_y_size(&self) -> Option<i32> {
        self.kind.get_y_size()
    }

    #[must_use]
    pub fn get_bounding_box(&self, offset: BlockPos, rotation: pumpkin_data::Rotation) -> BlockBox {
        self.kind.get_bounding_box(offset, rotation)
    }

    #[must_use]
    pub fn get_shuffled_jigsaw_blocks(
        &self,
        offset: BlockPos,
        rotation: pumpkin_data::Rotation,
        random: &mut pumpkin_util::random::RandomGenerator,
    ) -> Vec<JigsawBlock> {
        self.kind
            .get_shuffled_jigsaw_blocks(offset, rotation, random)
    }
}

impl TemplatePool {
    #[must_use]
    pub fn get_max_size(&self) -> i32 {
        self.elements
            .iter()
            .filter_map(|element| element.get_y_size())
            .max()
            .unwrap_or(0)
    }
    pub fn get_random_element(
        &self,
        random: &mut pumpkin_util::random::RandomGenerator,
    ) -> Arc<PoolElement> {
        let total_weight: u32 = self.elements.iter().map(|e| e.weight).sum();
        if total_weight == 0 {
            return Arc::clone(&self.elements[0]);
        }
        let mut r = random.next_bounded_i32(total_weight as i32) as u32;
        for element in &self.elements {
            if r < element.weight {
                return Arc::clone(element);
            }
            r -= element.weight;
        }
        Arc::clone(&self.elements[0])
    }

    #[must_use]
    pub fn from_static(
        static_pool: &'static pumpkin_data::template_pool::StaticTemplatePool,
    ) -> Self {
        let elements = static_pool
            .elements
            .iter()
            .map(|e| {
                let projection = match e.projection {
                    pumpkin_data::template_pool::TemplatePoolProjection::Rigid => {
                        JigsawProjection::Rigid
                    }
                    pumpkin_data::template_pool::TemplatePoolProjection::TerrainMatching => {
                        JigsawProjection::TerrainMatching
                    }
                };
                let kind = Self::convert_static_kind(&e.kind);
                Arc::new(PoolElement {
                    weight: e.weight,
                    projection,
                    kind,
                })
            })
            .collect();

        Self {
            id: static_pool.id.to_string(),
            fallback: static_pool.fallback.to_string(),
            elements,
        }
    }

    fn convert_static_kind(
        kind: &pumpkin_data::template_pool::StaticPoolElementKind,
    ) -> PoolElementKind {
        match kind {
            pumpkin_data::template_pool::StaticPoolElementKind::Empty => PoolElementKind::Empty,
            pumpkin_data::template_pool::StaticPoolElementKind::Single {
                location,
                processors,
                legacy,
            } => {
                let proc_ref = if processors.is_empty() {
                    ProcessorListRef::Empty
                } else {
                    ProcessorListRef::Named((*processors).to_string())
                };
                PoolElementKind::Single {
                    template: (*location).to_string(),
                    resolved_template: None,
                    processors: proc_ref,
                    legacy: *legacy,
                }
            }
            pumpkin_data::template_pool::StaticPoolElementKind::List(elements) => {
                let list = elements.iter().map(Self::convert_static_kind).collect();
                PoolElementKind::List(list)
            }
            pumpkin_data::template_pool::StaticPoolElementKind::Feature(feature) => {
                let feat_name = feature.strip_prefix("minecraft:").unwrap_or(feature);
                pumpkin_data::placed_feature::PlacedFeature::from_name(feat_name)
                    .map_or(PoolElementKind::Empty, PoolElementKind::Feature)
            }
        }
    }

    /// Gets a cached pool from the embedded static definitions.
    #[must_use]
    pub fn discover(id: &str) -> Option<Arc<Self>> {
        static CACHE: std::sync::LazyLock<dashmap::DashMap<String, Arc<TemplatePool>>> =
            std::sync::LazyLock::new(dashmap::DashMap::new);

        let canonical_id = if id.contains(':') {
            id.to_string()
        } else {
            format!("minecraft:{id}")
        };

        // Previously cached pools
        if let Some(pool) = CACHE.get(&canonical_id) {
            return Some(Arc::clone(&pool));
        }

        // Special empty pool
        if canonical_id == "minecraft:empty" || id == "empty" {
            let pool = Arc::new(Self {
                id: "minecraft:empty".to_string(),
                fallback: "minecraft:empty".to_string(),
                elements: Vec::new(),
            });
            CACHE.insert(canonical_id, Arc::clone(&pool));
            return Some(pool);
        }

        // Static template pools from pumpkin-data
        if let Some(static_pool) = pumpkin_data::template_pool::get_template_pool(&canonical_id) {
            let pool = Arc::new(Self::from_static(static_pool));
            CACHE.insert(canonical_id, Arc::clone(&pool));
            return Some(pool);
        }

        None
    }

    #[must_use]
    pub fn get_shuffled_elements(
        &self,
        random: &mut pumpkin_util::random::RandomGenerator,
    ) -> Vec<Arc<PoolElement>> {
        let mut elements = self
            .elements
            .iter()
            .flat_map(|element| std::iter::repeat_n(Arc::clone(element), element.weight as usize))
            .collect::<Vec<_>>();
        for index in (1..elements.len()).rev() {
            let other = random.next_bounded_i32(index as i32 + 1) as usize;
            elements.swap(index, other);
        }
        elements
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum JigsawJointType {
    Rollable,
    Aligned,
}

impl JigsawJointType {
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Rollable => "rollable",
            Self::Aligned => "aligned",
        }
    }

    #[allow(clippy::should_implement_trait)]
    #[must_use]
    pub fn from_str(s: &str) -> Self {
        match s {
            "aligned" => Self::Aligned,
            _ => Self::Rollable,
        }
    }
}

#[derive(Clone)]
pub struct JigsawBlock {
    pub pos: BlockPos,
    pub name: String,
    pub target: String,
    pub pool: String,
    pub final_state: String,
    pub joint: JigsawJointType,
    pub facing: pumpkin_util::BlockDirection,
    pub up: pumpkin_util::BlockDirection,
    pub selection_priority: i32,
    pub placement_priority: i32,
}

impl JigsawBlock {
    #[must_use]
    pub fn from_template_block(
        block: &crate::generation::structure::template::TemplateBlock,
        palette: &PaletteEntry,
    ) -> Option<Self> {
        use pumpkin_data::block_properties::{Orientation, Property};
        use pumpkin_util::BlockDirection;

        if palette.id.to_block_id() != pumpkin_data::BlockId::JIGSAW {
            return None;
        }

        let nbt = block.nbt.as_ref()?;

        // Resolve facing and up from orientation property
        let orientation = palette
            .id
            .properties()
            .find_map(|p| match p {
                Property::Orientation(o) => Some(o),
                _ => None,
            })
            .unwrap_or(Orientation::NorthUp);

        let (facing, up) = match orientation {
            Orientation::DownEast => (BlockDirection::Down, BlockDirection::East),
            Orientation::DownNorth => (BlockDirection::Down, BlockDirection::North),
            Orientation::DownSouth => (BlockDirection::Down, BlockDirection::South),
            Orientation::DownWest => (BlockDirection::Down, BlockDirection::West),
            Orientation::UpEast => (BlockDirection::Up, BlockDirection::East),
            Orientation::UpNorth => (BlockDirection::Up, BlockDirection::North),
            Orientation::UpSouth => (BlockDirection::Up, BlockDirection::South),
            Orientation::UpWest => (BlockDirection::Up, BlockDirection::West),
            Orientation::WestUp => (BlockDirection::West, BlockDirection::Up),
            Orientation::EastUp => (BlockDirection::East, BlockDirection::Up),
            Orientation::NorthUp => (BlockDirection::North, BlockDirection::Up),
            Orientation::SouthUp => (BlockDirection::South, BlockDirection::Up),
        };

        Some(Self {
            pos: BlockPos(block.pos),
            name: nbt.get_string("name").unwrap_or_default().to_string(),
            target: nbt.get_string("target").unwrap_or_default().to_string(),
            pool: nbt.get_string("pool").unwrap_or_default().to_string(),
            final_state: nbt
                .get_string("final_state")
                .unwrap_or_default()
                .to_string(),
            joint: JigsawJointType::from_str(nbt.get_string("joint").unwrap_or_default()),
            facing,
            up,
            selection_priority: nbt.get_int("selection_priority").unwrap_or(0),
            placement_priority: nbt.get_int("placement_priority").unwrap_or(0),
        })
    }

    #[must_use]
    pub fn can_attach(
        source: &Self,
        target_facing: pumpkin_util::BlockDirection,
        target_name: &str,
    ) -> bool {
        source.facing.opposite() == target_facing && source.target == target_name
    }
}

impl From<&pumpkin_data::structure_metadata::StaticJigsawBlock> for JigsawBlock {
    fn from(static_jigsaw: &pumpkin_data::structure_metadata::StaticJigsawBlock) -> Self {
        Self {
            pos: BlockPos(pumpkin_util::math::vector3::Vector3::new(
                static_jigsaw.pos[0],
                static_jigsaw.pos[1],
                static_jigsaw.pos[2],
            )),
            name: static_jigsaw.name.to_string(),
            target: static_jigsaw.target.to_string(),
            pool: static_jigsaw.pool.to_string(),
            final_state: static_jigsaw.final_state.to_string(),
            joint: match static_jigsaw.joint {
                pumpkin_data::structure_metadata::StaticJigsawJointType::Aligned => {
                    JigsawJointType::Aligned
                }
                pumpkin_data::structure_metadata::StaticJigsawJointType::Rollable => {
                    JigsawJointType::Rollable
                }
            },
            facing: static_jigsaw.facing,
            up: static_jigsaw.up,
            selection_priority: static_jigsaw.selection_priority,
            placement_priority: static_jigsaw.placement_priority,
        }
    }
}

#[derive(Clone)]
pub struct JigsawJunction {
    pub source_x: i32,
    pub source_ground_y: i32,
    pub source_z: i32,
    pub delta_y: i32,
    pub projection: JigsawProjection,
}

pub struct PoolElementStructurePiece {
    pub piece: crate::generation::structure::structures::StructurePiece,
    pub element: Arc<PoolElement>,
    pub pos: BlockPos,
    pub rotation: BlockRotation,
    pub mirror: BlockMirror,
    pub jigsaw_blocks: Vec<JigsawBlock>,
    pub junctions: Vec<JigsawJunction>,
    pub ground_level_delta: i32,
    pub liquid_settings: LiquidSettings,
    pub projection: JigsawProjection,
}

impl StructurePieceBase for PoolElementStructurePiece {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
    fn get_structure_piece(&self) -> &crate::generation::structure::structures::StructurePiece {
        &self.piece
    }

    fn get_structure_piece_mut(
        &mut self,
    ) -> &mut crate::generation::structure::structures::StructurePiece {
        &mut self.piece
    }

    fn place(
        &mut self,
        chunk: &mut crate::ProtoChunk,
        block_registry: &dyn crate::world::WorldPortalExt,
        random: &mut pumpkin_util::random::RandomGenerator,
        _seed: i64,
        chunk_box: &pumpkin_util::math::block_box::BlockBox,
    ) {
        let origin =
            pumpkin_util::math::vector3::Vector3::new(self.pos.0.x, self.pos.0.y, self.pos.0.z);

        self.element
            .for_each_template(|_name, processor_list, legacy, template| {
                let corner = self.rotation.rotate_offset(
                    template.size.x.saturating_sub(1),
                    template.size.z.saturating_sub(1),
                );
                let placement_origin = pumpkin_util::math::vector3::Vector3::new(
                    origin.x + corner.0.min(0),
                    origin.y,
                    origin.z + corner.1.min(0),
                );
                let processors = match processor_list {
                    ProcessorListRef::Named(name) => {
                        crate::generation::structure::template::processor::load_processor_list(name)
                    }
                    ProcessorListRef::Empty => Arc::from([]),
                };
                crate::generation::structure::template::place_template(
                    chunk,
                    &template,
                    placement_origin,
                    (0, 0),
                    self.rotation,
                    legacy,
                    self.liquid_settings == LiquidSettings::ApplyWaterlog,
                    processors.as_ref(),
                    Some(chunk_box),
                );
                crate::generation::structure::template::place_template_entities(
                    chunk,
                    &template,
                    placement_origin,
                    self.rotation,
                    chunk_box,
                );
            });

        if let Some(feature) = self.element.feature()
            && let Some(placed_feature) =
                crate::generation::feature::placed_features::PLACED_FEATURES.get(&feature)
        {
            placed_feature.generate_in_proto_chunk(
                chunk,
                block_registry,
                feature,
                random,
                self.pos,
            );
        }
    }
}

pub fn place_pool_element_templates(
    piece: &PoolElementStructurePiece,
    placer: &mut impl BlockPlacer,
    chunk_box: Option<&BlockBox>,
    keep_jigsaws: bool,
) {
    let origin = Vector3::new(piece.pos.0.x, piece.pos.0.y, piece.pos.0.z);

    piece
        .element
        .for_each_template(|_name, processor_list, legacy, template| {
            let corner = piece.rotation.rotate_offset(
                template.size.x.saturating_sub(1),
                template.size.z.saturating_sub(1),
            );
            let placement_origin = Vector3::new(
                origin.x + corner.0.min(0),
                origin.y,
                origin.z + corner.1.min(0),
            );
            let processors = match processor_list {
                ProcessorListRef::Named(name) => {
                    crate::generation::structure::template::processor::load_processor_list(name)
                }
                ProcessorListRef::Empty => Arc::from([]),
            };
            crate::generation::structure::template::place_template_with_options(
                placer,
                &template,
                placement_origin,
                (0, 0),
                piece.rotation,
                legacy,
                piece.liquid_settings == LiquidSettings::ApplyWaterlog,
                processors.as_ref(),
                chunk_box,
                keep_jigsaws,
            );
        });
}

impl PoolElementStructurePiece {
    pub fn add_junction(&mut self, junction: JigsawJunction) {
        self.junctions.push(junction);
    }
}

pub struct JigsawGenerator {
    pub start_pool: String,
    pub size: i32,
    pub start_jigsaw_name: Option<String>,
    pub use_expansion_hack: bool,
    pub pool_aliases: &'static [pumpkin_data::structures::PoolAliasBinding],
}

impl JigsawGenerator {
    #[must_use]
    pub fn new(start_pool: &str, size: i32) -> Self {
        Self {
            start_pool: start_pool.to_string(),
            size,
            start_jigsaw_name: None,
            use_expansion_hack: false,
            pool_aliases: &[],
        }
    }

    #[must_use]
    pub fn new_with_pool(start_pool: &str, size: i32) -> Self {
        Self {
            start_pool: start_pool.to_string(),
            size,
            start_jigsaw_name: None,
            use_expansion_hack: false,
            pool_aliases: &[],
        }
    }

    #[must_use]
    pub fn with_start_jigsaw(mut self, name: &str) -> Self {
        self.start_jigsaw_name = Some(name.to_string());
        self
    }

    #[must_use]
    pub const fn with_expansion_hack(mut self, use_hack: bool) -> Self {
        self.use_expansion_hack = use_hack;
        self
    }

    #[must_use]
    pub const fn with_pool_aliases(
        mut self,
        aliases: &'static [pumpkin_data::structures::PoolAliasBinding],
    ) -> Self {
        self.pool_aliases = aliases;
        self
    }
}

impl StructureGenerator for JigsawGenerator {
    fn get_structure_position(
        &self,
        context: StructureGeneratorContext<'_>,
    ) -> Option<StructurePosition> {
        let mut context = context;
        let structure = context
            .structure_key
            .map(|key| pumpkin_data::structures::Structure::get(&key));

        let start_y = if let Some(s) = structure {
            s.start_height.map_or(context.sea_level, |hp| {
                hp.get(&mut context.random, context.min_y as i8, context.height)
            })
        } else {
            context.sea_level
        };

        let start_pos = BlockPos::new(
            crate::generation::positions::chunk_pos::start_block_x(context.chunk_x),
            start_y,
            crate::generation::positions::chunk_pos::start_block_z(context.chunk_z),
        );

        let project_start_to_heightmap = structure
            .and_then(|s| s.project_start_to_heightmap)
            .is_some();

        let max_distance = structure
            .and_then(|s| s.max_distance_from_center)
            .unwrap_or(80); // Vanilla default is 80

        let liquid_settings =
            structure
                .and_then(|s| s.liquid_settings)
                .map_or(LiquidSettings::ApplyWaterlog, |ls| match ls {
                    "ignore_waterlogging" => LiquidSettings::IgnoreWaterlogDone,
                    _ => LiquidSettings::ApplyWaterlog,
                });

        let dimension_padding =
            structure
                .and_then(|s| s.dimension_padding)
                .map_or(DimensionPadding::ZERO, |dp| DimensionPadding {
                    top: dp,
                    bottom: dp,
                });

        let use_expansion_hack = self.use_expansion_hack
            || structure
                .and_then(|s| s.use_expansion_hack)
                .unwrap_or(false);

        let start_jigsaw = self
            .start_jigsaw_name
            .as_deref()
            .or_else(|| structure.and_then(|s| s.start_jigsaw_name));

        let pool_aliases = if !self.pool_aliases.is_empty() {
            self.pool_aliases
        } else if let Some(s) = structure {
            s.pool_aliases
        } else {
            &[]
        };

        let mut alias_random =
            pumpkin_util::random::legacy_rand::LegacyRand::from_seed(context.seed as u64)
                .next_splitter()
                .split_pos(start_pos.0.x, start_pos.0.y, start_pos.0.z);
        let pool_alias_lookup = PoolAliasLookup::from_bindings(pool_aliases, &mut alias_random);

        let start_pool = if self.start_pool.is_empty() {
            structure
                .and_then(|s| s.start_pool)
                .unwrap_or(&self.start_pool)
        } else {
            &self.start_pool
        };

        JigsawPlacement::add_pieces(
            &mut context,
            start_pool,
            start_jigsaw,
            self.size,
            start_pos,
            use_expansion_hack,
            project_start_to_heightmap,
            &MaxDistance::new(max_distance),
            dimension_padding,
            liquid_settings,
            &pool_alias_lookup,
        )
    }
}
