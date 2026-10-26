pub use pumpkin_data::chunk::{
    Parameter, ParameterPoint, ParameterRange, TargetPoint, quantize_coord, unquantize_coord,
};

#[must_use]
pub const fn to_long(float: f32) -> i64 {
    quantize_coord(float)
}

pub type NoiseValuePoint = TargetPoint;

