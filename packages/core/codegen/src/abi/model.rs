//! Borrowed view-models the ABI templates render: the Core ABI, the runtime ABI, the word tables,
//! and the golden XXH64 vectors. No owned intermediate spec: every name, type, and doc string
//! is a `&'a str` into the `nve-abi` descriptors.

use nve_abi::RuntimeFnSignature;
use nve_abi::schema::{
    AbiAliasDescriptor, AbiConstDescriptor, AbiEnumDescriptor, AbiEnumVariantDescriptor,
    AbiFileDescriptor,
};
use nve_hash::{PRIME1, xxh64};

use super::layout::{RecordLayout, layout_records, layout_records_on};
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

/// The Core ABI (`nve_abi.generated.h`): app-module vtables, values, events, and the word-table
/// view every scene reader indexes.
#[derive(Debug)]
pub struct AbiModel<'a> {
    pub core: &'a AbiFileDescriptor,
    /// `NveHostStatus` (`invoke` / `channel_publish` result codes), not part of a file descriptor.
    pub host_status: &'a AbiEnumDescriptor,
    pub core_hash: U64Split,
    /// Core records with their 32-bit and 64-bit C layouts.
    pub records: Vec<RecordLayout<'a>>,
    /// Index into `records` of `NveTableView` (the scene readers' table descriptor).
    table_view: usize,
}

impl<'a> AbiModel<'a> {
    pub fn new(
        core: &'a AbiFileDescriptor,
        host_status: &'a AbiEnumDescriptor,
    ) -> Result<Self, CodegenError> {
        let records = layout_records(&[core])?;
        super::validate::abi(&[core], &records)?;
        let table_view = find_record(&records, "NveTableView")?;
        Ok(Self {
            core,
            host_status,
            core_hash: U64Split::new(nve_abi::NVE_CORE_ABI_SCHEMA_HASH),
            records,
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

/// The embedding-host runtime ABI (`nve_runtime_abi.generated.h`), rendered on top of the Core
/// header it includes: its records must not redeclare any record of `base`.
#[derive(Debug)]
pub struct RuntimeAbiModel<'a> {
    pub file: &'a AbiFileDescriptor,
    pub base_aliases: &'a [AbiAliasDescriptor],
    pub hash: U64Split,
    pub records: Vec<RecordLayout<'a>>,
    /// Host callback typedefs (`NveRuntimeWakeFn`, `NveRuntimeMeasureTextFn`).
    pub callbacks: Vec<RuntimeFn>,
    /// `NveRuntimeApi` function-pointer slots, in vtable order.
    pub slots: Vec<RuntimeFn>,
    /// Index into `records` of `NveSceneView` (the scene every host reads).
    scene_view: usize,
}

/// One runtime C signature with its validated `dart:ffi` spellings.
#[derive(Debug)]
pub struct RuntimeFn {
    pub sig: &'static RuntimeFnSignature,
    /// `dart:ffi` native function type (`ffi.Int32 Function(ffi.Pointer<ffi.Void> handle, …)`).
    pub dart_native: String,
    /// Dart-side `asFunction` type (`int Function(ffi.Pointer<ffi.Void> handle, …)`).
    pub dart: String,
}

impl core::ops::Deref for RuntimeFn {
    type Target = RuntimeFnSignature;
    fn deref(&self) -> &RuntimeFnSignature {
        self.sig
    }
}

/// The opaque handle typedef every runtime slot takes (`typedef void* NveRuntimeHandle`).
pub const RUNTIME_HANDLE: &str = "NveRuntimeHandle";

impl<'a> RuntimeAbiModel<'a> {
    /// Validates `callbacks` / `slots` (every type a known scalar, record, handle, or callback
    /// typedef; every parameter named) on top of `base`, whose records the runtime records may
    /// store by value (`NveRuntimeTextStyle::font_family: NveStrSlice`).
    pub fn new(
        file: &'a AbiFileDescriptor,
        base: &AbiModel<'a>,
        callbacks: &'static [RuntimeFnSignature],
        slots: &'static [RuntimeFnSignature],
    ) -> Result<Self, CodegenError> {
        let records = layout_records_on(&base.records, &[file])?;
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
        let names = super::spell::dart::CNames {
            records: base.records.iter().chain(&records).map(|r| r.name).chain([RUNTIME_HANDLE]).collect(),
            callbacks: callbacks
                .iter()
                .map(|c| c.name)
                .chain(CALLBACK_ALIASES)
                .chain(["NveGetInterfaceFn"])
                .collect(),
            aliases: base.core.aliases.iter().map(|a| (a.name, a.target)).collect(),
        };
        let spell = |sigs: &'static [RuntimeFnSignature]| -> Result<Vec<RuntimeFn>, CodegenError> {
            sigs.iter()
                .map(|sig| {
                    let fail = |why: String| CodegenError::validation(format!("runtime signature `{}`: {why}", sig.name));
                    Ok(RuntimeFn {
                        sig,
                        dart_native: names.c_fn(sig.ret, sig.params, false).map_err(fail)?,
                        dart: names.c_fn(sig.ret, sig.params, true).map_err(fail)?,
                    })
                })
                .collect()
        };
        let scene_view = find_record(&records, "NveSceneView")?;
        Ok(Self {
            file,
            base_aliases: base.core.aliases,
            hash: U64Split::new(nve_abi::NVE_RUNTIME_SCHEMA_HASH),
            records,
            callbacks: spell(callbacks)?,
            slots: spell(slots)?,
            scene_view,
        })
    }

    pub fn scene_view(&self) -> &RecordLayout<'a> {
        &self.records[self.scene_view]
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
