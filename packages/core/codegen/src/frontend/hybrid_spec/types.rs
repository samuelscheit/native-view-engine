//! Errors, identifiers, types, and literals of the hybrid Rust spec frontend.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Expr, ExprLit, ExprUnary, GenericArgument, Ident, Lit, PathArguments, Type, UnOp};

use crate::error::CodegenError;
use crate::schema::hybrid::{HybridDefaultValue, HybridType};

/// Maximum type nesting (`Option<Vec<...>>`): lowered types and their recursive walks stay shallow.
const MAX_NESTING: usize = 128;

/// A parse error located at `span` (1-based line, 1-based column).
pub(super) fn error_at(span: Span, message: impl Into<String>) -> CodegenError {
    let start = span.start();
    CodegenError::parse(start.line, start.column + 1, message)
}

/// A `syn` error at its own location. `syn` reports "unexpected end of input" at the call site
/// (an empty span at offset 0), so that case is located at the end of `source` instead.
pub(super) fn syn_error(e: &syn::Error, source: &str) -> CodegenError {
    let span = e.span();
    if !span.byte_range().is_empty() || span.byte_range().start > 0 {
        return error_at(span, e.to_string());
    }
    let line = source.lines().count().max(1);
    let column = source.lines().last().map_or(0, |l| l.chars().count()) + 1;
    CodegenError::parse(line, column, e.to_string())
}

/// The identifier as written. Raw identifiers (`r#type`) are rejected: every target language
/// would need its own escaping for them.
pub(super) fn ident(i: &Ident) -> Result<String, CodegenError> {
    let name = i.to_string();
    if name.starts_with("r#") {
        return Err(error_at(
            i.span(),
            format!("raw identifier `{name}` is not supported in hybrid specs"),
        ));
    }
    Ok(name)
}

/// Converts a Rust type from a hybrid spec into the hybrid IR.
pub(super) fn convert_type(ty: &Type) -> Result<HybridType, CodegenError> {
    convert_at(ty, 0)
}

fn convert_at(ty: &Type, depth: usize) -> Result<HybridType, CodegenError> {
    if depth >= MAX_NESTING {
        return Err(error_at(ty.span(), "maximum type nesting depth exceeded"));
    }
    let inner = |t: &Type| convert_at(t, depth + 1);
    match ty {
        Type::Tuple(t) if t.elems.is_empty() => Ok(HybridType::Void),
        Type::Paren(p) => inner(&p.elem),
        Type::Ptr(p) if p.mutability.is_some() => Ok(HybridType::MutPtr(Box::new(inner(&p.elem)?))),
        Type::Ptr(p) => Ok(HybridType::ConstPtr(Box::new(inner(&p.elem)?))),
        Type::Array(a) => Ok(HybridType::FixedArray {
            elem: Box::new(inner(&a.elem)?),
            len: array_len(&a.len)?,
        }),
        Type::Reference(r) => match r.elem.as_ref() {
            Type::Slice(s) => Ok(sequence(inner(&s.elem)?)),
            other => inner(other),
        },
        Type::Path(p) if p.qself.is_none() => {
            let segments = &p.path.segments;
            let Some(last) = segments.last() else {
                return Err(error_at(ty.span(), "empty type path"));
            };
            if let Some(s) = segments
                .iter()
                .rev()
                .skip(1)
                .find(|s| !s.arguments.is_none())
            {
                return Err(error_at(
                    s.arguments.span(),
                    "generic arguments are only supported on the last path segment",
                ));
            }
            let name = ident(&last.ident)?;
            let args = generic_types(&last.arguments)?;
            match (name.as_str(), args.as_slice()) {
                ("Option" | "Vec" | "Result", []) => Err(error_at(
                    last.span(),
                    format!("`{name}` needs its type arguments"),
                )),
                (_, []) => Ok(scalar(&name)),
                ("Option", [t]) => Ok(HybridType::Optional(Box::new(inner(t)?))),
                ("Vec", [t]) => Ok(sequence(inner(t)?)),
                ("Result", [ok, err]) => Ok(HybridType::Result {
                    ok: Box::new(inner(ok)?),
                    err: Box::new(inner(err)?),
                }),
                _ => Err(error_at(
                    last.span(),
                    format!("unsupported generic type '{name}<...>' in hybrid spec"),
                )),
            }
        }
        _ => Err(error_at(ty.span(), "unsupported type in hybrid spec")),
    }
}

/// `[u8]` / `Vec<u8>` are byte buffers; any other element is a sequence.
fn sequence(elem: HybridType) -> HybridType {
    match elem {
        HybridType::U8 => HybridType::Bytes,
        other => HybridType::Vec(Box::new(other)),
    }
}

fn scalar(name: &str) -> HybridType {
    match name {
        "c_void" => HybridType::Void,
        "bool" => HybridType::Bool,
        "u8" => HybridType::U8,
        "u16" => HybridType::U16,
        "usize" => HybridType::Usize,
        "i32" => HybridType::I32,
        "u32" => HybridType::U32,
        "i64" => HybridType::I64,
        "u64" => HybridType::U64,
        "f32" => HybridType::F32,
        "f64" => HybridType::F64,
        "String" | "str" => HybridType::String,
        "NveValue" => HybridType::Value,
        // Enums and handle aliases are resolved once every declaration is known.
        other => HybridType::Struct(other.to_owned()),
    }
}

/// The type arguments of `<...>`; `None` for a plain segment. Empty `<>` is an error.
fn generic_types(args: &PathArguments) -> Result<Vec<&Type>, CodegenError> {
    match args {
        PathArguments::None => Ok(Vec::new()),
        PathArguments::AngleBracketed(a) if a.args.is_empty() => Err(error_at(
            a.span(),
            "empty `<>` is not a valid hybrid spec type",
        )),
        PathArguments::AngleBracketed(a) => a
            .args
            .iter()
            .map(|g| match g {
                GenericArgument::Type(t) => Ok(t),
                other => Err(error_at(
                    other.span(),
                    "only type arguments are supported in hybrid specs",
                )),
            })
            .collect(),
        PathArguments::Parenthesized(p) => Err(error_at(
            p.span(),
            "function-style type arguments are not supported",
        )),
    }
}

fn array_len(len: &Expr) -> Result<usize, CodegenError> {
    match len {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => i
            .base10_parse::<usize>()
            .map_err(|_| error_at(i.span(), "fixed array length exceeds usize::MAX")),
        other => Err(error_at(
            other.span(),
            "expected an integer literal fixed array length",
        )),
    }
}

/// An unsigned integer literal (`u64`; suffixes ignored), else a located error.
pub(super) fn unsigned(expr: &Expr, what: &str) -> Result<u64, CodegenError> {
    match expr {
        Expr::Lit(ExprLit {
            lit: Lit::Int(i), ..
        }) => i
            .base10_parse::<u64>()
            .map_err(|_| error_at(i.span(), format!("{what} is out of u64 range"))),
        other => Err(error_at(
            other.span(),
            format!("expected an unsigned integer for {what}"),
        )),
    }
}

/// An unsigned integer literal that must fit `u32`, located at the literal.
pub(super) fn unsigned_u32(expr: &Expr, what: &str) -> Result<u32, CodegenError> {
    let value = unsigned(expr, what)?;
    u32::try_from(value)
        .map_err(|_| error_at(expr.span(), format!("{what} {value} exceeds u32::MAX")))
}

/// `true` when the integer literal was written in hexadecimal (`0x...`), kept for emitted consts.
pub(super) fn is_hex(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) if i.to_string().starts_with("0x"))
}

/// A `#[prop(default = ...)]` value: bool, (negative) integer or finite float, string, `None`, or
/// `Enum::Variant`.
pub(super) fn default_value(expr: &Expr) -> Result<HybridDefaultValue, CodegenError> {
    let unsupported = || error_at(expr.span(), "unsupported #[prop(default = ...)] value");
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => match lit {
            Lit::Bool(b) => Ok(HybridDefaultValue::Bool(b.value)),
            Lit::Str(s) => Ok(HybridDefaultValue::String(s.value())),
            Lit::Int(_) => unsigned(expr, "default").map(HybridDefaultValue::UInt),
            Lit::Float(f) => finite(f, 1.0),
            _ => Err(unsupported()),
        },
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: operand,
            ..
        }) => match operand.as_ref() {
            Expr::Lit(ExprLit {
                lit: Lit::Float(f), ..
            }) => finite(f, -1.0),
            Expr::Lit(ExprLit {
                lit: Lit::Int(i), ..
            }) => {
                let out_of_range = || error_at(i.span(), "integer literal out of i64 range");
                let magnitude = i.base10_parse::<u64>().map_err(|_| out_of_range())?;
                let value = if magnitude == i64::MIN.unsigned_abs() {
                    i64::MIN
                } else {
                    i64::try_from(magnitude)
                        .ok()
                        .and_then(i64::checked_neg)
                        .ok_or_else(out_of_range)?
                };
                Ok(HybridDefaultValue::Int(value))
            }
            _ => Err(unsupported()),
        },
        Expr::Path(p) if p.qself.is_none() => {
            if p.path.leading_colon.is_some()
                || p.path.segments.iter().any(|s| !s.arguments.is_none())
            {
                return Err(error_at(
                    p.span(),
                    "default paths are `None` or `Enum::Variant`, without `::` prefix or generics",
                ));
            }
            let segments = p
                .path
                .segments
                .iter()
                .map(|s| ident(&s.ident))
                .collect::<Result<Vec<_>, _>>()?;
            match <[String; 1]>::try_from(segments) {
                Ok([none]) if none == "None" => Ok(HybridDefaultValue::None),
                Ok(_) => Err(unsupported()),
                Err(segments) => match <[String; 2]>::try_from(segments) {
                    Ok([enum_name, variant]) => {
                        Ok(HybridDefaultValue::EnumVariant { enum_name, variant })
                    }
                    Err(_) => Err(unsupported()),
                },
            }
        }
        _ => Err(unsupported()),
    }
}

/// `sign * f` when finite (`1e400` parses to infinity, which no target can spell).
fn finite(f: &syn::LitFloat, sign: f64) -> Result<HybridDefaultValue, CodegenError> {
    let value = f
        .base10_parse::<f64>()
        .map_err(|e| error_at(f.span(), e.to_string()))?;
    if !value.is_finite() {
        return Err(error_at(f.span(), "float default must be finite"));
    }
    Ok(HybridDefaultValue::Float(sign * value))
}
