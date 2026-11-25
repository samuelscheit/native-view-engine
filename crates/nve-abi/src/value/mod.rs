//! Zero-copy `#[repr(C)]` FFI value & direct event ABI types + compact binary `NveBinValue` codec.
//!
//! Two complementary zero-JSON paths share the exact same value model (`NveValueTag`):
//! 1. **In-process direct FFI** (`NveValue`, `NveMapEntry`, `NveDirectEvent`, `NveEventBatch`):
//!    Passed directly by pointer across C++ ↔ Rust (e.g. from `nve::BumpArena`) with zero copy,
//!    zero heap allocation, and zero serialization, accessed safely via borrowed `NveValueRef<'a>`
//!    and `NveDirectEventRef<'a>`.
//! 2. **Compact binary wire format** (`encode_bin_value` / borrowed `BinValueView<'a>`):
//!    Self-describing little-endian binary encoding with O(1) container skipping (`u32 byte_len`)
//!    and a strict nesting depth guard (`NVE_BIN_VALUE_MAX_DEPTH = 64`) for `EventPayload::Custom`
//!    crossing non-shared boundaries (WASM, IPC, replay logs).

mod bin_codec;
mod bin_view;
mod ffi_types;
mod ffi_views;
#[cfg(test)]
mod tests;

/// Maximum permitted container nesting depth for `NveBinValue` encoding & decoding.
pub const NVE_BIN_VALUE_MAX_DEPTH: usize = 64;

pub use bin_codec::{encode_bin_value, encode_bin_value_to_vec};
pub use bin_view::{BinArrayIter, BinMapIter, BinValueView};
pub use ffi_types::{
    NveDirectEvent, NveEventBatch, NveMapEntry, NveStrSlice, NveValue, NveValueArray,
    NveValueData, NveValueMap, NveValueTag,
};
pub use ffi_views::{
    NveArrayRef, NveDirectEventRef, NveEventBatchRef, NveMapRef, NvePropsRef, NveValueRef,
};
