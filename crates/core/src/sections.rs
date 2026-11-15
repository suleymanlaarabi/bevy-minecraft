//! Uniform, paletted and directly indexed sections with copy-on-write snapshots.

use super::{VoxelCell, coordinates::SECTION_VOLUME};
use std::{collections::HashSet, sync::Arc};

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

/// Palette and indices share one allocation. The unused palette tail repeats its first value.
#[derive(Clone, Debug)]
pub struct Palette<T, const N: usize, const BYTES: usize> {
    pub(crate) values: [T; N],
    pub(crate) len: u16,
    pub(crate) indices: [u8; BYTES],
}
impl<T: Copy, const N: usize, const BYTES: usize> Palette<T, N, BYTES> {
    #[inline]
    fn index(&self, index: usize) -> usize {
        if N == 16 {
            usize::from((self.indices[index / 2] >> ((index & 1) * 4)) & 15)
        } else {
            usize::from(self.indices[index])
        }
    }
    #[inline]
    fn get(&self, index: usize) -> T {
        self.values[self.index(index)]
    }
    fn copy_mapped<U: Copy>(&self, start: usize, out: &mut [U], values: &[U]) {
        assert!(start <= SECTION_VOLUME && out.len() <= SECTION_VOLUME - start);
        assert!(values.len() >= usize::from(self.len));
        for (index, out) in (start..).zip(out) {
            // SAFETY: the range is checked above; palette construction and decoding
            // validate indices below len, which the mapped values cover.
            let palette = unsafe {
                if N == 16 {
                    (*self.indices.get_unchecked(index / 2) >> ((index & 1) * 4)) & 15
                } else {
                    *self.indices.get_unchecked(index)
                }
            };
            // SAFETY: the validated palette index is below len <= values.len().
            *out = unsafe { *values.get_unchecked(usize::from(palette)) };
        }
    }
}

fn arc_bytes<T: ?Sized>(value: &T) -> usize {
    let alignment = align_of_val(value).max(align_of::<usize>());
    ((2 * size_of::<usize>()).next_multiple_of(align_of_val(value)) + size_of_val(value))
        .next_multiple_of(alignment)
}

/// Allocation identities are tracked only when an explicit memory census is requested.
#[derive(Default)]
pub struct AllocationCensus {
    seen: HashSet<usize>,
    pub bytes: usize,
}
impl AllocationCensus {
    pub fn clear(&mut self) {
        self.seen.clear();
        self.bytes = 0;
    }

    pub fn shared<T: ?Sized>(&mut self, allocation: &Arc<T>) -> bool {
        if !self
            .seen
            .insert(Arc::as_ptr(allocation) as *const () as usize)
        {
            return false;
        }
        self.bytes += arc_bytes(allocation.as_ref());
        true
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
    Palette4(Arc<Palette<T, 16, { SECTION_VOLUME / 2 }>>),
    Palette8(Arc<Palette<T, 256, SECTION_VOLUME>>),
}

impl<T: SectionValue> Section<T> {
    pub fn from_cells(cells: &[T]) -> Self {
        assert_eq!(cells.len(), SECTION_VOLUME, "one section of cells");
        let first = cells[0];
        if cells.iter().all(|&cell| cell == first) {
            return Self::Uniform(first);
        }
        if let Some(palette) = Self::paletted(cells) {
            return palette;
        }
        Self::direct(cells.try_into().expect("one section of cells"))
    }

    fn paletted(cells: &[T]) -> Option<Self> {
        // A bounded stack index avoids one hash-map allocation per published section.
        // Eight-bit cells only benefit from the four-bit representation.
        let limit = if size_of::<T>() == 1 { 16 } else { 256 };
        let mut slots = [u16::MAX; 512];
        let mut values = [cells[0]; 256];
        let mut len = 0usize;
        let mut indices = [0u8; SECTION_VOLUME];
        for (&value, index) in cells.iter().zip(&mut indices) {
            let mut slot = (value.bits().wrapping_mul(0x9e37_79b9) >> 23) as usize;
            while slots[slot] != u16::MAX && values[slots[slot] as usize] != value {
                slot = (slot + 1) & 511;
            }
            if slots[slot] == u16::MAX {
                if len == limit {
                    return None;
                }
                values[len] = value;
                slots[slot] = len as u16;
                len += 1;
            }
            *index = slots[slot] as u8;
        }
        if len <= 16 {
            Some(Self::Palette4(Arc::new(Palette {
                values: std::array::from_fn(|i| values[i]),
                len: len as u16,
                indices: std::array::from_fn(|i| indices[i * 2] | indices[i * 2 + 1] << 4),
            })))
        } else {
            Some(Self::Palette8(Arc::new(Palette {
                values,
                len: len as u16,
                indices,
            })))
        }
    }

    pub fn palette(&self) -> Option<&[T]> {
        match self {
            Self::Palette4(p) => Some(&p.values[..usize::from(p.len)]),
            Self::Palette8(p) => Some(&p.values[..usize::from(p.len)]),
            _ => None,
        }
    }

    /// Map a published palette once, then decode rows without a catalog lookup per voxel.
    pub fn copy_palette<U: Copy>(&self, start: usize, out: &mut [U], values: &[U]) -> bool {
        match self {
            Self::Palette4(p) => p.copy_mapped(start, out, values),
            Self::Palette8(p) => p.copy_mapped(start, out, values),
            _ => return false,
        }
        true
    }

    #[inline]
    pub fn get(&self, index: usize) -> T {
        let index = index % SECTION_VOLUME;
        match self {
            Self::Uniform(value) => *value,
            Self::Narrow { prefix, cells } => T::from_bits(*prefix | u32::from(cells[index])),
            Self::Dense(cells) => cells[index],
            Self::Palette4(p) => p.get(index),
            Self::Palette8(p) => p.get(index),
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
            Self::Palette4(p) => p.copy_mapped(start, out, &p.values),
            Self::Palette8(p) => p.copy_mapped(start, out, &p.values),
        }
    }

    pub fn any(&self, predicate: impl Fn(T) -> bool) -> bool {
        match self {
            Self::Uniform(value) => predicate(*value),
            Self::Narrow { prefix, cells } => cells
                .iter()
                .any(|&cell| predicate(T::from_bits(*prefix | u32::from(cell)))),
            Self::Dense(cells) => cells.iter().any(|&cell| predicate(cell)),
            Self::Palette4(p) => p.values[..usize::from(p.len)]
                .iter()
                .copied()
                .any(predicate),
            Self::Palette8(p) => p.values[..usize::from(p.len)]
                .iter()
                .copied()
                .any(predicate),
        }
    }

    pub fn retained_bytes(&self) -> usize {
        size_of::<Self>()
            + match self {
                Self::Uniform(_) => 0,
                Self::Narrow { cells, .. } => arc_bytes(cells.as_ref()),
                Self::Dense(cells) => arc_bytes(cells.as_ref()),
                Self::Palette4(p) => arc_bytes(p.as_ref()),
                Self::Palette8(p) => arc_bytes(p.as_ref()),
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
            Self::Palette4(_) | Self::Palette8(_) => {
                // Promote once on mutation; propagation and edits keep direct indexed writes.
                let mut cells = std::array::from_fn::<_, SECTION_VOLUME, _>(|i| self.get(i));
                cells[index] = value;
                *self = Self::direct(&cells);
            }
        }
    }

    fn narrow(cells: &[T; SECTION_VOLUME]) -> Option<Self> {
        let prefix = cells[0].bits() & 0xffff_0000;
        (T::NARROW && cells.iter().all(|cell| cell.bits() & 0xffff_0000 == prefix)).then(|| {
            Self::Narrow {
                prefix,
                cells: Arc::new(std::array::from_fn(|i| cells[i].bits() as u16)),
            }
        })
    }
    fn direct(cells: &[T; SECTION_VOLUME]) -> Self {
        Self::narrow(cells).unwrap_or_else(|| Self::Dense(Arc::new(*cells)))
    }

    pub fn census(&self, census: &mut AllocationCensus) {
        match self {
            Self::Uniform(_) => {}
            Self::Narrow { cells, .. } => {
                census.shared(cells);
            }
            Self::Dense(cells) => {
                census.shared(cells);
            }
            Self::Palette4(p) => {
                census.shared(p);
            }
            Self::Palette8(p) => {
                census.shared(p);
            }
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
            (Self::Palette4(a), Self::Palette4(b)) => Arc::ptr_eq(a, b),
            (Self::Palette8(a), Self::Palette8(b)) => Arc::ptr_eq(a, b),
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
        if matches!(
            self,
            Self::Uniform(_) | Self::Palette4(_) | Self::Palette8(_)
        ) {
            return None;
        }
        let mut cells = [self.get(0); SECTION_VOLUME];
        self.copy_cells(0, &mut cells);
        if cells.iter().all(|&cell| cell == cells[0]) {
            return Some(Self::Uniform(cells[0]));
        }
        Self::paletted(&cells).or_else(|| {
            if matches!(self, Self::Dense(_)) && T::NARROW {
                Self::narrow(&cells)
            } else {
                None
            }
        })
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
    pub fn census(&self, census: &mut AllocationCensus) {
        if census.shared(&self.0) {
            for section in self.0.iter() {
                section.census(census);
            }
        }
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
