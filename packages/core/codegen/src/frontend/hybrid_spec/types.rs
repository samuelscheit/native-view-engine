//! `syn` type, literal, and attribute-value conversion for the hybrid Rust spec frontend.

use proc_macro2::Span;
use syn::spanned::Spanned;
use syn::{Expr, ExprLit, ExprUnary, GenericArgument, Lit, PathArguments, Type, UnOp};

use crate::error::CodegenError;
use crate::schema::hybrid::{HybridDefaultValue, HybridType};

/// Maximum nesting of types (`Option<Vec<*const T>>`, ...) accepted from a spec.
const MAX_TYPE_DEPTH: usize = 128;

/// A parse error located at `span` (1-based line and column).
pub(super) fn error_at(span: Span, message: impl Into<String>) -> CodegenError {
    let start = span.start();
    CodegenError::parse(start.line.max(1), start.column + 1, message)
}

impl From<syn::Error> for CodegenError {
    fn from(e: syn::Error) -> Self {
        error_at(e.span(), e.to_string())
    }
}

/// Converts a Rust type from a hybrid spec into the hybrid IR.
pub(super) fn convert_type(ty: &Type) -> Result<HybridType, CodegenError> {
    convert_at(ty, 0)
}

fn convert_at(ty: &Type, depth: usize) -> Result<HybridType, CodegenError> {
    if depth >= MAX_TYPE_DEPTH {
        return Err(error_at(ty.span(), "maximum type nesting depth exceeded"));
    }
    let inner = |t: &Type| convert_at(t, depth + 1).map(Box::new);
    match ty {
        Type::Tuple(t) if t.elems.is_empty() => Ok(HybridType::Void),
        Type::Paren(p) => convert_at(&p.elem, depth + 1),
        Type::Ptr(p) if p.mutability.is_some() => Ok(HybridType::MutPtr(inner(&p.elem)?)),
        Type::Ptr(p) => Ok(HybridType::ConstPtr(inner(&p.elem)?)),
        Type::Array(a) => Ok(HybridType::FixedArray {
            elem: inner(&a.elem)?,
            len: array_len(&a.len)?,
        }),
        Type::Reference(r) => match r.elem.as_ref() {
            Type::Slice(s) => sequence(*inner(&s.elem)?),
            other => convert_at(other, depth + 1),
        },
        Type::Path(p) if p.qself.is_none() => {
            let Some(seg) = p.path.segments.last() else {
                return Err(error_at(ty.span(), "empty type path"));
            };
            let name = seg.ident.to_string();
            let args = generic_types(&seg.arguments)?;
            match (name.as_str(), args.as_slice()) {
                (_, []) => Ok(scalar(&name)),
                ("Option", [t]) => Ok(HybridType::Optional(inner(t)?)),
                ("Vec", [t]) => sequence(*inner(t)?),
                ("Result", [ok, err]) => Ok(HybridType::Result {
                    ok: inner(ok)?,
                    err: inner(err)?,
                }),
                _ => Err(error_at(
                    seg.span(),
                    format!("unsupported generic type '{name}<...>' in hybrid spec"),
                )),
            }
        }
        _ => Err(error_at(ty.span(), "unsupported type in hybrid spec")),
    }
}

/// `[u8]` / `Vec<u8>` are byte buffers; any other element is a sequence.
fn sequence(elem: HybridType) -> Result<HybridType, CodegenError> {
    Ok(match elem {
        HybridType::U8 => HybridType::Bytes,
        other => HybridType::Vec(Box::new(other)),
    })
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

fn generic_types(args: &PathArguments) -> Result<Vec<&Type>, CodegenError> {
    match args {
        PathArguments::None => Ok(Vec::new()),
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

/// `true` when the integer literal was written in hexadecimal (`0x...`), kept for emitted consts.
pub(super) fn is_hex(expr: &Expr) -> bool {
    matches!(expr, Expr::Lit(ExprLit { lit: Lit::Int(i), .. }) if i.to_string().starts_with("0x"))
}

/// A `#[prop(default = ...)]` value: bool, (negative) integer or float, string, `None`, or
/// `Enum::Variant`.
pub(super) fn default_value(expr: &Expr) -> Result<HybridDefaultValue, CodegenError> {
    let unsupported = || error_at(expr.span(), "unsupported #[prop(default = ...)] value");
    match expr {
        Expr::Lit(ExprLit { lit, .. }) => match lit {
            Lit::Bool(b) => Ok(HybridDefaultValue::Bool(b.value)),
            Lit::Str(s) => Ok(HybridDefaultValue::String(s.value())),
            Lit::Int(_) => unsigned(expr, "default").map(HybridDefaultValue::UInt),
            Lit::Float(f) => f
                .base10_parse()
                .map(HybridDefaultValue::Float)
                .map_err(Into::into),
            _ => Err(unsupported()),
        },
        Expr::Unary(ExprUnary {
            op: UnOp::Neg(_),
            expr: operand,
            ..
        }) => match operand.as_ref() {
            Expr::Lit(ExprLit {
                lit: Lit::Float(f), ..
            }) => f
                .base10_parse::<f64>()
                .map(|v| HybridDefaultValue::Float(-v))
                .map_err(Into::into),
            Expr::Lit(ExprLit {
                lit: Lit::Int(i), ..
            }) => {
                let magnitude = i
                    .base10_parse::<u64>()
                    .map_err(|_| error_at(i.span(), "integer literal out of i64 range"))?;
                let value = if magnitude == i64::MIN.unsigned_abs() {
                    i64::MIN
                } else {
                    i64::try_from(magnitude)
                        .ok()
                        .and_then(i64::checked_neg)
                        .ok_or_else(|| error_at(i.span(), "integer literal out of i64 range"))?
                };
                Ok(HybridDefaultValue::Int(value))
            }
            _ => Err(unsupported()),
        },
        Expr::Path(p) if p.qself.is_none() => {
            let segments: Vec<String> = p
                .path
                .segments
                .iter()
                .map(|s| s.ident.to_string())
                .collect();
            match segments.as_slice() {
                [none] if none == "None" => Ok(HybridDefaultValue::None),
                [enum_name, variant] => Ok(HybridDefaultValue::EnumVariant {
                    enum_name: enum_name.clone(),
                    variant: variant.clone(),
                }),
                _ => Err(unsupported()),
            }
        }
        _ => Err(unsupported()),
    }
}
