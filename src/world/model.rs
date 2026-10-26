use crate::inventory::{HOTBAR_SIZE, ItemKind, ItemStack, PlayerInventory, SLOT_COUNT};
use crate::voxel::VoxelKind;
use serde::{Deserialize, Serialize};
use std::{
    io,
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) const FORMAT_VERSION: u32 = 5;
use hollow_worldgen::GENERATOR_VERSION;

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
    pub format: u32,
    pub generator: String,
    pub pumpkin_commit: String,
    pub minecraft_version: String,
    pub dimension: String,
    pub fast_version: u32,
    pub registry: u64,
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
            format: FORMAT_VERSION,
            generator: GENERATOR_VERSION.into(),
            pumpkin_commit: hollow_worldgen::PUMPKIN_COMMIT.into(),
            minecraft_version: hollow_worldgen::MINECRAFT_VERSION.into(),
            dimension: "overworld".into(),
            fast_version: hollow_worldgen::FAST_GENERATION_VERSION,
            registry: hollow_worldgen::state::registry_identity(),
        }
    }
    pub(super) fn validate(&self) -> io::Result<()> {
        if self.format != FORMAT_VERSION
            || self.generator != GENERATOR_VERSION
            || self.pumpkin_commit != hollow_worldgen::PUMPKIN_COMMIT
            || self.minecraft_version != hollow_worldgen::MINECRAFT_VERSION
            || self.dimension != "overworld"
            || self.fast_version != hollow_worldgen::FAST_GENERATION_VERSION
            || self.registry != hollow_worldgen::state::registry_identity()
        {
            return Err(io::Error::other(
                "Incompatible world version, dimension, generation settings, or block registry.",
            ));
        }
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct SavedStack {
    pub name: String,
    pub count: u8,
}
impl SavedStack {
    /// Keep the block identity when its item registry entry has a different key.
    pub(crate) fn capture(stack: ItemStack) -> Self {
        let name = match stack.item() {
            ItemKind::Block(kind) => kind.name(),
            item => item.name(),
        };
        Self {
            name: name.into(),
            count: stack.count(),
        }
    }
    fn item(&self) -> io::Result<ItemKind> {
        VoxelKind::from_key(&self.name)
            .filter(|kind| kind.is_itemizable())
            .map(ItemKind::Block)
            .or_else(|| ItemKind::from_name(&self.name))
            .filter(|item| item.is_valid())
            .ok_or_else(|| io::Error::other("Unknown saved item."))
    }
    /// Saved stacks must match the current registry's limits exactly.
    pub(crate) fn restore(&self) -> io::Result<ItemStack> {
        let item = self.item()?;
        if self.count == 0 || self.count > item.max_stack_size() {
            return Err(io::Error::other("Invalid saved item stack."));
        }
        Ok(ItemStack::new(item, self.count))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct SavedPlayer {
    pub position: [f32; 3],
    pub rotation: [f32; 4],
    pub pitch: f32,
    pub slots: Vec<Option<SavedStack>>,
    pub selected: usize,
    pub cursor: Option<SavedStack>,
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
        if self.slots.len() != SLOT_COUNT
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
    pub(super) fn validate(&self) -> io::Result<()> {
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
    pub player: Option<SavedPlayer>,
    pub drops: Vec<SavedDrop>,
    // Edits to unloaded chunks must survive even when no chunk has been materialized.
    pub pending: Vec<([i32; 3], u16)>,
    /// Bounded support cascades resume after saving or unloading.
    pub support_pending: Vec<[i32; 3]>,
    /// Days since the world was created ([`crate::sky::TimeOfDay`]).
    pub time_of_day: Option<f64>,
}
impl Checkpoint {
    /// Equality for skipping autosaves. Drop timers tick every frame even in an idle
    /// world, so only the drops' items and resting positions count; the time of day
    /// always moves and is saved along with real changes.
    pub(super) fn same_state(&self, other: &Self) -> bool {
        self.player == other.player
            && self.pending == other.pending
            && self.support_pending == other.support_pending
            && self.drops.len() == other.drops.len()
            && self
                .drops
                .iter()
                .zip(&other.drops)
                .all(|(a, b)| a.stack == b.stack && a.position == b.position)
    }
}
