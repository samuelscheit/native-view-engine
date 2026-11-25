//! Binary compact binary vector draw-list IR (`NVE_DRAW1`) for `SkiaCanvas` + image resize modes.
//!
//! Layout of a binary `NVE_DRAW1` buffer (all multi-byte integers and IEEE-754 floats little-endian):
//! - `[0..9]`:   9-byte ASCII magic `b"NVE_DRAW1"`
//! - `[9..11]`:  `u16` format version (`NVE_DRAW_VERSION == 1`)
//! - `[11..15]`: `u32` op count (strictly capped at [`MAX_DRAW_OPS`])
//! - `[15..]`:   packed stream of tagged vector operations (`Clear`, `FillRect`, `FillRoundedRect`,
//!   `FillCircle`, `StrokeLine`, `StrokePolyline`, `ClipRect`, `RestoreClip`).
//!
//! Designed for zero-copy transport inside [`nve_abi::RawPropValue::Bytes`] (with an optional
//! compact ASCII base64 attribute wrapper for Dioxus `rsx!` string attributes).

mod base64;
mod codec;
mod draw_list;
mod types;

#[cfg(test)]
mod tests;

pub use codec::{DrawListBuilder, DrawListDecoder};
pub use draw_list::{DrawList, DrawListRecorder};
pub use types::{
    CanvasOp, DrawListError, DrawListHeader, ImageResizeMode, DRAW_LIST_ATTR_PREFIX, MAX_DRAW_OPS,
    MAX_POLYLINE_POINTS, NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};
