//! Word-table schema ABI shared by the Rust `NodeStore`, the `NVE2` wire and every foreign host.
//!
//! A table is a fixed row of `<= 32` little-endian `u32` words per node. Every word belongs to
//! exactly one effect class so a write marks precisely the pipeline stages it invalidates. Hosts
//! bind a [`TableKey`] once and read rows zero-copy through [`NveTableView`] using generated
//! word offsets.

use nve_abi_macros::NveAbi;
use nve_hash::{Key64, Xxh64Hasher};

/// One row word.
pub type Word = u32;

/// Maximum words per table row (the per-row dirty mask is one `u32`).
pub const MAX_TABLE_WORDS: usize = 32;

/// Maximum number of table slots per process (`NodeRow::tables` is a `u128`).
pub const MAX_TABLE_SLOTS: usize = 128;

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
    };
}

/// Storage shape of a field fed to [`pack_table`].
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PackKind {
    /// Packed scalar occupying `bits` (`1..=32`) inside one word.
    Scalar = 0,
    /// Wide `Option<T>`: 1-bit presence flag packed into an effect-matched bitfield word + 1 whole value word.
    WideOpt = 1,
    /// Two consecutive whole words (`u64` / `i64` / `f64`).
    Wide64 = 2,
    /// Two consecutive words `{ offset, len }` into the text arena.
    Str = 3,
    /// One word holding a `ResourceId`.
    Resource = 4,
}

/// Compile-time input record for one field passed to [`pack_table`].
#[derive(Debug, Clone, Copy)]
pub struct PackInput {
    pub name: &'static str,
    pub field_kind: FieldKind,
    pub opt_encoding: OptEncoding,
    pub pack_kind: PackKind,
    pub bits: u8,
    pub effects: u8,
    pub default_lo: u32,
    pub default_hi: u32,
    pub default_present: bool,
    pub no_inherit: bool,
}

/// Compile-time output of [`pack_table`] (first-fit bin-packing over effect-matched row words).
#[derive(Debug, Clone, Copy)]
pub struct PackLayout<const N: usize> {
    pub word_count: usize,
    pub defaults: [Word; MAX_TABLE_WORDS],
    pub word_effects: [u8; MAX_TABLE_WORDS],
    pub str_words: u32,
    pub resource_words: u32,
    pub events_word: u8,
    pub fields: [FieldDesc; N],
}

/// First-fit compile-time word-table packer.
///
/// Packs sub-word scalar fields and wide-`Option` presence bits into the first open bitfield word
/// with matching `effects` that has enough free bits, keeping total row word count `<= 32`.
pub const fn pack_table<const N: usize>(
    inputs: [PackInput; N],
    inherited: bool,
    has_events: bool,
) -> PackLayout<N> {
    assert!(
        !(inherited && has_events),
        "NveTable cannot combine `inherited` and `events`"
    );
    if inherited {
        assert!(
            N <= MAX_TABLE_WORDS,
            "inherited NveTable supports at most 32 fields (u32 explicit mask)"
        );
    }

    let mut defaults = [0u32; MAX_TABLE_WORDS];
    let mut word_effects = [0u8; MAX_TABLE_WORDS];
    let mut open_used = [0u8; MAX_TABLE_WORDS];
    let mut open_fx = [0u8; MAX_TABLE_WORDS];
    let mut str_words = 0u32;
    let mut resource_words = 0u32;
    let mut fields = [FieldDesc::EMPTY; N];
    let mut next_word = 0usize;
    let mut events_word = NO_VAL_WORD;

    if inherited {
        // Hidden word 0 holds `__explicit: u32` with zero word_effects (per-field words carry effects).
        next_word = 1;
    }
    if has_events {
        events_word = next_word as u8;
        word_effects[next_word] = effect::EVENTS | effect::REALIZE;
        next_word += 1;
    }

    let mut i = 0usize;
    while i < N {
        let f = inputs[i];
        if !inherited {
            assert!(
                !f.no_inherit,
                "FieldDesc::no_inherit is only valid on inherited tables"
            );
        }
        match f.pack_kind {
            PackKind::Scalar => {
                assert!(f.bits >= 1 && f.bits <= 32, "scalar field bits must be 1..=32");
                assert!(
                    !f.default_present,
                    "PackInput::default_present is only valid for PackKind::WideOpt"
                );
                if f.bits == 32 {
                    assert!(next_word < MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                    let w = next_word;
                    next_word += 1;
                    word_effects[w] = f.effects;
                    defaults[w] = f.default_lo;
                    fields[i] = FieldDesc {
                        name: f.name,
                        kind: f.field_kind,
                        opt_encoding: f.opt_encoding,
                        word: w as u8,
                        shift: 0,
                        bits: 32,
                        effects: f.effects,
                        val_word: NO_VAL_WORD,
                        no_inherit: f.no_inherit,
                    };
                } else {
                    let mut chosen = MAX_TABLE_WORDS;
                    let mut scan = 0usize;
                    while scan < next_word {
                        if open_used[scan] > 0
                            && open_fx[scan] == f.effects
                            && (open_used[scan] as usize) + (f.bits as usize) <= 32
                        {
                            chosen = scan;
                            break;
                        }
                        scan += 1;
                    }
                    let (w, shift) = if chosen < MAX_TABLE_WORDS {
                        let sh = open_used[chosen];
                        open_used[chosen] = sh + f.bits;
                        (chosen, sh)
                    } else {
                        assert!(next_word < MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                        let nw = next_word;
                        next_word += 1;
                        open_used[nw] = f.bits;
                        open_fx[nw] = f.effects;
                        word_effects[nw] = f.effects;
                        (nw, 0u8)
                    };
                    let mask = ((1u32 << f.bits) - 1) << shift;
                    defaults[w] |= (f.default_lo << shift) & mask;
                    fields[i] = FieldDesc {
                        name: f.name,
                        kind: f.field_kind,
                        opt_encoding: f.opt_encoding,
                        word: w as u8,
                        shift,
                        bits: f.bits,
                        effects: f.effects,
                        val_word: NO_VAL_WORD,
                        no_inherit: f.no_inherit,
                    };
                }
            }
            PackKind::WideOpt => {
                // 1-bit presence flag first-fit into an open bitfield word of matching effect class.
                let mut chosen = MAX_TABLE_WORDS;
                let mut scan = 0usize;
                while scan < next_word {
                    if open_used[scan] > 0
                        && open_fx[scan] == f.effects
                        && (open_used[scan] as usize) < 32
                    {
                        chosen = scan;
                        break;
                    }
                    scan += 1;
                }
                let (p_word, p_shift) = if chosen < MAX_TABLE_WORDS {
                    let sh = open_used[chosen];
                    open_used[chosen] = sh + 1;
                    (chosen, sh)
                } else {
                    assert!(next_word < MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                    let nw = next_word;
                    next_word += 1;
                    open_used[nw] = 1;
                    open_fx[nw] = f.effects;
                    word_effects[nw] = f.effects;
                    (nw, 0u8)
                };
                // Full 32-bit value word.
                assert!(next_word < MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                let v_word = next_word;
                next_word += 1;
                word_effects[v_word] = f.effects;
                if f.default_present {
                    defaults[p_word] |= 1u32 << p_shift;
                    defaults[v_word] = f.default_lo;
                }
                fields[i] = FieldDesc {
                    name: f.name,
                    kind: f.field_kind,
                    opt_encoding: f.opt_encoding,
                    word: p_word as u8,
                    shift: p_shift,
                    bits: 1,
                    effects: f.effects,
                    val_word: v_word as u8,
                    no_inherit: f.no_inherit,
                };
            }
            PackKind::Wide64 => {
                assert!(
                    !f.default_present,
                    "PackInput::default_present is only valid for PackKind::WideOpt"
                );
                assert!(next_word + 2 <= MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                let w = next_word;
                next_word += 2;
                word_effects[w] = f.effects;
                word_effects[w + 1] = f.effects;
                defaults[w] = f.default_lo;
                defaults[w + 1] = f.default_hi;
                fields[i] = FieldDesc {
                    name: f.name,
                    kind: f.field_kind,
                    opt_encoding: f.opt_encoding,
                    word: w as u8,
                    shift: 0,
                    bits: 64,
                    effects: f.effects,
                    val_word: NO_VAL_WORD,
                    no_inherit: f.no_inherit,
                };
            }
            PackKind::Str => {
                assert!(
                    !f.default_present,
                    "PackInput::default_present is only valid for PackKind::WideOpt"
                );
                assert!(next_word + 2 <= MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                let w = next_word;
                next_word += 2;
                str_words |= 1u32 << w;
                word_effects[w] = f.effects;
                word_effects[w + 1] = f.effects;
                fields[i] = FieldDesc {
                    name: f.name,
                    kind: FieldKind::Str,
                    opt_encoding: OptEncoding::Required,
                    word: w as u8,
                    shift: 0,
                    bits: 64,
                    effects: f.effects,
                    val_word: NO_VAL_WORD,
                    no_inherit: f.no_inherit,
                };
            }
            PackKind::Resource => {
                assert!(
                    !f.default_present,
                    "PackInput::default_present is only valid for PackKind::WideOpt"
                );
                assert!(next_word < MAX_TABLE_WORDS, "table exceeds 32 words; split it");
                let w = next_word;
                next_word += 1;
                resource_words |= 1u32 << w;
                word_effects[w] = f.effects;
                fields[i] = FieldDesc {
                    name: f.name,
                    kind: FieldKind::Resource,
                    opt_encoding: OptEncoding::Required,
                    word: w as u8,
                    shift: 0,
                    bits: 32,
                    effects: f.effects,
                    val_word: NO_VAL_WORD,
                    no_inherit: f.no_inherit,
                };
            }
        }
        i += 1;
    }

    assert!(next_word >= 1 && next_word <= MAX_TABLE_WORDS, "table must have 1..=32 words");

    PackLayout {
        word_count: next_word,
        defaults,
        word_effects,
        str_words,
        resource_words,
        events_word,
        fields,
    }
}

/// Copies the leading `W` words of a `pack_table` output into an exact-length const array.
pub const fn trim_words<const W: usize>(src: &[Word; MAX_TABLE_WORDS]) -> [Word; W] {
    let mut out = [0u32; W];
    let mut i = 0;
    while i < W {
        out[i] = src[i];
        i += 1;
    }
    out
}

/// Copies the leading `W` effect bytes of a `pack_table` output into an exact-length const array.
pub const fn trim_bytes<const W: usize>(src: &[u8; MAX_TABLE_WORDS]) -> [u8; W] {
    let mut out = [0u8; W];
    let mut i = 0;
    while i < W {
        out[i] = src[i];
        i += 1;
    }
    out
}

/// Returns `flags | table_flags::AFFECTS_LAYOUT` when any word carries layout/measure effects.
pub const fn finalize_table_flags(base_flags: u8, word_effects: &[u8]) -> u8 {
    let mut i = 0;
    let mut out = base_flags;
    while i < word_effects.len() {
        if word_effects[i] & (effect::LAYOUT | effect::MEASURE) != 0 {
            out |= table_flags::AFFECTS_LAYOUT;
            break;
        }
        i += 1;
    }
    out
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
/// node capacity). Sparse tables: `rows[r]` belongs to `owners[r]` (raw `NodeId`).
#[repr(C)]
#[derive(Debug, Clone, Copy, NveAbi)]
#[nve_abi(size_64 = 48)]
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
    /// `row_count * words` row words.
    pub rows: *const u32,
    /// Raw `NodeId` owner per row (sparse tables) or null (dense tables).
    pub owners: *const crate::NodeId,
    /// Row indices changed this frame.
    pub dirty_rows: *const u32,
}
