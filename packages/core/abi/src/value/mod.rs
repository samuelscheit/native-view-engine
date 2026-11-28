//! Zero-copy `#[repr(C)]` FFI value, prop/command slab, & direct event ABI types,
//! plus optional compact binary `NveBinValue` codec (`feature = "wire"`).

#[cfg(feature = "wire")]
mod bin_codec;
#[cfg(feature = "wire")]
mod bin_view;
#[cfg(feature = "wire")]
mod bin_writer;
#[cfg(all(test, feature = "wire"))]
mod bin_writer_tests;
mod ffi_types;
mod ffi_views;
#[cfg(test)]
mod tests;
#[cfg(feature = "wire")]
mod value_tree;

/// Maximum permitted container nesting depth for `NveBinValue` encoding & decoding.
#[cfg(feature = "wire")]
pub const NVE_BIN_VALUE_MAX_DEPTH: usize = 64;

#[cfg(feature = "wire")]
pub use bin_codec::{encode_bin_value, encode_bin_value_to_vec};
#[cfg(feature = "wire")]
pub use bin_view::{BinArrayIter, BinMapIter, BinValueView};
#[cfg(feature = "wire")]
pub use bin_writer::{BinArrayWriter, BinMapWriter, BinWriter};
#[cfg(feature = "wire")]
pub use value_tree::NveValueTree;
pub(crate) use ffi_types::guarded_slice;
pub use ffi_types::{
    NveCommandSlice, NveMapEntry, NvePropEntry, NvePropSlice, NveStrSlice, NveValue,
    NveValueArray, NveValueData, NveValueMap, NveValueTag,
};
pub use ffi_views::{
    NveArrayRef, NveCommandSliceRef, NveMapRef, NvePropSliceRef, NvePropsRef, NveValueRef,
};
