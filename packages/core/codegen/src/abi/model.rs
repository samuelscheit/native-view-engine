//! Borrowed view-models the ABI templates render: the Core ABI, the runtime ABI, the word tables,
//! and the golden XXH64 vectors. No owned intermediate spec: every name, type, and doc string
//! is a `&'a str` into the `nve-abi` descriptors.

use nve_abi::schema::{
    AbiAliasDescriptor, AbiConstDescriptor, AbiEnumDescriptor, AbiEnumVariantDescriptor,
    AbiFileDescriptor, AbiMethodDescriptor, AbiVtableDescriptor,
};
use nve_hash::{PRIME1, xxh64};

use super::layout::{RecordLayout, layout_records, layout_records_on};
use crate::error::CodegenError;

/// Empty alias slice for Askama templates that need an explicitly typed `&[AbiAliasDescriptor]`.
pub const NO_ALIASES: &[AbiAliasDescriptor] = &[];

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

/// The Core ABI (`nve_abi.generated.h`): app-module vtables, values, events, and the word-table
/// view every scene reader indexes.
#[derive(Debug)]
pub struct AbiModel<'a> {
    pub core: &'a AbiFileDescriptor,
    pub core_hash: U64Split,
    /// Core records with their 32-bit and 64-bit C layouts.
    pub records: Vec<RecordLayout<'a>>,
    /// Declared Core callback typedef names (`NveHostInvokeFn`, `NveHostChannelPublishFn`, `NveGetInterfaceFn`).
    callbacks: Vec<&'a str>,
    /// Index into `records` of `NveTableView` (the scene readers' table descriptor).
    table_view: usize,
}

impl<'a> AbiModel<'a> {
    pub fn new(core: &'a AbiFileDescriptor) -> Result<Self, CodegenError> {
        let records = layout_records(&[core])?;
        super::validate::abi(&[core], &records)?;
        let table_view = find_record(&records, "NveTableView")?;
        let callbacks = core.callbacks.iter().map(|c| c.name).collect();
        Ok(Self {
            core,
            core_hash: U64Split::new(nve_abi::NVE_CORE_ABI_SCHEMA_HASH),
            records,
            callbacks,
            table_view,
        })
    }

    pub fn table_view(&self) -> &RecordLayout<'a> {
        &self.records[self.table_view]
    }

    pub fn consts(&self) -> impl Iterator<Item = &'a AbiConstDescriptor> + use<'a> {
        self.core.consts.iter()
    }

    pub fn enums(&self) -> impl Iterator<Item = &'a AbiEnumDescriptor> + use<'a> {
        self.core.enums.iter()
    }

    pub fn all_aliases(&self) -> &'a [AbiAliasDescriptor] {
        self.core.aliases
    }

    pub fn all_callbacks(&self) -> &[&'a str] {
        &self.callbacks
    }

    /// Modular `nve_get_interface` vtables (`NveSurfaceApi..NveHostApi`).
    pub fn modules(&self) -> impl Iterator<Item = &'a AbiVtableDescriptor> + use<'a> {
        self.core
            .vtables
            .iter()
            .filter(|v| v.interface_id.is_some())
    }
}

/// The embedding-host runtime ABI (`nve_runtime_abi.generated.h`), rendered on top of the Core
/// header it includes: its records must not redeclare any record of `base`.
#[derive(Debug)]
pub struct RuntimeAbiModel<'a> {
    pub file: &'a AbiFileDescriptor,
    pub hash: U64Split,
    pub records: Vec<RecordLayout<'a>>,
    /// Combined Core + Runtime aliases for C & Dart type resolution.
    all_aliases: Vec<AbiAliasDescriptor>,
    /// Combined Core + Runtime callback names for Dart `NativeFunction` spelling.
    all_callbacks: Vec<&'a str>,
    /// Index into `file.vtables` of `NveRuntimeApi`.
    vtable: usize,
    /// Index into `records` of `NveSceneView` (the scene every host reads).
    scene_view: usize,
}

impl<'a> RuntimeAbiModel<'a> {
    /// Validates `file` (every record, alias, callback, and vtable slot) on top of `base`, whose
    /// records/aliases/callbacks the runtime ABI may reference.
    pub fn new(file: &'a AbiFileDescriptor, base: &AbiModel<'a>) -> Result<Self, CodegenError> {
        let records = layout_records_on(&base.records, &[base.core], &[file])?;
        super::validate::abi_on(&[base.core], &base.records, &[file], &records)?;
        if let Some(r) = records
            .iter()
            .find(|r| base.records.iter().any(|b| b.name == r.name))
        {
            return Err(CodegenError::validation(format!(
                "the runtime ABI redeclares `{}`, which `nve_abi.generated.h` already declares",
                r.name
            )));
        }
        let vtable = file
            .vtables
            .iter()
            .position(|v| v.name == "NveRuntimeApi")
            .ok_or_else(|| {
                CodegenError::validation("the runtime ABI declares no `NveRuntimeApi` vtable")
            })?;
        let scene_view = find_record(&records, "NveSceneView")?;
        let mut all_aliases = Vec::with_capacity(base.core.aliases.len() + file.aliases.len());
        all_aliases.extend_from_slice(base.core.aliases);
        all_aliases.extend_from_slice(file.aliases);
        let all_callbacks = base
            .core
            .callbacks
            .iter()
            .chain(file.callbacks)
            .map(|c| c.name)
            .collect();
        Ok(Self {
            file,
            hash: U64Split::new(nve_abi::NVE_RUNTIME_SCHEMA_HASH),
            records,
            all_aliases,
            all_callbacks,
            vtable,
            scene_view,
        })
    }

    pub fn scene_view(&self) -> &RecordLayout<'a> {
        &self.records[self.scene_view]
    }

    pub fn vtable(&self) -> &'a AbiVtableDescriptor {
        &self.file.vtables[self.vtable]
    }

    pub fn slots(&self) -> &'a [AbiMethodDescriptor] {
        self.vtable().methods
    }

    pub fn all_aliases(&self) -> &[AbiAliasDescriptor] {
        &self.all_aliases
    }

    pub fn all_callbacks(&self) -> &[&'a str] {
        &self.all_callbacks
    }
}

/// Module name of a modular vtable (`module_name`, falling back to `name`).
pub fn module_name(v: &AbiVtableDescriptor) -> &str {
    v.module_name.unwrap_or(v.name)
}

/// Interface ID integer of a modular vtable.
pub fn interface_id(v: &AbiVtableDescriptor) -> u32 {
    v.interface_id.map_or(0, |id| id.raw())
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

/// Index of the record called `name` (a codegen error when the ABI does not declare it).
fn find_record(records: &[RecordLayout<'_>], name: &str) -> Result<usize, CodegenError> {
    records
        .iter()
        .position(|r| r.name == name)
        .ok_or_else(|| CodegenError::validation(format!("the ABI declares no record `{name}`")))
}
