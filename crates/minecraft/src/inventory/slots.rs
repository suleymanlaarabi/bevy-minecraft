//! Minecraft's stack type applied to the shared inventory operations.
pub use hollow_features::slots::{Slots, click, collect, distribute, insert, swap, transfer};
pub type ItemSlots<const N: usize> = hollow_features::slots::ItemSlots<super::ItemStack, N>;
