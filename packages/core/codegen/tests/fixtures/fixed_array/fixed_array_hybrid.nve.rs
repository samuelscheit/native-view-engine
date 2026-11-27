//! `[T; N]` across every shape a spec can hold it in: fields of a non-POD struct (the `NveBinValue`
//! codec), fields of a POD struct (C arrays, nested too), method arguments and returns, options,
//! `Result`s, and view props. `FixedBundle` is declared before the structs it holds: emitters
//! order structs by dependency. `[u32; 1]` is a scalar in Swift's C import (a 1-tuple).

#[nve_hybrid_enum]
pub enum FixedKind {
    Alpha = 0,
    Beta = 1,
    Gamma = 7,
}

#[nve_hybrid_struct]
pub struct FixedBundle {
    pub label: String,
    pub coords: [u32; 4],
    pub points: [FixedPoint; 2],
    pub matrix: [[u32; 2]; 3],
    pub tag: Option<[u8; 3]>,
}

#[nve_hybrid_struct]
pub struct FixedPod {
    pub v: [f32; 4],
    pub points: [FixedPoint; 2],
    pub kinds: [FixedKind; 2],
    pub grid: [[u32; 2]; 3],
    pub flags: [[bool; 33]; 2],
    pub one: [u32; 1],
    pub cells: [[FixedPoint; 1]; 2],
}

#[nve_hybrid_struct]
pub struct FixedPoint {
    pub x: i32,
    pub y: i32,
}

#[nve_hybrid_module(name = "FixedCodec")]
pub trait FixedCodecSpec {
    fn echo(&mut self, bundle: FixedBundle) -> FixedBundle;
    fn swap(&mut self, coords: [u32; 4]) -> [u32; 4];
    fn tag(&mut self, present: bool) -> Option<[u8; 3]>;
    /// `0`: `Err`, `1`: `Ok(None)`, otherwise `Ok(Some(..))`.
    fn try_tag(&mut self, mode: u32) -> Result<Option<[u8; 3]>, String>;
    fn retag(&mut self, tag: Option<[u8; 3]>) -> Option<[u8; 3]>;
    fn mirror(&mut self, pod: FixedPod) -> FixedPod;
    fn maybe_mirror(&mut self, pod: Option<FixedPod>) -> Option<FixedPod>;
}

#[nve_hybrid_view(name = "FixedView")]
pub trait FixedViewSpec {
    #[prop]
    fn coords(&self) -> [u32; 4];

    #[prop]
    fn tag(&self) -> Option<[u8; 3]>;

    #[prop]
    fn pod(&self) -> FixedPod;

    #[method]
    fn summary(&self) -> FixedBundle;
}
