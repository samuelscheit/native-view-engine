//! The word-table view-model every host table mirror renders: per-table metadata (inheritance,
//! `EventSet` word), per-field host specs (kind, `Option` encoding, wide value word, explicit
//! bit, resource kind), and the named value sets, all validated once so every host constructor
//! accepts them.

use std::collections::BTreeSet;

use nve_abi::table::{
    FieldDesc, FieldKind, MAX_TABLE_WORDS, NO_VAL_WORD, OptEncoding, Storage, TableDesc,
    ValueSetDesc, table_flags,
};
use nve_hash::Key64;

use super::model::U64Split;
use crate::error::CodegenError;

/// One word table, validated once so every host constructor accepts its field specs.
#[derive(Debug)]
pub struct TableModel<'a> {
    pub desc: &'a TableDesc,
    /// `nve.text_input` -> `TextInput` (hosts nest it under `NveTables`).
    pub ident: String,
    /// `nve.text_input` -> `TEXT_INPUT` (C macro / Java constant prefix).
    pub screaming: String,
    pub key: U64Split,
    pub schema: U64Split,
    pub fields: Vec<FieldModel<'a>>,
}

impl TableModel<'_> {
    pub fn dense(&self) -> bool {
        self.desc.storage == Storage::Dense
    }

    /// Word of the `EventSet<E>` listener mask, or [`NO_VAL_WORD`].
    pub fn events_word(&self) -> u8 {
        self.desc.events_word
    }

    /// Word 0 is the explicit-override mask (`#[nve_table(inherited)]`).
    pub fn inherited(&self) -> bool {
        self.desc.inherited
    }

    /// Resource fields with their `FlatResource::KIND`.
    pub fn resource_fields(&self) -> impl Iterator<Item = (&FieldModel<'_>, U64Split)> {
        self.fields.iter().filter_map(|f| f.resource_kind.map(|k| (f, k)))
    }
}

/// One field as every host `FieldSpec` spells it.
#[derive(Debug)]
pub struct FieldModel<'a> {
    pub desc: &'a FieldDesc,
    /// Host bit width: a two-word field (`Str`, 64-bit) is `bits = 64` in Rust but `(shift 0,
    /// bits 32)` on every host (the second word is implied by the kind).
    pub bits: u8,
    /// `OptEncoding::WideBit` value word (`word` / `shift` hold the presence bit), else
    /// [`NO_VAL_WORD`].
    pub val_word: u8,
    /// Bit of the field in an inherited table's explicit mask (word 0), else [`NO_VAL_WORD`].
    pub explicit_bit: u8,
    /// An unset value resolves from the parent row (inherited table, not `#[no_inherit]`).
    pub inherits: bool,
    /// `FlatResource::KIND` of a `Resource` field.
    pub resource_kind: Option<U64Split>,
}

impl FieldModel<'_> {
    pub fn name(&self) -> &str {
        self.desc.name
    }
}

/// Fields spanning two whole words (`{ offset, len }` / `lo, hi`).
fn two_word(k: FieldKind) -> bool {
    matches!(k, FieldKind::Str | FieldKind::U64 | FieldKind::I64 | FieldKind::F64)
}

/// Validates `tables` (unique names, keys that match their names, fields every host reader
/// accepts) and wraps them for rendering, in the given order.
pub fn tables_model<'a>(tables: &[&'a TableDesc]) -> Result<Vec<TableModel<'a>>, CodegenError> {
    let mut names = BTreeSet::new();
    let mut out = Vec::with_capacity(tables.len());
    for desc in tables {
        super::validate::table(desc)?;
        let short = desc.name.strip_prefix("nve.").unwrap_or(desc.name);
        let ident = super::spell::pascal(short);
        if !names.insert(ident.clone()) {
            return Err(table_error(desc, "duplicate table name"));
        }
        if desc.key != Key64::of(desc.name) {
            return Err(table_error(desc, "key is not `xxh64(name)`"));
        }
        let words = desc.words();
        if words == 0 || words > MAX_TABLE_WORDS || desc.word_effects.len() != words {
            return Err(table_error(desc, "word count out of range"));
        }
        check_table_words(desc, words)?;
        let fields = desc
            .fields
            .iter()
            .enumerate()
            .map(|(i, f)| field_model(desc, i, f, words))
            .collect::<Result<Vec<_>, _>>()?;
        out.push(TableModel {
            desc,
            screaming: short.replace('.', "_").to_ascii_uppercase(),
            ident,
            key: U64Split::new(desc.key.0),
            schema: U64Split::new(desc.schema_hash),
            fields,
        });
    }
    Ok(out)
}

/// The hidden words (explicit mask, `EventSet`) agree with the table flags and stay field-free.
fn check_table_words(desc: &TableDesc, words: usize) -> Result<(), CodegenError> {
    let flag = |bit: u8| desc.flags & bit != 0;
    if flag(table_flags::INHERITED) != desc.inherited {
        return Err(table_error(desc, "`inherited` disagrees with `table_flags::INHERITED`"));
    }
    let has_events = desc.events_word != NO_VAL_WORD;
    if flag(table_flags::HAS_EVENTS) != has_events || desc.events.is_some() != has_events {
        return Err(table_error(
            desc,
            "`events_word`, `events`, and `table_flags::HAS_EVENTS` disagree",
        ));
    }
    if has_events && (usize::from(desc.events_word) >= words || desc.inherited) {
        return Err(table_error(desc, "the `EventSet` word is out of range or shares word 0"));
    }
    if desc.events.is_some_and(|set| !set.flags) {
        return Err(table_error(desc, "the `EventSet` values must be a flag set"));
    }
    if desc.inherited && desc.fields.len() > 32 {
        return Err(table_error(desc, "an inherited table holds at most 32 fields"));
    }
    Ok(())
}

fn field_model<'a>(
    desc: &TableDesc,
    index: usize,
    f: &'a FieldDesc,
    words: usize,
) -> Result<FieldModel<'a>, CodegenError> {
    let bad = |why: &str| Err(table_error(desc, &format!("field `{}` {why}", f.name)));
    let (span, shape_ok) = if two_word(f.kind) {
        (2, f.shift == 0 && f.bits == 64)
    } else {
        (1, (1..=32).contains(&f.bits) && u32::from(f.shift) + u32::from(f.bits) <= 32)
    };
    if !shape_ok || usize::from(f.word) + span > words {
        return bad("does not fit its row");
    }
    let wide = f.opt_encoding == OptEncoding::WideBit;
    if wide != (f.val_word != NO_VAL_WORD)
        || wide && (f.bits != 1 || usize::from(f.val_word) >= words || f.val_word == f.word)
    {
        return bad("has a value word that disagrees with its `Option` encoding");
    }
    let hidden = [desc.inherited.then_some(0), (desc.events_word != NO_VAL_WORD).then_some(desc.events_word)];
    let touched = f.word..f.word + span as u8;
    if hidden.into_iter().flatten().any(|w| touched.contains(&w) || wide && w == f.val_word) {
        return bad("overlaps the explicit-mask or `EventSet` word");
    }
    if f.no_inherit && !desc.inherited {
        return bad("is `#[no_inherit]` on a table that does not inherit");
    }
    let resource_kind = match (f.kind, desc.resource_kind(usize::from(f.word))) {
        (FieldKind::Resource, Some(kind)) => Some(U64Split::new(kind.0)),
        (FieldKind::Resource, None) => return bad("has no `FlatResource::KIND`"),
        _ => None,
    };
    Ok(FieldModel {
        desc: f,
        bits: if two_word(f.kind) { 32 } else { f.bits },
        val_word: f.val_word,
        explicit_bit: if desc.inherited { index as u8 } else { NO_VAL_WORD },
        inherits: desc.inherited && !f.no_inherit,
        resource_kind,
    })
}

/// Distinct named value sets (`WordEnum` variants, flag bits, `EventSet` listener bits) the
/// `tables` carry, in first-use order. Sets sharing a name must be identical; names must spell
/// identifiers, every emitted `NVE_<SET>_<VALUE>` constant must be unique across all sets and
/// clear of the `NVE_TABLE_*` namespace, and every value must fit its field once encoded.
pub fn value_sets(tables: &[TableModel<'_>]) -> Result<Vec<&'static ValueSetDesc>, CodegenError> {
    let mut out: Vec<&'static ValueSetDesc> = Vec::new();
    let mut constants = BTreeSet::new();
    for t in tables {
        let fields = t.desc.fields.iter().filter_map(|f| f.values.map(|set| (set, Some(f))));
        for (set, field) in t.desc.events.map(|set| (set, None)).into_iter().chain(fields) {
            let fail = |why: String| table_error(t.desc, &format!("value set `{}`: {why}", set.name));
            // Every field that carries the set must hold every value, also when the set was
            // already emitted for a wider field.
            if let Some(f) = field
                && let Some(&(name, _)) = set.values.iter().find(|&&(_, v)| !fits(f, v))
            {
                return Err(fail(format!("value `{name}` exceeds field `{}`", f.name)));
            }
            if let Some(seen) = out.iter().find(|s| s.name == set.name) {
                if *seen != set {
                    return Err(fail("defined twice differently".into()));
                }
                continue;
            }
            check_set(set, &mut constants).map_err(fail)?;
            out.push(set);
        }
    }
    Ok(out)
}

/// `v` stored with `f`'s `Option` encoding fits `f.bits` (a wide value word holds any `u32`).
fn fits(f: &FieldDesc, v: u32) -> bool {
    let stored = match f.opt_encoding {
        OptEncoding::PlusOne => u64::from(v) + 1,
        OptEncoding::Bit0Present => (u64::from(v) << 1) | 1,
        OptEncoding::WideBit => return true,
        OptEncoding::Required | OptEncoding::ZeroNiche | OptEncoding::TagXor => u64::from(v),
    };
    f.bits >= 32 || stored >> f.bits == 0
}

/// A set's name and member spellings are identifiers unique after each host's escaping, its
/// values are distinct (single bits for a flag set), and its `NVE_<SET>_<VALUE>` constants are
/// new.
fn check_set(set: &ValueSetDesc, constants: &mut BTreeSet<String>) -> Result<(), String> {
    let ident = |s: &str| {
        s.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
            && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    };
    let prefix = super::spell::screaming(set.name);
    if !ident(set.name) || set.values.is_empty() || prefix == "TABLE" || prefix.starts_with("TABLE_") {
        return Err("needs an identifier name outside `TABLE*` and at least one value".into());
    }
    // Per-set member spellings each host renders (keyword-escaped), and the values: a collision
    // after escaping or a repeated value would not compile (Swift raw values) or would alias two
    // members.
    let (mut swift, mut dart, mut values) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for &(name, value) in set.values {
        let constant = format!("{prefix}_{}", super::spell::screaming(name));
        if !ident(name) || !constants.insert(constant.clone()) {
            return Err(format!("bad or duplicate name `{name}` (NVE_{constant})"));
        }
        if !swift.insert(super::spell::swift::variant(name)) || !dart.insert(super::spell::dart::variant(name)) {
            return Err(format!("name `{name}` collides with another member once escaped"));
        }
        if !values.insert(value) {
            return Err(format!("duplicate value {value} (`{name}`)"));
        }
        if set.flags && !value.is_power_of_two() {
            return Err(format!("flag `{name}` is not a single bit"));
        }
    }
    Ok(())
}

pub(super) fn table_error(desc: &TableDesc, why: &str) -> CodegenError {
    CodegenError::validation(format!("table `{}`: {why}", desc.name))
}

/// `FieldKind` as spelled by each host runtime's enum.
pub mod kind {
    use nve_abi::table::FieldKind;

    /// `(kind, C, Swift / Dart, TypeScript, Java)`.
    type Row = (FieldKind, &'static str, &'static str, &'static str, &'static str);

    const NAMES: [Row; 14] = [
        (FieldKind::U32, "NVE_FIELD_U32", "u32", "U32", "U32"),
        (FieldKind::I32, "NVE_FIELD_I32", "i32", "I32", "I32"),
        (FieldKind::F32, "NVE_FIELD_F32", "f32", "F32", "F32"),
        (FieldKind::Bool, "NVE_FIELD_BOOL", "bool", "Bool", "BOOL"),
        (FieldKind::Enum, "NVE_FIELD_ENUM", "enumeration", "Enum", "ENUM"),
        (FieldKind::Color, "NVE_FIELD_COLOR", "color", "Color", "COLOR"),
        (FieldKind::Length, "NVE_FIELD_LENGTH", "length", "Length", "LENGTH"),
        (FieldKind::Str, "NVE_FIELD_STR", "str", "Str", "STR"),
        (FieldKind::Resource, "NVE_FIELD_RESOURCE", "resource", "Resource", "RESOURCE"),
        (FieldKind::Node, "NVE_FIELD_NODE", "node", "Node", "NODE"),
        (FieldKind::U64, "NVE_FIELD_U64", "u64", "U64", "U64"),
        (FieldKind::I64, "NVE_FIELD_I64", "i64", "I64", "I64"),
        (FieldKind::F64, "NVE_FIELD_F64", "f64", "F64", "F64"),
        (FieldKind::GridLine, "NVE_FIELD_GRID_LINE", "gridLine", "GridLine", "GRID_LINE"),
    ];

    fn row(k: FieldKind) -> Row {
        NAMES[k as usize]
    }

    /// C `NveFieldKind` constant.
    pub fn c(k: &FieldKind) -> &'static str {
        row(*k).1
    }

    /// Swift `FieldKind` case.
    pub fn swift(k: &FieldKind) -> &'static str {
        row(*k).2
    }

    /// Java `FieldKind` constant.
    pub fn java(k: &FieldKind) -> &'static str {
        row(*k).4
    }

    /// Dart `FieldKind` value (`bool` is a keyword there).
    pub fn dart(k: &FieldKind) -> &'static str {
        match k {
            FieldKind::Bool => "boolean",
            _ => row(*k).2,
        }
    }

    /// TypeScript `FieldKind` key.
    pub fn ts(k: &FieldKind) -> &'static str {
        row(*k).3
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn names_are_indexed_by_discriminant() {
            for (i, row) in super::NAMES.iter().enumerate() {
                assert_eq!(row.0 as usize, i);
            }
        }
    }
}

/// `OptEncoding` as spelled by each host runtime's enum.
pub mod opt {
    use nve_abi::table::OptEncoding;

    /// `(encoding, C, Swift / Dart, TypeScript, Java)`.
    type Row = (OptEncoding, &'static str, &'static str, &'static str, &'static str);

    const NAMES: [Row; 6] = [
        (OptEncoding::Required, "NVE_OPT_REQUIRED", "required", "Required", "REQUIRED"),
        (OptEncoding::ZeroNiche, "NVE_OPT_ZERO_NICHE", "zeroNiche", "ZeroNiche", "ZERO_NICHE"),
        (OptEncoding::Bit0Present, "NVE_OPT_BIT0_PRESENT", "bit0Present", "Bit0Present", "BIT0_PRESENT"),
        (OptEncoding::PlusOne, "NVE_OPT_PLUS_ONE", "plusOne", "PlusOne", "PLUS_ONE"),
        (OptEncoding::TagXor, "NVE_OPT_TAG_XOR", "tagXor", "TagXor", "TAG_XOR"),
        (OptEncoding::WideBit, "NVE_OPT_WIDE_BIT", "wideBit", "WideBit", "WIDE_BIT"),
    ];

    fn row(o: OptEncoding) -> Row {
        NAMES[o as usize]
    }

    /// C `NveOptEncoding` constant.
    pub fn c(o: &OptEncoding) -> &'static str {
        row(*o).1
    }

    /// Swift / Dart `OptEncoding` case.
    pub fn swift(o: &OptEncoding) -> &'static str {
        row(*o).2
    }

    /// Java `OptEncoding` constant.
    pub fn java(o: &OptEncoding) -> &'static str {
        row(*o).4
    }

    /// TypeScript `OptEncoding` key.
    pub fn ts(o: &OptEncoding) -> &'static str {
        row(*o).3
    }

    #[cfg(test)]
    mod tests {
        #[test]
        fn names_are_indexed_by_discriminant() {
            for (i, row) in super::NAMES.iter().enumerate() {
                assert_eq!(row.0 as usize, i);
            }
        }
    }
}
