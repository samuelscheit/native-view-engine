//! `[T; N]` fields in a non-POD struct (a `String` field keeps it off the C POD path), so the
//! arrays cross the `NveBinValue` codec.

#[nve_hybrid_struct]
pub struct FixedPoint {
    pub x: i32,
    pub y: i32,
}

#[nve_hybrid_struct]
pub struct FixedBundle {
    pub label: String,
    pub coords: [u32; 4],
    pub points: [FixedPoint; 2],
    pub tag: Option<[u8; 3]>,
}

#[nve_hybrid_module(name = "FixedCodec")]
pub trait FixedCodecSpec {
    fn echo(&mut self, bundle: FixedBundle) -> FixedBundle;
    fn swap(&mut self, coords: [u32; 4]) -> [u32; 4];
}
