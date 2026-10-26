//! Nibble-light working buffers used during generation.
#[derive(Debug, Clone)]
pub enum LightContainer {
    Empty(u8),
    Full(Box<[u8]>),
}

impl LightContainer {
    pub const DIM: usize = 16;
    pub const ARRAY_SIZE: usize = Self::DIM * Self::DIM * Self::DIM / 2;

    #[must_use]
    pub fn new_empty(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        Self::Empty(default)
    }

    #[must_use]
    pub fn new(data: Box<[u8]>) -> Self {
        assert!(
            data.len() == Self::ARRAY_SIZE,
            "Data length must be {}",
            Self::ARRAY_SIZE
        );
        Self::Full(data)
    }

    #[must_use]
    pub fn new_filled(default: u8) -> Self {
        assert!(default <= 15, "Default value must be between 0 and 15");
        let value = default << 4 | default;
        Self::Full([value; Self::ARRAY_SIZE].into())
    }

    #[must_use]
    pub const fn is_empty(&self) -> bool {
        matches!(self, Self::Empty(_))
    }

    #[inline]
    const fn index(x: usize, y: usize, z: usize) -> usize {
        y * 16 * 16 + z * 16 + x
    }

    #[inline]
    #[must_use]
    pub fn get(&self, x: usize, y: usize, z: usize) -> u8 {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                (data[index >> 1] >> (4 * (index & 1))) & 0x0F
            }
            Self::Empty(default) => *default,
        }
    }

    #[inline]
    pub fn set(&mut self, x: usize, y: usize, z: usize, value: u8) {
        match self {
            Self::Full(data) => {
                let index = Self::index(x, y, z);
                let shift = 4 * (index & 1);
                let mask = 0x0F << shift;
                data[index >> 1] = (data[index >> 1] & !mask) | (value << shift);
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set(x, y, z, value);
                }
            }
        }
    }

    #[inline]
    pub fn set_column_y_range(
        &mut self,
        x: usize,
        z: usize,
        y_start: usize,
        y_end: usize,
        value: u8,
    ) {
        if y_start >= y_end {
            return;
        }
        match self {
            Self::Full(data) => {
                let shift = 4 * (x & 1);
                let mask = 0x0F << shift;
                let val = (value & 0x0F) << shift;
                let mut byte_idx = (y_start * 256 + z * 16 + x) >> 1;
                for _ in y_start..y_end {
                    data[byte_idx] = (data[byte_idx] & !mask) | val;
                    byte_idx += 128;
                }
            }
            Self::Empty(default) => {
                if value != *default {
                    *self = Self::new_filled(*default);
                    self.set_column_y_range(x, z, y_start, y_end, value);
                }
            }
        }
    }

    #[inline]
    pub fn fill(&mut self, value: u8) {
        *self = Self::new_filled(value);
    }
}

impl Default for LightContainer {
    fn default() -> Self {
        Self::new_empty(15)
    }
}
