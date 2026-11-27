//! `NveTable` trait (implemented by `#[derive(NveTable)]`), typed property handles, and the
//! process-global table slot registry.

mod field;

use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use nve_abi::table::{table_flags, Storage, TableDesc, TableKey, Word, MAX_TABLE_SLOTS};

use crate::row::{RowRef, RowWriter};

pub use field::{Prop, Res, ResProp, ScalarField, Str, StrProp, TableField};

/// Maximum number of compiled static table slots (`0..MAX_STATIC_TABLE_SLOTS`).
/// Wire-declared dynamic tables occupy the disjoint upper range
/// `MAX_STATIC_TABLE_SLOTS..MAX_TABLE_SLOTS` so static and dynamic slots can never collide.
pub const MAX_STATIC_TABLE_SLOTS: usize = 96;

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
    pub(crate) fn inherited(&self) -> bool {
        match self {
            Self::Static(d) => d.inherited,
            Self::Dynamic(d) => (d.flags & table_flags::INHERITED) != 0,
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
    if desc.inherited != ((desc.flags & table_flags::INHERITED) != 0)
        || !validate_desc_invariants(
            desc.words(),
            desc.flags,
            desc.str_words,
            desc.resource_words,
            desc.defaults,
            desc.word_effects,
        )
    {
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
        .expect("NveTable static registration must succeed (unique key and <= 96 tables)");
    cell.store(slot.0, Ordering::Release);
    slot
}

/// Validates a descriptor's word/string/resource/default invariants before registration.
pub(crate) fn validate_desc_invariants(
    words: usize,
    flags: u8,
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
    if (flags & table_flags::INHERITED) != 0
        && (words < 2
            || defaults[0] != 0
            || word_effects[0] != 0
            || ((str_words | resource_words) & 1) != 0)
    {
        return false;
    }
    let valid_mask = if words == 32 { u32::MAX } else { (1u32 << words) - 1 };
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
