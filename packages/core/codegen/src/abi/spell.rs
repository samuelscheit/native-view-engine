//! Per-language spellings of ABI types, names, and values, called from the Askama templates.
//!
//! Every function takes the borrowed `nve-abi` descriptors directly. [`super::layout`] and
//! [`super::validate`] reject every type spelling a template cannot express before rendering
//! starts, so the `/* unsupported */` fallback below is unreachable (and would fail the host
//! compile if it were not).

use super::layout::{FieldLayout, Scalar, Ty, Width};
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

/// Java `int` literal: hex above `i32::MAX`, where a decimal literal would not compile.
pub fn java_int(value: &u64, hex: &bool) -> String {
    uint(value, &(*hex || *value > 0x7FFF_FFFF))
}

/// `NveRenderNode` -> `RENDER_NODE` (Java / TS constant prefix of a record).
pub fn record_prefix(name: &str) -> String {
    screaming(name.strip_prefix("Nve").unwrap_or(name))
}

/// Constant name without the `NVE_` namespace (`NVE_ABI_MAGIC` -> `ABI_MAGIC`).
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
    use super::*;

    /// `dart:ffi` native type (`ffi.Uint32`, `ffi.Pointer<NveValue>`, `NveStrSlice`).
    pub fn native(spelling: &str) -> String {
        match Ty::parse(spelling) {
            Ok(Ty::Unit | Ty::Void) => "ffi.Void".to_owned(),
            Ok(Ty::Scalar(s)) => native_scalar(s).to_owned(),
            Ok(Ty::Ptr { pointee, .. }) => format!("ffi.Pointer<{}>", native(pointee)),
            Ok(Ty::Array { elem, .. }) => format!("ffi.Array<{}>", native(elem)),
            Ok(Ty::Named("NveHostInvokeFn")) => {
                "ffi.Pointer<ffi.NativeFunction<NveHostInvokeNative>>".to_owned()
            }
            Ok(Ty::Named("NveHostChannelPublishFn")) => {
                "ffi.Pointer<ffi.NativeFunction<NveHostChannelPublishNative>>".to_owned()
            }
            Ok(Ty::Named("NveAppHandle" | "NveFabricHostHandle")) => {
                "ffi.Pointer<ffi.Void>".to_owned()
            }
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
    pub fn field(f: &FieldLayout<'_>) -> String {
        let name = camel(f.desc.name);
        match f.ty {
            Ty::Scalar(s) => {
                let dart = match s {
                    Scalar::Bool => "bool",
                    Scalar::F32 | Scalar::F64 => "double",
                    _ => "int",
                };
                format!("@{}()\n  external {dart} {name};", native_scalar(s))
            }
            Ty::Array { len, .. } => {
                format!(
                    "@ffi.Array({len})\n  external {} {name};",
                    native(f.desc.abi_type())
                )
            }
            _ => format!("external {} {name};", native(f.desc.abi_type())),
        }
    }

    /// `(dart:ffi native, Dart-side)` types of one C type spelling from a runtime slot signature
    /// (`const NveMapEntry*`, `uint32_t`, `NveRuntimeWakeFn`, `NveBufferSlice`).
    pub fn c_type(spelling: &str) -> (String, String) {
        let c = spelling.trim();
        let c = c.strip_prefix("const ").unwrap_or(c).trim();
        let c = c.strip_prefix("struct ").unwrap_or(c).trim();
        if let Some(pointee) = c.strip_suffix('*') {
            let ptr = format!("ffi.Pointer<{}>", c_type(pointee).0);
            return (ptr.clone(), ptr);
        }
        let pair = |native: &str, dart: &str| (native.to_owned(), dart.to_owned());
        match c {
            "void" => pair("ffi.Void", "void"),
            "int32_t" => pair("ffi.Int32", "int"),
            "uint8_t" => pair("ffi.Uint8", "int"),
            "uint16_t" => pair("ffi.Uint16", "int"),
            "uint32_t" => pair("ffi.Uint32", "int"),
            "uint64_t" => pair("ffi.Uint64", "int"),
            "int64_t" => pair("ffi.Int64", "int"),
            "size_t" => pair("ffi.Size", "int"),
            "float" => pair("ffi.Float", "double"),
            "double" => pair("ffi.Double", "double"),
            callback if callback.ends_with("Fn") => {
                let ty = format!(
                    "ffi.Pointer<ffi.NativeFunction<{}Native>>",
                    callback.trim_end_matches("Fn")
                );
                (ty.clone(), ty)
            }
            named => pair(named, named),
        }
    }

    /// `RET Function(T name, …)` of a C signature: `dart:ffi` native types, or the Dart-side
    /// types of its `asFunction` form when `dart_side`.
    pub fn c_fn(ret: &str, params: &str, dart_side: bool) -> String {
        let pick = |c: &str| {
            let (native, dart) = c_type(c);
            if dart_side { dart } else { native }
        };
        let list: Vec<String> = params
            .split(',')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(|p| {
                let split = p
                    .rfind(|ch: char| ch.is_whitespace() || ch == '*')
                    .unwrap_or(0);
                let (ty, name) = p.split_at(split + 1);
                format!("{} {}", pick(ty), to_camel_case(name.trim()))
            })
            .collect();
        format!("{} Function({})", pick(ret), list.join(", "))
    }

    /// Dart typedef stem of a C callback typedef (`NveRuntimeWakeFn` -> `NveRuntimeWake`).
    pub fn callback_stem(name: &str) -> &str {
        name.trim_end_matches("Fn")
    }

    /// Dart enum constant name; Dart keywords get a `Value` suffix.
    pub fn variant(name: &str) -> String {
        let id = to_camel_case(name);
        match id.as_str() {
            "null" | "bool" | "default" | "true" | "false" | "switch" | "case" | "class" | "const"
            | "final" | "in" | "is" | "new" | "this" | "var" | "void" | "with" | "enum" | "super"
            | "return" | "do" | "if" | "else" | "for" | "while" => format!("{id}Value"),
            _ => id,
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
