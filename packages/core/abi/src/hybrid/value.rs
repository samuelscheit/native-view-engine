use crate::DecodeError;
use crate::value::{
    BinValueView, BinWriter, NveValueRef, NveValueTag, NveValueTree, encode_bin_value,
};

use super::codec::{HybridCodec, encode_to_vec};

/// Owned dynamic value (`NveValue` in a hybrid spec), held as one validated `NveBinValue`.
///
/// Every constructor validates (or produces) exactly one well-formed value, so encoders embed the
/// bytes verbatim and [`as_view`](Self::as_view) never re-validates. The default is `null`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveHybridValue {
    bytes: Vec<u8>,
}

impl Default for NveHybridValue {
    #[inline]
    fn default() -> Self {
        Self::null()
    }
}

impl NveHybridValue {
    #[inline]
    pub fn null() -> Self {
        Self {
            bytes: vec![NveValueTag::Null as u8],
        }
    }

    /// Validates `bytes` as exactly one `NveBinValue`.
    pub fn from_bin(bytes: Vec<u8>) -> Result<Self, DecodeError> {
        BinValueView::parse(&bytes)?;
        Ok(Self { bytes })
    }

    #[inline]
    pub fn from_view(view: BinValueView<'_>) -> Self {
        Self {
            bytes: view.as_raw_bytes().to_vec(),
        }
    }

    /// Encodes a C `NveValue` tree.
    pub fn from_ffi_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        let mut w = BinWriter::with_capacity(64);
        encode_bin_value(value, &mut w);
        Self::from_bin(w.finish()?)
    }

    /// Encodes any hybrid type as a dynamic value.
    pub fn encode<T: HybridCodec>(value: &T) -> Result<Self, DecodeError> {
        Self::from_bin(encode_to_vec(value)?)
    }

    #[inline]
    pub fn as_view(&self) -> BinValueView<'_> {
        BinValueView::from_validated(&self.bytes)
    }

    #[inline]
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    #[inline]
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }

    /// The value as a C `NveValue` tree borrowing `self`.
    #[inline]
    pub fn to_tree(&self) -> NveValueTree<'_> {
        NveValueTree::new(self.as_view())
    }
}
