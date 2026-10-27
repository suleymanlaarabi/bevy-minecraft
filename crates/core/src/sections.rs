//! Uniform/dense section layers and copy-on-write snapshots.

use super::coordinates::SECTION_VOLUME;
use std::sync::Arc;

/// One section of a per-cell layer. Most sections hold a single value (open sky,
/// deep stone), stored without its 4096 cells.
#[derive(Clone, Debug)]
pub enum Section<T> {
    Uniform(T),
    Dense(Arc<[T; SECTION_VOLUME]>),
}

impl<T: Copy + PartialEq> Section<T> {
    pub fn from_cells(cells: &[T]) -> Self {
        let first = cells[0];
        if cells.iter().all(|&cell| cell == first) {
            Self::Uniform(first)
        } else {
            Self::Dense(Arc::new(cells.try_into().expect("one section of cells")))
        }
    }

    /// Value of a cell; only the low bits of the column index are used.
    #[inline]
    pub fn get(&self, index: usize) -> T {
        match self {
            Self::Uniform(value) => *value,
            Self::Dense(cells) => cells[index % SECTION_VOLUME],
        }
    }

    #[inline]
    pub fn uniform(&self) -> Option<T> {
        match self {
            Self::Uniform(value) => Some(*value),
            Self::Dense(_) => None,
        }
    }

    /// Copies the cells `start..start + out.len()` of this section.
    #[inline]
    pub fn copy_cells(&self, start: usize, out: &mut [T]) {
        match self {
            Self::Uniform(value) => out.fill(*value),
            Self::Dense(cells) => out.copy_from_slice(&cells[start..start + out.len()]),
        }
    }

    pub fn any(&self, predicate: impl Fn(T) -> bool) -> bool {
        match self {
            Self::Uniform(value) => predicate(*value),
            Self::Dense(cells) => cells.iter().any(|&cell| predicate(cell)),
        }
    }

    fn set(&mut self, index: usize, value: T) {
        let index = index % SECTION_VOLUME;
        match self {
            Self::Uniform(fill) => {
                let mut cells = Arc::new([*fill; SECTION_VOLUME]);
                Arc::get_mut(&mut cells).expect("new section")[index] = value;
                *self = Self::Dense(cells);
            }
            Self::Dense(cells) => Arc::make_mut(cells)[index] = value,
        }
    }

    fn same_cells(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Uniform(a), Self::Uniform(b)) => a == b,
            (Self::Dense(a), Self::Dense(b)) => Arc::ptr_eq(a, b) || a == b,
            (Self::Uniform(value), Self::Dense(cells))
            | (Self::Dense(cells), Self::Uniform(value)) => cells.iter().all(|cell| cell == value),
        }
    }
}

/// A per-cell layer of a chunk column, compressed by section. Cloning is O(1); an
/// edit copies only its section while a task still reads the previous one.
#[derive(Clone, Debug)]
pub struct Layer<T>(Arc<[Section<T>]>);

impl<T: Copy + PartialEq> Layer<T> {
    pub fn section_count(&self) -> usize {
        self.0.len()
    }
    pub fn from_sections(sections: impl IntoIterator<Item = Section<T>>) -> Self {
        Self(sections.into_iter().collect())
    }

    pub fn filled(value: T, section_count: usize) -> Self {
        Self::from_sections((0..section_count).map(|_| Section::Uniform(value)))
    }

    /// Compresses a column of any supported dimension one section at a time.
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

    /// Returns whether the cell changed.
    pub fn set(&mut self, index: usize, value: T) -> bool {
        if self.get(index) == value {
            return false;
        }
        Arc::make_mut(&mut self.0)[index / SECTION_VOLUME].set(index, value);
        true
    }

    /// One bit per section whose cells differ from `other`.
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
