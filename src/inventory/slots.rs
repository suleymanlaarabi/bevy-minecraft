//! Allocation-free stack operations, statically dispatched over slot owners.
use super::{DragMode, ItemKind, ItemStack};

use bevy::prelude::*;
use std::{
    ops::{Deref, DerefMut},
    sync::Arc,
};

/// Contiguous slots with immutable, copy-on-write save snapshots.
#[derive(Component, Clone, Debug, PartialEq, Eq)]
pub struct ItemSlots<const N: usize>(Option<Arc<[Option<ItemStack>; N]>>);
impl<const N: usize> Default for ItemSlots<N> {
    fn default() -> Self {
        Self(None)
    }
}
impl<const N: usize> From<[Option<ItemStack>; N]> for ItemSlots<N> {
    fn from(slots: [Option<ItemStack>; N]) -> Self {
        Self(Some(Arc::new(slots)))
    }
}
impl<const N: usize> Deref for ItemSlots<N> {
    type Target = [Option<ItemStack>; N];
    fn deref(&self) -> &Self::Target {
        self.0.as_deref().unwrap_or(&[None; N])
    }
}
impl<const N: usize> DerefMut for ItemSlots<N> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        Arc::make_mut(self.0.get_or_insert_with(|| Arc::new([None; N])))
    }
}
impl<const N: usize> ItemSlots<N> {
    pub fn slot(&self, index: usize) -> Option<ItemStack> {
        self.get(index).copied().flatten()
    }
    pub fn set(&mut self, index: usize, stack: Option<ItemStack>) -> bool {
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
        self.iter().all(Option::is_none)
    }
}

pub(crate) trait Slots {
    fn len(&self) -> usize;
    fn get(&self, index: usize) -> Option<ItemStack>;
    fn set(&mut self, index: usize, stack: Option<ItemStack>);
    fn writable(&self, index: usize) -> bool {
        index < self.len()
    }
    fn capacity(&self, index: usize, item: ItemKind) -> u8;
    fn collectible(&self, index: usize) -> bool {
        self.writable(index)
    }
}
fn stack(item: ItemKind, count: u8) -> Option<ItemStack> {
    (count != 0).then(|| ItemStack::new(item, count))
}
pub(crate) fn swap(slots: &mut impl Slots, a: usize, b: usize) {
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
pub(crate) fn click(
    slots: &mut impl Slots,
    index: usize,
    cursor: &mut Option<ItemStack>,
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
            *cursor = stack(current.item(), count);
            slots.set(index, stack(current.item(), current.count() - count));
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
            slots.set(index, stack(held.item(), count));
            *cursor = stack(held.item(), held.count() - count);
        }
        Some(current) if current.item() == held.item() => {
            let count =
                capacity
                    .saturating_sub(current.count())
                    .min(if right { 1 } else { held.count() });
            if count == 0 {
                return;
            }
            slots.set(index, stack(held.item(), current.count() + count));
            *cursor = stack(held.item(), held.count() - count);
        }
        Some(current) if !right && held.count() <= capacity => {
            slots.set(index, Some(held));
            *cursor = Some(current);
        }
        _ => {}
    }
}
pub(crate) fn collect(slots: &mut impl Slots, cursor: &mut Option<ItemStack>) {
    let Some(mut held) = *cursor else {
        return;
    };
    for index in 0..slots.len() {
        if held.count() == held.max_stack_size() {
            break;
        }
        if !slots.collectible(index) {
            continue;
        }
        let Some(current) = slots.get(index) else {
            continue;
        };
        if held.item() != current.item() {
            continue;
        }
        let count = (held.max_stack_size() - held.count()).min(current.count());
        if count == 0 {
            continue;
        }
        slots.set(index, stack(current.item(), current.count() - count));
        held = ItemStack::new(held.item(), held.count() + count);
    }
    *cursor = Some(held);
}
pub(crate) fn distribute(
    slots: &mut impl Slots,
    cursor: &mut Option<ItemStack>,
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
        if !visit || remaining == 0 || !compatible(slots, index) {
            continue;
        }
        let previous = slots.get(index).map_or(0, ItemStack::count);
        let moved = amount
            .min(remaining)
            .min(slots.capacity(index, held.item()).saturating_sub(previous));
        if moved != 0 {
            slots.set(index, stack(held.item(), previous + moved));
            remaining -= moved;
        }
    }
    *cursor = stack(held.item(), remaining);
}
fn slots_compatible(slots: &impl Slots, index: usize, held: ItemStack) -> bool {
    slots.writable(index)
        && slots.capacity(index, held.item()) > slots.get(index).map_or(0, ItemStack::count)
        && slots
            .get(index)
            .is_none_or(|current| current.item() == held.item())
}
pub(crate) fn transfer(slots: &mut impl Slots, source: usize, targets: core::ops::Range<usize>) {
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
pub(crate) fn insert(
    slots: &mut impl Slots,
    held: ItemStack,
    targets: impl IntoIterator<Item = usize> + Clone,
) -> Option<ItemStack> {
    let mut left = held.count();
    for empty in [false, true] {
        for target in targets.clone() {
            if !slots.writable(target) {
                continue;
            }
            let current = slots.get(target);
            if current.is_none() != empty
                || current.is_some_and(|stack| stack.item() != held.item())
            {
                continue;
            }
            let previous = current.map_or(0, ItemStack::count);
            let moved = left.min(slots.capacity(target, held.item()).saturating_sub(previous));
            if moved == 0 {
                continue;
            }
            slots.set(target, stack(held.item(), previous + moved));
            left -= moved;
            if left == 0 {
                break;
            }
        }
        if left == 0 {
            break;
        }
    }
    stack(held.item(), left)
}
