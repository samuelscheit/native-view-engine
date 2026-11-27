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
/// `X.DEFAULTS`, the C `_FIELD_LIST` X-macro, ...): a field of the same name would collide.
const GENERATED_TABLE_MEMBERS: &[&str] = &[
    "key",
    "name",
    "schema_hash",
    "words",
    "dense",
    "flags",
    "str_words",
    "resource_words",
    "defaults",
    "field_list",
    "fields",
];

/// Consts fit `u32`, every signature / alias type resolves, and no record needs an alignment a
/// target cannot express.
pub(super) fn abi(
    files: &[&AbiFileDescriptor],
    records: &[RecordLayout<'_>],
) -> Result<(), CodegenError> {
    let mut known: BTreeSet<&str> = records.iter().map(|r| r.name).collect();
    let slab_aligned: BTreeSet<&str> = records
        .iter()
        .filter(|r| r.slab_align.is_some())
        .map(|r| r.name)
        .collect();
    for file in files {
        known.extend(file.aliases.iter().map(|a| a.name));
        known.extend(file.enums.iter().map(|e| e.name));
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

/// The record / alias / enum names the ABI declares, and the records whose alignment only C cannot
/// reproduce by value.
struct Known<'a> {
    names: BTreeSet<&'a str>,
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
        // `()` only as a return type; `c_void` only behind a pointer.
        Ty::Unit if pos == Position::Return => Ok(()),
        Ty::Void if pos == Position::Pointee => Ok(()),
        Ty::Unit | Ty::Void => Err(invalid(format!(
            "`{}`: `{spelling}` is only valid as a return type or pointee",
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
    // `[a-z][a-z0-9_]*(\.[a-z][a-z0-9_]*)*`: every dot-separated segment is an identifier.
    let segment_ok = |seg: &str| {
        seg.starts_with(|c: char| c.is_ascii_lowercase())
            && seg
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
    };
    let name_ok = desc.name.split('.').all(segment_ok);
    if !name_ok {
        return Err(table_error(desc, "names must match `[a-z][a-z0-9._]*`"));
    }
    let mut seen = BTreeSet::new();
    for f in desc.fields {
        if !seen.insert(f.name) {
            return Err(table_error(
                desc,
                &format!("field `{}` is declared twice", f.name),
            ));
        }
        if GENERATED_TABLE_MEMBERS.contains(&f.name) {
            return Err(table_error(
                desc,
                &format!(
                    "field `{}` collides with a generated table member of the same name",
                    f.name
                ),
            ));
        }
        let ident_ok = f.name.starts_with(|c: char| c.is_ascii_lowercase())
            && f.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !ident_ok {
            return Err(table_error(
                desc,
                &format!("field `{}` must match `[a-z][a-z0-9_]*`", f.name),
            ));
        }
        let camel = super::spell::camel(f.name);
        if KEYWORDS.contains(&camel.as_str()) {
            return Err(table_error(
                desc,
                &format!("field `{}` spells the reserved word `{camel}`", f.name),
            ));
        }
    }
    Ok(())
}

fn invalid(message: String) -> CodegenError {
    CodegenError::validation(message)
}

#[cfg(test)]
mod tests;
