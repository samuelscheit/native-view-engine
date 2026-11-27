//! Drift guard for the static host runtimes (`packages/core/host-{cxx,swift,java,dart,ts}`).
//!
//! Every constant those packages mirror by hand — host / hybrid status codes, `FieldKind`, the
//! `NveBinValue` tags, max nesting depth, max table words, `Length` unit tags, and the golden XXH64
//! digests their suites assert — is rendered here from the Rust source of truth (`nve_abi`,
//! `nve_store`, `nve_hash`, and codegen's `NveHybridStatus`) and must appear verbatim in each
//! language. Matching ignores case, whitespace, and `_`, so `InternalError = -8`,
//! `INTERNAL_ERROR(-8)`, and `internalError(-8)` are the same entry.

use nve_abi::table::{FieldKind, MAX_TABLE_WORDS};
use nve_abi::{NVE_BIN_VALUE_MAX_DEPTH, NveHostStatus, NveValueTag};
use nve_hash::{PRIME1, xxh64};
use nve_store::Length;

use crate::{RepoIndex, Violation};

const CXX: &str = "packages/core/host-cxx/include/nve/host/";
const SWIFT: &str = "packages/core/host-swift/Sources/NveHost/";
const JAVA: &str = "packages/core/host-java/src/main/java/dev/nve/host/";
const DART: &str = "packages/core/host-dart/lib/src/";
const TS: &str = "packages/core/host-ts/src/";
const HYBRID_STATUS_SOURCE: &str = "packages/core/codegen/src/targets/rust/hybrid/mod.rs";

/// How one language spells a `(name, value)` entry.
#[derive(Clone, Copy)]
enum Syntax {
    /// `Name = 1` (C++ / Swift / Java fields), optionally with a C prefix (`NVE_FIELD_U32 = 0`).
    Assign(&'static str),
    /// `NAME(1)` (Java / Dart enhanced enums).
    Call,
    /// `Name: 1` (TS `as const` objects).
    Colon,
    /// Values are declaration order: the names must appear as one ordered list.
    Ordered,
}

/// One mirror site: a file, the declaration that opens the block (`""` = whole file), its syntax,
/// and per-language renames of Rust variant names.
struct Site {
    file: String,
    marker: &'static str,
    syntax: Syntax,
    renames: &'static [(&'static str, &'static str)],
}

fn site(dir: &str, file: &str, marker: &'static str, syntax: Syntax) -> Site {
    Site {
        file: format!("{dir}{file}"),
        marker,
        syntax,
        renames: &[],
    }
}

fn normalize(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .flat_map(char::to_lowercase)
        .collect()
}

/// The declaration block starting at `marker` up to its first `}` (values always precede it).
fn block<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    if marker.is_empty() {
        return Some(text);
    }
    let start = text.find(marker)?;
    let len = text[start..].find('}')?;
    Some(&text[start..start + len])
}

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
        let Some(body) = block(&text, s.marker) else {
            out.push(Violation::new(
                &s.file,
                0,
                s.marker,
                format!("`{set}` declaration not found"),
            ));
            continue;
        };
        let hay = normalize(body);
        let name = |n: &str| {
            s.renames
                .iter()
                .find(|(from, _)| *from == n)
                .map_or(n, |(_, to)| *to)
                .to_owned()
        };
        let needles: Vec<String> = match s.syntax {
            Syntax::Ordered => {
                let mut sorted = entries.to_vec();
                sorted.sort_by_key(|(_, v)| *v);
                if !sorted
                    .iter()
                    .enumerate()
                    .all(|(i, (_, v))| usize::try_from(*v) == Ok(i))
                {
                    out.push(Violation::new(
                        &s.file,
                        0,
                        set,
                        "values are not 0-based dense: use explicit codes, not order",
                    ));
                    continue;
                }
                vec![
                    sorted
                        .iter()
                        .map(|(n, _)| name(n))
                        .collect::<Vec<_>>()
                        .join(","),
                ]
            }
            Syntax::Assign(prefix) => entries
                .iter()
                .map(|(n, v)| format!("{prefix}{}={v}", name(n)))
                .collect(),
            Syntax::Call => entries
                .iter()
                .map(|(n, v)| format!("{}({v})", name(n)))
                .collect(),
            Syntax::Colon => entries
                .iter()
                .map(|(n, v)| format!("{}:{v}", name(n)))
                .collect(),
        };
        for needle in needles {
            if !hay.contains(&normalize(&needle)) {
                out.push(Violation::new(
                    &s.file,
                    0,
                    &needle,
                    format!("`{set}` drifted from its Rust source of truth (expected `{needle}`)"),
                ));
            }
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
    use NveHostStatus as S;
    [
        S::Ok,
        S::Pending,
        S::BufferTooSmall,
        S::Unsupported,
        S::PermissionDenied,
        S::InvalidArg,
        S::NotFound,
        S::Cancelled,
        S::IoError,
        S::InternalError,
    ]
    .into_iter()
    .map(|s| (host_status_name(s).to_owned(), i64::from(s.as_i32())))
    .collect()
}

/// `NveHybridStatus` variants as emitted by codegen (`Name = N,` lines of its enum).
fn hybrid_status_entries(idx: &RepoIndex, out: &mut Vec<Violation>) -> Vec<(String, i64)> {
    let text = idx.read_text(HYBRID_STATUS_SOURCE).unwrap_or_default();
    let entries: Vec<(String, i64)> = block(&text, "pub enum NveHybridStatus")
        .unwrap_or_default()
        .lines()
        .filter_map(|l| {
            let (name, value) = l.trim().trim_end_matches(',').split_once(" = ")?;
            Some((name.to_owned(), value.parse().ok()?))
        })
        .collect();
    if entries.is_empty() {
        out.push(Violation::new(
            HYBRID_STATUS_SOURCE,
            0,
            "NveHybridStatus",
            "could not read the hybrid status enum",
        ));
    }
    entries
}

/// The 19 golden vectors every host suite asserts: `(input, seed)`.
fn golden_vectors() -> Vec<(Vec<u8>, u64)> {
    let seq = |n: usize| (0..n).map(|i| (i * 31 + 7) as u8).collect::<Vec<u8>>();
    let cyan = |n: usize| {
        let mut state: u64 = 0x9E37_79B1;
        (0..n)
            .map(|_| {
                let b = (state >> 56) as u8;
                state = state.wrapping_mul(PRIME1);
                b
            })
            .collect::<Vec<u8>>()
    };
    let (p32, wire) = (0x9E37_79B1, 0x4E56_4557_4952_4532);
    let text = |s: &str| s.as_bytes().to_vec();
    vec![
        (vec![], 0),
        (text("a"), 0),
        (text("abc"), 0),
        (text("nve.visual"), 0),
        (vec![0x00, 0x01, 0x7F, 0x80, 0xFF], 0),
        (text("héllo wörld — 日本語 🚀"), 0),
        (seq(101), 0),
        (seq(101), wire),
        (seq(37), PRIME1),
        (seq(1024), 0),
        (cyan(1), 0),
        (cyan(4), 0),
        (cyan(14), 0),
        (cyan(222), 0),
        (cyan(1024), 0),
        (cyan(1), p32),
        (cyan(222), p32),
        (cyan(32), PRIME1),
        (cyan(1024), PRIME1),
    ]
}

/// Checks every hand-mirrored constant and golden digest in the five host runtimes.
pub fn check_host_runtime_mirrors(idx: &RepoIndex) -> Vec<Violation> {
    use Syntax::{Assign, Call, Colon, Ordered};
    let mut out = Vec::new();

    let statuses = |ts_marker: &'static str,
                    cxx: &'static str,
                    swift: &'static str,
                    java: &'static str,
                    dart: &'static str| {
        [
            site(CXX, "hybrid_runtime.hpp", cxx, Assign("")),
            site(SWIFT, "HybridRuntime.swift", swift, Assign("")),
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
    let hybrid = hybrid_status_entries(idx, &mut out);
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
    ]
    .into_iter()
    .map(|k| (field_kind_name(k).to_owned(), k as i64))
    .collect();
    check_set(
        idx,
        "FieldKind",
        &kinds,
        &[
            site(
                CXX,
                "table_word.h",
                "typedef enum NveFieldKind",
                Assign("nve_field_"),
            ),
            Site {
                renames: &[("Enum", "enumeration")],
                ..site(
                    SWIFT,
                    "TableReader.swift",
                    "public enum FieldKind",
                    Assign(""),
                )
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

    let tags: Vec<(String, i64)> = (0..8)
        .filter_map(NveValueTag::from_u32)
        .map(|t| (value_tag_name(t).to_owned(), t as i64))
        .collect();
    check_set(
        idx,
        "NveValueTag",
        &tags,
        &[
            site(CXX, "bin_codec.hpp", "enum class BinTag", Assign("")),
            site(SWIFT, "BinCodec.swift", "public enum BinTag", Assign("")),
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
            site(
                CXX,
                "table_word.h",
                "typedef enum NveLengthUnit",
                Assign("nve_length_"),
            ),
            site(SWIFT, "TableReader.swift", "public enum Unit", Assign("")),
            site(JAVA, "Length.java", "public final class Length", Assign("")),
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
            // C spells the limit as a macro (`#define NVE_MAX_TABLE_WORDS 32u`).
            let syntax = if name == "NVE_MAX_TABLE_WORDS" {
                "#define "
            } else {
                ""
            };
            let needle = if syntax.is_empty() {
                format!("{name} = {value}")
            } else {
                format!("{syntax}{name} {value}u")
            };
            let path = format!("{dir}{file}");
            if !idx
                .read_text(&path)
                .is_some_and(|t| normalize(&t).contains(&normalize(&needle)))
            {
                out.push(Violation::new(
                    &path,
                    0,
                    &needle,
                    format!("`{set}` drifted (expected `{needle}`)"),
                ));
            }
        }
    }

    let golden = [
        ("packages/core/host-cxx/tests/vectors.h", false),
        (
            "packages/core/host-swift/Tests/NveHostTests/GoldenVectors.swift",
            false,
        ),
        (
            "packages/core/host-java/src/test/java/dev/nve/host/Xxh64Test.java",
            false,
        ),
        ("packages/core/host-dart/test/src/xxh64_suite.dart", true),
        ("packages/core/host-ts/test/golden.ts", true),
    ];
    for (file, halves) in golden {
        let hay = normalize(&idx.read_text(file).unwrap_or_default());
        for (bytes, seed) in golden_vectors() {
            let d = xxh64(&bytes, seed);
            let needle = if halves {
                format!("key64(0x{:08x},0x{:08x})", d as u32, (d >> 32) as u32)
            } else {
                format!("0x{d:016x}")
            };
            if !hay.contains(&needle) {
                out.push(Violation::new(
                    file,
                    0,
                    &needle,
                    "golden XXH64 digest missing or drifted from `nve_hash::xxh64`",
                ));
            }
        }
    }
    out
}
