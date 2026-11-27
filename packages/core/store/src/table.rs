//! `NveTable` trait (implemented by `#[derive(NveTable)]`), typed property handles, and the
//! process-global table slot registry.

use core::marker::PhantomData;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use nve_abi::table::{FieldKind, MAX_TABLE_SLOTS, Storage, TableDesc, TableKey, Word, table_flags};

/// Maximum number of compiled static table slots (`0..MAX_STATIC_TABLE_SLOTS`).
/// Wire-declared dynamic tables occupy the disjoint upper range
/// `MAX_STATIC_TABLE_SLOTS..MAX_TABLE_SLOTS` so static and dynamic slots can never collide.
pub const MAX_STATIC_TABLE_SLOTS: usize = 96;

use crate::resource::Resource;
use crate::row::{RowRef, RowWriter};
use crate::value::WordValue;

/// Process-local table slot (bit index into `NodeRow::tables`).
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct TableSlot(pub u8);

impl TableSlot {
    /// Sentinel slot meaning plain container with no realizing table (`NVE_PRIMARY_SLOT_NONE`).
    pub const NONE: Self = Self(nve_abi::NVE_PRIMARY_SLOT_NONE);

    #[inline(always)]
    pub const fn is_none(self) -> bool {
        self.0 == nve_abi::NVE_PRIMARY_SLOT_NONE
    }

    #[inline(always)]
    pub const fn bit(self) -> u128 {
        if self.0 >= MAX_TABLE_SLOTS as u8 {
            0
        } else {
            1u128 << self.0
        }
    }
}

/// A word table. Implemented by `#[derive(NveTable)]`; the struct itself is the schema.
///
/// # Safety
///
/// Implementors must guarantee that:
/// - [`Self::DESC`] satisfies [`validate_desc_invariants`] (`str_words`, `resource_words`,
///   `defaults`, and `word_effects` are mutually consistent and string/resource defaults are zero).
/// - [`Self::from_row_ref`] and [`Self::from_row_writer`] only read/write word indices matching
///   the declared field kinds in [`Self::DESC`].
/// - [`Self::slot`] returns the static slot (`< MAX_STATIC_TABLE_SLOTS`) registered for
///   [`Self::DESC`] via [`register_static`].
pub unsafe trait NveTable: 'static {
    const DESC: TableDesc;
    /// Zero-copy typed row reader tied to the borrowing `NodeStore`'s lifetime.
    type Ref<'a>: Copy;
    /// Fluent typed row writer.
    type Mut<'a>;

    /// Sealed row-reader constructor used by `NodeStore`/`NodeRef`.
    #[doc(hidden)]
    fn from_row_ref<'a>(row: RowRef<'a>) -> Self::Ref<'a>;

    /// Sealed row-writer constructor used by `Transaction`/`NodeMut`.
    #[doc(hidden)]
    fn from_row_writer<'a>(writer: RowWriter<'a>) -> Self::Mut<'a>;

    /// Process-local slot, registered on first use.
    fn slot() -> TableSlot;
}

/// Marker field type: a UTF-8 string stored as two words `{ offset, len }` in the text arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Str;

/// Marker field type: a `ResourceId` word referencing a deduplicated blob of type `R`.
pub struct Res<R: Resource>(PhantomData<fn() -> R>);

/// Field type metadata consumed by `#[derive(NveTable)]`.
pub trait TableField {
    const KIND: FieldKind;
    /// Bits occupied in its (first) word.
    const BITS: u8;
    /// Named values the field holds (`word_enum!` variants); `#[values(PATH)]` overrides it.
    const VALUES: Option<&'static nve_abi::table::ValueSetDesc> = None;
    /// Registers codecs the field needs to cross a wire boundary.
    #[inline]
    fn register() {}
}

impl<T: WordValue> TableField for T {
    const KIND: FieldKind = T::KIND;
    const BITS: u8 = T::BITS;
    const VALUES: Option<&'static nve_abi::table::ValueSetDesc> = T::VALUES;
}

impl TableField for Str {
    const KIND: FieldKind = FieldKind::Str;
    const BITS: u8 = 32;
}

impl<R: Resource> TableField for Res<R> {
    const KIND: FieldKind = FieldKind::Resource;
    const BITS: u8 = 32;
    fn register() {
        crate::resource::register_codec::<R>();
    }
}

/// Typed handle to a packed scalar field: `tx.set(node, Visual::OPACITY, 0.5)`.
pub struct Prop<T, V> {
    pub(crate) word: u8,
    pub(crate) shift: u8,
    pub(crate) mask: u32,
    _p: PhantomData<fn() -> (T, V)>,
}

impl<T, V> Clone for Prop<T, V> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T, V> Copy for Prop<T, V> {}

impl<T: NveTable, V: WordValue> Prop<T, V> {
    /// Constructs a scalar property handle for `T`.
    ///
    /// # Safety
    ///
    /// `(word, shift, bits)` must address a declared scalar field of `T::DESC` that does not
    /// overlap any string or resource word.
    #[doc(hidden)]
    #[inline(always)]
    pub const unsafe fn new(word: u8, shift: u8, bits: u8) -> Self {
        let mask = if bits >= 32 {
            u32::MAX
        } else {
            ((1u32 << bits) - 1) << shift
        };
        Self {
            word,
            shift,
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
    pub const fn mask(self) -> u32 {
        self.mask
    }

    #[inline(always)]
    pub fn get(self, row: &[Word]) -> V {
        V::from_bits((row[self.word as usize] & self.mask) >> self.shift)
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

/// Owned descriptor for a runtime/wire-declared dynamic table scoped to a single
/// [`crate::wire::WireDecoder`] / [`crate::NodeStore`] (never leaked into global `REGISTRY`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OwnedTableDesc {
    pub(crate) key: TableKey,
    pub(crate) name: Box<str>,
    pub(crate) schema_hash: u64,
    pub(crate) storage: Storage,
    pub(crate) flags: u8,
    pub(crate) defaults: Box<[Word]>,
    pub(crate) word_effects: Box<[u8]>,
    pub(crate) str_words: u32,
    pub(crate) resource_words: u32,
}

/// Borrowed descriptor schema fields compared for full wire/registry equality.
pub(crate) struct TableDescSpec<'a> {
    pub(crate) schema_hash: u64,
    pub(crate) storage: Storage,
    pub(crate) flags: u8,
    pub(crate) defaults: &'a [Word],
    pub(crate) word_effects: &'a [u8],
    pub(crate) str_words: u32,
    pub(crate) resource_words: u32,
}

impl OwnedTableDesc {
    #[inline(always)]
    pub(crate) fn effects(&self) -> u8 {
        self.word_effects.iter().fold(0, |a, &e| a | e)
    }

    pub(crate) fn matches_spec(&self, spec: &TableDescSpec<'_>) -> bool {
        self.schema_hash == spec.schema_hash
            && self.storage == spec.storage
            && (self.flags & !table_flags::HOST_SLOT) == (spec.flags & !table_flags::HOST_SLOT)
            && self.defaults.as_ref() == spec.defaults
            && self.word_effects.as_ref() == spec.word_effects
            && self.str_words == spec.str_words
            && self.resource_words == spec.resource_words
    }
}

/// Descriptor backing one [`crate::column::Column`]: compiled static `&'static TableDesc` or
/// session-scoped `Arc<OwnedTableDesc>` from wire `DeclareTables` (freed on store/decoder drop).
#[derive(Debug, Clone)]
pub(crate) enum TableMeta {
    Static(&'static TableDesc),
    Dynamic(Arc<OwnedTableDesc>),
}

impl TableMeta {
    #[inline(always)]
    pub(crate) fn key(&self) -> TableKey {
        match self {
            Self::Static(d) => d.key,
            Self::Dynamic(d) => d.key,
        }
    }

    #[inline(always)]
    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Static(d) => d.name,
            Self::Dynamic(d) => &d.name,
        }
    }

    #[inline(always)]
    pub(crate) fn schema_hash(&self) -> u64 {
        match self {
            Self::Static(d) => d.schema_hash,
            Self::Dynamic(d) => d.schema_hash,
        }
    }

    #[inline(always)]
    pub(crate) fn storage(&self) -> Storage {
        match self {
            Self::Static(d) => d.storage,
            Self::Dynamic(d) => d.storage,
        }
    }

    #[inline(always)]
    pub(crate) fn flags(&self) -> u8 {
        match self {
            Self::Static(d) => d.flags,
            Self::Dynamic(d) => d.flags,
        }
    }

    #[inline(always)]
    pub(crate) fn words(&self) -> usize {
        self.defaults().len()
    }

    #[inline(always)]
    pub(crate) fn defaults(&self) -> &[Word] {
        match self {
            Self::Static(d) => d.defaults,
            Self::Dynamic(d) => &d.defaults,
        }
    }

    #[inline(always)]
    pub(crate) fn word_effects(&self) -> &[u8] {
        match self {
            Self::Static(d) => d.word_effects,
            Self::Dynamic(d) => &d.word_effects,
        }
    }

    #[inline(always)]
    pub(crate) fn str_words(&self) -> u32 {
        match self {
            Self::Static(d) => d.str_words,
            Self::Dynamic(d) => d.str_words,
        }
    }

    #[inline(always)]
    pub(crate) fn resource_words(&self) -> u32 {
        match self {
            Self::Static(d) => d.resource_words,
            Self::Dynamic(d) => d.resource_words,
        }
    }

    #[inline(always)]
    pub(crate) fn effects(&self) -> u8 {
        match self {
            Self::Static(d) => d.effects(),
            Self::Dynamic(d) => d.effects(),
        }
    }

    #[inline(always)]
    pub(crate) fn as_static(&self) -> Option<&'static TableDesc> {
        match self {
            Self::Static(d) => Some(*d),
            Self::Dynamic(_) => None,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Process-global slot registry
// ---------------------------------------------------------------------------------------------

static REGISTRY: RwLock<Vec<&'static TableDesc>> = RwLock::new(Vec::new());

/// Error registering a table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegistryError {
    /// All `MAX_STATIC_TABLE_SLOTS` slots are taken.
    Full,
    /// A table with this key is registered with a different schema.
    SchemaMismatch { name: &'static str, key: TableKey },
}

pub(crate) fn static_desc_matches(existing: &TableDesc, spec: &TableDescSpec<'_>) -> bool {
    existing.schema_hash == spec.schema_hash
        && existing.storage == spec.storage
        && (existing.flags & !table_flags::HOST_SLOT) == (spec.flags & !table_flags::HOST_SLOT)
        && existing.defaults == spec.defaults
        && existing.word_effects == spec.word_effects
        && existing.str_words == spec.str_words
        && existing.resource_words == spec.resource_words
}

fn register_locked(desc: &'static TableDesc) -> Result<TableSlot, RegistryError> {
    if !validate_desc_invariants(
        desc.words(),
        desc.str_words,
        desc.resource_words,
        desc.defaults,
        desc.word_effects,
    ) {
        return Err(RegistryError::SchemaMismatch {
            name: desc.name,
            key: desc.key,
        });
    }
    let mut reg = REGISTRY.write().unwrap_or_else(|e| e.into_inner());
    if let Some(i) = reg.iter().position(|d| d.key == desc.key) {
        let spec = TableDescSpec {
            schema_hash: desc.schema_hash,
            storage: desc.storage,
            flags: desc.flags,
            defaults: desc.defaults,
            word_effects: desc.word_effects,
            str_words: desc.str_words,
            resource_words: desc.resource_words,
        };
        if !static_desc_matches(reg[i], &spec) {
            return Err(RegistryError::SchemaMismatch {
                name: desc.name,
                key: desc.key,
            });
        }
        return Ok(TableSlot(i as u8));
    }
    if reg.len() >= MAX_STATIC_TABLE_SLOTS {
        return Err(RegistryError::Full);
    }
    reg.push(desc);
    Ok(TableSlot((reg.len() - 1) as u8))
}

/// Slot of a compiled table; caches it in the table's `static` slot cell. Used by the derive.
#[doc(hidden)]
#[cold]
pub fn register_static(desc: &'static TableDesc, cell: &AtomicU8, register: fn()) -> TableSlot {
    register();
    let slot = register_locked(desc)
        .expect("NveTable static registration must succeed (unique key and <= 64 tables)");
    cell.store(slot.0, Ordering::Release);
    slot
}

/// Validates a descriptor's word/string/resource/default invariants before registration.
pub(crate) fn validate_desc_invariants(
    words: usize,
    str_words: u32,
    resource_words: u32,
    defaults: &[Word],
    word_effects: &[u8],
) -> bool {
    if words == 0 || words > nve_abi::table::MAX_TABLE_WORDS {
        return false;
    }
    if defaults.len() != words || word_effects.len() != words {
        return false;
    }
    let valid_mask = if words == 32 {
        u32::MAX
    } else {
        (1u32 << words) - 1
    };
    if (str_words & !valid_mask) != 0 || (resource_words & !valid_mask) != 0 {
        return false;
    }
    // No string bit at the last word (strings occupy two consecutive words `w` and `w + 1`).
    if (str_words & (1u32 << (words - 1))) != 0 {
        return false;
    }
    // No overlapping consecutive string words.
    if (str_words & (str_words << 1)) != 0 {
        return false;
    }
    // No overlap between string word pairs and resource words.
    if ((str_words | (str_words << 1)) & resource_words) != 0 {
        return false;
    }
    // String and resource defaults must be zero so uninitialized rows never point into slabs/resources.
    let mut sm = str_words;
    while sm != 0 {
        let w = sm.trailing_zeros() as usize;
        sm &= sm - 1;
        if defaults[w] != 0 || defaults[w + 1] != 0 {
            return false;
        }
    }
    let mut rm = resource_words;
    while rm != 0 {
        let w = rm.trailing_zeros() as usize;
        rm &= rm - 1;
        if defaults[w] != 0 {
            return false;
        }
    }
    true
}

/// Looks up a compiled/static table by key without mutating `REGISTRY`.
pub(crate) fn lookup_static(key: TableKey) -> Option<(TableSlot, &'static TableDesc)> {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    reg.iter()
        .position(|d| d.key == key)
        .map(|i| (TableSlot(i as u8), reg[i]))
}

/// Descriptor of a static slot, if registered.
pub(crate) fn try_desc(slot: TableSlot) -> Option<&'static TableDesc> {
    REGISTRY
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(slot.0 as usize)
        .copied()
}

/// Slot registered for `key`, if any.
pub fn slot_of(key: TableKey) -> Option<TableSlot> {
    let reg = REGISTRY.read().unwrap_or_else(|e| e.into_inner());
    reg.iter()
        .position(|d| d.key == key)
        .map(|i| TableSlot(i as u8))
}

/// Descriptor of a registered slot.
pub fn desc(slot: TableSlot) -> &'static TableDesc {
    REGISTRY.read().unwrap_or_else(|e| e.into_inner())[slot.0 as usize]
}

/// Deterministic 64-bit XXH64 digest over an explicit slice of static table descriptors
/// (sorted and deduplicated by `TableKey` so slice order, duplicate descriptors, and process-global
/// lazy registration state never affect the hash).
pub fn tables_schema_hash(descs: &[&'static TableDesc]) -> u64 {
    let mut pairs: Vec<(u64, u64)> = descs.iter().map(|d| (d.key.0, d.schema_hash)).collect();
    pairs.sort_unstable();
    pairs.dedup();
    let mut hasher = nve_abi::Xxh64Hasher::new(0x4E56_4554_424C_5331); // "NVETBLS1"
    for (key, hash) in pairs {
        hasher.update(&key.to_le_bytes());
        hasher.update(&hash.to_le_bytes());
    }
    hasher.digest()
}
