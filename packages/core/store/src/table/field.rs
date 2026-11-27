//! `TableField`, `ScalarField`, and typed property handles (`Prop`, `StrProp`, `ResProp`).

use core::marker::PhantomData;

use nve_abi::table::{FieldKind, OptEncoding, PackKind, Word, NO_VAL_WORD};

use crate::resource::Resource;
use crate::row::{RowRef, RowWriter};
use crate::table::NveTable;
use crate::value::{Rgba, WordValue};

/// Marker field type: a UTF-8 string stored as two words `{ offset, len }` in the text arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Str;

/// Marker field type: a `ResourceId` word referencing a deduplicated blob of type `R`.
pub struct Res<R: Resource>(PhantomData<fn() -> R>);

/// Field type metadata consumed by `#[derive(NveTable)]` and compile-time `pack_table`.
pub trait TableField {
    const KIND: FieldKind;
    /// Optionality & bit-encoding discriminator hashed into `TableDesc::schema_hash`.
    const OPT_ENCODING: OptEncoding;
    /// Bits occupied in its primary word (`1..=32` for scalar, `1` for wide `Option<T>`, `64` for 64-bit/`Str`).
    const BITS: u8;
    /// Word packing layout shape (`Scalar`, `WideOpt`, `Wide64`, `Str`, `Resource`).
    const PACK_KIND: PackKind;
    /// Registers codecs the field needs to cross a wire boundary.
    #[inline]
    fn register() {}
}

/// Uniform read/write codec for scalar, narrow `Option<T: OptWord>`, wide `Option<f32|u32|i32|Rgba>`,
/// and 64-bit (`u64`/`i64`/`f64`) table fields.
pub trait ScalarField: TableField + Copy {
    fn read_row(row: &[Word], word: usize, shift: u32, mask: u32, val_word: usize) -> Self;
    fn write_row(
        self,
        writer: &mut RowWriter<'_>,
        word: usize,
        shift: u32,
        mask: u32,
        val_word: usize,
    );

    #[inline(always)]
    fn read_ref(row: RowRef<'_>, word: usize, shift: u32, mask: u32, val_word: usize) -> Self {
        Self::read_row(row.words(), word, shift, mask, val_word)
    }
}

impl<T: WordValue> TableField for T {
    const KIND: FieldKind = T::KIND;
    const OPT_ENCODING: OptEncoding = T::OPT_ENCODING;
    const BITS: u8 = T::BITS;
    const PACK_KIND: PackKind = PackKind::Scalar;
}

impl<T: WordValue> ScalarField for T {
    #[inline(always)]
    fn read_row(row: &[Word], word: usize, shift: u32, mask: u32, _val_word: usize) -> Self {
        T::from_masked_bits((row[word] & mask) >> shift, mask.count_ones())
    }

    #[inline(always)]
    fn write_row(
        self,
        writer: &mut RowWriter<'_>,
        word: usize,
        shift: u32,
        mask: u32,
        _val_word: usize,
    ) {
        writer.put(word, shift, mask, self.to_masked_bits(mask.count_ones()));
    }
}

macro_rules! impl_wide_opt_field {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl TableField for Option<$ty> {
                const KIND: FieldKind = <$ty as WordValue>::KIND;
                const OPT_ENCODING: OptEncoding = OptEncoding::WideBit;
                const BITS: u8 = 1;
                const PACK_KIND: PackKind = PackKind::WideOpt;
            }

            impl ScalarField for Option<$ty> {
                #[inline(always)]
                fn read_row(
                    row: &[Word],
                    word: usize,
                    shift: u32,
                    _mask: u32,
                    val_word: usize,
                ) -> Self {
                    if (row[word] >> shift) & 1 != 0 {
                        Some(<$ty as WordValue>::from_bits(row[val_word]))
                    } else {
                        None
                    }
                }

                #[inline(always)]
                fn write_row(
                    self,
                    writer: &mut RowWriter<'_>,
                    word: usize,
                    shift: u32,
                    mask: u32,
                    val_word: usize,
                ) {
                    let (present, bits) = match self {
                        Some(v) => (1u32, <$ty as WordValue>::to_bits(v)),
                        None => (0u32, 0u32),
                    };
                    writer.put(word, shift, mask, present);
                    writer.put(val_word, 0, u32::MAX, bits);
                }
            }
        )+
    };
}

impl_wide_opt_field!(f32, u32, i32, Rgba);

impl TableField for u64 {
    const KIND: FieldKind = FieldKind::U64;
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    const BITS: u8 = 64;
    const PACK_KIND: PackKind = PackKind::Wide64;
}

impl ScalarField for u64 {
    #[inline(always)]
    fn read_row(row: &[Word], word: usize, _shift: u32, _mask: u32, _val_word: usize) -> Self {
        (row[word] as u64) | ((row[word + 1] as u64) << 32)
    }
    #[inline(always)]
    fn write_row(
        self,
        writer: &mut RowWriter<'_>,
        word: usize,
        _shift: u32,
        _mask: u32,
        _val_word: usize,
    ) {
        writer.put(word, 0, u32::MAX, self as u32);
        writer.put(word + 1, 0, u32::MAX, (self >> 32) as u32);
    }
}

impl TableField for i64 {
    const KIND: FieldKind = FieldKind::I64;
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    const BITS: u8 = 64;
    const PACK_KIND: PackKind = PackKind::Wide64;
}

impl ScalarField for i64 {
    #[inline(always)]
    fn read_row(row: &[Word], word: usize, shift: u32, mask: u32, val_word: usize) -> Self {
        u64::read_row(row, word, shift, mask, val_word) as i64
    }
    #[inline(always)]
    fn write_row(
        self,
        writer: &mut RowWriter<'_>,
        word: usize,
        shift: u32,
        mask: u32,
        val_word: usize,
    ) {
        (self as u64).write_row(writer, word, shift, mask, val_word);
    }
}

impl TableField for f64 {
    const KIND: FieldKind = FieldKind::F64;
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    const BITS: u8 = 64;
    const PACK_KIND: PackKind = PackKind::Wide64;
}

impl ScalarField for f64 {
    #[inline(always)]
    fn read_row(row: &[Word], word: usize, shift: u32, mask: u32, val_word: usize) -> Self {
        f64::from_bits(u64::read_row(row, word, shift, mask, val_word))
    }
    #[inline(always)]
    fn write_row(
        self,
        writer: &mut RowWriter<'_>,
        word: usize,
        shift: u32,
        mask: u32,
        val_word: usize,
    ) {
        self.to_bits().write_row(writer, word, shift, mask, val_word);
    }
}

impl TableField for Str {
    const KIND: FieldKind = FieldKind::Str;
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    const BITS: u8 = 64;
    const PACK_KIND: PackKind = PackKind::Str;
}

impl<R: Resource> TableField for Res<R> {
    const KIND: FieldKind = FieldKind::Resource;
    const OPT_ENCODING: OptEncoding = OptEncoding::Required;
    const BITS: u8 = 32;
    const PACK_KIND: PackKind = PackKind::Resource;
    fn register() {
        crate::resource::register_codec::<R>();
    }
}

/// Typed handle to a packed scalar or wide-optional field: `tx.set(node, Visual::OPACITY, 0.5)`.
pub struct Prop<T, V> {
    pub(crate) word: u8,
    pub(crate) shift: u8,
    pub(crate) val_word: u8,
    pub(crate) mask: u32,
    _p: PhantomData<fn() -> (T, V)>,
}

impl<T, V> Clone for Prop<T, V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, V> Copy for Prop<T, V> {}

impl<T: NveTable, V: ScalarField> Prop<T, V> {
    /// Constructs a scalar property handle for `T`.
    ///
    /// # Safety
    ///
    /// `(word, shift, bits)` must address a declared scalar field of `T::DESC` that does not
    /// overlap any string or resource word.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new(word: u8, shift: u8, bits: u8) -> Self {
        // SAFETY: caller upholds field-offset invariants; NO_VAL_WORD signals non-wide scalar.
        unsafe { Self::new_wide(word, shift, bits, NO_VAL_WORD) }
    }

    /// Constructs a scalar or wide-optional property handle for `T`.
    ///
    /// # Safety
    ///
    /// `(word, shift, bits, val_word)` must address a declared field of `T::DESC` that does not
    /// overlap any string or resource word.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new_wide(word: u8, shift: u8, bits: u8, val_word: u8) -> Self {
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            ((1u32 << bits) - 1) << shift
        };
        Self {
            word,
            shift,
            val_word,
            mask,
            _p: PhantomData,
        }
    }

    #[inline(always)]
    pub const fn word(self) -> u8 {
        self.word
    }

    #[inline(always)]
    pub const fn shift(self) -> u8 {
        self.shift
    }

    #[inline(always)]
    pub const fn val_word(self) -> u8 {
        self.val_word
    }

    #[inline(always)]
    pub const fn mask(self) -> u32 {
        self.mask
    }

    #[inline(always)]
    pub fn get(self, row: &[Word]) -> V {
        V::read_row(
            row,
            self.word as usize,
            self.shift as u32,
            self.mask,
            self.val_word as usize,
        )
    }
}

/// Typed handle to a string field.
pub struct StrProp<T> {
    pub(crate) word: u8,
    _p: PhantomData<fn() -> T>,
}

impl<T> Clone for StrProp<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for StrProp<T> {}

impl<T: NveTable> StrProp<T> {
    /// Constructs a string property handle for `T`.
    ///
    /// # Safety
    ///
    /// `word` must have its bit set in `T::DESC.str_words`.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new(word: u8) -> Self {
        Self {
            word,
            _p: PhantomData,
        }
    }

    #[inline(always)]
    pub const fn word(self) -> u8 {
        self.word
    }
}

/// Typed handle to a resource field.
pub struct ResProp<T, R> {
    pub(crate) word: u8,
    _p: PhantomData<fn() -> (T, R)>,
}

impl<T, R> Clone for ResProp<T, R> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, R> Copy for ResProp<T, R> {}

impl<T: NveTable, R: Resource> ResProp<T, R> {
    /// Constructs a resource property handle for `T`.
    ///
    /// # Safety
    ///
    /// `word` must have its bit set in `T::DESC.resource_words`.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new(word: u8) -> Self {
        Self {
            word,
            _p: PhantomData,
        }
    }

    #[inline(always)]
    pub const fn word(self) -> u8 {
        self.word
    }
}
