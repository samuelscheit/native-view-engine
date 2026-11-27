// `NveBinValue` codec of every hybrid type, shared by all generated Rust hybrid bindings.
//
// Integers cross as `I64` (`u64` bit-cast, so it round-trips), floats as `F64`, `Vec<u8>` (a
// spec's `Bytes`) as `Bytes`, other `Vec<T>` / `[T; N]` as `Array`, structs as `Map` keyed by
// field name, and `None` as `Null`.

use crate::DecodeError;
use crate::value::{BinMapWriter, BinValueView, BinWriter, NveMapRef, NveValueRef, NveValueTag};

use super::NveHybridValue;

/// Binary encoding of one hybrid type, plus its decoding from a C `NveValue` (direct props).
pub trait HybridCodec: Sized {
    fn encode_bin(&self, w: &mut BinWriter);
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError>;
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError>;

    /// Value of an absent struct field: an error unless the type is optional.
    #[inline]
    fn decode_missing(field: &'static str) -> Result<Self, DecodeError> {
        Err(DecodeError::MissingField(field))
    }
}

/// Types that encode as an `Array` element of `Vec<T>`. Every hybrid type except `u8`, whose
/// vector is the spec's `Bytes` (encoded as one `Bytes` run, not an `Array`).
pub trait HybridElem: HybridCodec {}

/// Field lookup in an encoded struct.
pub trait HybridFields {
    fn field<T: HybridCodec>(&self, key: &'static str) -> Result<T, DecodeError>;
}

/// Field-wise codec of a generated struct; [`hybrid_struct_codec!`](crate::hybrid_struct_codec)
/// derives its [`HybridCodec`] from it.
pub trait HybridStruct: Sized {
    fn encode_fields(&self, m: &mut BinMapWriter<'_>);
    fn decode_fields(fields: &impl HybridFields) -> Result<Self, DecodeError>;
}

/// `value` encoded as one `NveBinValue`.
pub fn encode_to_vec<T: HybridCodec>(value: &T) -> Result<Vec<u8>, DecodeError> {
    let mut w = BinWriter::with_capacity(64);
    value.encode_bin(&mut w);
    w.finish()
}

/// Decodes a non-POD trampoline argument passed as one `NveBinValue` buffer.
#[inline]
pub fn decode_bin_arg<T: HybridCodec>(bytes: &[u8]) -> Result<T, DecodeError> {
    T::decode_bin(BinValueView::parse(bytes)?)
}

#[inline]
pub fn encode_struct<S: HybridStruct>(s: &S, w: &mut BinWriter) {
    w.map(|m| s.encode_fields(m));
}

#[inline]
pub fn decode_struct_bin<S: HybridStruct>(view: BinValueView<'_>) -> Result<S, DecodeError> {
    expect_bin(view, NveValueTag::Map)?;
    S::decode_fields(&view)
}

/// A struct prop arrives as a C `Map`, or as `Bytes` holding its `NveBinValue` encoding.
pub fn decode_struct_value_ref<S: HybridStruct>(value: NveValueRef<'_>) -> Result<S, DecodeError> {
    if let Some(bytes) = value.as_bytes() {
        return decode_struct_bin(BinValueView::parse(bytes)?);
    }
    let map = value
        .as_map()
        .ok_or_else(|| ref_mismatch(value, NveValueTag::Map))?;
    S::decode_fields(&map)
}

/// Decodes a `#[repr(u32)]` enum discriminant.
#[inline]
pub fn decode_enum<E>(
    raw: i64,
    name: &'static str,
    from_u32: fn(u32) -> Option<E>,
) -> Result<E, DecodeError> {
    u32::try_from(raw)
        .ok()
        .and_then(from_u32)
        .ok_or(DecodeError::InvalidEnum(name, raw as u32))
}

impl HybridFields for BinValueView<'_> {
    #[inline]
    fn field<T: HybridCodec>(&self, key: &'static str) -> Result<T, DecodeError> {
        match self.get(key) {
            Some(view) => T::decode_bin(view),
            None => T::decode_missing(key),
        }
    }
}

impl HybridFields for NveMapRef<'_> {
    #[inline]
    fn field<T: HybridCodec>(&self, key: &'static str) -> Result<T, DecodeError> {
        match self.get(key) {
            Some(value) => T::decode_value_ref(value),
            None => T::decode_missing(key),
        }
    }
}

#[inline]
fn expect_bin(view: BinValueView<'_>, expected: NveValueTag) -> Result<(), DecodeError> {
    if view.tag() == expected {
        Ok(())
    } else {
        Err(bin_mismatch(view, expected))
    }
}

#[inline]
fn bin_mismatch(view: BinValueView<'_>, expected: NveValueTag) -> DecodeError {
    DecodeError::UnexpectedTag {
        expected,
        found: view.tag() as u32,
    }
}

#[inline]
fn ref_mismatch(value: NveValueRef<'_>, expected: NveValueTag) -> DecodeError {
    DecodeError::UnexpectedTag {
        expected,
        found: value.raw().tag,
    }
}

#[inline]
fn narrow<T: TryFrom<i64>>(v: i64, name: &'static str) -> Result<T, DecodeError> {
    T::try_from(v).map_err(|_| DecodeError::OutOfRange(name))
}

impl HybridCodec for bool {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.bool(*self);
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        view.as_bool()
            .ok_or_else(|| bin_mismatch(view, NveValueTag::Bool))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        value
            .as_bool()
            .ok_or_else(|| ref_mismatch(value, NveValueTag::Bool))
    }
}

impl HybridCodec for i64 {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.i64(*self);
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        view.as_i64()
            .ok_or_else(|| bin_mismatch(view, NveValueTag::I64))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        value
            .as_i64()
            .ok_or_else(|| ref_mismatch(value, NveValueTag::I64))
    }
}

/// Integers narrower than `i64` reject out-of-range values instead of truncating them.
macro_rules! narrow_int_codec {
    ($($t:ty),*) => {$(
        impl HybridCodec for $t {
            #[inline]
            fn encode_bin(&self, w: &mut BinWriter) {
                w.i64(i64::from(*self));
            }
            #[inline]
            fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
                narrow(i64::decode_bin(view)?, stringify!($t))
            }
            #[inline]
            fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
                narrow(i64::decode_value_ref(value)?, stringify!($t))
            }
        }
    )*};
}
narrow_int_codec!(u8, u16, u32, i32);

impl HybridCodec for usize {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        match i64::try_from(*self) {
            Ok(v) => w.i64(v),
            Err(_) => w.fail(DecodeError::OutOfRange("usize")),
        }
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        narrow(i64::decode_bin(view)?, "usize")
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        narrow(i64::decode_value_ref(value)?, "usize")
    }
}

impl HybridCodec for u64 {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.i64(i64::from_ne_bytes(self.to_ne_bytes()));
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        i64::decode_bin(view).map(|v| u64::from_ne_bytes(v.to_ne_bytes()))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        i64::decode_value_ref(value).map(|v| u64::from_ne_bytes(v.to_ne_bytes()))
    }
}

/// Floats accept an `I64` too (a whole number written by a dynamically typed host).
impl HybridCodec for f64 {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.f64(*self);
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        view.as_number()
            .ok_or_else(|| bin_mismatch(view, NveValueTag::F64))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        value
            .as_number()
            .ok_or_else(|| ref_mismatch(value, NveValueTag::F64))
    }
}

impl HybridCodec for f32 {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.f64(f64::from(*self));
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        f64::decode_bin(view).map(|v| v as f32)
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        f64::decode_value_ref(value).map(|v| v as f32)
    }
}

impl HybridCodec for String {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.str(self);
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        view.as_str()
            .map(str::to_owned)
            .ok_or_else(|| bin_mismatch(view, NveValueTag::Str))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        value
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| ref_mismatch(value, NveValueTag::Str))
    }
}

impl HybridCodec for Vec<u8> {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.bytes(self);
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        view.as_bytes()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| bin_mismatch(view, NveValueTag::Bytes))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        value
            .as_bytes()
            .map(<[u8]>::to_vec)
            .ok_or_else(|| ref_mismatch(value, NveValueTag::Bytes))
    }
}

impl<T: HybridCodec> HybridCodec for Option<T> {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        match self {
            Some(v) => v.encode_bin(w),
            None => w.null(),
        }
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        if view.is_null() {
            Ok(None)
        } else {
            T::decode_bin(view).map(Some)
        }
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        if value.is_null() {
            Ok(None)
        } else {
            T::decode_value_ref(value).map(Some)
        }
    }
    #[inline]
    fn decode_missing(_field: &'static str) -> Result<Self, DecodeError> {
        Ok(None)
    }
}

/// A `Vec` prop arrives as a C `Array`, or as `Bytes` holding its `NveBinValue` encoding.
impl<T: HybridElem> HybridCodec for Vec<T> {
    fn encode_bin(&self, w: &mut BinWriter) {
        w.array(|a| self.iter().for_each(|item| item.encode_bin(a.item())));
    }
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        let items = view
            .as_array()
            .ok_or_else(|| bin_mismatch(view, NveValueTag::Array))?;
        items.map(T::decode_bin).collect()
    }
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        if let Some(bytes) = value.as_bytes() {
            return Self::decode_bin(BinValueView::parse(bytes)?);
        }
        let items = value
            .as_array()
            .ok_or_else(|| ref_mismatch(value, NveValueTag::Array))?;
        items.iter().map(T::decode_value_ref).collect()
    }
}

impl<T: HybridCodec + Copy + Default, const N: usize> HybridCodec for [T; N] {
    fn encode_bin(&self, w: &mut BinWriter) {
        w.array(|a| self.iter().for_each(|item| item.encode_bin(a.item())));
    }
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        let items = view
            .as_array()
            .ok_or_else(|| bin_mismatch(view, NveValueTag::Array))?;
        fixed_array(items.map(T::decode_bin))
    }
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        let items = value
            .as_array()
            .ok_or_else(|| ref_mismatch(value, NveValueTag::Array))?;
        fixed_array(items.iter().map(T::decode_value_ref))
    }
}

fn fixed_array<T: Copy + Default, const N: usize>(
    items: impl ExactSizeIterator<Item = Result<T, DecodeError>>,
) -> Result<[T; N], DecodeError> {
    if items.len() != N {
        return Err(DecodeError::OutOfRange("fixed-size array length"));
    }
    let mut out = [T::default(); N];
    for (slot, item) in out.iter_mut().zip(items) {
        *slot = item?;
    }
    Ok(out)
}

impl HybridCodec for NveHybridValue {
    #[inline]
    fn encode_bin(&self, w: &mut BinWriter) {
        w.value(self.as_view());
    }
    #[inline]
    fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {
        Ok(Self::from_view(view))
    }
    #[inline]
    fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {
        Self::from_ffi_ref(value)
    }
}

impl HybridElem for bool {}
impl HybridElem for u16 {}
impl HybridElem for u32 {}
impl HybridElem for i32 {}
impl HybridElem for i64 {}
impl HybridElem for u64 {}
impl HybridElem for usize {}
impl HybridElem for f32 {}
impl HybridElem for f64 {}
impl HybridElem for String {}
impl HybridElem for Vec<u8> {}
impl HybridElem for NveHybridValue {}
impl<T: HybridCodec> HybridElem for Option<T> {}
impl<T: HybridElem> HybridElem for Vec<T> {}
impl<T: HybridCodec + Copy + Default, const N: usize> HybridElem for [T; N] {}

/// Implements [`HybridCodec`] and [`HybridElem`] for a generated struct from its [`HybridStruct`].
#[macro_export]
macro_rules! hybrid_struct_codec {
    ($ty:ident) => {
        impl $crate::hybrid::HybridCodec for $ty {
            #[inline]
            fn encode_bin(&self, w: &mut $crate::BinWriter) {
                $crate::hybrid::encode_struct(self, w)
            }
            #[inline]
            fn decode_bin(
                view: $crate::BinValueView<'_>,
            ) -> ::core::result::Result<Self, $crate::DecodeError> {
                $crate::hybrid::decode_struct_bin(view)
            }
            #[inline]
            fn decode_value_ref(
                value: $crate::NveValueRef<'_>,
            ) -> ::core::result::Result<Self, $crate::DecodeError> {
                $crate::hybrid::decode_struct_value_ref(value)
            }
        }
        impl $crate::hybrid::HybridElem for $ty {}
    };
}

/// Implements [`HybridCodec`] and [`HybridElem`] for a generated `#[repr(u32)]` enum from its
/// inherent `to_u32` / `from_u32`.
#[macro_export]
macro_rules! hybrid_enum_codec {
    ($ty:ident) => {
        impl $crate::hybrid::HybridCodec for $ty {
            #[inline]
            fn encode_bin(&self, w: &mut $crate::BinWriter) {
                w.i64(i64::from(self.to_u32()))
            }
            #[inline]
            fn decode_bin(
                view: $crate::BinValueView<'_>,
            ) -> ::core::result::Result<Self, $crate::DecodeError> {
                let raw = <i64 as $crate::hybrid::HybridCodec>::decode_bin(view)?;
                $crate::hybrid::decode_enum(raw, stringify!($ty), $ty::from_u32)
            }
            #[inline]
            fn decode_value_ref(
                value: $crate::NveValueRef<'_>,
            ) -> ::core::result::Result<Self, $crate::DecodeError> {
                let raw = <i64 as $crate::hybrid::HybridCodec>::decode_value_ref(value)?;
                $crate::hybrid::decode_enum(raw, stringify!($ty), $ty::from_u32)
            }
        }
        impl $crate::hybrid::HybridElem for $ty {}
    };
}
