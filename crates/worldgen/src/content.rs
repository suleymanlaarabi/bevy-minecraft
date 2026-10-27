//! Minecraft is an import provider for the shared content catalog.
use hollow_core::{BlockId, ShapeId, StateId, content::*};
use pumpkin_data::{BlockDirection, BlockState, BlockStateId};
use std::{collections::BTreeMap, sync::OnceLock};

// The native generator exposes value-only cells to worker threads. Bind its adapter
// once at startup; reusable engine catalogs themselves remain ordinary owned values.
static CONTENT: OnceLock<ContentCatalog> = OnceLock::new();
pub fn install(catalog: ContentCatalog) {
    assert!(
        CONTENT.set(catalog).is_ok(),
        "Content adapter already initialized"
    );
}
pub fn catalog() -> &'static ContentCatalog {
    CONTENT.get().expect("Content not finalized")
}
pub fn try_catalog() -> Option<&'static ContentCatalog> {
    CONTENT.get()
}
pub fn metadata(state: &BlockState) -> StateDefinition {
    let mut solid_faces = 0;
    let mut center_faces = 0;
    for side in BlockDirection::all() {
        let i = side.to_index();
        solid_faces |= u8::from(state.is_side_solid(side)) << i;
        center_faces |= u8::from(state.is_center_solid(side)) << i;
    }
    StateDefinition {
        block: BlockId(u32::from(state.id.to_block_id().as_u16())),
        fluid: FluidState::default(),
        containers: ContainerRange::default(),
        washable: state
            .id
            .to_block_id()
            .has_tag(pumpkin_data::tag::Block::MINECRAFT_WASHED_AWAY_BY_FLUIDS),
        collision: ShapeId(0),
        selection: ShapeId(0),
        opacity: state.opacity,
        luminance: state.luminance,
        hardness: state.hardness,
        air: state.is_air(),
        replaceable: state.replaceable(),
        solid: state.is_solid(),
        full_cube: state.is_full_cube(),
        full_collision: state.collision_shapes.len() == 1 && {
            let bounds = crate::state::block_box(
                pumpkin_data::block_properties::COLLISION_SHAPES
                    [state.collision_shapes[0] as usize],
            );
            bounds.min == bevy_math::Vec3::ZERO && bounds.max == bevy_math::Vec3::ONE
        },
        occludes: state.can_occlude(),
        solid_render: state.is_solid_render(),
        tool_required: state.tool_required(),
        solid_faces,
        center_faces,
    }
}
pub fn import() -> ContentBuilder {
    let mut builder = ContentBuilder::new();
    // Native shape lists repeat across thousands of states. Deduplicate only at
    // import, retaining one box allocation per distinct shape, not per state.
    let mut shapes = BTreeMap::<&'static [u16], ShapeId>::from([(&[] as &[u16], ShapeId(0))]);
    for raw in 0..BlockStateId::COUNT {
        let state = BlockStateId::new(raw).unwrap().to_state();
        let mut shape = |ids: &'static [u16]| {
            *shapes.entry(ids).or_insert_with(|| {
                builder.shape(
                    ids.iter()
                        .map(|&id| {
                            crate::state::block_box(
                                pumpkin_data::block_properties::COLLISION_SHAPES[id as usize],
                            )
                        })
                        .collect::<Box<[_]>>(),
                )
            })
        };
        let collision = shape(state.collision_shapes);
        let selection = shape(state.outline_shapes);
        builder.states.push(StateDefinition {
            collision,
            selection,
            ..metadata(state)
        });
    }
    for raw in 0..pumpkin_data::BlockId::COUNT {
        let block = pumpkin_data::BlockId::new_or_air(raw).to_block();
        let start = u32::from(block.states[0].id.as_u16());
        let mut properties = Vec::new();
        for property in block.states[0].id.properties() {
            let key = property.key();
            let mut values = Vec::new();
            for state in block.states {
                let value = state.id.property(key).unwrap().value_str();
                if !values.iter().any(|v: &String| v == value) {
                    values.push(value.to_owned());
                }
            }
            properties.push(PropertyDefinition {
                name: key.to_str().to_owned(),
                values: values.into(),
                stride: 0,
            });
        }
        let mut stride = 1;
        for property in properties.iter_mut().rev() {
            property.stride = stride;
            stride *= property.values.len() as u32;
        }
        assert_eq!(
            stride as usize,
            block.states.len(),
            "Native property layout"
        );
        builder.import_block(BlockDefinition {
            name: format!("minecraft:{}", block.name),
            states: start..start + block.states.len() as u32,
            default_state: StateId(u32::from(block.default_state.id.as_u16())),
            properties: properties.into(),
            resources: Box::new([]),
            slipperiness: block.slipperiness,
            velocity_multiplier: block.velocity_multiplier,
            jump_velocity_multiplier: block.jump_velocity_multiplier,
        });
    }
    builder
}
