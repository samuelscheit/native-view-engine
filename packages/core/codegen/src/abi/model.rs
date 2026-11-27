//! Borrowed view-models the ABI templates render: the merged Core + Scene ABI, the word tables,
//! and the golden XXH64 vectors. No owned intermediate spec: every name, type, and doc string
//! is a `&'a str` into the `nve-abi` descriptors.

use std::collections::BTreeSet;

use nve_abi::schema::{
    AbiAliasDescriptor, AbiConstDescriptor, AbiEnumDescriptor, AbiEnumVariantDescriptor,
    AbiFileDescriptor,
};
use nve_abi::table::{FieldDesc, FieldKind, MAX_TABLE_WORDS, Storage, TableDesc};
use nve_hash::{Key64, PRIME1, xxh64};

use super::layout::{RecordLayout, layout_records};
use crate::error::CodegenError;

/// A 64-bit value with its `(lo, hi)` `u32` halves: Dart / TypeScript spell only the halves, so
/// no integer literal above 2^53 ever reaches a JavaScript number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct U64Split {
    pub value: u64,
    pub lo: u32,
    pub hi: u32,
}

impl U64Split {
    pub const fn new(value: u64) -> Self {
        Self {
            value,
            lo: value as u32,
            hi: (value >> 32) as u32,
        }
    }

    /// `0x0123456789ABCDEF`
    pub fn hex(&self) -> HexLiteral {
        HexLiteral {
            value: self.value,
            digits: 16,
        }
    }

    /// `0x89ABCDEF`
    pub fn lo_hex(&self) -> HexLiteral {
        HexLiteral {
            value: u64::from(self.lo),
            digits: 8,
        }
    }

    /// `0x01234567`
    pub fn hi_hex(&self) -> HexLiteral {
        HexLiteral {
            value: u64::from(self.hi),
            digits: 8,
        }
    }
}

/// An upper-case `0x...` literal of fixed width, written straight into the template output.
#[derive(Debug, Clone, Copy)]
pub struct HexLiteral {
    value: u64,
    digits: usize,
}

impl std::fmt::Display for HexLiteral {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "0x{:0width$X}", self.value, width = self.digits)
    }
}

/// Aliases whose C type is a callback signature `NveAbi` does not describe: the templates spell
/// their function-pointer types by hand.
pub const CALLBACK_ALIASES: [&str; 2] = ["NveHostInvokeFn", "NveHostChannelPublishFn"];

/// The merged Core + Scene ABI.
#[derive(Debug)]
pub struct AbiModel<'a> {
    pub core: &'a AbiFileDescriptor,
    pub scene: &'a AbiFileDescriptor,
    /// `NveHostStatus` (`invoke` / `channel_publish` result codes), not part of a file descriptor.
    pub host_status: &'a AbiEnumDescriptor,
    pub core_hash: U64Split,
    pub scene_hash: U64Split,
    /// Core records, then Scene records, with their 32-bit and 64-bit C layouts.
    pub records: Vec<RecordLayout<'a>>,
    /// Indices into `records` of the records the scene readers are built around.
    scene_view: usize,
    table_view: usize,
    render_node: usize,
}

impl<'a> AbiModel<'a> {
    pub fn new(
        core: &'a AbiFileDescriptor,
        scene: &'a AbiFileDescriptor,
        host_status: &'a AbiEnumDescriptor,
    ) -> Result<Self, CodegenError> {
        let records = layout_records(&[core, scene])?;
        super::validate::abi(&[core, scene], &records)?;
        let find = |name: &str| {
            records.iter().position(|r| r.name == name).ok_or_else(|| {
                CodegenError::validation(format!("the ABI declares no record `{name}`"))
            })
        };
        let (scene_view, table_view, render_node) = (
            find("NveSceneView")?,
            find("NveTableView")?,
            find("NveRenderNode")?,
        );
        Ok(Self {
            core,
            scene,
            host_status,
            core_hash: U64Split::new(nve_abi::NVE_CORE_ABI_SCHEMA_HASH),
            scene_hash: U64Split::new(nve_abi::NVE_SCENE_SCHEMA_HASH),
            records,
            scene_view,
            table_view,
            render_node,
        })
    }

    pub fn scene_view(&self) -> &RecordLayout<'a> {
        &self.records[self.scene_view]
    }

    pub fn table_view(&self) -> &RecordLayout<'a> {
        &self.records[self.table_view]
    }

    pub fn render_node(&self) -> &RecordLayout<'a> {
        &self.records[self.render_node]
    }

    pub fn consts(&self) -> impl Iterator<Item = &'a AbiConstDescriptor> + use<'a> {
        self.core.consts.iter().chain(self.scene.consts)
    }

    pub fn enums(&self) -> impl Iterator<Item = &'a AbiEnumDescriptor> + use<'a> {
        self.core
            .enums
            .iter()
            .chain(std::iter::once(self.host_status))
    }

    /// Aliases that are plain types (handles), not callback signatures.
    pub fn value_aliases(&self) -> impl Iterator<Item = &'a AbiAliasDescriptor> + use<'a> {
        self.core
            .aliases
            .iter()
            .filter(|a| !CALLBACK_ALIASES.contains(&a.name))
    }
}

/// The embedding-host runtime ABI (`nve_runtime_abi.generated.h`), rendered on top of the Core +
/// Scene header it includes: its records must not redeclare any record of `base`.
#[derive(Debug)]
pub struct RuntimeAbiModel<'a> {
    pub file: &'a AbiFileDescriptor,
    pub hash: U64Split,
    pub records: Vec<RecordLayout<'a>>,
}

impl<'a> RuntimeAbiModel<'a> {
    pub fn new(file: &'a AbiFileDescriptor, base: &AbiModel<'_>) -> Result<Self, CodegenError> {
        let records = layout_records(&[file])?;
        super::validate::abi(&[file], &records)?;
        if let Some(r) = records
            .iter()
            .find(|r| base.records.iter().any(|b| b.name == r.name))
        {
            return Err(CodegenError::validation(format!(
                "the runtime ABI redeclares `{}`, which `nve_abi.generated.h` already declares",
                r.name
            )));
        }
        Ok(Self {
            file,
            hash: U64Split::new(nve_abi::NVE_RUNTIME_SCHEMA_HASH),
            records,
        })
    }
}

/// C constant prefix of an enum's variants (`c_prefix`, else `SCREAMING_NAME_`).
pub fn c_enum_prefix(e: &AbiEnumDescriptor) -> String {
    e.c_prefix.map_or_else(
        || format!("{}_", super::spell::screaming(e.name)),
        str::to_owned,
    )
}

/// `true` for the variant Dart's `fromCode` maps every unknown code to.
pub fn is_dart_fallback(e: &AbiEnumDescriptor, v: &AbiEnumVariantDescriptor) -> bool {
    e.dart_fallback == Some(v.name)
}

/// [`c_enum_prefix`] without the `NVE_` namespace (`NVE_VAL_` -> `VAL_`).
pub fn bare_enum_prefix(e: &AbiEnumDescriptor) -> String {
    let prefix = c_enum_prefix(e);
    prefix.strip_prefix("NVE_").unwrap_or(&prefix).to_owned()
}

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
}

impl TableModel<'_> {
    pub fn dense(&self) -> bool {
        self.desc.storage == Storage::Dense
    }
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
        for f in desc.fields {
            check_field(desc, f, words)?;
        }
        out.push(TableModel {
            desc,
            screaming: short.replace('.', "_").to_ascii_uppercase(),
            ident,
            key: U64Split::new(desc.key.0),
            schema: U64Split::new(desc.schema_hash),
        });
    }
    Ok(out)
}

/// Bit width as the host `FieldSpec`s spell it: a two-word `Str` is `bits = 64` in Rust but
/// `(shift 0, bits 32)` on every host (the second word is implied by the kind).
pub fn host_bits(f: &FieldDesc) -> u8 {
    if f.kind == FieldKind::Str { 32 } else { f.bits }
}

fn check_field(desc: &TableDesc, f: &FieldDesc, words: usize) -> Result<(), CodegenError> {
    let (span, shape_ok) = match f.kind {
        FieldKind::Str => (2, f.shift == 0 && f.bits == 64),
        _ => (
            1,
            (1..=32).contains(&f.bits) && u32::from(f.shift) + u32::from(f.bits) <= 32,
        ),
    };
    if !shape_ok || usize::from(f.word) + span > words {
        return Err(table_error(
            desc,
            &format!("field `{}` does not fit its row", f.name),
        ));
    }
    Ok(())
}

pub(super) fn table_error(desc: &TableDesc, why: &str) -> CodegenError {
    CodegenError::validation(format!("table `{}`: {why}", desc.name))
}

/// `FieldKind` as spelled by each host runtime's enum.
pub mod kind {
    use nve_abi::table::FieldKind;

    /// `(kind, C, Swift, TypeScript, Java)`.
    type Row = (
        FieldKind,
        &'static str,
        &'static str,
        &'static str,
        &'static str,
    );

    const NAMES: [Row; 10] = [
        (FieldKind::U32, "NVE_FIELD_U32", "u32", "U32", "U32"),
        (FieldKind::I32, "NVE_FIELD_I32", "i32", "I32", "I32"),
        (FieldKind::F32, "NVE_FIELD_F32", "f32", "F32", "F32"),
        (FieldKind::Bool, "NVE_FIELD_BOOL", "bool", "Bool", "BOOL"),
        (
            FieldKind::Enum,
            "NVE_FIELD_ENUM",
            "enumeration",
            "Enum",
            "ENUM",
        ),
        (
            FieldKind::Color,
            "NVE_FIELD_COLOR",
            "color",
            "Color",
            "COLOR",
        ),
        (
            FieldKind::Length,
            "NVE_FIELD_LENGTH",
            "length",
            "Length",
            "LENGTH",
        ),
        (FieldKind::Str, "NVE_FIELD_STR", "str", "Str", "STR"),
        (
            FieldKind::Resource,
            "NVE_FIELD_RESOURCE",
            "resource",
            "Resource",
            "RESOURCE",
        ),
        (FieldKind::Node, "NVE_FIELD_NODE", "node", "Node", "NODE"),
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

/// One golden XXH64 vector: every host suite hashes `bytes` with `seed` and expects `expected`.
#[derive(Debug, Clone)]
pub struct HashVector {
    pub label: &'static str,
    pub bytes: Vec<u8>,
    /// Lowercase hex of `bytes`, computed once (hosts decode it; no language needs a large array
    /// literal).
    pub hex: String,
    pub seed: U64Split,
    pub expected: U64Split,
}

fn lower_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(2 * bytes.len());
    for &b in bytes {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 0xF)]));
    }
    out
}

/// Longest vector input in bytes (C suites size their scratch buffers with it).
pub fn max_len(vectors: &[HashVector]) -> usize {
    vectors.iter().map(|v| v.bytes.len()).max().unwrap_or(0)
}

/// `NveWireFrame` v2 trailer seed (`"NVEWIRE2"`).
const WIRE_SEED: u64 = 0x4E56_4557_4952_4532;
/// `PRIME32_1`, as a 64-bit seed.
const PRIME32_SEED: u64 = 0x9E37_79B1;

/// The golden vectors: every tail length branch, multi-stripe input, non-ASCII UTF-8, the
/// official Cyan4973 sanity buffer, and seeds with the high bit set.
pub fn hash_vectors() -> Vec<HashVector> {
    let seq = |n: usize| (0..n).map(|i| (i * 31 + 7) as u8).collect::<Vec<u8>>();
    let cyan = |n: usize| {
        let mut state: u64 = PRIME32_SEED;
        (0..n)
            .map(|_| {
                let b = (state >> 56) as u8;
                state = state.wrapping_mul(PRIME1);
                b
            })
            .collect::<Vec<u8>>()
    };
    let text = |s: &str| s.as_bytes().to_vec();
    let vectors: [(&'static str, Vec<u8>, u64); 19] = [
        ("empty", Vec::new(), 0),
        ("a", text("a"), 0),
        ("abc", text("abc"), 0),
        ("nve.visual", text("nve.visual"), 0),
        ("high_bits", vec![0x00, 0x01, 0x7F, 0x80, 0xFF], 0),
        ("utf8", text("héllo wörld — 日本語 🚀"), 0),
        ("seq101", seq(101), 0),
        ("seq101_wire", seq(101), WIRE_SEED),
        ("seq37_prime1", seq(37), PRIME1),
        ("seq1024", seq(1024), 0),
        ("cyan1", cyan(1), 0),
        ("cyan4", cyan(4), 0),
        ("cyan14", cyan(14), 0),
        ("cyan222", cyan(222), 0),
        ("cyan1024", cyan(1024), 0),
        ("cyan1_p32", cyan(1), PRIME32_SEED),
        ("cyan222_p32", cyan(222), PRIME32_SEED),
        ("cyan32_p1", cyan(32), PRIME1),
        ("cyan1024_p1", cyan(1024), PRIME1),
    ];
    vectors
        .into_iter()
        .map(|(label, bytes, seed)| HashVector {
            label,
            hex: lower_hex(&bytes),
            expected: U64Split::new(xxh64(&bytes, seed)),
            seed: U64Split::new(seed),
            bytes,
        })
        .collect()
}
