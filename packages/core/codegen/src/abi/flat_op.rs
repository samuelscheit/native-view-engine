//! Generic Askama view-model and emitter for flat 4-byte-aligned (`u32`-word) operation-stream
//! codecs (`nve_codec::FlatOpStreamDesc`).
//!
//! One generic template per host language (`c`, `swift`, `java`, `dart`, `ts`) renders any
//! validated [`FlatOpStreamDesc`] into a zero-allocation streaming validator + decoder next to the
//! static `packages/core/host-*` runtimes. `nve-codegen` stays module-free: callers (such as
//! `nve-cli`) pass `&nve_canvas::DRAW_LIST_CODEC_DESC` or any other stream descriptor.

use std::collections::{BTreeMap, BTreeSet};

use askama::Template;
use nve_codec::{FlatOpFieldType, FlatOpStreamDesc, FloatConstraint};
use nve_hash::Key64;

use super::model::U64Split;
use super::{CXX, DART, JAVA, SWIFT, TS, render, spell};
use crate::error::CodegenError;
use crate::schema::export::GeneratedFile;

/// Upper bound on wire element/op/depth counts so `count * 8` always fits signed 32-bit `int`
/// across Java, Dart, Swift (`arm64_32`), C++, and TypeScript without overflow.
const MAX_SAFE_COUNT: u32 = 0x0FFF_FFFF;

/// Reserved words across C++, Swift, Java, Dart, and TypeScript that cannot be field names.
const RESERVED_FIELD_NAMES: &[&str] = &[
    "abstract", "as", "assert", "async", "auto", "await", "bool", "boolean", "break", "byte",
    "case", "catch", "char", "class", "const", "continue", "default", "defer", "deinit", "delete",
    "do", "double", "else", "enum", "export", "extends", "extension", "extern", "false", "final",
    "finally", "float", "for", "func", "function", "goto", "guard", "if", "implements", "import",
    "in", "init", "inline", "inout", "instanceof", "int", "interface", "internal", "is", "let",
    "long", "mutating", "namespace", "native", "new", "nil", "noexcept", "null", "nullptr",
    "operator", "override", "package", "private", "protected", "protocol", "public", "register",
    "repeat", "rethrow", "rethrows", "return", "self", "short", "signed", "sizeof", "static",
    "struct", "subscript", "super", "switch", "synchronized", "template", "this", "throw",
    "throws", "transient", "true", "try", "typealias", "typedef", "typename", "typeof", "union",
    "unsigned", "using", "var", "virtual", "void", "volatile", "where", "while", "with", "yield",
];

/// Validated scalar wire kind inside a [`FlatOpScalarField`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScalarKind {
    U32,
    ColorRgba,
    F32Finite,
    F32NonNegative,
}

impl ScalarKind {
    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32Finite | Self::F32NonNegative)
    }

    pub const fn is_non_negative(self) -> bool {
        matches!(self, Self::F32NonNegative)
    }

    pub const fn is_color(self) -> bool {
        matches!(self, Self::ColorRgba)
    }

    pub const fn cxx_type(self) -> &'static str {
        match self {
            Self::U32 => "uint32_t",
            Self::ColorRgba => "ColorRgba",
            Self::F32Finite | Self::F32NonNegative => "float",
        }
    }

    pub const fn cxx_default(self) -> &'static str {
        match self {
            Self::U32 => "0u",
            Self::ColorRgba => "{}",
            Self::F32Finite | Self::F32NonNegative => "0.0f",
        }
    }

    pub const fn java_type(self) -> &'static str {
        match self {
            Self::U32 | Self::ColorRgba => "int",
            Self::F32Finite | Self::F32NonNegative => "float",
        }
    }

    pub const fn swift_type(self) -> &'static str {
        match self {
            Self::U32 | Self::ColorRgba => "UInt32",
            Self::F32Finite | Self::F32NonNegative => "Float",
        }
    }

    pub const fn dart_type(self) -> &'static str {
        match self {
            Self::U32 | Self::ColorRgba => "int",
            Self::F32Finite | Self::F32NonNegative => "double",
        }
    }

    pub const fn ts_type(self) -> &'static str {
        "number"
    }
}

/// Role of an operation in an optional balanced push/pop scope discipline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScopeRole {
    None,
    Push,
    Pop,
}

/// One fixed scalar field inside an operation payload.
#[derive(Debug, Clone)]
struct FlatOpScalarField<'a> {
    name: &'a str,
    camel: String,
    byte_offset: usize,
    kind: ScalarKind,
}

/// Trailing variable-length `(x, y)` `f32` vertex slice inside an operation payload.
#[derive(Debug, Clone)]
struct FlatOpPointSliceField<'a> {
    name: &'a str,
    camel: String,
    count_byte_offset: usize,
    max_const_screaming: &'a str,
    max_const_pascal: String,
    max_const_camel: String,
    coord_non_negative: bool,
}

/// Deduplicated slice-length bound emitted as a stream constant.
#[derive(Debug, Clone)]
struct FlatOpSliceLimit<'a> {
    screaming: &'a str,
    pascal: String,
    camel: String,
    max_len: u32,
}

/// Validated balanced push/pop scope metadata.
#[derive(Debug, Clone)]
struct FlatOpScopeModel<'a> {
    limit_screaming: &'a str,
    limit_pascal: String,
    limit_camel: String,
    max_depth: u32,
    pop_pascal: &'a str,
    pop_snake: String,
}

/// One validated operation variant.
#[derive(Debug, Clone)]
struct FlatOpModel<'a> {
    name: &'a str,
    snake: String,
    camel: String,
    screaming: String,
    opcode: u32,
    scope_role: ScopeRole,
    scalars: Vec<FlatOpScalarField<'a>>,
    point_slice: Option<FlatOpPointSliceField<'a>>,
    /// Byte length of all fixed fields plus any trailing `Point2Slice` length word.
    fixed_bytes: usize,
}

impl FlatOpModel<'_> {
    fn is_scope_push(&self) -> bool {
        self.scope_role == ScopeRole::Push
    }

    fn is_scope_pop(&self) -> bool {
        self.scope_role == ScopeRole::Pop
    }

    fn has_point_slice(&self) -> bool {
        self.point_slice.is_some()
    }
}

/// Complete validated view-model consumed by all 5 Askama flat-op templates.
#[derive(Debug, Clone)]
struct FlatOpStreamModel<'a> {
    name: &'a str,
    snake: String,
    screaming: String,
    kind_name: &'a str,
    kind: U64Split,
    max_ops_screaming: &'a str,
    max_ops_pascal: String,
    max_ops_camel: String,
    max_ops: u32,
    slice_limits: Vec<FlatOpSliceLimit<'a>>,
    has_point_slice: bool,
    has_color: bool,
    scope: Option<FlatOpScopeModel<'a>>,
    ops: Vec<FlatOpModel<'a>>,
}

impl<'a> FlatOpStreamModel<'a> {
    /// Validates `desc` up-front so every target language compiles cleanly without runtime escapes.
    pub fn new(desc: &'a FlatOpStreamDesc) -> Result<Self, CodegenError> {
        let err = |msg: String| {
            CodegenError::validation(format!("flat op stream `{}`: {msg}", desc.name))
        };

        if !is_pascal_ident(desc.name) {
            return Err(err("name must match `[A-Z][A-Za-z0-9]*`".into()));
        }
        if desc.kind_name.is_empty() || !desc.kind_name.split('.').all(is_snake_ident) {
            return Err(err(format!(
                "kind_name `{}` must be dot-separated `[a-z][a-z0-9_]*` segments",
                desc.kind_name
            )));
        }
        if !is_screaming_ident(desc.max_ops_const_name) {
            return Err(err(format!(
                "max_ops_const_name `{}` must match `[A-Z][A-Z0-9_]*`",
                desc.max_ops_const_name
            )));
        }
        if desc.max_ops == 0 || desc.max_ops > MAX_SAFE_COUNT {
            return Err(err(format!(
                "max_ops {} must be in 1..={MAX_SAFE_COUNT}",
                desc.max_ops
            )));
        }
        if desc.ops.is_empty() {
            return Err(err("must declare at least one operation".into()));
        }

        let mut seen_op_names = BTreeSet::new();
        let mut seen_opcodes = BTreeSet::new();
        let mut slice_limit_map: BTreeMap<&'a str, u32> = BTreeMap::new();
        let mut slice_limits = Vec::new();
        let mut ops = Vec::with_capacity(desc.ops.len());
        let mut has_point_slice = false;
        let mut has_color = false;

        for op in desc.ops {
            if !is_pascal_ident(op.name) {
                return Err(err(format!("op `{}` must match `[A-Z][A-Za-z0-9]*`", op.name)));
            }
            if !seen_op_names.insert(op.name) {
                return Err(err(format!("duplicate op name `{}`", op.name)));
            }
            if op.opcode == 0 {
                return Err(err(format!("op `{}` opcode must be >= 1", op.name)));
            }
            if !seen_opcodes.insert(op.opcode) {
                return Err(err(format!("duplicate opcode {} on `{}`", op.opcode, op.name)));
            }

            let mut seen_fields = BTreeSet::new();
            let mut scalars = Vec::new();
            let mut point_slice = None;

            for (idx, field) in op.fields.iter().enumerate() {
                if !is_snake_ident(field.name) {
                    return Err(err(format!(
                        "op `{}` field `{}` must match `[a-z][a-z0-9_]*`",
                        op.name, field.name
                    )));
                }
                let camel = spell::camel(field.name);
                if RESERVED_FIELD_NAMES.contains(&field.name)
                    || RESERVED_FIELD_NAMES.contains(&camel.as_str())
                {
                    return Err(err(format!(
                        "op `{}` field `{}` collides with a reserved keyword",
                        op.name, field.name
                    )));
                }
                if !seen_fields.insert(camel.clone()) {
                    return Err(err(format!(
                        "op `{}` has duplicate or colliding field `{}`",
                        op.name, field.name
                    )));
                }
                let byte_offset = idx * 4;
                match field.ty {
                    FlatOpFieldType::U32 => scalars.push(FlatOpScalarField {
                        name: field.name,
                        camel,
                        byte_offset,
                        kind: ScalarKind::U32,
                    }),
                    FlatOpFieldType::ColorRgba => {
                        has_color = true;
                        scalars.push(FlatOpScalarField {
                            name: field.name,
                            camel,
                            byte_offset,
                            kind: ScalarKind::ColorRgba,
                        });
                    }
                    FlatOpFieldType::F32(constraint) => {
                        let kind = match constraint {
                            FloatConstraint::Finite => ScalarKind::F32Finite,
                            FloatConstraint::NonNegative => ScalarKind::F32NonNegative,
                        };
                        scalars.push(FlatOpScalarField {
                            name: field.name,
                            camel,
                            byte_offset,
                            kind,
                        });
                    }
                    FlatOpFieldType::Point2Slice {
                        max_len,
                        max_const_name,
                        coord_constraint,
                    } => {
                        if idx + 1 != op.fields.len() {
                            return Err(err(format!(
                                "op `{}` Point2Slice field `{}` must be the final field",
                                op.name, field.name
                            )));
                        }
                        if !is_screaming_ident(max_const_name) {
                            return Err(err(format!(
                                "op `{}` max_const_name `{max_const_name}` must match `[A-Z][A-Z0-9_]*`",
                                op.name
                            )));
                        }
                        if max_len == 0 || max_len > MAX_SAFE_COUNT {
                            return Err(err(format!(
                                "op `{}` max_len {max_len} must be in 1..={MAX_SAFE_COUNT}",
                                op.name
                            )));
                        }
                        if max_const_name == desc.max_ops_const_name {
                            return Err(err(format!(
                                "slice limit `{max_const_name}` collides with max_ops_const_name"
                            )));
                        }
                        match slice_limit_map.get(max_const_name) {
                            Some(&prev) if prev != max_len => {
                                return Err(err(format!(
                                    "slice limit `{max_const_name}` defined with conflicting max_len ({prev} vs {max_len})"
                                )));
                            }
                            None => {
                                slice_limit_map.insert(max_const_name, max_len);
                                slice_limits.push(FlatOpSliceLimit {
                                    screaming: max_const_name,
                                    pascal: screaming_to_pascal(max_const_name),
                                    camel: spell::camel(max_const_name),
                                    max_len,
                                });
                            }
                            Some(_) => {}
                        }
                        let coord_non_negative = matches!(coord_constraint, FloatConstraint::NonNegative);
                        has_point_slice = true;
                        point_slice = Some(FlatOpPointSliceField {
                            name: field.name,
                            camel,
                            count_byte_offset: byte_offset,
                            max_const_screaming: max_const_name,
                            max_const_pascal: screaming_to_pascal(max_const_name),
                            max_const_camel: spell::camel(max_const_name),
                            coord_non_negative,
                        });
                    }
                }
            }

            let fixed_bytes = op.fields.len() * 4;
            let scope_role = match desc.scope {
                Some(s) if op.opcode == s.push_opcode => ScopeRole::Push,
                Some(s) if op.opcode == s.pop_opcode => ScopeRole::Pop,
                _ => ScopeRole::None,
            };

            ops.push(FlatOpModel {
                name: op.name,
                snake: spell::snake(op.name),
                camel: spell::camel(op.name),
                screaming: spell::screaming(op.name),
                opcode: op.opcode,
                scope_role,
                scalars,
                point_slice,
                fixed_bytes,
            });
        }

        let scope = match desc.scope {
            None => None,
            Some(s) => {
                if !is_screaming_ident(s.limit_name) {
                    return Err(err(format!(
                        "scope limit_name `{}` must match `[A-Z][A-Z0-9_]*`",
                        s.limit_name
                    )));
                }
                if s.max_depth == 0 || s.max_depth > MAX_SAFE_COUNT {
                    return Err(err(format!(
                        "scope max_depth {} must be in 1..={MAX_SAFE_COUNT}",
                        s.max_depth
                    )));
                }
                if s.limit_name == desc.max_ops_const_name
                    || slice_limit_map.contains_key(s.limit_name)
                {
                    return Err(err(format!(
                        "scope limit_name `{}` collides with another limit constant",
                        s.limit_name
                    )));
                }
                if s.push_opcode == s.pop_opcode {
                    return Err(err("scope push_opcode and pop_opcode must differ".into()));
                }
                let push_op = ops
                    .iter()
                    .find(|o| o.opcode == s.push_opcode)
                    .ok_or_else(|| err(format!("scope push_opcode {} not declared", s.push_opcode)))?;
                if push_op.has_point_slice() {
                    return Err(err(format!(
                        "scope push op `{}` cannot contain a Point2Slice",
                        push_op.name
                    )));
                }
                let pop_op = ops
                    .iter()
                    .find(|o| o.opcode == s.pop_opcode)
                    .ok_or_else(|| err(format!("scope pop_opcode {} not declared", s.pop_opcode)))?;
                if pop_op.fixed_bytes != 0 || pop_op.has_point_slice() {
                    return Err(err(format!(
                        "scope pop op `{}` must have zero fields so unwind can invoke it without arguments",
                        pop_op.name
                    )));
                }
                Some(FlatOpScopeModel {
                    limit_screaming: s.limit_name,
                    limit_pascal: screaming_to_pascal(s.limit_name),
                    limit_camel: spell::camel(s.limit_name),
                    max_depth: s.max_depth,
                    pop_pascal: pop_op.name,
                    pop_snake: pop_op.snake.clone(),
                })
            }
        };

        Ok(Self {
            name: desc.name,
            snake: spell::snake(desc.name),
            screaming: spell::screaming(desc.name),
            kind_name: desc.kind_name,
            kind: U64Split::new(Key64::of(desc.kind_name).0),
            max_ops_screaming: desc.max_ops_const_name,
            max_ops_pascal: screaming_to_pascal(desc.max_ops_const_name),
            max_ops_camel: spell::camel(desc.max_ops_const_name),
            max_ops: desc.max_ops,
            slice_limits,
            has_point_slice,
            has_color,
            scope,
            ops,
        })
    }
}

#[derive(Template)]
#[template(path = "c/flat_op_codec.hpp", escape = "none")]
struct CxxFlatOpCodec<'a> {
    m: &'a FlatOpStreamModel<'a>,
}

#[derive(Template)]
#[template(path = "swift/FlatOpCodec.swift", escape = "none")]
struct SwiftFlatOpCodec<'a> {
    m: &'a FlatOpStreamModel<'a>,
}

#[derive(Template)]
#[template(path = "java/FlatOpCodec.java", escape = "none")]
struct JavaFlatOpCodec<'a> {
    m: &'a FlatOpStreamModel<'a>,
}

#[derive(Template)]
#[template(path = "dart/flat_op_codec.dart", escape = "none")]
struct DartFlatOpCodec<'a> {
    m: &'a FlatOpStreamModel<'a>,
}

#[derive(Template)]
#[template(path = "ts/flat_op_codec.ts", escape = "none")]
struct TsFlatOpCodec<'a> {
    m: &'a FlatOpStreamModel<'a>,
}

/// Generates C++, Swift, Java, Dart, and TypeScript flat word-op stream decoders for `desc`.
pub fn generate_flat_op_codec_files(
    desc: &FlatOpStreamDesc,
) -> Result<Vec<GeneratedFile>, CodegenError> {
    let m = FlatOpStreamModel::new(desc)?;
    Ok(vec![
        render(
            &CxxFlatOpCodec { m: &m },
            format!("{CXX}/include/nve/generated/nve_{}.generated.hpp", m.snake),
        )?,
        render(
            &SwiftFlatOpCodec { m: &m },
            format!("{SWIFT}/Sources/NveHost/Nve{}.generated.swift", m.name),
        )?,
        render(
            &JavaFlatOpCodec { m: &m },
            format!("{JAVA}/main/java/dev/nve/host/generated/Nve{}.java", m.name),
        )?,
        render(
            &DartFlatOpCodec { m: &m },
            format!("{DART}/lib/nve_{}.generated.dart", m.snake),
        )?,
        render(
            &TsFlatOpCodec { m: &m },
            format!("{TS}/src/generated/nve_{}.generated.ts", m.snake),
        )?,
    ])
}

fn screaming_to_pascal(s: &str) -> String {
    let mut out = spell::camel(s);
    if let Some(first) = out.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    out
}

fn is_pascal_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_alphanumeric())
}

fn is_snake_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

fn is_screaming_ident(s: &str) -> bool {
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase())
        && chars.all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_')
}
