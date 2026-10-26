use pumpkin_data::{
    BlockState,
    material_rule::{ConditionMaterialRule, MaterialCondition, MaterialRule, SequenceMaterialRule},
};

use super::MaterialRuleContext;
use crate::{
    ProtoChunk,
    generation::{
        noise::router::surface_height_sampler::SurfaceHeightEstimateSampler,
        surface::test_condition,
    },
};

/// A world's rule tree flattened once. Failed conditions jump past their
/// complete subtree; a selected block still returns immediately from the tree.
pub struct PreparedMaterialRule {
    instructions: Box<[Instruction]>,
}

enum Instruction {
    Test(
        &'static pumpkin_data::material_rule::MaterialCondition,
        usize,
    ),
    Biome([u64; 4], bool, usize),
    ColumnTest(&'static MaterialCondition, usize, usize),
    Block(&'static BlockState),
    Badlands,
    OreVein(&'static pumpkin_data::material_rule::OreVeinMaterialRule),
}

impl PreparedMaterialRule {
    pub fn new(rule: &'static MaterialRule) -> Self {
        fn emit(rule: &'static MaterialRule, instructions: &mut Vec<Instruction>) {
            match rule {
                MaterialRule::Block(block) => {
                    instructions.push(Instruction::Block(block.result_state))
                }
                MaterialRule::Badlands(_) => instructions.push(Instruction::Badlands),
                MaterialRule::OreVein(vein) => instructions.push(Instruction::OreVein(vein)),
                MaterialRule::Sequence(sequence) => {
                    for rule in sequence.sequence {
                        emit(rule, instructions);
                    }
                }
                MaterialRule::Condition(condition) => {
                    let index = instructions.len();
                    instructions.push(Instruction::Test(&condition.if_true, 0));
                    emit(condition.then_run, instructions);
                    let end = instructions.len();
                    let mut test = &condition.if_true;
                    let mut inverted = false;
                    while let MaterialCondition::Not(not) = test {
                        test = not.invert;
                        inverted = !inverted;
                    }
                    instructions[index] = match test {
                        MaterialCondition::Biome(biome) => {
                            let mut mask = [0; 4];
                            for biome in biome.biome_is {
                                let id = usize::from(biome.id);
                                mask[id / 64] |= 1u64 << (id % 64);
                            }
                            Instruction::Biome(mask, inverted, end)
                        }
                        MaterialCondition::NoiseThreshold(_) | MaterialCondition::Hole(_) => {
                            Instruction::ColumnTest(&condition.if_true, index, end)
                        }
                        _ => Instruction::Test(&condition.if_true, end),
                    };
                }
            }
        }
        let mut instructions = Vec::new();
        emit(rule, &mut instructions);
        Self {
            instructions: instructions.into_boxed_slice(),
        }
    }

    pub fn apply(
        &self,
        chunk: &mut ProtoChunk,
        context: &mut MaterialRuleContext,
        height_sampler: &mut SurfaceHeightEstimateSampler,
    ) -> Option<&'static BlockState> {
        if context.column_conditions.len() < self.instructions.len() {
            context
                .column_conditions
                .resize(self.instructions.len(), None);
        }
        let mut pc = 0;
        while let Some(instruction) = self.instructions.get(pc) {
            match instruction {
                Instruction::Biome(mask, inverted, fail) => {
                    context.resolve_biome(chunk);
                    let id = usize::from(context.biome.id);
                    if ((mask[id / 64] >> (id % 64)) & 1 != 0) == *inverted {
                        pc = *fail;
                        continue;
                    }
                }
                Instruction::ColumnTest(condition, index, fail) => {
                    let passes = if let Some(value) = context.column_conditions[*index] {
                        value
                    } else {
                        let value = test_condition(condition, chunk, context, height_sampler);
                        context.column_conditions[*index] = Some(value);
                        value
                    };
                    if !passes {
                        pc = *fail;
                        continue;
                    }
                }
                Instruction::Test(condition, fail) => {
                    if !test_condition(condition, chunk, context, height_sampler) {
                        pc = *fail;
                        continue;
                    }
                }
                Instruction::Block(state) => return Some(state),
                Instruction::Badlands => return Some(BadLandsMaterialRule::try_apply(context)),
                Instruction::OreVein(vein) => {
                    if let Some(state) = context.try_apply_ore_vein(vein) {
                        return Some(state);
                    }
                }
            }
            pc += 1;
        }
        None
    }
}

pub fn try_apply_material_rule(
    rule: &MaterialRule,
    chunk: &mut ProtoChunk,
    context: &mut MaterialRuleContext,
    surface_height_estimate_sampler: &mut SurfaceHeightEstimateSampler,
) -> Option<&'static BlockState> {
    match rule {
        MaterialRule::Badlands(_badlands) => Some(BadLandsMaterialRule::try_apply(context)),
        MaterialRule::Block(block) => Some(BlockMaterialRule::try_apply(block.result_state)),
        MaterialRule::Sequence(sequence) => {
            try_apply_sequence(sequence, chunk, context, surface_height_estimate_sampler)
        }
        MaterialRule::Condition(condition) => {
            try_apply_condition(condition, chunk, context, surface_height_estimate_sampler)
        }
        MaterialRule::OreVein(ore_vein) => context.try_apply_ore_vein(ore_vein),
    }
}

pub struct BadLandsMaterialRule;

impl BadLandsMaterialRule {
    pub fn try_apply(context: &mut MaterialRuleContext) -> &'static BlockState {
        context.terrain_builder.get_terracotta_block(
            context.block_pos_x,
            context.block_pos_y,
            context.block_pos_z,
        )
    }
}

pub struct BlockMaterialRule;

impl BlockMaterialRule {
    pub const fn try_apply(state: &'static BlockState) -> &'static BlockState {
        state
    }
}

pub fn try_apply_sequence(
    rule: &SequenceMaterialRule,
    chunk: &mut ProtoChunk,
    context: &mut MaterialRuleContext,
    surface_height_estimate_sampler: &mut SurfaceHeightEstimateSampler,
) -> Option<&'static BlockState> {
    for seq in rule.sequence {
        if let Some(state) =
            try_apply_material_rule(seq, chunk, context, surface_height_estimate_sampler)
        {
            return Some(state);
        }
    }
    None
}

pub fn try_apply_condition(
    rule: &ConditionMaterialRule,
    chunk: &mut ProtoChunk,
    context: &mut MaterialRuleContext,
    surface_height_estimate_sampler: &mut SurfaceHeightEstimateSampler,
) -> Option<&'static BlockState> {
    if test_condition(
        &rule.if_true,
        chunk,
        context,
        surface_height_estimate_sampler,
    ) {
        return try_apply_material_rule(
            rule.then_run,
            chunk,
            context,
            surface_height_estimate_sampler,
        );
    }
    None
}
