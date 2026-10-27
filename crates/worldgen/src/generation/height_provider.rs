use std::num::NonZero;

use pumpkin_util::{
    random::{RandomGenerator, RandomImpl},
    y_offset::YOffset,
};
use tracing::warn;

pub enum HeightProvider {
    Uniform(UniformHeightProvider),
    Trapezoid(TrapezoidHeightProvider),
    VeryBiasedToBottom(VeryBiasedToBottomHeightProvider),
}

impl HeightProvider {
    pub fn get(&self, random: &mut RandomGenerator, min_y: i8, height: u16) -> i32 {
        match self {
            Self::Uniform(provider) => provider.get(random, min_y, height),
            Self::Trapezoid(provider) => provider.get(random, min_y, height),
            Self::VeryBiasedToBottom(provider) => provider.get(random, min_y, height),
        }
    }
}

pub struct VeryBiasedToBottomHeightProvider {
    pub min_inclusive: YOffset,
    pub max_inclusive: YOffset,
    pub inner: Option<NonZero<u32>>,
}

impl VeryBiasedToBottomHeightProvider {
    pub fn get(&self, random: &mut RandomGenerator, min_y: i8, height: u16) -> i32 {
        let min = self.min_inclusive.get_y(min_y as i16, height);
        let max = self.max_inclusive.get_y(min_y as i16, height);
        let inner = self.inner.map_or(1, std::num::NonZero::get) as i32;

        if max - min - inner < 0 {
            warn!("Empty height range: min={min}, max={max}, inner={inner}");
            return min;
        }

        let upper_inclusive = random.next_inbetween_i32(min + inner, max);
        let biased_upper_inclusive = random.next_inbetween_i32(min, upper_inclusive - 1);
        random.next_inbetween_i32(min, biased_upper_inclusive - 1 + inner)
    }
}

pub struct UniformHeightProvider {
    pub min_inclusive: YOffset,
    pub max_inclusive: YOffset,
}

impl UniformHeightProvider {
    pub fn get(&self, random: &mut RandomGenerator, min_y: i8, height: u16) -> i32 {
        let min = self.min_inclusive.get_y(min_y as i16, height);
        let max = self.max_inclusive.get_y(min_y as i16, height);

        if min > max {
            // TODO: investigate why min > max occurs (e.g. min=136, max=127)
            return min;
        }

        random.next_inbetween_i32(min, max)
    }
}

pub struct TrapezoidHeightProvider {
    pub min_inclusive: YOffset,
    pub max_inclusive: YOffset,
    pub plateau: Option<i32>,
}

impl TrapezoidHeightProvider {
    pub fn get(&self, random: &mut RandomGenerator, min_y: i8, height: u16) -> i32 {
        let plateau = self.plateau.unwrap_or(0);
        let i = self.min_inclusive.get_y(min_y as i16, height);
        let j = self.max_inclusive.get_y(min_y as i16, height);

        if i > j {
            warn!("Empty height range");
            return i;
        }

        let k = j - i;
        if plateau >= k {
            return random.next_inbetween_i32(i, j);
        }

        let l = (k - plateau) / 2;
        let m = k - l;
        i + random.next_inbetween_i32(0, m) + random.next_inbetween_i32(0, l)
    }
}
