//! Shared voxel storage, geometry and compact runtime identities.
pub mod coordinates;
pub mod sections;
pub mod shape;

use serde::{Deserialize, Serialize};

/// Runtime identities are dense within their own registered domain.
macro_rules! identity {
    ($($name:ident),* $(,)?) => {$ (
        #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[repr(transparent)]
        pub struct $name(pub u32);
        impl $name {
            pub const fn index(self) -> usize { self.0 as usize }
        }
    )*};
}
identity!(BlockId, StateId, ShapeId, ResourceId);
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[repr(transparent)]
pub struct FluidId(pub u16);
impl FluidId {
    pub const NONE: Self = Self(0);
    pub const fn index(self) -> usize {
        self.0 as usize
    }
}
pub mod content;

/// Statically dispatched cell semantics; storage and world access know no content registry.
pub trait VoxelCell: Copy + Eq + std::fmt::Debug + Send + Sync + 'static {
    type Kind: Copy;
    fn id(self) -> u32;
    fn from_id(id: u32, content: &content::ContentCatalog) -> Option<Self>;
    fn kind(self, content: &content::ContentCatalog) -> Self::Kind;
    fn default_state(kind: Self::Kind, content: &content::ContentCatalog) -> Self;
    fn air() -> Self;
    fn shape_offset(self, _position: bevy_math::IVec3) -> Option<bevy_math::Vec3> {
        None
    }
    fn is_liquid(self, content: &content::ContentCatalog) -> bool;
    fn is_air(self, content: &content::ContentCatalog) -> bool;
    fn is_opaque(self, content: &content::ContentCatalog) -> bool;
}
pub mod codec;

/// Standard catalog-backed cell. No process-wide registry or parallel state IDs.
impl VoxelCell for StateId {
    type Kind = BlockId;
    fn id(self) -> u32 {
        self.0
    }
    fn from_id(id: u32, content: &content::ContentCatalog) -> Option<Self> {
        ((id as usize) < content.state_count()).then_some(Self(id))
    }
    fn kind(self, content: &content::ContentCatalog) -> BlockId {
        content.state(self).block
    }
    fn default_state(kind: BlockId, content: &content::ContentCatalog) -> Self {
        content.block(kind).default_state
    }
    fn air() -> Self {
        Self(0)
    }
    fn is_liquid(self, content: &content::ContentCatalog) -> bool {
        content.state(self).fluid.id != FluidId::NONE
    }
    fn is_air(self, content: &content::ContentCatalog) -> bool {
        content.state(self).air
    }
    fn is_opaque(self, content: &content::ContentCatalog) -> bool {
        let definition = content.state(self);
        definition.full_cube && definition.opacity == 15
    }
}
