use crate::inventory::{HOTBAR_SIZE, ItemKind, ItemStack, PlayerInventory, SLOT_COUNT};
use crate::voxel::VoxelKind;
use serde::{Deserialize, Serialize};
use std::{
    io,
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorldQuality {
    #[default]
    Fast,
    Vanilla,
}
impl WorldQuality {
    pub fn label(self) -> &'static str {
        match self {
            Self::Fast => "Fast",
            Self::Vanilla => "Vanilla",
        }
    }
    pub(crate) fn native(self) -> hollow_worldgen::TerrainQuality {
        match self {
            Self::Fast => hollow_worldgen::TerrainQuality::Fast,
            Self::Vanilla => hollow_worldgen::TerrainQuality::Vanilla,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct WorldMetadata {
    pub id: String,
    pub name: String,
    pub seed: i64,
    pub quality: WorldQuality,
    pub created_at: u64,
    pub last_played: u64,
}
impl WorldMetadata {
    pub(super) fn new(name: String, seed: i64, quality: WorldQuality) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            name,
            seed,
            quality,
            created_at: now(),
            last_played: now(),
        }
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        if uuid::Uuid::parse_str(&self.id).is_err() || self.name.trim().is_empty() {
            return Err(io::Error::other("Invalid world metadata."));
        }
        Ok(())
    }
}

pub(super) fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum SavedItem {
    Block(u16),
    Item(u16),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedStack {
    pub item: SavedItem,
    pub count: u8,
    pub damage: u32,
    pub contents: crate::inventory::ConsumableContents,
}
impl SavedStack {
    pub(crate) fn capture(stack: ItemStack) -> Self {
        Self {
            item: match stack.item() {
                ItemKind::Block(kind) => SavedItem::Block(kind.0.as_u16()),
                ItemKind::Item(id) => SavedItem::Item(id),
            },
            count: stack.count(),
            damage: stack.damage(),
            contents: stack.contents(),
        }
    }
    fn item(&self) -> io::Result<ItemKind> {
        match self.item {
            SavedItem::Block(id) => {
                pumpkin_data::BlockId::new(id).map(|id| ItemKind::Block(VoxelKind(id)))
            }
            SavedItem::Item(id) => Some(ItemKind::Item(id)),
        }
        .filter(|item| item.is_valid())
        .ok_or_else(|| io::Error::other("Unknown saved item."))
    }
    /// Saved stacks must match the current registry's limits exactly.
    pub(crate) fn restore(&self) -> io::Result<ItemStack> {
        let item = self.item()?;
        if self.count == 0 || self.count > item.max_stack_size() {
            return Err(io::Error::other("Invalid saved item stack."));
        }
        ItemStack::new(item, self.count)
            .with_damage(self.damage)
            .and_then(|stack| stack.with_contents(self.contents))
            .ok_or_else(|| {
                io::Error::other(
                    "Invalid saved item durability or unsupported consumable contents.",
                )
            })
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedPlayer {
    pub survival: crate::survival::SurvivalSnapshot,
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub pitch: f32,
    pub slots: Vec<Option<SavedStack>>,
    pub selected: usize,
    pub cursor: Option<SavedStack>,
    pub crafting: [Option<SavedStack>; 9],
}
impl SavedPlayer {
    /// Makes the pose loadable: a save must never write a world `inventory` rejects.
    /// An out-of-range position (falling into the void) falls back to the last saved one.
    pub(super) fn sanitize(&mut self, fallback: Option<&SavedPlayer>) {
        let rotation = bevy::math::Quat::from_array(self.rotation);
        self.rotation = if rotation.is_finite() && rotation.length_squared() > 1e-6 {
            rotation.normalize().to_array()
        } else {
            bevy::math::Quat::IDENTITY.to_array()
        };
        self.pitch = if self.pitch.is_finite() {
            self.pitch
                .clamp(-std::f32::consts::FRAC_PI_2, std::f32::consts::FRAC_PI_2)
        } else {
            0.0
        };
        let valid = |p: &[f32; 3]| {
            p.iter().all(|v| v.is_finite() && v.abs() <= 30_000_000.0)
                && (-8.0..=512.0).contains(&p[1])
        };
        if !valid(&self.position) {
            self.position = fallback
                .map(|player| player.position)
                .filter(valid)
                .unwrap_or_else(|| {
                    let [x, y, z] = self.position.map(|v| if v.is_finite() { v } else { 0.0 });
                    [
                        x.clamp(-30_000_000.0, 30_000_000.0),
                        y.clamp(-8.0, 512.0),
                        z.clamp(-30_000_000.0, 30_000_000.0),
                    ]
                });
        }
    }

    pub(crate) fn inventory(&self) -> io::Result<PlayerInventory> {
        if !self.survival.validate()
            || self.slots.len() != SLOT_COUNT
            || self.selected >= HOTBAR_SIZE
            || !self
                .position
                .iter()
                .chain(self.rotation.iter())
                .chain([&self.pitch])
                .all(|n| n.is_finite())
            || self.position[1] < -8.0
            || self.position[1] > 512.0
        {
            return Err(io::Error::other("Invalid player data."));
        }
        let norm: f32 = self.rotation.iter().map(|x| x * x).sum();
        if !(0.99..=1.01).contains(&norm)
            || self.position.iter().any(|p| p.abs() > 30_000_000.0)
            || self.pitch.abs() > std::f32::consts::FRAC_PI_2
        {
            return Err(io::Error::other("Invalid player position or orientation."));
        }
        let mut slots = [None; SLOT_COUNT];
        for (slot, saved) in slots.iter_mut().zip(&self.slots) {
            if let Some(saved) = saved {
                *slot = Some(saved.restore()?);
            }
        }
        if slots
            .iter()
            .enumerate()
            .any(|(index, stack)| stack.is_some_and(|stack| !PlayerInventory::fits(index, stack)))
        {
            return Err(io::Error::other("Invalid equipped item."));
        }
        if let Some(cursor) = &self.cursor {
            let _ = cursor.restore()?;
        }
        for stack in self.crafting.iter().flatten() {
            let _ = stack.restore()?;
        }
        let inventory = PlayerInventory::from_saved(slots);
        Ok(inventory)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedDrop {
    pub stack: SavedStack,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub remaining: f64,
    pub pickup_remaining: f64,
}
impl SavedDrop {
    pub(crate) fn validate(&self) -> io::Result<()> {
        let _ = self.stack.restore()?;
        if !self
            .position
            .iter()
            .chain(self.velocity.iter())
            .all(|v| v.is_finite())
            || !self.remaining.is_finite()
            || !(0.0..=300.0).contains(&self.remaining)
            || !self.pickup_remaining.is_finite()
            || !(0.0..=0.35).contains(&self.pickup_remaining)
        {
            return Err(io::Error::other("Invalid dropped item."));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    pub gameplay_tick: u64,
    pub player: Option<SavedPlayer>,
    // Edits to unloaded chunks must survive even when no chunk has been materialized.
    pub pending: Vec<([i32; 3], u16)>,
    /// Days since the world was created ([`crate::sky::TimeOfDay`]).
    pub time_of_day: Option<f64>,
}
impl Checkpoint {
    /// Column revisions decide gameplay persistence. World time advances even
    /// when this global checkpoint state is otherwise unchanged.
    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.player == other.player && self.pending == other.pending
    }
}
