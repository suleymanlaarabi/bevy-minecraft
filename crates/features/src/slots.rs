//! Allocation-free stack operations, statically dispatched over slot owners.
use bevy_ecs::prelude::*;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

/// Item identity and its immutable stack limit, resolved before slot operations.
pub trait StackItem: Copy {
    fn max_stack_size(self) -> u8;
}

/// Small stack values stay inline; immutable metadata belongs to the item's catalog.
pub trait Stack: Copy + Eq + Send + Sync + 'static {
    type Item: StackItem;
    fn item(self) -> Self::Item;
    fn count(self) -> u8;
    fn with_count(self, count: u8) -> Option<Self>;
    fn can_merge(self, other: Self) -> bool;
    fn max_stack_size(self) -> u8 {
        self.item().max_stack_size()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragMode {
    OneEach,
    Even,
}

/// Contiguous slots with immutable, copy-on-write save snapshots.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
#[component(summary_tick)]
pub struct ItemSlots<T: Stack, const N: usize>(Option<Arc<[Option<T>; N]>>);
impl<T: Stack, const N: usize> Default for ItemSlots<T, N> {
    fn default() -> Self {
        Self(None)
    }
}
impl<T: Stack, const N: usize> From<[Option<T>; N]> for ItemSlots<T, N> {
    fn from(slots: [Option<T>; N]) -> Self {
        Self(Some(Arc::new(slots)))
    }
}
impl<T: Stack, const N: usize> Deref for ItemSlots<T, N> {
    type Target = [Option<T>; N];
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().unwrap_or(&[None; N])
    }
}
impl<T: Stack, const N: usize> DerefMut for ItemSlots<T, N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(self.0.get_or_insert_with(|| Arc::new([None; N])))
    }
}
impl<T: Stack, const N: usize> ItemSlots<T, N> {
    pub fn slot(&self, index: usize) -> Option<T> {
        self.as_slice().get(index).copied().flatten()
    }
    pub fn set(&mut self, index: usize, stack: Option<T>) -> bool {
        if index >= N || self.slot(index) == stack {
            return false;
        }
        self[index] = stack;
        if stack.is_none() && self.iter().all(Option::is_none) {
            self.0 = None;
        }
        true
    }
    pub fn is_empty(&self) -> bool {
        self.0
            .as_ref()
            .is_none_or(|slots| slots.iter().all(Option::is_none))
    }
}

pub trait Slots {
    type Stack: Stack;
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<Self::Stack>;
    fn set(&mut self, index: usize, stack: Option<Self::Stack>);
    fn writable(&self, index: usize) -> bool {
        index < self.len()
    }
    fn capacity(&self, index: usize, item: <Self::Stack as Stack>::Item) -> u8;
    fn collectible(&self, index: usize) -> bool {
        self.writable(index)
    }
}
pub fn swap<T: Stack>(slots: &mut impl Slots<Stack = T>, a: usize, b: usize) {
    if a == b || !slots.writable(a) || !slots.writable(b) {
        return;
    }
    let first = slots.get(a);
    let second = slots.get(b);
    if first.is_some_and(|stack| stack.count() > slots.capacity(b, stack.item()))
        || second.is_some_and(|stack| stack.count() > slots.capacity(a, stack.item()))
    {
        return;
    }
    slots.set(a, second);
    slots.set(b, first);
}
pub fn click<T: Stack>(
    slots: &mut impl Slots<Stack = T>,
    index: usize,
    cursor: &mut Option<T>,
    right: bool,
) {
    if !slots.writable(index) {
        return;
    }
    let current = slots.get(index);
    let Some(held) = *cursor else {
        if let Some(current) = current {
            let count = if right {
                current.count().div_ceil(2)
            } else {
                current.count()
            };
            *cursor = current.with_count(count);
            slots.set(index, current.with_count(current.count() - count));
        }
        return;
    };
    let capacity = slots.capacity(index, held.item());
    if capacity == 0 {
        return;
    }
    match current {
        None => {
            let count = if right { 1 } else { held.count().min(capacity) };
            slots.set(index, held.with_count(count));
            *cursor = held.with_count(held.count() - count);
        }
        Some(current) if current.can_merge(held) => {
            let count =
                capacity
                    .saturating_sub(current.count())
                    .min(if right { 1 } else { held.count() });
            if count == 0 {
                return;
            }
            slots.set(index, held.with_count(current.count() + count));
            *cursor = held.with_count(held.count() - count);
        }
        Some(current) if !right && held.count() <= capacity => {
            slots.set(index, Some(held));
            *cursor = Some(current);
        }
        _ => {}
    }
}
pub fn collect<T: Stack>(slots: &mut impl Slots<Stack = T>, cursor: &mut Option<T>) {
    let Some(mut held) = *cursor else {
        return;
    };
    let capacity = held.max_stack_size();
    for index in 0..slots.len() {
        if held.count() == capacity {
            break;
        }
        if !slots.collectible(index) {
            continue;
        }
        let Some(current) = slots.get(index) else {
            continue;
        };
        if !held.can_merge(current) {
            continue;
        }
        let count = (capacity - held.count()).min(current.count());
        slots.set(index, current.with_count(current.count() - count));
        held = held.with_count(held.count() + count).unwrap();
    }
    *cursor = Some(held);
}
pub fn distribute<T: Stack>(
    slots: &mut impl Slots<Stack = T>,
    cursor: &mut Option<T>,
    visited: &[bool],
    mode: DragMode,
) {
    let Some(held) = *cursor else {
        return;
    };
    let compatible = |slots: &_, index| slots_compatible(slots, index, held);
    let count = visited
        .iter()
        .enumerate()
        .filter(|(i, v)| **v && compatible(slots, *i))
        .count();
    if count == 0 {
        return;
    }
    let amount = match mode {
        DragMode::OneEach => 1,
        DragMode::Even => held.count() as usize / count,
    } as u8;
    if amount == 0 {
        return;
    }
    let mut remaining = held.count();
    for (index, &visit) in visited.iter().enumerate() {
        if remaining == 0 {
            break;
        }
        if !visit || !compatible(slots, index) {
            continue;
        }
        let previous = slots.get(index).map_or(0, T::count);
        let moved = amount
            .min(remaining)
            .min(slots.capacity(index, held.item()).saturating_sub(previous));
        if moved != 0 {
            slots.set(index, held.with_count(previous + moved));
            remaining -= moved;
        }
    }
    *cursor = held.with_count(remaining);
}
fn slots_compatible<T: Stack>(slots: &impl Slots<Stack = T>, index: usize, held: T) -> bool {
    slots.writable(index)
        && slots.capacity(index, held.item()) > slots.get(index).map_or(0, T::count)
        && slots
            .get(index)
            .is_none_or(|current| current.can_merge(held))
}
pub fn transfer<T: Stack>(
    slots: &mut impl Slots<Stack = T>,
    source: usize,
    targets: core::ops::Range<usize>,
) {
    if !slots.writable(source) {
        return;
    }
    let Some(held) = slots.get(source) else {
        return;
    };
    let remaining = insert(slots, held, targets.filter(|&index| index != source));
    if remaining != Some(held) {
        slots.set(source, remaining);
    }
}
/// Shared merge/fill operation for pickups, quick moves, and crafting remainders.
pub fn insert<T: Stack>(
    slots: &mut impl Slots<Stack = T>,
    held: T,
    targets: impl IntoIterator<Item = usize> + Clone,
) -> Option<T> {
    let mut left = held.count();
    for empty in [false, true] {
        for target in targets.clone() {
            if !slots.writable(target) {
                continue;
            }
            let current = slots.get(target);
            if current.is_none() != empty || current.is_some_and(|stack| !stack.can_merge(held)) {
                continue;
            }
            let previous = current.map_or(0, T::count);
            let moved = left.min(slots.capacity(target, held.item()).saturating_sub(previous));
            if moved == 0 {
                continue;
            }
            slots.set(target, held.with_count(previous + moved));
            left -= moved;
            if left == 0 {
                return None;
            }
        }
    }
    held.with_count(left)
}

/// Fixed transaction buffers use the same insertion rules as ECS inventories.
impl<T: Stack, const N: usize> Slots for [Option<T>; N] {
    type Stack = T;
    fn len(&self) -> usize {
        N
    }
    fn get(&self, index: usize) -> Option<T> {
        self.as_slice().get(index).copied().flatten()
    }
    fn set(&mut self, index: usize, stack: Option<T>) {
        self[index] = stack;
    }
    fn capacity(&self, index: usize, item: T::Item) -> u8 {
        if index < N { item.max_stack_size() } else { 0 }
    }
}
