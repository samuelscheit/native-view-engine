//! Up-front validation of everything the ABI templates spell. The templates render with
//! `escape = "none"`, so whatever reaches them must already be a valid identifier, type, and
//! literal in every target language.

use std::collections::BTreeSet;

use nve_abi::schema::AbiFileDescriptor;
use nve_abi::table::TableDesc;

use super::layout::{RecordLayout, Ty};
use super::model::table_error;
use crate::error::CodegenError;

/// Words reserved in the languages that spell table fields as lowerCamel members: Swift
/// (`static let`) and Dart (`static final`). Java uses SCREAMING constants and TypeScript object
/// keys accept any name; contextual words (`get`, `set`, `type`, ...) are valid members everywhere.
const KEYWORDS: &[&str] = &[
    // Swift
    "associatedtype",
    "as",
    "break",
    "case",
    "catch",
    "class",
    "continue",
    "default",
    "defer",
    "deinit",
    "do",
    "else",
    "enum",
    "extension",
    "fallthrough",
    "false",
    "fileprivate",
    "for",
    "func",
    "guard",
    "if",
    "import",
    "in",
    "init",
    "inout",
    "internal",
    "is",
    "let",
    "nil",
    "open",
    "operator",
    "precedencegroup",
    "private",
    "protocol",
    "public",
    "repeat",
    "rethrows",
    "return",
    "self",
    "static",
    "struct",
    "subscript",
    "super",
    "switch",
    "throw",
    "throws",
    "true",
    "try",
    "typealias",
    "var",
    "where",
    "while",
    // Dart (reserved words only; built-in identifiers are valid members)
    "assert",
    "const",
    "extends",
    "final",
    "finally",
    "new",
    "null",
    "rethrow",
    "this",
    "void",
    "with",
];

/// Members every generated table already has (`NveTables.X.key`, `NVE_TABLE_X_WORDS`,
/// `X.DEFAULTS`, the C `_FIELD_LIST` X-macro, ...) and Dart's `Object` members, paired with their
/// `camelCase` spelling: a field that spells the same `camelCase` or `SCREAMING_SNAKE` name would
/// collide.
const GENERATED_TABLE_MEMBERS: &[(&str, &str)] = &[
    ("key", "key"),
    ("name", "name"),
    ("schema_hash", "schemaHash"),
    ("words", "words"),
    ("dense", "dense"),
    ("flags", "flags"),
    ("str_words", "strWords"),
    ("resource_words", "resourceWords"),
    ("defaults", "defaults"),
    ("field_list", "fieldList"),
    ("fields", "fields"),
    ("hash_code", "hashCode"),
    ("runtime_type", "runtimeType"),
    ("to_string", "toString"),
    ("no_such_method", "noSuchMethod"),
];

/// Consts fit `u32`, every signature / alias type resolves, and no record needs an alignment a
/// target cannot express.
pub(super) fn abi(
    files: &[&AbiFileDescriptor],
    records: &[RecordLayout<'_>],
) -> Result<(), CodegenError> {
    let mut known: BTreeSet<&str> = records.iter().map(|r| r.name).collect();
    let enums: BTreeSet<&str> = files.iter().flat_map(|f| f.enums).map(|e| e.name).collect();
    let slab_aligned: BTreeSet<&str> = records
        .iter()
        .filter(|r| r.slab_align.is_some())
        .map(|r| r.name)
        .collect();
    for file in files {
        known.extend(file.aliases.iter().map(|a| a.name));
        for c in file.consts {
            if c.ty != "u32" || c.value > u64::from(u32::MAX) {
                return Err(invalid(format!(
                    "const `{}` must be a u32 value (is {} {})",
                    c.name, c.ty, c.value
                )));
            }
        }
    }
    let known = Known {
        names: known,
        enums,
        slab_aligned,
    };
    for file in files {
        for a in file.aliases {
            check_type(a.target, Position::AliasTarget, &known, "alias", a.name)?;
        }
        let module_methods = file.modules.iter().flat_map(|m| {
            m.methods
                .iter()
                .map(move |f| (m.name, f.name, f.params, f.return_type))
        });
        let c_functions = file
            .c_functions
            .iter()
            .map(|f| ("C", f.name, f.params, f.return_type));
        for (owner, name, params, ret) in module_methods.chain(c_functions) {
            for p in params {
                check_type(p.ty, Position::Param, &known, owner, name)?;
            }
            check_type(ret, Position::Return, &known, owner, name)?;
        }
    }
    // Dart `ffi.Struct` cannot over-align: it may only skip an `align(N)` that changes nothing on
    // the data models Dart FFI runs on (64-bit and ILP32; i386 is C-only).
    for r in records {
        if let Some(n) = r.alignas
            && (r.natural_align.p64() < n || r.natural_align.p32() < n)
        {
            return Err(invalid(format!(
                "`{}` needs `align({n})` on 64-bit / ILP32, which Dart `ffi.Struct` cannot express",
                r.name
            )));
        }
    }
    Ok(())
}

/// Where a type appears in a C signature.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Position {
    Param,
    Return,
    Pointee,
    AliasTarget,
}

/// The record and alias names the ABI declares, its enums (rejected in signatures), and the
/// records whose alignment C cannot reproduce by value.
struct Known<'a> {
    names: BTreeSet<&'a str>,
    enums: BTreeSet<&'a str>,
    slab_aligned: BTreeSet<&'a str>,
}

/// `spelling` is expressible at `pos` of `owner::name`'s signature (formatted only on error).
fn check_type(
    spelling: &str,
    pos: Position,
    known: &Known<'_>,
    owner: &str,
    name: &str,
) -> Result<(), CodegenError> {
    let ctx = || format!("{owner}::{name}");
    match Ty::parse(spelling)? {
        Ty::Scalar(_) => Ok(()),
        Ty::Unit if pos == Position::Return => Ok(()),
        Ty::Void if pos == Position::Pointee => Ok(()),
        Ty::Unit => Err(invalid(format!(
            "`{}`: `()` is only valid as a return type",
            ctx()
        ))),
        Ty::Void => Err(invalid(format!(
            "`{}`: `c_void` is only valid behind a pointer",
            ctx()
        ))),
        // C / Dart see an enum as an `int`-sized C enum whatever its Rust repr, by value and behind
        // a pointer alike: signatures carry the integer repr.
        Ty::Named(n) if known.enums.contains(n) => Err(invalid(format!(
            "`{}` uses enum `{n}`: C enums are `int`-sized whatever the Rust repr; use its integer repr",
            ctx()
        ))),
        // Behind a pointer the alignment is the producer's concern (see `slab_align`).
        Ty::Ptr { pointee, .. } => check_type(pointee, Position::Pointee, known, owner, name),
        Ty::Named(n) if known.slab_aligned.contains(n) && pos != Position::Pointee => {
            Err(invalid(format!(
                "`{}` passes `{n}` by value, but C does not reproduce its Rust-only `align(N)`; pass a pointer",
                ctx()
            )))
        }
        Ty::Named(n) if known.names.contains(n) => Ok(()),
        Ty::Named(n) => Err(invalid(format!(
            "`{}` uses `{n}`, which the ABI does not declare",
            ctx()
        ))),
        Ty::Array { .. } => Err(invalid(format!(
            "`{}`: arrays cannot cross a C signature by value",
            ctx()
        ))),
    }
}

/// The table name is `[a-z0-9._]` (it lands in C strings, Dart `'...'` with `$` interpolation, and
/// identifiers), and every field is a `snake_case` identifier no target reserves.
pub(super) fn table(desc: &TableDesc) -> Result<(), CodegenError> {
    if desc.name.is_empty() {
        return Err(table_error(desc, "the name is empty"));
    }
    // `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*`: every dot-separated segment is an identifier.
    if let Some(seg) = desc.name.split('.').find(|seg| !is_snake_ident(seg)) {
        return Err(table_error(
            desc,
            &format!("name segment `{seg}` must match `[a-z][a-z0-9_]*`"),
        ));
    }
    if desc.fields.is_empty() {
        return Ok(());
    }
    // Every target spelling (`camelCase` in Swift / Java / Dart / TS, `SCREAMING_SNAKE` in C) of
    // every field and generated member is distinct: `key_` is `key`, `foo_1` is `foo1`. For
    // `[a-z][a-z0-9_]*` identifiers, equal `SCREAMING_SNAKE` implies equal `camelCase`, so storing
    // all `camelCase` spellings in one flat buffer catches both with two allocations per table.
    let total_bytes: usize = desc.fields.iter().map(|f| f.name.len()).sum();
    let mut buf = String::with_capacity(total_bytes);
    let mut entries: Vec<(usize, usize, usize, &str)> = Vec::with_capacity(desc.fields.len());
    for (idx, f) in desc.fields.iter().enumerate() {
        if !is_snake_ident(f.name) {
            return Err(table_error(
                desc,
                &format!("field `{}` must match `[a-z][a-z0-9_]*`", f.name),
            ));
        }
        let start = buf.len();
        push_snake_camel(f.name, &mut buf);
        let end = buf.len();
        let camel = &buf[start..end];
        if KEYWORDS.contains(&camel) {
            return Err(table_error(
                desc,
                &format!("field `{}` spells the reserved word `{camel}`", f.name),
            ));
        }
        if let Some(&(member, _)) = GENERATED_TABLE_MEMBERS.iter().find(|&&(_, c)| c == camel) {
            return Err(table_error(
                desc,
                &format!(
                    "field `{}` collides with a generated table member (`{member}`) as `{camel}`",
                    f.name
                ),
            ));
        }
        entries.push((start, end, idx, f.name));
    }
    entries.sort_unstable_by(|a, b| (&buf[a.0..a.1], a.2).cmp(&(&buf[b.0..b.1], b.2)));
    let mut earliest: Option<(usize, &str, &str, &str)> = None;
    for pair in entries.windows(2) {
        let (prev, cur) = (pair[0], pair[1]);
        let spelling = &buf[cur.0..cur.1];
        if &buf[prev.0..prev.1] == spelling
            && earliest.is_none_or(|(best_idx, ..)| cur.2 < best_idx)
        {
            earliest = Some((cur.2, prev.3, cur.3, spelling));
        }
    }
    if let Some((_, prev, cur, spelling)) = earliest {
        if prev == cur {
            return Err(table_error(
                desc,
                &format!("field `{cur}` is declared twice"),
            ));
        }
        return Err(table_error(
            desc,
            &format!("field `{cur}` collides with another field (`{prev}`) as `{spelling}`"),
        ));
    }
    Ok(())
}

/// Appends `spell::camel(name)` for a validated `is_snake_ident` (`[a-z][a-z0-9_]*`) without
/// allocating.
fn push_snake_camel(name: &str, out: &mut String) {
    let mut cap = false;
    for b in name.bytes() {
        if b == b'_' {
            cap = true;
        } else if cap {
            out.push(char::from(b.to_ascii_uppercase()));
            cap = false;
        } else {
            out.push(char::from(b));
        }
    }
}

/// `[a-z][a-z0-9_]*`.
fn is_snake_ident(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_lowercase())
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn invalid(message: String) -> CodegenError {
    CodegenError::validation(message)
}

#[cfg(test)]
mod tests;
