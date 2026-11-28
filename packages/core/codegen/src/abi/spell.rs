//! Per-language spellings of ABI types, names, and values, called from the Askama templates.
//!
//! Every function takes the borrowed `nve-abi` descriptors directly. [`super::layout`] and
//! [`super::validate`] reject every type spelling a template cannot express before rendering
//! starts, so the `/* unsupported */` fallback below is unreachable (and would fail the host
//! compile if it were not).

use nve_abi::schema::{AbiAliasDescriptor, AbiParamDescriptor};

use super::layout::{FieldLayout, Scalar, Ty, Width};
use super::model::{HexLiteral, U64Split};
use crate::naming::{screaming_snake_to_camel_case, to_camel_case, to_pascal_case, to_snake_case};

/// `SCREAMING_SNAKE` of a Rust identifier (`NveValueTag` -> `NVE_VALUE_TAG`, `hit_test` -> `HIT_TEST`).
pub fn screaming(name: &str) -> String {
    let mut s = to_snake_case(name);
    s.make_ascii_uppercase();
    s
}

/// `lowerCamel` of a snake / SCREAMING identifier (`NVE_ABI_MAGIC` -> `nveAbiMagic`). A leading `_`
/// is dropped: Dart would make the name library-private.
pub fn camel(name: &str) -> String {
    let name = name.trim_start_matches('_');
    if !name.bytes().any(|b| b.is_ascii_lowercase()) {
        screaming_snake_to_camel_case(name)
    } else {
        to_camel_case(name)
    }
}

/// `snake_case` of a Rust identifier (`Surface` -> `surface`).
pub fn snake(name: &str) -> String {
    to_snake_case(name)
}

/// `UpperCamel` of a snake / dotted identifier (`nve.text_input` -> `NveTextInput`).
pub fn pascal(name: &str) -> String {
    to_pascal_case(&name.replace('.', "_"))
}

/// Enum discriminant / constant: hex from `0x1000` up, decimal below.
pub fn int(value: &i64) -> String {
    if *value >= 0x1000 {
        format!("0x{value:X}")
    } else {
        value.to_string()
    }
}

/// Unsigned constant spelled as declared (`hex = true` -> `0x...`).
pub fn uint(value: &u64, hex: &bool) -> String {
    if *hex {
        format!("0x{value:X}")
    } else {
        value.to_string()
    }
}

/// 64-bit unsigned constant (`hex = true` -> 16-digit `0x...`).
pub fn uint64(value: &u64, hex: &bool) -> String {
    if *hex {
        format!("0x{value:016X}")
    } else {
        value.to_string()
    }
}

/// Low 32 bits of a 64-bit constant as `0x........`.
pub fn lo_hex(value: &u64) -> HexLiteral {
    U64Split::new(*value).lo_hex()
}

/// High 32 bits of a 64-bit constant as `0x........`.
pub fn hi_hex(value: &u64) -> HexLiteral {
    U64Split::new(*value).hi_hex()
}

/// Java `int` literal: hex above `i32::MAX`, where a decimal literal would not compile.
pub fn java_int(value: &u64, hex: &bool) -> String {
    uint(value, &(*hex || *value > 0x7FFF_FFFF))
}

/// `NveNodeRow` -> `NODE_ROW` (Java / TS constant prefix of a record).
pub fn record_prefix(name: &str) -> String {
    screaming(name.strip_prefix("Nve").unwrap_or(name))
}

/// Constant prefix of one runtime ABI enum's values: its name without the `NveRuntime` / `Nve`
/// namespace, `SCREAMING` (`NveRuntimeStatus` -> `STATUS`), so each enum owns its own prefix.
pub fn enum_prefix(name: &str) -> String {
    screaming(
        name.strip_prefix("NveRuntime")
            .or_else(|| name.strip_prefix("Nve"))
            .unwrap_or(name),
    )
}

/// Constant name without the `NVE_` namespace (`NVE_ABI_MAGIC` -> `ABI_MAGIC`).
pub fn bare(name: &str) -> &str {
    name.strip_prefix("NVE_").unwrap_or(name)
}

/// Java `int` constants for a layout value: one constant when every data model agrees, else
/// `_64` / `_32` (or one constant when those agree) plus `_I386` where i386 differs from ILP32.
pub fn java_width(name: String, w: &Width) -> String {
    let line = |suffix: &str, v: usize| format!("    public static final int {name}{suffix} = {v};");
    let mut out = match w.fixed() {
        Some(n) => return line("", n),
        None if w.p64() == w.p32() => line("", w.p64()),
        None => format!("{}\n{}", line("_64", w.p64()), line("_32", w.p32())),
    };
    if w.i386() != w.p32() {
        out.push('\n');
        out.push_str(&line("_I386", w.i386()));
    }
    out
}

/// Single-line comment text: newlines folded so a doc can never end the comment early.
pub fn one_line(doc: &str) -> String {
    doc.split('\n')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" ")
        .replace("*/", "* /")
}

pub mod c {
    use super::*;

    pub fn scalar(s: Scalar) -> &'static str {
        match s {
            Scalar::Bool => "bool",
            Scalar::U8 => "uint8_t",
            Scalar::U16 => "uint16_t",
            Scalar::U32 => "uint32_t",
            Scalar::U64 => "uint64_t",
            Scalar::I32 => "int32_t",
            Scalar::I64 => "int64_t",
            Scalar::F32 => "float",
            Scalar::F64 => "double",
            Scalar::Usize => "size_t",
        }
    }

    fn resolve_c_alias<'a>(aliases: &'a [AbiAliasDescriptor], name: &'a str) -> &'a str {
        aliases
            .iter()
            .find(|a| a.rust_name == Some(name))
            .map_or(name, |a| a.name)
    }

    /// C spelling of an ABI type (arrays spell their element type; see [`decl`]).
    pub fn ty(aliases: &[AbiAliasDescriptor], spelling: &str) -> String {
        match Ty::parse(spelling) {
            Ok(Ty::Unit | Ty::Void) => "void".to_owned(),
            Ok(Ty::Scalar(s)) => scalar(s).to_owned(),
            Ok(Ty::Ptr {
                mutable: false,
                pointee,
            }) => format!("const {}*", ty(aliases, pointee)),
            Ok(Ty::Ptr {
                mutable: true,
                pointee,
            }) => format!("{}*", ty(aliases, pointee)),
            Ok(Ty::Array { elem, .. }) => ty(aliases, elem),
            Ok(Ty::Nullable(cb)) => cb.to_owned(),
            Ok(Ty::Named(name)) => resolve_c_alias(aliases, name).to_owned(),
            Err(_) => "/* unsupported */".to_owned(),
        }
    }

    /// `type name` / `type name[N]` declaration of a field or parameter.
    pub fn decl(aliases: &[AbiAliasDescriptor], spelling: &str, name: &str) -> String {
        match Ty::parse(spelling) {
            Ok(Ty::Array { len, .. }) => format!("{} {name}[{len}]", ty(aliases, spelling)),
            _ => format!("{} {name}", ty(aliases, spelling)),
        }
    }

    /// C parameter list (`void` when empty, else comma-separated `type name`).
    pub fn fn_params(aliases: &[AbiAliasDescriptor], params: &[AbiParamDescriptor]) -> String {
        if params.is_empty() {
            "void".to_owned()
        } else {
            params
                .iter()
                .map(|p| decl(aliases, p.ty, p.name))
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    /// `16u`, `(sizeof(void*) == 8u ? 16u : 8u)`, or, when i386 System V differs from other 32-bit
    /// targets, `(sizeof(void*) == 8u ? 24u : (NVE_ABI_U64_FIELD_ALIGN == 8u ? 16u : 12u))`.
    pub fn width(w: &Width) -> String {
        if let Some(n) = w.fixed() {
            return format!("{n}u");
        }
        let p32 = if w.p32() == w.i386() {
            format!("{}u", w.p32())
        } else {
            format!(
                "(NVE_ABI_U64_FIELD_ALIGN == 8u ? {}u : {}u)",
                w.p32(),
                w.i386()
            )
        };
        format!("(sizeof(void*) == 8u ? {}u : {p32})", w.p64())
    }

    pub fn field_decl(aliases: &[AbiAliasDescriptor], f: &FieldLayout<'_>) -> String {
        match f.ty {
            Ty::Array { elem, len } => format!("{} {}[{len}]", ty(aliases, elem), f.desc.name),
            _ => format!("{} {}", ty(aliases, f.desc.abi_type()), f.desc.name),
        }
    }
}

pub mod dart {
    use super::*;

    /// Follows value-alias chains (`NodeId`/`NveNodeId` -> `u32`, `NveAppHandle` -> `*mut c_void`)
    /// to their underlying ABI type, stopping at callbacks and aliases emitted as Dart typedefs
    /// (`keep_aliases`, e.g. `NveRuntimeHandle`).
    fn resolve_alias<'a>(
        spelling: &'a str,
        aliases: &'a [AbiAliasDescriptor],
        keep_aliases: &'a [AbiAliasDescriptor],
        callbacks: &[&str],
    ) -> &'a str {
        let mut cur = spelling;
        for _ in 0..=aliases.len() {
            let Ok(Ty::Named(name)) = Ty::parse(cur) else {
                break;
            };
            if callbacks.contains(&name) || keep_aliases.iter().any(|k| k.name == name) {
                break;
            }
            match aliases
                .iter()
                .find(|a| a.name == name || a.rust_name == Some(name))
            {
                Some(alias) => cur = alias.target,
                None => break,
            }
        }
        cur
    }

    /// `dart:ffi` native type (`ffi.Uint32`, `ffi.Pointer<NveValue>`, `NveStrSlice`,
    /// `ffi.Pointer<ffi.NativeFunction<NveRuntimeWakeNative>>`).
    pub fn native(
        aliases: &[AbiAliasDescriptor],
        keep_aliases: &[AbiAliasDescriptor],
        callbacks: &[&str],
        spelling: &str,
    ) -> String {
        match Ty::parse(resolve_alias(spelling, aliases, keep_aliases, callbacks)) {
            Ok(Ty::Unit | Ty::Void) => "ffi.Void".to_owned(),
            Ok(Ty::Scalar(s)) => native_scalar(s).to_owned(),
            Ok(Ty::Ptr { pointee, .. }) => format!(
                "ffi.Pointer<{}>",
                native(aliases, keep_aliases, callbacks, pointee)
            ),
            Ok(Ty::Array { elem, .. }) => format!(
                "ffi.Array<{}>",
                native(aliases, keep_aliases, callbacks, elem)
            ),
            Ok(Ty::Nullable(cb)) => {
                format!(
                    "ffi.Pointer<ffi.NativeFunction<{}Native>>",
                    callback_stem(cb)
                )
            }
            Ok(Ty::Named(name)) if callbacks.contains(&name) => {
                format!(
                    "ffi.Pointer<ffi.NativeFunction<{}Native>>",
                    callback_stem(name)
                )
            }
            Ok(Ty::Named(name)) => name.to_owned(),
            Err(_) => "/* unsupported */".to_owned(),
        }
    }

    /// Dart-side `asFunction` type (`int`, `double`, `bool`, `void`, `ffi.Pointer<...>`).
    pub fn dart_side(
        aliases: &[AbiAliasDescriptor],
        keep_aliases: &[AbiAliasDescriptor],
        callbacks: &[&str],
        spelling: &str,
    ) -> String {
        match Ty::parse(resolve_alias(spelling, aliases, keep_aliases, callbacks)) {
            Ok(Ty::Unit | Ty::Void) => "void".to_owned(),
            Ok(Ty::Scalar(s)) => dart_scalar(s).to_owned(),
            _ => native(aliases, keep_aliases, callbacks, spelling),
        }
    }

    /// Single-line `RET Function(T name, ...)` signature (`native` or `dart_side`).
    pub fn fn_sig(
        aliases: &[AbiAliasDescriptor],
        keep_aliases: &[AbiAliasDescriptor],
        callbacks: &[&str],
        params: &[AbiParamDescriptor],
        ret: &str,
        as_dart: bool,
    ) -> String {
        let pick = |ty: &str| {
            if as_dart {
                dart_side(aliases, keep_aliases, callbacks, ty)
            } else {
                native(aliases, keep_aliases, callbacks, ty)
            }
        };
        let list: Vec<String> = params
            .iter()
            .map(|p| format!("{} {}", pick(p.ty), camel(p.name)))
            .collect();
        format!("{} Function({})", pick(ret), list.join(", "))
    }

    fn native_scalar(s: Scalar) -> &'static str {
        match s {
            Scalar::Bool => "ffi.Bool",
            Scalar::U8 => "ffi.Uint8",
            Scalar::U16 => "ffi.Uint16",
            Scalar::U32 => "ffi.Uint32",
            Scalar::U64 => "ffi.Uint64",
            Scalar::I32 => "ffi.Int32",
            Scalar::I64 => "ffi.Int64",
            Scalar::F32 => "ffi.Float",
            Scalar::F64 => "ffi.Double",
            Scalar::Usize => "ffi.Size",
        }
    }

    fn dart_scalar(s: Scalar) -> &'static str {
        match s {
            Scalar::Bool => "bool",
            Scalar::F32 | Scalar::F64 => "double",
            _ => "int",
        }
    }

    /// `external` field declaration (with its `@ffi.*` annotation) of an `ffi.Struct` / `ffi.Union`.
    pub fn field(
        aliases: &[AbiAliasDescriptor],
        keep_aliases: &[AbiAliasDescriptor],
        callbacks: &[&str],
        f: &FieldLayout<'_>,
    ) -> String {
        let name = camel(f.desc.name);
        let resolved = resolve_alias(f.desc.abi_type(), aliases, keep_aliases, callbacks);
        match Ty::parse(resolved) {
            Ok(Ty::Scalar(s)) => {
                format!(
                    "@{}()\n  external {} {name};",
                    native_scalar(s),
                    dart_scalar(s)
                )
            }
            Ok(Ty::Array { len, .. }) => {
                format!(
                    "@ffi.Array({len})\n  external {} {name};",
                    native(aliases, keep_aliases, callbacks, resolved)
                )
            }
            _ => format!(
                "external {} {name};",
                native(aliases, keep_aliases, callbacks, resolved)
            ),
        }
    }

    /// Dart typedef stem of a C callback typedef (`NveRuntimeWakeFn` -> `NveRuntimeWake`).
    pub fn callback_stem(name: &str) -> &str {
        name.trim_end_matches("Fn")
    }

    /// Dart enum constant name; Dart keywords get a `Value` suffix.
    pub fn variant(name: &str) -> String {
        let id = camel(name);
        match id.as_str() {
            "null" | "bool" | "default" | "true" | "false" | "switch" | "case" | "class"
            | "const" | "final" | "in" | "is" | "new" | "this" | "var" | "void" | "with"
            | "enum" | "super" | "return" | "do" | "if" | "else" | "for" | "while" => {
                format!("{id}Value")
            }
            _ => id,
        }
    }
}

pub mod swift {
    use super::*;

    /// Swift enum case / static constant name: `lowerCamel`, with a trailing `_` for Swift
    /// keywords (and `none`, which would shadow `Optional.none`).
    pub fn variant(name: &str) -> String {
        const RESERVED: &[&str] = &[
            "Any", "Self", "as", "break", "case", "catch", "class", "continue", "default", "defer",
            "deinit", "do", "else", "enum", "extension", "fallthrough", "false", "for", "func",
            "guard", "if", "import", "in", "init", "inout", "internal", "is", "let", "nil", "none",
            "operator", "private", "protocol", "public", "repeat", "rethrows", "return", "self",
            "static", "struct", "subscript", "super", "switch", "throw", "throws", "true", "try",
            "typealias", "var", "where", "while",
        ];
        let id = camel(name);
        if RESERVED.contains(&id.as_str()) {
            format!("{id}_")
        } else {
            id
        }
    }
}

pub mod ts {
    use super::*;

    /// TS constant-object key of an enum variant (`PassThrough` -> `PASS_THROUGH`).
    pub fn key(name: &str) -> String {
        screaming(name)
    }
}
