//! Up-front validation of everything the ABI templates spell. The templates render with
//! `escape = "none"`, so whatever reaches them must already be a valid identifier, type, and
//! literal in every target language.

use std::collections::BTreeSet;

use nve_abi::schema::{AbiFileDescriptor, AbiParamDescriptor};
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

/// Consts fit `u32`, every signature / alias type resolves, and no record needs an alignment a
/// target cannot express.
pub(super) fn abi(
    files: &[&AbiFileDescriptor],
    records: &[RecordLayout<'_>],
) -> Result<(), CodegenError> {
    let mut known: BTreeSet<&str> = records.iter().map(|r| r.name).collect();
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
    for file in files {
        for a in file.aliases {
            check_type(a.target, &known, a.name)?;
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
            let ctx = format!("{owner}::{name}");
            params
                .iter()
                .try_for_each(|p: &AbiParamDescriptor| check_type(p.ty, &known, &ctx))?;
            check_type(ret, &known, &ctx)?;
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

fn check_type(spelling: &str, known: &BTreeSet<&str>, ctx: &str) -> Result<(), CodegenError> {
    match Ty::parse(spelling)? {
        Ty::Unit | Ty::Void | Ty::Scalar(_) => Ok(()),
        Ty::Ptr { pointee, .. } => check_type(pointee, known, ctx),
        Ty::Array { .. } => Err(invalid(format!(
            "`{ctx}`: arrays cannot cross a C signature by value"
        ))),
        Ty::Named(n) if known.contains(n) => Ok(()),
        Ty::Named(n) => Err(invalid(format!(
            "`{ctx}` uses `{n}`, which the ABI does not declare"
        ))),
    }
}

/// The table name is `[a-z0-9._]` (it lands in C strings, Dart `'...'` with `$` interpolation, and
/// identifiers), and every field is a `snake_case` identifier no target reserves.
pub(super) fn table(desc: &TableDesc) -> Result<(), CodegenError> {
    let name_ok = desc.name.starts_with(|c: char| c.is_ascii_lowercase())
        && desc
            .name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_');
    if !name_ok {
        return Err(table_error(desc, "names must match `[a-z][a-z0-9._]*`"));
    }
    for f in desc.fields {
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
mod tests {
    use nve_abi::schema::{
        AbiCFunctionDescriptor, AbiConstDescriptor, AbiEnumDescriptor, AbiFieldDescriptor,
        AbiFileDescriptor, AbiParamDescriptor, AbiRecordDescriptor, AbiStructDescriptor,
    };
    use nve_abi::table::{FieldDesc, FieldKind, Storage, TableDesc};

    use super::super::layout::layout_records;

    const fn file(
        consts: &'static [AbiConstDescriptor],
        enums: &'static [AbiEnumDescriptor],
        records: &'static [AbiRecordDescriptor],
        c_functions: &'static [AbiCFunctionDescriptor],
    ) -> AbiFileDescriptor {
        AbiFileDescriptor {
            namespace: None,
            consts,
            aliases: &[],
            enums,
            records,
            modules: &[],
            c_functions,
        }
    }

    const fn record(
        name: &'static str,
        fields: &'static [AbiFieldDescriptor],
        size: usize,
        explicit_align: Option<usize>,
    ) -> AbiRecordDescriptor {
        AbiRecordDescriptor::Struct(AbiStructDescriptor {
            name,
            doc: None,
            size,
            // What `rustc` reports: an explicit alignment wins over the fields' natural 8.
            align: match explicit_align {
                Some(a) => a,
                None => 8,
            },
            explicit_align,
            expected_size: None,
            expected_size_64: None,
            fields,
        })
    }

    const fn field(name: &'static str, ty: &'static str, offset: usize) -> AbiFieldDescriptor {
        AbiFieldDescriptor {
            name,
            ty,
            c_type: None,
            doc: None,
            offset,
            expected_offset: None,
            expected_offset_64: None,
        }
    }

    fn error(f: &'static AbiFileDescriptor) -> String {
        match layout_records(&[f]).and_then(|r| super::abi(&[f], &r)) {
            Err(e) => e.to_string(),
            Ok(()) => String::from("<accepted>"),
        }
    }

    #[test]
    fn consts_signatures_and_arrays_are_validated() {
        static BIG: AbiFileDescriptor = file(
            &[AbiConstDescriptor {
                name: "X",
                ty: "u32",
                value: 1 << 32,
                hex: true,
                doc: None,
            }],
            &[],
            &[],
            &[],
        );
        assert!(
            error(&BIG).contains("must be a u32 value"),
            "{}",
            error(&BIG)
        );

        static UNKNOWN: AbiFileDescriptor = file(
            &[],
            &[],
            &[],
            &[AbiCFunctionDescriptor {
                name: "f",
                params: &[AbiParamDescriptor {
                    name: "a",
                    ty: "*const Missing",
                }],
                return_type: "()",
                doc: None,
            }],
        );
        assert!(
            error(&UNKNOWN).contains("uses `Missing`"),
            "{}",
            error(&UNKNOWN)
        );

        static BY_VALUE_ARRAY: AbiFileDescriptor = file(
            &[],
            &[],
            &[],
            &[AbiCFunctionDescriptor {
                name: "f",
                params: &[],
                return_type: "[u8; 4]",
                doc: None,
            }],
        );
        assert!(
            error(&BY_VALUE_ARRAY).contains("arrays cannot cross"),
            "{}",
            error(&BY_VALUE_ARRAY)
        );

        static ZERO_LEN: AbiFileDescriptor = file(
            &[],
            &[],
            &[record("Z", &[field("a", "[u8; 0]", 0)], 0, None)],
            &[],
        );
        assert!(
            error(&ZERO_LEN).contains("zero-length arrays"),
            "{}",
            error(&ZERO_LEN)
        );

        static HUGE: AbiFileDescriptor = file(
            &[],
            &[],
            &[record(
                "H",
                &[field("a", "[u64; 18446744073709551615]", 0)],
                0,
                None,
            )],
            &[],
        );
        assert!(error(&HUGE).contains("overflows usize"), "{}", error(&HUGE));

        static ENUM_FIELD: AbiFileDescriptor = file(
            &[],
            &[AbiEnumDescriptor {
                name: "E",
                repr: "u8",
                doc: None,
                c_prefix: None,
                dart_fallback: None,
                variants: &[],
            }],
            &[record("R", &[field("e", "E", 0)], 1, None)],
            &[],
        );
        assert!(
            error(&ENUM_FIELD).contains("stores enum `E` by value"),
            "{}",
            error(&ENUM_FIELD)
        );

        // `align(16)` on a 4-byte record changes its size everywhere: C can, Dart `ffi.Struct` cannot.
        static OVER: AbiFileDescriptor = file(
            &[],
            &[],
            &[record("O", &[field("a", "u32", 0)], 16, Some(16))],
            &[],
        );
        assert!(
            error(&OVER).contains("Dart `ffi.Struct` cannot express"),
            "{}",
            error(&OVER)
        );
    }

    const fn table(name: &'static str, fields: &'static [FieldDesc]) -> TableDesc {
        TableDesc {
            key: nve_hash::Key64::of(name),
            name,
            schema_hash: 0,
            storage: Storage::Sparse,
            flags: 0,
            defaults: &[0],
            word_effects: &[0],
            str_words: 0,
            resource_words: 0,
            fields,
        }
    }

    const fn u32_field(name: &'static str) -> FieldDesc {
        FieldDesc {
            name,
            kind: FieldKind::U32,
            word: 0,
            shift: 0,
            bits: 32,
            effects: 0,
        }
    }

    #[test]
    fn table_names_and_fields_are_valid_in_every_target() {
        static OK_FIELDS: [FieldDesc; 2] = [u32_field("set"), u32_field("get_value")];
        assert!(super::table(&table("nve.ok_2", &OK_FIELDS)).is_ok());
        for bad in ["nve.$x", "nve.Text", "", "1nve", "nve.te'xt"] {
            let t = table(bad, &[]);
            assert!(super::table(&t).is_err(), "{bad:?} must be rejected");
        }
        for (field, why) in [
            ("default", "reserved word `default`"),
            ("init", "reserved word `init`"),
            ("Bad", "must match"),
        ] {
            let fields = Box::leak(Box::new([u32_field(field)]));
            let message = super::table(&table("nve.t", fields))
                .map_or_else(|e| e.to_string(), |()| String::new());
            assert!(message.contains(why), "{field}: {message}");
        }
    }
}
