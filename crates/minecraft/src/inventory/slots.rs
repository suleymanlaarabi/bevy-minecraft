//! Minecraft's stack type applied to the shared inventory operations.
pub use bevy_voxel::features::slots::{Slots, click, collect, distribute, insert, swap, transfer};
pub type ItemSlots<const N: usize> = bevy_voxel::features::slots::ItemSlots<super::ItemStack, N>;
