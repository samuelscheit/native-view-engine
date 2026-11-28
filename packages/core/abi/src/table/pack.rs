//! Compile-time first-fit word packer behind `#[derive(NveTable)]`: turns per-field
//! [`PackInput`]s into row words, defaults, effects, and [`FieldDesc`]s.

use super::{FieldDesc, FieldKind, OptEncoding, ValueSetDesc, Word, effect, table_flags, MAX_TABLE_WORDS, NO_VAL_WORD};

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
    /// Named values mirrored to hosts (see [`FieldDesc::values`]).
    pub values: Option<&'static ValueSetDesc>,
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
                        values: f.values,
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
                        values: f.values,
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
                    values: f.values,
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
                    values: f.values,
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
                    values: f.values,
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
                    values: f.values,
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
