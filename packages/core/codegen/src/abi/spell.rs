//! Per-language spellings of ABI types, names, and values, called from the Askama templates.
//!
//! Every function takes the borrowed `nve-abi` descriptors directly. [`super::layout`] and
//! [`super::validate`] reject every type spelling a template cannot express before rendering
//! starts, so the `/* unsupported */` fallback below is unreachable (and would fail the host
//! compile if it were not).

use nve_abi::schema::AbiAliasDescriptor;

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

/// Constant name without the `NVE_` namespace (`NVE_ABI_MAGIC` -> `ABI_MAGIC`).
/// Constant prefix of one runtime ABI enum's values: its name without the `NveRuntime` / `Nve`
/// namespace, `SCREAMING` (`NveRuntimeStatus` -> `STATUS`), so each enum owns its own prefix.
pub fn enum_prefix(name: &str) -> String {
    screaming(name.strip_prefix("NveRuntime").or_else(|| name.strip_prefix("Nve")).unwrap_or(name))
}

pub fn bare(name: &str) -> &str {
    name.strip_prefix("NVE_").unwrap_or(name)
}

/// Java `int` constants for a layout value: one constant when every data model agrees, else
/// `_64` / `_32` (or one constant when those agree) plus `_I386` where i386 differs from ILP32.
pub fn java_width(name: String, w: &Width) -> String {
    let line =
        |suffix: &str, v: usize| format!("    public static final int {name}{suffix} = {v};");
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

    /// C spelling of an ABI type (arrays spell their element type; see [`decl`]).
    pub fn ty(spelling: &str) -> String {
        match Ty::parse(spelling) {
            Ok(Ty::Unit | Ty::Void) => "void".to_owned(),
            Ok(Ty::Scalar(s)) => scalar(s).to_owned(),
            Ok(Ty::Ptr {
                mutable: false,
                pointee,
            }) => format!("const {}*", ty(pointee)),
            Ok(Ty::Ptr {
                mutable: true,
                pointee,
            }) => format!("{}*", ty(pointee)),
            Ok(Ty::Array { elem, .. }) => ty(elem),
            Ok(Ty::Named(name)) => name.to_owned(),
            Err(_) => "/* unsupported */".to_owned(),
        }
    }

    /// `type name` / `type name[N]` declaration of a field or parameter.
    pub fn decl(spelling: &str, name: &str) -> String {
        match Ty::parse(spelling) {
            Ok(Ty::Array { len, .. }) => format!("{} {name}[{len}]", ty(spelling)),
            _ => format!("{} {name}", ty(spelling)),
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

    pub fn field_decl(f: &FieldLayout<'_>) -> String {
        match f.ty {
            Ty::Array { elem, len } => format!("{} {}[{len}]", ty(elem), f.desc.name),
            _ => format!("{} {}", ty(f.desc.abi_type()), f.desc.name),
        }
    }
}

pub mod dart {
    use std::collections::BTreeMap;

    use super::*;

    /// Follows value-alias chains (`NveNodeId` -> `u32`, `NveAppHandle` -> `*mut c_void`) to their
    /// underlying ABI type; callback aliases stay named for their custom `Native` typedefs.
    fn resolve_alias<'a>(spelling: &'a str, aliases: &'a [AbiAliasDescriptor]) -> &'a str {
        let mut cur = spelling;
        for _ in 0..=aliases.len() {
            let Ok(Ty::Named(name)) = Ty::parse(cur) else {
                break;
            };
            if matches!(name, "NveHostInvokeFn" | "NveHostChannelPublishFn") {
                break;
            }
            match aliases.iter().find(|a| a.name == name) {
                Some(alias) => cur = alias.target,
                None => break,
            }
        }
        cur
    }

    /// `dart:ffi` native type (`ffi.Uint32`, `ffi.Pointer<NveValue>`, `NveStrSlice`).
    pub fn native(aliases: &[AbiAliasDescriptor], spelling: &str) -> String {
        match Ty::parse(resolve_alias(spelling, aliases)) {
            Ok(Ty::Unit | Ty::Void) => "ffi.Void".to_owned(),
            Ok(Ty::Scalar(s)) => native_scalar(s).to_owned(),
            Ok(Ty::Ptr { pointee, .. }) => format!("ffi.Pointer<{}>", native(aliases, pointee)),
            Ok(Ty::Array { elem, .. }) => format!("ffi.Array<{}>", native(aliases, elem)),
            Ok(Ty::Named("NveHostInvokeFn")) => {
                "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_owned()
            }
            Ok(Ty::Named("NveHostChannelPublishFn")) => {
                "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_owned()
            }
            Ok(Ty::Named("NveAppHandle")) => "ffi.Pointer<ffi.Void>".to_owned(),
            Ok(Ty::Named(name)) => name.to_owned(),
            Err(_) => "/* unsupported */".to_owned(),
        }
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

    /// `external` field declaration (with its `@ffi.*` annotation) of an `ffi.Struct` / `ffi.Union`.
    pub fn field(aliases: &[AbiAliasDescriptor], f: &FieldLayout<'_>) -> String {
        let name = camel(f.desc.name);
        let resolved = resolve_alias(f.desc.abi_type(), aliases);
        match Ty::parse(resolved) {
            Ok(Ty::Scalar(s)) => {
                let dart = match s {
                    Scalar::Bool => "bool",
                    Scalar::F32 | Scalar::F64 => "double",
                    _ => "int",
                };
                format!("@{}()\n  external {dart} {name};", native_scalar(s))
            }
            Ok(Ty::Array { len, .. }) => {
                format!(
                    "@ffi.Array({len})\n  external {} {name};",
                    native(aliases, resolved)
                )
            }
            _ => format!("external {} {name};", native(aliases, resolved)),
        }
    }

    /// The named C types a runtime signature may spell besides scalars and pointers.
    pub struct CNames<'a> {
        /// Records and opaque typedefs (`NveBufferSlice`, `NveRuntimeHandle`).
        pub records: std::collections::BTreeSet<&'a str>,
        /// Function-pointer typedefs (`NveRuntimeWakeFn`); each has a Dart `<Stem>Native` typedef.
        pub callbacks: std::collections::BTreeSet<&'a str>,
        /// Scalar / handle typedefs from the Core ABI (`NveNodeId` -> `u32`, `NveTableKey` -> `u64`).
        pub aliases: BTreeMap<&'a str, &'a str>,
    }

    impl CNames<'_> {
        /// `(dart:ffi native, Dart-side)` types of one C type spelling (`const NveMapEntry*`,
        /// `uint32_t`, `NveRuntimeWakeFn`, `const char* const*`); `Err` for any type the runtime
        /// ABI does not declare, so no signature silently renders as an unresolved Dart name.
        pub fn c_type(&self, spelling: &str) -> Result<(String, String), String> {
            let c = spelling.trim();
            let c = c.strip_prefix("const ").unwrap_or(c).trim_start();
            let c = c.strip_prefix("struct ").unwrap_or(c).trim_start();
            if let Some(pointee) = c.strip_suffix('*') {
                // `T* const*`: the `const` qualifies the inner pointer, not the pointee type.
                let pointee = pointee.trim_end();
                let pointee = match pointee.strip_suffix("const") {
                    Some(inner) if inner.ends_with([' ', '*']) => inner,
                    _ => pointee,
                };
                let ptr = format!("ffi.Pointer<{}>", self.c_type(pointee)?.0);
                return Ok((ptr.clone(), ptr));
            }
            let pair = |native: &str, dart: &str| Ok((native.to_owned(), dart.to_owned()));
            match c {
                "void" => pair("ffi.Void", "void"),
                "bool" => pair("ffi.Bool", "bool"),
                "char" => pair("ffi.Char", "int"),
                "int8_t" => pair("ffi.Int8", "int"),
                "int16_t" => pair("ffi.Int16", "int"),
                "int32_t" => pair("ffi.Int32", "int"),
                "int64_t" => pair("ffi.Int64", "int"),
                "uint8_t" => pair("ffi.Uint8", "int"),
                "uint16_t" => pair("ffi.Uint16", "int"),
                "uint32_t" => pair("ffi.Uint32", "int"),
                "uint64_t" => pair("ffi.Uint64", "int"),
                "size_t" => pair("ffi.Size", "int"),
                "float" => pair("ffi.Float", "double"),
                "double" => pair("ffi.Double", "double"),
                callback if self.callbacks.contains(callback) => {
                    let ty = format!("ffi.Pointer<ffi.NativeFunction<{}Native>>", callback_stem(callback));
                    Ok((ty.clone(), ty))
                }
                named if self.records.contains(named) => pair(named, named),
                alias if self.aliases.contains_key(alias) => match self.aliases[alias] {
                    "bool" => pair("ffi.Bool", "bool"),
                    "u8" => pair("ffi.Uint8", "int"),
                    "u16" => pair("ffi.Uint16", "int"),
                    "u32" => pair("ffi.Uint32", "int"),
                    "u64" => pair("ffi.Uint64", "int"),
                    "i32" => pair("ffi.Int32", "int"),
                    "i64" => pair("ffi.Int64", "int"),
                    "f32" => pair("ffi.Float", "double"),
                    "f64" => pair("ffi.Double", "double"),
                    "usize" => pair("ffi.Size", "int"),
                    "*mut c_void" | "*const c_void" => pair("ffi.Pointer<ffi.Void>", "ffi.Pointer<ffi.Void>"),
                    other => Err(format!("unsupported alias target `{other}` for `{alias}`")),
                },
                unknown => Err(format!("unknown C type `{unknown}`")),
            }
        }

        /// `RET Function(T name, …)` of a C signature: `dart:ffi` native types, or the Dart-side
        /// types of its `asFunction` form when `dart_side`. `params` is `void` or a list of named
        /// parameters; a by-value `void` parameter or an unnamed one is an `Err`.
        pub fn c_fn(&self, ret: &str, params: &str, dart_side: bool) -> Result<String, String> {
            let pick = |c: &str| -> Result<String, String> {
                let (native, dart) = self.c_type(c)?;
                Ok(if dart_side { dart } else { native })
            };
            let params = params.trim();
            let list = if params == "void" || params.is_empty() {
                Vec::new()
            } else {
                params
                    .split(',')
                    .map(|p| {
                        let p = p.trim();
                        let split = p.rfind(|ch: char| ch.is_whitespace() || ch == '*').map_or(0, |i| i + 1);
                        let (ty, name) = p.split_at(split);
                        let named = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
                            && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                        if !named || ty.trim().is_empty() {
                            return Err(format!("parameter `{p}` needs a type and a name"));
                        }
                        // Resolved, so `const void x` is refused like `void x`.
                        if self.c_type(ty)?.0 == "ffi.Void" {
                            return Err(format!("parameter `{p}` is `void`"));
                        }
                        Ok(format!("{} {}", pick(ty)?, to_camel_case(name)))
                    })
                    .collect::<Result<Vec<_>, _>>()?
            };
            Ok(format!("{} Function({})", pick(ret)?, list.join(", ")))
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
            "null" | "bool" | "default" | "true" | "false" | "switch" | "case" | "class" | "const"
            | "final" | "in" | "is" | "new" | "this" | "var" | "void" | "with" | "enum" | "super"
            | "return" | "do" | "if" | "else" | "for" | "while" => format!("{id}Value"),
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
        if RESERVED.contains(&id.as_str()) { format!("{id}_") } else { id }
    }
}

pub mod ts {
    use super::*;

    /// TS constant-object key of an enum variant (`PassThrough` -> `PASS_THROUGH`).
    pub fn key(name: &str) -> String {
        screaming(name)
    }
}
