//! Drift guard for the static host runtimes (`packages/core/host-{cxx,swift,java,dart,ts}`).
//!
//! Every constant those packages mirror by hand — host / hybrid status codes, `FieldKind`, the
//! `NveBinValue` tags, max nesting depth, max table words, and `Length` unit tags — is rendered
//! here from the Rust source of truth (`nve_abi`, including `nve_abi::hybrid::NveHybridStatus::ALL`,
//! and `nve_store`) and must appear verbatim in each language. Their golden XXH64 vectors are
//! generated from `nve_hash` by `nve codegen abi` instead, so no host carries a hand-copied
//! digest. Matching ignores case, whitespace, and `_`, so `InternalError = -8`,
//! `INTERNAL_ERROR(-8)`, and `internalError(-8)` are the same entry.

use nve_abi::hybrid::NveHybridStatus;
use nve_abi::table::{FieldKind, MAX_TABLE_WORDS};
use nve_abi::{NVE_BIN_VALUE_MAX_DEPTH, NveHostStatus, NveValueTag};
use nve_store::Length;

use crate::{RepoIndex, Violation};

pub mod decl;

use decl::Syntax;

const CXX: &str = "packages/core/host-cxx/include/nve/host/";
const SWIFT: &str = "packages/core/host-swift/Sources/NveHost/";
const JAVA: &str = "packages/core/host-java/src/main/java/dev/nve/host/";
const DART: &str = "packages/core/host-dart/lib/src/";
const TS: &str = "packages/core/host-ts/src/";

/// One mirror site: a file, the declaration that opens the block, its syntax, the C prefix its
/// names carry (if any), and per-language renames of Rust variant names.
struct Site {
    file: String,
    marker: &'static str,
    syntax: Syntax,
    prefix: &'static str,
    renames: &'static [(&'static str, &'static str)],
}

fn site(dir: &str, file: &str, marker: &'static str, syntax: Syntax) -> Site {
    Site {
        file: format!("{dir}{file}"),
        marker,
        syntax,
        prefix: "",
        renames: &[],
    }
}

/// Each site's declarations must equal `entries` exactly (see [`decl::check_block`]).
fn check_set(
    idx: &RepoIndex,
    set: &str,
    entries: &[(String, i64)],
    sites: &[Site],
    out: &mut Vec<Violation>,
) {
    for s in sites {
        let Some(text) = idx.read_text(&s.file) else {
            out.push(Violation::new(
                &s.file,
                0,
                set,
                "host runtime mirror file is missing",
            ));
            continue;
        };
        let Some(body) = decl::code_block(&text, s.marker) else {
            out.push(Violation::new(
                &s.file,
                0,
                s.marker,
                format!("`{set}` declaration not found"),
            ));
            continue;
        };
        for problem in decl::check_block(entries, s.syntax, s.prefix, s.renames, &body) {
            out.push(Violation::new(
                &s.file,
                0,
                s.marker,
                format!("`{set}` drifted from its Rust source of truth: {problem}"),
            ));
        }
    }
}

const fn host_status_name(s: NveHostStatus) -> &'static str {
    match s {
        NveHostStatus::Ok => "Ok",
        NveHostStatus::Pending => "Pending",
        NveHostStatus::BufferTooSmall => "BufferTooSmall",
        NveHostStatus::Unsupported => "Unsupported",
        NveHostStatus::PermissionDenied => "PermissionDenied",
        NveHostStatus::InvalidArg => "InvalidArg",
        NveHostStatus::NotFound => "NotFound",
        NveHostStatus::Cancelled => "Cancelled",
        NveHostStatus::IoError => "IoError",
        NveHostStatus::InternalError => "InternalError",
    }
}

const fn field_kind_name(k: FieldKind) -> &'static str {
    match k {
        FieldKind::U32 => "U32",
        FieldKind::I32 => "I32",
        FieldKind::F32 => "F32",
        FieldKind::Bool => "Bool",
        FieldKind::Enum => "Enum",
        FieldKind::Color => "Color",
        FieldKind::Length => "Length",
        FieldKind::Str => "Str",
        FieldKind::Resource => "Resource",
        FieldKind::Node => "Node",
        FieldKind::U64 => "U64",
        FieldKind::I64 => "I64",
        FieldKind::F64 => "F64",
        FieldKind::GridLine => "GridLine",
    }
}

const fn value_tag_name(t: NveValueTag) -> &'static str {
    match t {
        NveValueTag::Null => "Null",
        NveValueTag::Bool => "Bool",
        NveValueTag::I64 => "I64",
        NveValueTag::F64 => "F64",
        NveValueTag::Str => "Str",
        NveValueTag::Bytes => "Bytes",
        NveValueTag::Array => "Array",
        NveValueTag::Map => "Map",
    }
}

/// Every `NveHostStatus` (adding a variant fails `host_status_name`'s exhaustive match first).
fn host_status_entries() -> Vec<(String, i64)> {
    NveHostStatus::ALL
        .into_iter()
        .map(|s| (host_status_name(s).to_owned(), i64::from(s.as_i32())))
        .collect()
}

/// Every `nve_abi::hybrid::NveHybridStatus`, by name and code.
fn hybrid_status_entries() -> Vec<(String, i64)> {
    NveHybridStatus::ALL
        .into_iter()
        .map(|s| (s.name().to_owned(), i64::from(s as i32)))
        .collect()
}

/// Checks every hand-mirrored constant in the five host runtimes. (Golden XXH64 vectors are
/// generated from `nve_hash` by `nve codegen abi`, so no host carries a hand-copied digest.)
pub fn check_host_runtime_mirrors(idx: &RepoIndex) -> Vec<Violation> {
    use Syntax::{Assign, Call, Colon, Ordered};
    let mut out = Vec::new();

    let statuses = |ts_marker: &'static str,
                    cxx: &'static str,
                    swift: &'static str,
                    java: &'static str,
                    dart: &'static str| {
        [
            site(CXX, "hybrid_runtime.hpp", cxx, Assign),
            site(SWIFT, "HybridRuntime.swift", swift, Assign),
            site(JAVA, "HybridRuntime.java", java, Call),
            site(DART, "hybrid_runtime.dart", dart, Call),
            site(TS, "hybrid_runtime.ts", ts_marker, Colon),
        ]
    };
    check_set(
        idx,
        "NveHostStatus",
        &host_status_entries(),
        &statuses(
            "export const HostStatus",
            "enum class HostStatus",
            "public enum HostStatus",
            "public enum HostStatus",
            "enum HostStatus",
        ),
        &mut out,
    );
    let hybrid = hybrid_status_entries();
    check_set(
        idx,
        "NveHybridStatus",
        &hybrid,
        &statuses(
            "export const HybridStatus",
            "enum class HybridStatus",
            "public enum HybridStatus",
            "public enum HybridStatus",
            "enum HybridStatus",
        ),
        &mut out,
    );

    let kinds: Vec<(String, i64)> = [
        FieldKind::U32,
        FieldKind::I32,
        FieldKind::F32,
        FieldKind::Bool,
        FieldKind::Enum,
        FieldKind::Color,
        FieldKind::Length,
        FieldKind::Str,
        FieldKind::Resource,
        FieldKind::Node,
        FieldKind::U64,
        FieldKind::I64,
        FieldKind::F64,
        FieldKind::GridLine,
    ]
    .into_iter()
    .map(|k| (field_kind_name(k).to_owned(), k as i64))
    .collect();
    check_set(
        idx,
        "FieldKind",
        &kinds,
        &[
            Site {
                prefix: "nve_field_",
                ..site(CXX, "table_word.h", "typedef enum NveFieldKind", Assign)
            },
            Site {
                renames: &[("Enum", "enumeration")],
                ..site(SWIFT, "TableReader.swift", "public enum FieldKind", Assign)
            },
            site(JAVA, "FieldKind.java", "public enum FieldKind", Ordered),
            Site {
                renames: &[("Bool", "boolean"), ("Enum", "enumeration")],
                ..site(DART, "table_reader.dart", "enum FieldKind", Ordered)
            },
            site(TS, "table_reader.ts", "export const FieldKind", Colon),
        ],
        &mut out,
    );

    let tags: Vec<(String, i64)> = NveValueTag::ALL
        .into_iter()
        .map(|t| (value_tag_name(t).to_owned(), t as i64))
        .collect();
    check_set(
        idx,
        "NveValueTag",
        &tags,
        &[
            site(CXX, "bin_codec.hpp", "enum class BinTag", Assign),
            site(SWIFT, "BinCodec.swift", "public enum BinTag", Assign),
            site(JAVA, "BinTag.java", "public enum BinTag", Ordered),
            Site {
                renames: &[("Null", "nullValue"), ("Bool", "boolValue")],
                ..site(DART, "bin_codec.dart", "enum BinTag", Ordered)
            },
            site(TS, "bin_codec.ts", "export const BinTag", Colon),
        ],
        &mut out,
    );

    let unit = |l: Length| i64::from(l.to_word() & 3);
    let units = vec![
        ("Points".to_owned(), unit(Length::points(1.0))),
        ("Percent".to_owned(), unit(Length::percent(50.0))),
        ("Auto".to_owned(), unit(Length::AUTO)),
    ];
    check_set(
        idx,
        "Length unit tags",
        &units,
        &[
            Site {
                prefix: "nve_length_",
                ..site(CXX, "table_word.h", "typedef enum NveLengthUnit", Assign)
            },
            site(SWIFT, "TableReader.swift", "public enum Unit", Assign),
            site(JAVA, "Length.java", "public final class Length", Assign),
            site(DART, "table_reader.dart", "enum LengthUnit", Ordered),
            site(TS, "table_reader.ts", "export const LengthUnit", Colon),
        ],
        &mut out,
    );

    let limits = [
        (
            "max depth",
            NVE_BIN_VALUE_MAX_DEPTH as i64,
            [
                "kBinValueMaxDepth",
                "binValueMaxDepth",
                "MAX_DEPTH",
                "binValueMaxDepth",
                "BIN_VALUE_MAX_DEPTH",
            ],
            [
                "bin_codec.hpp",
                "BinCodec.swift",
                "BinTag.java",
                "bin_codec.dart",
                "bin_codec.ts",
            ],
        ),
        (
            "max table words",
            MAX_TABLE_WORDS as i64,
            [
                "NVE_MAX_TABLE_WORDS",
                "maxTableWords",
                "MAX_WORDS",
                "maxTableWords",
                "MAX_TABLE_WORDS",
            ],
            [
                "table_word.h",
                "TableReader.swift",
                "TableReader.java",
                "table_reader.dart",
                "table_reader.ts",
            ],
        ),
    ];
    for (set, value, names, files) in limits {
        for ((dir, name), file) in [CXX, SWIFT, JAVA, DART, TS]
            .into_iter()
            .zip(names)
            .zip(files)
        {
            // Declarations only (`name = N`, or C's `#define name Nu`), comments ignored, exact value.
            let path = format!("{dir}{file}");
            let values = decl::declared_values(&idx.read_text(&path).unwrap_or_default(), name);
            if values.is_empty() || values.iter().any(|v| *v != value) {
                out.push(Violation::new(
                    &path,
                    0,
                    name,
                    format!("`{set}`: `{name}` must be declared as {value}, found {values:?}"),
                ));
            }
        }
    }

    out
}

/// The in-place `NveBinValue` validators: `(file, function signature, identifiers they must never use)`.
/// They check strings and skip bytes inside the input buffer; any view, slice, copy, or decode here
/// would allocate once per value (measured guards exist for Java and Swift; this makes the
/// guarantee mechanical for every runtime, including Dart and TS which have no allocation counter).
const IN_PLACE_VALIDATORS: &[(&str, &str, &[&str])] = &[
    (
        "packages/core/host-dart/lib/src/bin_codec.dart",
        "int _checkUtf8(int count)",
        &[
            "sublistView",
            "decodeUtf8PreserveBom",
            "decode",
            "utf8String",
            "readStr",
            "readMapKey",
            "readBytes",
            "take",
            "toBytes",
        ],
    ),
    (
        "packages/core/host-dart/lib/src/bin_codec.dart",
        "void _checkValue(int depth)",
        &[
            "sublistView",
            "decodeUtf8PreserveBom",
            "decode",
            "utf8String",
            "readStr",
            "readMapKey",
            "readBytes",
            "take",
            "toBytes",
        ],
    ),
    (
        "packages/core/host-ts/src/bin_codec.ts",
        "private checkUtf8Run(): void",
        &[
            "subarray",
            "slice",
            "decode",
            "utf8",
            "readStr",
            "readMapKey",
            "readBytes",
            "bytesView",
        ],
    ),
    (
        "packages/core/host-ts/src/bin_codec.ts",
        "private checkAt(depth: number): void",
        &[
            "subarray",
            "slice",
            "decode",
            "utf8",
            "readStr",
            "readMapKey",
            "readBytes",
            "bytesView",
        ],
    ),
    (
        "packages/core/host-java/src/main/java/dev/nve/host/BinReader.java",
        "private void checkUtf8Run()",
        &[
            "String",
            "utf8",
            "readStr",
            "readMapKey",
            "readBytes",
            "bytes",
            "readRawValue",
            "copyOf",
            "arraycopy",
        ],
    ),
    (
        "packages/core/host-java/src/main/java/dev/nve/host/BinReader.java",
        "private void checkValue(int depth)",
        &[
            "String",
            "utf8",
            "readStr",
            "readMapKey",
            "readBytes",
            "bytes",
            "readRawValue",
            "copyOf",
            "arraycopy",
        ],
    ),
    (
        "packages/core/host-swift/Sources/NveHost/BinCodec.swift",
        "private mutating func checkUtf8(_ count: Int)",
        &[
            "String",
            "Array",
            "utf8",
            "take",
            "readStr",
            "readMapKey",
            "readBytes",
            "readRawValue",
        ],
    ),
    (
        "packages/core/host-swift/Sources/NveHost/BinCodec.swift",
        "private mutating func checkValue(depth: Int)",
        &[
            "String",
            "Array",
            "utf8",
            "take",
            "readStr",
            "readMapKey",
            "readBytes",
            "readRawValue",
        ],
    ),
];

/// Every in-place validator exists and never calls a view / slice / copy / decode helper.
pub fn check_in_place_validators(idx: &RepoIndex) -> Vec<Violation> {
    let mut out = Vec::new();
    for &(file, signature, forbidden) in IN_PLACE_VALIDATORS {
        let text = idx.read_text(file).unwrap_or_default();
        let Some(body) = decl::code_block(&text, signature) else {
            out.push(Violation::new(
                file,
                0,
                signature,
                "in-place validator not found (renamed? update IN_PLACE_VALIDATORS)",
            ));
            continue;
        };
        for name in decl::forbidden_identifiers(&body, forbidden) {
            out.push(Violation::new(
                file,
                0,
                signature,
                format!("in-place validator `{signature}` uses `{name}`, which views, copies, or decodes the input"),
            ));
        }
    }
    out
}
