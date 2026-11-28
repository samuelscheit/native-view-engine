//! Word-table schema ABI shared by the Rust `NodeStore`, the `NVE2` wire and every foreign host.
//!
//! A table is a fixed row of `<= 32` little-endian `u32` words per node. Every word belongs to
//! exactly one effect class so a write marks precisely the pipeline stages it invalidates. Hosts
//! bind a [`TableKey`] once and read rows zero-copy through [`NveTableView`] using generated
//! word offsets.

mod pack;

use nve_abi_macros::NveAbi;
use nve_hash::{Key64, Xxh64Hasher};

pub use pack::{
    PackInput, PackKind, PackLayout, finalize_table_flags, pack_table, trim_bytes, trim_words,
};

/// One row word.
pub type Word = u32;

/// Maximum words per table row (the per-row dirty mask is one `u32`).
pub const MAX_TABLE_WORDS: usize = 32;

/// Maximum number of table slots per process (`NodeRow::tables` is a `u128`).
pub const MAX_TABLE_SLOTS: usize = 128;

/// `NveNodeRow::primary_slot` sentinel: a plain container with no realizing table. Any value in
/// `0..MAX_TABLE_SLOTS` is a `TableSlot` index into `NveSceneView::tables`.
pub const NVE_PRIMARY_SLOT_NONE: u8 = 0xFF;

/// `log2` of the node indices one [`NveTableView::row_index_pages`] page covers.
pub const NVE_ROW_INDEX_PAGE_BITS: u32 = 8;
/// Entries per [`NveTableView::row_index_pages`] page.
pub const NVE_ROW_INDEX_PAGE_LEN: u32 = 1 << NVE_ROW_INDEX_PAGE_BITS;
/// [`NveTableView::row_index_dir`] entry of a node-index range that has no page.
pub const NVE_ROW_INDEX_NO_PAGE: u32 = u32::MAX;

/// Stable 64-bit identity of a table: `xxh64(name)` baked at compile time.
pub type TableKey = Key64;

/// Pipeline stages a word invalidates.
pub mod effect {
    /// Box geometry inputs (Flexbox / Grid / position).
    pub const LAYOUT: u8 = 1 << 0;
    /// Intrinsic content size inputs (text, image natural size).
    pub const MEASURE: u8 = 1 << 1;
    /// Display-list / paint inputs.
    pub const PAINT: u8 = 1 << 2;
    /// Compositor-only inputs (opacity, transform) — no relayout, no repaint.
    pub const COMPOSITE: u8 = 1 << 3;
    /// Accessibility tree / semantic attributes.
    pub const SEMANTICS: u8 = 1 << 4;
    /// Native host class / DOM tag selection.
    pub const REALIZE: u8 = 1 << 5;
    /// Event subscription / interaction routing.
    pub const EVENTS: u8 = 1 << 6;
    pub const ALL: u8 = 0x7F;
}

/// Table storage strategy.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Storage {
    /// Row `i` lives at node index `i` (tables present on most nodes).
    Dense = 0,
    /// Packed rows + owner list, node → row via a paged index (rare tables).
    Sparse = 1,
}

/// `DeclareTables` flags.
pub mod table_flags {
    /// Table is owned by a host / out-of-tree module unknown to the core: pass through untouched.
    pub const HOST_SLOT: u8 = 1 << 0;
    /// Publish rows through `NveSceneView::tables`.
    pub const PUBLISH_TO_SCENE: u8 = 1 << 1;
    /// Any change to the table invalidates layout.
    pub const AFFECTS_LAYOUT: u8 = 1 << 2;
    /// Word 0 is the hidden `__explicit: u32` bitmask (`#[nve_table(inherited)]`).
    pub const INHERITED: u8 = 1 << 3;
    /// Word 0 is the `EventSet<E>` subscription bitmask (`#[nve_table(events = E)]`).
    pub const HAS_EVENTS: u8 = 1 << 4;
}

/// Sentinel `FieldDesc::val_word` / `TableDesc::events_word` meaning "not applicable".
pub const NO_VAL_WORD: u8 = u8::MAX;

/// How a field's bits are interpreted (drives codegen getters and wire string/resource handling).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FieldKind {
    U32 = 0,
    I32 = 1,
    F32 = 2,
    Bool = 3,
    /// Small enum / integer packed in `bits`.
    Enum = 4,
    /// Packed `RGBA8888` (`r | g<<8 | b<<16 | a<<24`).
    Color = 5,
    /// Tagged length (`Auto` / points / percent) — see `nve_store::Length`.
    Length = 6,
    /// Two words `{ offset, len }` into the text slab.
    Str = 7,
    /// `ResourceId` into the resource store.
    Resource = 8,
    /// Node reference (`NodeId` raw value, `0` = none).
    Node = 9,
    /// Two-word unsigned 64-bit integer (`lo`, `hi`).
    U64 = 10,
    /// Two-word signed 64-bit integer (`lo`, `hi`).
    I64 = 11,
    /// Two-word 64-bit float (`lo`, `hi`).
    F64 = 12,
    /// Packed 16-bit CSS Grid line placement (`Auto`, `Line(i16)`, `Span(u16)`).
    GridLine = 13,
}

/// Optionality and wire-bit encoding strategy for a [`FieldDesc`].
///
/// Included in [`compute_table_schema_hash`] so `T` and `Option<T>` never collide even when
/// niche encoding keeps their bit widths equal (e.g. `Length`, `GridLine`, 3-variant `WordEnum`).
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum OptEncoding {
    /// Non-optional field (required value, `Str`, or `Res<R>`).
    Required = 0,
    /// Optional handle using zero as the `None` niche (`Option<NodeId>`, `Option<SurfaceId>`).
    ZeroNiche = 1,
    /// Optional sub-word integer/bitflags with presence bit at bit 0 and payload in bits `1..`.
    Bit0Present = 2,
    /// Optional `bool` or `WordEnum` where `0 = None` and `discriminant + 1 = Some(v)`.
    PlusOne = 3,
    /// Optional tagged value (`Length`, `GridLine`) XORed with its unused tag niche so `0 = None`.
    TagXor = 4,
    /// Wide optional (`Option<f32|u32|i32|Rgba>`): 1-bit presence flag in `word` + full `val_word`.
    WideBit = 5,
}

/// Named values a field holds: the variants of a `word_enum!` or the bits of a flag set. Host
/// mirrors emit one `NVE_<NAME>_<VALUE>` constant each, so foreign code never hand-copies them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValueSetDesc {
    /// Rust type / set name (`AriaRole`, `TextDecoration`).
    pub name: &'static str,
    /// `true` for a bit-flag set (values combine with `|`), `false` for enum variants (exactly
    /// one value at a time). Hosts mirror flags as namespaced masks and variants as enums.
    pub flags: bool,
    /// `(variant, value)` pairs in declaration order.
    pub values: &'static [(&'static str, u32)],
}

/// One field of a table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FieldDesc {
    pub name: &'static str,
    pub kind: FieldKind,
    /// Optionality & bit-encoding strategy (`Required` for non-optional fields).
    pub opt_encoding: OptEncoding,
    /// First word (for wide `Option<T>`, the word holding the 1-bit presence flag).
    pub word: u8,
    /// Bit offset inside `word` (`0` for whole-word fields).
    pub shift: u8,
    /// Bit width inside `word` (`32` for whole-word fields; `64` for `Str`/64-bit; `1` for wide `Option<T>` presence bit).
    pub bits: u8,
    /// Effect class of the field's words.
    pub effects: u8,
    /// Value word for wide `Option<T>` fields (`NO_VAL_WORD` for all other fields).
    pub val_word: u8,
    /// True when marked `#[no_inherit]` on an inherited table (explicitly tracked, never inherited).
    pub no_inherit: bool,
    /// Named values (enum variants / flag bits); not part of the schema hash.
    pub values: Option<&'static ValueSetDesc>,
}

impl FieldDesc {
    /// Placeholder descriptor used to initialize fixed-size `const` arrays inside [`pack_table`].
    pub const EMPTY: Self = Self {
        name: "",
        kind: FieldKind::U32,
        opt_encoding: OptEncoding::Required,
        word: 0,
        shift: 0,
        bits: 0,
        effects: 0,
        val_word: NO_VAL_WORD,
        no_inherit: false,
        values: None,
    };
}

/// Compile-time description of a table, emitted by `#[derive(NveTable)]`.
#[derive(Debug, Clone, Copy)]
pub struct TableDesc {
    pub key: TableKey,
    pub name: &'static str,
    /// `xxh64` over the canonical field layout; a mismatch across a boundary is a hard error.
    pub schema_hash: u64,
    pub storage: Storage,
    pub flags: u8,
    pub defaults: &'static [Word],
    /// Effect class per word (`len == defaults.len()`).
    pub word_effects: &'static [u8],
    /// Bit `i` set: word `i` is the offset word of a two-word string reference.
    pub str_words: u32,
    /// Bit `i` set: word `i` holds a `ResourceId`.
    pub resource_words: u32,
    /// Word holding the `EventSet` subscription mask (`NO_VAL_WORD` if none).
    pub events_word: u8,
    /// Listener bits of that `EventSet<E>` (`EventEnum::EVENT_SET`: `<E>Set`, one
    /// `(variant, 1 << id)` flag per event), mirrored to hosts; `None` without `events`.
    pub events: Option<&'static ValueSetDesc>,
    /// True when word 0 is the hidden `__explicit: u32` bitmask for `#[nve_table(inherited)]`.
    pub inherited: bool,
    pub fields: &'static [FieldDesc],
    /// `(word_index, FlatResource::KIND)` pairs for every `Res<R>` field in this table.
    pub resource_kinds: &'static [(u8, Key64)],
}

impl TableDesc {
    #[inline]
    pub const fn words(&self) -> usize {
        self.defaults.len()
    }

    /// Returns the expected `FlatResource::KIND` for resource `word`, if declared on this table.
    #[inline]
    pub const fn resource_kind(&self, word: usize) -> Option<Key64> {
        let mut i = 0;
        while i < self.resource_kinds.len() {
            if self.resource_kinds[i].0 as usize == word {
                return Some(self.resource_kinds[i].1);
            }
            i += 1;
        }
        None
    }

    /// Union of all word effects.
    pub const fn effects(&self) -> u8 {
        let mut acc = 0;
        let mut i = 0;
        while i < self.word_effects.len() {
            acc |= self.word_effects[i];
            i += 1;
        }
        acc
    }
}

/// Computes the canonical compile-time XXH64 `schema_hash` of a word table from its
/// canonical layout, storage, flags, default words, word effects, string/resource masks,
/// events/inheritance metadata, and per-field `FieldKind`/offsets (independent of Rust spelling).
#[allow(clippy::too_many_arguments)]
pub const fn compute_table_schema_hash(
    name: &str,
    storage: Storage,
    flags: u8,
    defaults: &[Word],
    word_effects: &[u8],
    str_words: u32,
    resource_words: u32,
    events_word: u8,
    inherited: bool,
    fields: &[FieldDesc],
) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    h.update(b"NVE_TABLE_V4;");
    h.update(name.as_bytes());
    h.update(&[0xFF, storage as u8, flags, events_word, inherited as u8]);
    h.update_u64(defaults.len() as u64);
    let mut i = 0;
    while i < defaults.len() {
        h.update(&defaults[i].to_le_bytes());
        i += 1;
    }
    h.update(word_effects);
    h.update(&str_words.to_le_bytes());
    h.update(&resource_words.to_le_bytes());
    h.update_u64(fields.len() as u64);
    let mut j = 0;
    while j < fields.len() {
        let f = &fields[j];
        h.update(f.name.as_bytes());
        h.update(&[
            0xFF,
            f.kind as u8,
            f.opt_encoding as u8,
            f.word,
            f.shift,
            f.bits,
            f.effects,
            f.val_word,
            f.no_inherit as u8,
        ]);
        j += 1;
    }
    h.digest()
}

/// Zero-copy view of one table published in `NveSceneView::tables`.
///
/// Dense tables: `owners` is null and row `i` belongs to node index `i` (`rows` spans the store's
/// node capacity). Sparse tables: `rows[r]` belongs to `owners[r]` (raw `NodeId`), and the store's
/// own paged node-index → row index is published alongside, so a host resolves a node's row in
/// O(1) without building an owner map: node index `n` lives in page
/// `p = row_index_dir[n >> NVE_ROW_INDEX_PAGE_BITS]` (absent when `n`'s directory slot is out of
/// range or `NVE_ROW_INDEX_NO_PAGE`), whose entry `row_index_pages[p * NVE_ROW_INDEX_PAGE_LEN +
/// (n & (NVE_ROW_INDEX_PAGE_LEN - 1))]` is `row + 1` (`0` = no row).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 72)]
pub struct NveTableView {
    /// Low 32 bits of the `TableKey`.
    #[nve_abi(offset = 0)]
    pub key_lo: u32,
    /// High 32 bits of the `TableKey`.
    #[nve_abi(offset = 4)]
    pub key_hi: u32,
    /// Process-local slot (bit index in `NodeRow::tables`).
    #[nve_abi(offset = 8)]
    pub slot: u32,
    /// Words per row.
    #[nve_abi(offset = 12)]
    pub words: u32,
    /// Number of rows reachable through `rows`.
    #[nve_abi(offset = 16)]
    pub row_count: u32,
    /// Number of entries in `dirty_rows`.
    #[nve_abi(offset = 20)]
    pub dirty_count: u32,
    /// Entries in `row_index_dir` (sparse tables; `0` for dense tables).
    #[nve_abi(offset = 24)]
    pub row_index_dir_len: u32,
    /// Pages of `NVE_ROW_INDEX_PAGE_LEN` entries in `row_index_pages` (sparse tables).
    #[nve_abi(offset = 28)]
    pub row_index_page_count: u32,
    /// `row_count * words` row words.
    pub rows: *const u32,
    /// Raw `NodeId` owner per row (sparse tables) or null (dense tables).
    pub owners: *const crate::NodeId,
    /// Row indices changed this frame.
    pub dirty_rows: *const u32,
    /// Sparse tables: page of each `NVE_ROW_INDEX_PAGE_LEN` node indices, or
    /// `NVE_ROW_INDEX_NO_PAGE`. Null for dense tables (row = node index).
    pub row_index_dir: *const u32,
    /// Sparse tables: `row_index_page_count * NVE_ROW_INDEX_PAGE_LEN` entries of `row + 1`
    /// (`0` = no row), addressed through `row_index_dir`. Null for dense tables.
    pub row_index_pages: *const u32,
}
