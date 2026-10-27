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
    fn from_id(id: u32) -> Option<Self>;
    fn kind(self) -> Self::Kind;
    fn default_state(kind: Self::Kind) -> Self;
    fn air() -> Self;
    fn is_liquid(self) -> bool;
    fn is_air(self) -> bool;
    fn is_opaque(self) -> bool;
}
pub mod codec;
