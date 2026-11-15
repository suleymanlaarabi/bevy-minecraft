//! Uniform and directly indexed section layers with copy-on-write snapshots.

use super::{VoxelCell, coordinates::SECTION_VOLUME};
use std::sync::Arc;

/// Lossless numeric storage; compact IDs are reconstructed without registry lookups.
pub trait SectionValue: Copy + PartialEq {
    const NARROW: bool;
    fn bits(self) -> u32;
    fn from_bits(bits: u32) -> Self;
}
impl<T: VoxelCell> SectionValue for T {
    const NARROW: bool = size_of::<T>() > size_of::<u16>();
    #[inline]
    fn bits(self) -> u32 {
        self.id()
    }
    #[inline]
    fn from_bits(bits: u32) -> Self {
        Self::from_storage_id(bits)
    }
}
impl SectionValue for u8 {
    const NARROW: bool = false;
    #[inline]
    fn bits(self) -> u32 {
        u32::from(self)
    }
    #[inline]
    fn from_bits(bits: u32) -> Self {
        bits as u8
    }
}

#[derive(Clone, Debug)]
pub enum Section<T> {
    Uniform(T),
    /// All cells share their high 16 ID bits. No palette indirection on reads.
    Narrow {
        prefix: u32,
        cells: Arc<[u16; SECTION_VOLUME]>,
    },
    Dense(Arc<[T; SECTION_VOLUME]>),
}

impl<T: SectionValue> Section<T> {
    pub fn from_cells(cells: &[T]) -> Self {
        assert_eq!(cells.len(), SECTION_VOLUME, "one section of cells");
        let first = cells[0];
        if cells.iter().all(|&cell| cell == first) {
            return Self::Uniform(first);
        }
        let prefix = first.bits() & 0xffff_0000;
        if T::NARROW && cells.iter().all(|cell| cell.bits() & 0xffff_0000 == prefix) {
            Self::Narrow {
                prefix,
                cells: Arc::new(std::array::from_fn(|i| cells[i].bits() as u16)),
            }
        } else {
            Self::Dense(Arc::new(cells.try_into().expect("one section of cells")))
        }
    }

    #[inline]
    pub fn get(&self, index: usize) -> T {
        let index = index % SECTION_VOLUME;
        match self {
            Self::Uniform(value) => *value,
            Self::Narrow { prefix, cells } => T::from_bits(*prefix | u32::from(cells[index])),
            Self::Dense(cells) => cells[index],
        }
    }

    #[inline]
    pub fn uniform(&self) -> Option<T> {
        if let Self::Uniform(value) = self {
            Some(*value)
        } else {
            None
        }
    }

    #[inline]
    pub fn copy_cells(&self, start: usize, out: &mut [T]) {
        match self {
            Self::Uniform(value) => out.fill(*value),
            Self::Narrow { prefix, cells } => {
                let cells = &cells[start..start + out.len()];
                for (out, &cell) in out.iter_mut().zip(cells) {
                    *out = T::from_bits(*prefix | u32::from(cell));
                }
            }
            Self::Dense(cells) => out.copy_from_slice(&cells[start..start + out.len()]),
        }
    }

    pub fn any(&self, predicate: impl Fn(T) -> bool) -> bool {
        match self {
            Self::Uniform(value) => predicate(*value),
            Self::Narrow { prefix, cells } => cells
                .iter()
                .any(|&cell| predicate(T::from_bits(*prefix | u32::from(cell)))),
            Self::Dense(cells) => cells.iter().any(|&cell| predicate(cell)),
        }
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + match self {
                Self::Uniform(_) => 0,
                Self::Narrow { .. } => 16 + SECTION_VOLUME * size_of::<u16>(),
                Self::Dense(_) => 16 + SECTION_VOLUME * size_of::<T>(),
            }
    }

    fn set(&mut self, index: usize, value: T) {
        let index = index % SECTION_VOLUME;
        match self {
            Self::Uniform(fill) if T::NARROW && fill.bits() >> 16 == value.bits() >> 16 => {
                let prefix = fill.bits() & 0xffff_0000;
                let mut cells = Arc::new([fill.bits() as u16; SECTION_VOLUME]);
                Arc::get_mut(&mut cells).unwrap()[index] = value.bits() as u16;
                *self = Self::Narrow { prefix, cells };
            }
            Self::Uniform(fill) => {
                let mut cells = Arc::new([*fill; SECTION_VOLUME]);
                Arc::get_mut(&mut cells).unwrap()[index] = value;
                *self = Self::Dense(cells);
            }
            Self::Narrow { prefix, cells } if value.bits() & 0xffff_0000 == *prefix => {
                Arc::make_mut(cells)[index] = value.bits() as u16
            }
            Self::Narrow { prefix, cells } => {
                let mut wide = Arc::new(std::array::from_fn(|i| {
                    T::from_bits(*prefix | u32::from(cells[i]))
                }));
                Arc::get_mut(&mut wide).unwrap()[index] = value;
                *self = Self::Dense(wide);
            }
            Self::Dense(cells) => Arc::make_mut(cells)[index] = value,
        }
    }

    /// Identity comparison for incremental consumers; does not scan cell contents.
    pub fn shared(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (
                Self::Narrow {
                    prefix: a,
                    cells: ac,
                },
                Self::Narrow {
                    prefix: b,
                    cells: bc,
                },
            ) => a == b && Arc::ptr_eq(ac, bc),
            (Self::Dense(a), Self::Dense(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }

    fn same_cells(&self, other: &Self) -> bool {
        self.shared(other)
            || match (self, other) {
                (Self::Uniform(value), cells) | (cells, Self::Uniform(value)) => {
                    !cells.any(|cell| cell != *value)
                }
                (Self::Dense(a), Self::Dense(b)) => a == b,
                (
                    Self::Narrow {
                        prefix: a,
                        cells: ac,
                    },
                    Self::Narrow {
                        prefix: b,
                        cells: bc,
                    },
                ) => a == b && ac == bc,
                _ => (0..SECTION_VOLUME).all(|i| self.get(i) == other.get(i)),
            }
    }

    fn compacted(&self) -> Option<Self> {
        match self {
            Self::Uniform(_) => None,
            Self::Narrow { prefix, cells } => {
                let first = cells[0];
                cells
                    .iter()
                    .all(|&cell| cell == first)
                    .then(|| Self::Uniform(T::from_bits(*prefix | u32::from(first))))
            }
            Self::Dense(cells) => {
                let first = cells[0];
                let prefix = first.bits() & 0xffff_0000;
                if cells.iter().all(|&cell| cell == first) {
                    Some(Self::Uniform(first))
                } else if T::NARROW && cells.iter().all(|cell| cell.bits() & 0xffff_0000 == prefix)
                {
                    Some(Self::Narrow {
                        prefix,
                        cells: Arc::new(std::array::from_fn(|i| cells[i].bits() as u16)),
                    })
                } else {
                    None
                }
            }
        }
    }
}

/// Cloning is O(1); edits copy only their section while snapshots still read it.
#[derive(Clone, Debug)]
pub struct Layer<T>(Arc<[Section<T>]>);

impl<T: SectionValue> Layer<T> {
    pub fn section_count(&self) -> usize {
        self.0.len()
    }
    pub fn from_sections(sections: impl IntoIterator<Item = Section<T>>) -> Self {
        Self(sections.into_iter().collect())
    }
    pub fn filled(value: T, section_count: usize) -> Self {
        Self::from_sections((0..section_count).map(|_| Section::Uniform(value)))
    }
    pub fn from_cells(cells: &[T]) -> Self {
        assert!(
            !cells.is_empty() && cells.len().is_multiple_of(SECTION_VOLUME),
            "Complete sections of cells"
        );
        Self::from_sections(cells.chunks_exact(SECTION_VOLUME).map(Section::from_cells))
    }
    #[inline]
    pub fn get(&self, index: usize) -> T {
        self.0[index / SECTION_VOLUME].get(index)
    }
    #[inline]
    pub fn section(&self, section: usize) -> &Section<T> {
        &self.0[section]
    }
    pub fn retained_bytes(&self) -> usize {
        16 + self.0.iter().map(Section::retained_bytes).sum::<usize>()
    }
    pub fn set(&mut self, index: usize, value: T) -> bool {
        if self.get(index) == value {
            return false;
        }
        Arc::make_mut(&mut self.0)[index / SECTION_VOLUME].set(index, value);
        true
    }
    pub fn compact_sections(&mut self, mask: u32) {
        for i in 0..self.section_count() {
            if mask & (1 << i) != 0
                && let Some(compacted) = self.section(i).compacted()
            {
                Arc::make_mut(&mut self.0)[i] = compacted;
            }
        }
    }
    pub fn changed_sections(&self, other: &Self) -> u32 {
        if Arc::ptr_eq(&self.0, &other.0) {
            return 0;
        }
        self.0
            .iter()
            .zip(other.0.iter())
            .enumerate()
            .filter(|(_, (old, new))| !old.same_cells(new))
            .fold(0, |mask, (section, _)| mask | (1 << section))
    }
}
