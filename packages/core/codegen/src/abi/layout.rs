//! ABI type spellings and the C layout of every record on 32-bit and 64-bit targets.
//!
//! `#[derive(NveAbi)]` records `rustc`'s field offsets for the build target only. Hosts also run on
//! 32-bit targets, so this module recomputes every record's C layout for the three data models NVE
//! runs on, from the field types:
//!
//! - **LP64 / LLP64** (64-bit pointers);
//! - **ILP32** with 8-byte-aligned 64-bit fields (`armeabi-v7a`, `arm64_32` watchOS, wasm32, Windows x86);
//! - **i386 System V** (Android x86, 32-bit Linux), where 64-bit fields are only 4-byte aligned
//!   inside structs.
//!
//! It then checks the model of the running build against `rustc` and every declared `offset` /
//! `size` contract. A record whose layout drifts is a codegen error, never a wrong `static_assert`.

use std::collections::{BTreeMap, BTreeSet};

use nve_abi::schema::{AbiFieldDescriptor, AbiFileDescriptor, AbiRecordDescriptor};

use crate::error::CodegenError;

/// Fixed-width scalar of the ABI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scalar {
    Bool,
    U8,
    U16,
    U32,
    U64,
    I32,
    I64,
    F32,
    F64,
    Usize,
}

impl Scalar {
    fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "bool" => Self::Bool,
            "u8" => Self::U8,
            "u16" => Self::U16,
            "u32" => Self::U32,
            "u64" => Self::U64,
            "i32" => Self::I32,
            "i64" => Self::I64,
            "f32" => Self::F32,
            "f64" => Self::F64,
            "usize" => Self::Usize,
            _ => return None,
        })
    }

    /// Size and in-struct alignment under each [`Model`].
    fn shape(self) -> Shape {
        match self {
            Self::Bool | Self::U8 => Shape::same(1),
            Self::U16 => Shape::same(2),
            Self::U32 | Self::I32 | Self::F32 => Shape::same(4),
            Self::U64 | Self::I64 | Self::F64 => Shape {
                size: Width::same(8),
                align: Width([8, 8, 4]),
            },
            Self::Usize => Shape::POINTER,
        }
    }
}

/// The C data models NVE hosts run on, in [`Width`] order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Model {
    /// 64-bit pointers.
    P64 = 0,
    /// 32-bit pointers, 8-byte-aligned 64-bit fields.
    P32 = 1,
    /// 32-bit pointers, 4-byte-aligned 64-bit fields (i386 System V).
    I386 = 2,
}

impl Model {
    /// The model of the running build.
    pub const fn host() -> Self {
        #[repr(C)]
        struct Probe {
            pad: u8,
            value: u64,
        }
        if size_of::<usize>() == 8 {
            Self::P64
        } else if std::mem::offset_of!(Probe, value) == 8 {
            Self::P32
        } else {
            Self::I386
        }
    }
}

/// One parsed ABI type spelling (`u32`, `*const NveValue`, `[u32; 5]`, `NveStrSlice`, `()`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ty<'a> {
    Unit,
    Void,
    Scalar(Scalar),
    Ptr { mutable: bool, pointee: &'a str },
    Array { elem: &'a str, len: usize },
    Named(&'a str),
}

impl<'a> Ty<'a> {
    pub fn parse(spelling: &'a str) -> Result<Self, CodegenError> {
        let s = spelling.trim();
        if s == "()" {
            return Ok(Self::Unit);
        }
        if s == "c_void" {
            return Ok(Self::Void);
        }
        if let Some(scalar) = Scalar::parse(s) {
            return Ok(Self::Scalar(scalar));
        }
        if let Some(pointee) = s.strip_prefix("*const ") {
            return Ok(Self::Ptr {
                mutable: false,
                pointee: pointee.trim(),
            });
        }
        if let Some(pointee) = s.strip_prefix("*mut ") {
            return Ok(Self::Ptr {
                mutable: true,
                pointee: pointee.trim(),
            });
        }
        if let Some(body) = s.strip_prefix('[').and_then(|b| b.strip_suffix(']')) {
            let (elem, len) = body
                .split_once(';')
                .ok_or_else(|| bad_type(spelling, "array needs `[T; N]`"))?;
            let len = len
                .trim()
                .parse()
                .map_err(|_| bad_type(spelling, "array length"))?;
            return Ok(Self::Array {
                elem: elem.trim(),
                len,
            });
        }
        if !s.is_empty() && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
            return Ok(Self::Named(s));
        }
        Err(bad_type(spelling, "unsupported ABI type spelling"))
    }
}

fn bad_type(spelling: &str, why: &str) -> CodegenError {
    CodegenError::validation(format!("ABI type `{spelling}`: {why}"))
}

/// A size or offset under each [`Model`] (`[P64, P32, I386]`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Width(pub [usize; 3]);

impl Width {
    const fn same(n: usize) -> Self {
        Self([n; 3])
    }

    pub const fn at(self, model: Model) -> usize {
        self.0[model as usize]
    }

    /// 64-bit value.
    pub const fn p64(self) -> usize {
        self.0[0]
    }

    /// 32-bit value (wasm32, `armeabi-v7a`, `arm64_32`): what Dart / TypeScript / Java readers use.
    pub const fn p32(self) -> usize {
        self.0[1]
    }

    /// i386 System V value.
    pub const fn i386(self) -> usize {
        self.0[2]
    }

    /// The value when every model agrees.
    pub const fn fixed(self) -> Option<usize> {
        let [a, b, c] = self.0;
        if a == b && b == c { Some(a) } else { None }
    }

    fn zip(self, other: Self, f: impl Fn(usize, usize) -> usize) -> Self {
        Self([
            f(self.0[0], other.0[0]),
            f(self.0[1], other.0[1]),
            f(self.0[2], other.0[2]),
        ])
    }

    /// [`Self::zip`] for arithmetic that can overflow `usize`.
    fn try_zip(
        self,
        other: Self,
        f: impl Fn(usize, usize) -> Option<usize>,
        ctx: &str,
    ) -> Result<Self, CodegenError> {
        let mut out = [0; 3];
        for (i, slot) in out.iter_mut().enumerate() {
            *slot = f(self.0[i], other.0[i])
                .ok_or_else(|| bad_type(ctx, "layout arithmetic overflows usize"))?;
        }
        Ok(Self(out))
    }
}

/// Size and alignment of one type.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shape {
    size: Width,
    align: Width,
}

impl Shape {
    const fn same(size: usize) -> Self {
        Self {
            size: Width::same(size),
            align: Width::same(size),
        }
    }
    const POINTER: Self = Self {
        size: Width([8, 4, 4]),
        align: Width([8, 4, 4]),
    };
}

/// One field of a [`RecordLayout`].
#[derive(Debug, Clone, Copy)]
pub struct FieldLayout<'a> {
    pub desc: &'a AbiFieldDescriptor,
    pub ty: Ty<'a>,
    pub offset: Width,
}

impl FieldLayout<'_> {
    pub fn name(&self) -> &str {
        self.desc.name
    }
}

/// C layout of one struct or union on both pointer widths.
#[derive(Debug, Clone)]
pub struct RecordLayout<'a> {
    pub name: &'a str,
    pub doc: Option<&'a str>,
    pub is_union: bool,
    pub size: Width,
    pub align: Width,
    pub fields: Vec<FieldLayout<'a>>,
    /// Rust `#[repr(C, align(N <= 16))]`, reproduced in C with `NVE_ALIGNAS(N)` (`_Alignas` /
    /// `alignas`) whenever it exceeds some data model's natural alignment.
    pub alignas: Option<usize>,
    /// Rust `align(N)` that only aligns Rust-owned slabs (it changes no size and no containing
    /// layout), so C keeps the natural alignment Swift's ClangImporter imports as a plain value.
    pub slab_align: Option<usize>,
    /// Alignment from the fields alone (before any `alignas`).
    pub natural_align: Width,
}

impl RecordLayout<'_> {
    /// `true` when no size or offset depends on the pointer width.
    pub fn is_fixed(&self) -> bool {
        self.size.fixed().is_some() && self.fields.iter().all(|f| f.offset.fixed().is_some())
    }
}

/// Computes the layout of every record of `files`, in declaration order, and checks it against
/// `rustc` and the declared contracts.
pub fn layout_records<'a>(
    files: &[&'a AbiFileDescriptor],
) -> Result<Vec<RecordLayout<'a>>, CodegenError> {
    layout_records_on(&[], files)
}

/// [`layout_records`] for files whose records may store records of an already laid out header
/// they include (`base`), e.g. the runtime ABI storing `NveStrSlice` by value.
pub fn layout_records_on<'a>(
    base: &[RecordLayout<'a>],
    files: &[&'a AbiFileDescriptor],
) -> Result<Vec<RecordLayout<'a>>, CodegenError> {
    let mut shapes: BTreeMap<&'a str, Shape> = base
        .iter()
        .map(|r| (r.name, Shape { size: r.size, align: r.align }))
        .collect();
    let mut enums: BTreeSet<&'a str> = BTreeSet::new();
    let embedded = embedded_by_value(files);
    // Enums have no storable shape (C enums are `int`-sized), so every by-value use, also through
    // an array or an alias, is rejected; they are collected before any alias is shaped.
    for e in files.iter().flat_map(|f| f.enums) {
        Scalar::parse(e.repr).ok_or_else(|| bad_type(e.repr, "enum repr"))?;
        enums.insert(e.name);
    }
    for alias in files.iter().flat_map(|f| f.aliases) {
        let shape = shape_of(Ty::parse(alias.target)?, &shapes, &enums, alias.name)?;
        shapes.insert(alias.name, shape);
    }
    let mut out = Vec::new();
    for file in files {
        for record in file.records {
            let layout = layout_record(record, &shapes, &enums, &embedded)?;
            check_against_rustc(record, &layout)?;
            shapes.insert(
                layout.name,
                Shape {
                    size: layout.size,
                    align: layout.align,
                },
            );
            out.push(layout);
        }
    }
    Ok(out)
}

fn shape_of(
    ty: Ty<'_>,
    shapes: &BTreeMap<&str, Shape>,
    enums: &BTreeSet<&str>,
    ctx: &str,
) -> Result<Shape, CodegenError> {
    Ok(match ty {
        // A pointer's size does not depend on its pointee, so the pointee is not walked.
        Ty::Ptr { .. } => Shape::POINTER,
        Ty::Named(name) if enums.contains(name) => {
            return Err(CodegenError::validation(format!(
                "`{ctx}` stores enum `{name}` by value: C enums are `int`-sized whatever the Rust repr; store its integer repr"
            )));
        }
        Ty::Scalar(s) => s.shape(),
        Ty::Array { len: 0, .. } => {
            return Err(bad_type(ctx, "zero-length arrays are not valid C"));
        }
        Ty::Array { elem, len } => {
            let e = shape_of(Ty::parse(elem)?, shapes, enums, ctx)?;
            let mut size = [0; 3];
            for (out, n) in size.iter_mut().zip(e.size.0) {
                *out = n
                    .checked_mul(len)
                    .ok_or_else(|| bad_type(ctx, "array size overflows usize"))?;
            }
            Shape {
                size: Width(size),
                align: e.align,
            }
        }
        Ty::Named(name) => *shapes.get(name).ok_or_else(|| {
            CodegenError::validation(format!("`{ctx}` uses `{name}` before it is declared"))
        })?,
        Ty::Unit | Ty::Void => {
            return Err(bad_type(ctx, "`()` / `c_void` cannot be stored by value"));
        }
    })
}

/// Names of records some other record stores by value (`T`, `[T; N]`, or through a type alias),
/// so their alignment is part of a containing layout.
fn embedded_by_value<'a>(files: &[&'a AbiFileDescriptor]) -> BTreeSet<&'a str> {
    let aliases: BTreeMap<&str, &str> = files
        .iter()
        .flat_map(|f| f.aliases)
        .map(|a| (a.name, a.target))
        .collect();
    let mut out = BTreeSet::new();
    for record in files.iter().flat_map(|f| f.records) {
        let fields = match record {
            AbiRecordDescriptor::Struct(s) => s.fields,
            AbiRecordDescriptor::Union(u) => u.fields,
        };
        for f in fields {
            let mut ty = Ty::parse(f.abi_type());
            // Alias chains are bounded by the alias count, so a cycle cannot loop; arrays are
            // peeled without spending that budget.
            for _ in 0..=aliases.len() {
                while let Ok(Ty::Array { elem, .. }) = ty {
                    ty = Ty::parse(elem);
                }
                let Ok(Ty::Named(name)) = ty else { break };
                match aliases.get(name) {
                    Some(target) => ty = Ty::parse(target),
                    None => {
                        out.insert(name);
                        break;
                    }
                }
            }
        }
    }
    out
}

fn layout_record<'a>(
    record: &'a AbiRecordDescriptor,
    shapes: &BTreeMap<&str, Shape>,
    enums: &BTreeSet<&str>,
    embedded: &BTreeSet<&str>,
) -> Result<RecordLayout<'a>, CodegenError> {
    let (name, doc, is_union, descs, explicit_align) = match record {
        AbiRecordDescriptor::Struct(s) => (s.name, s.doc, false, s.fields, s.explicit_align),
        AbiRecordDescriptor::Union(u) => (u.name, u.doc, true, u.fields, u.explicit_align),
    };
    let mut fields = Vec::with_capacity(descs.len());
    let (mut end, mut align) = (Width::same(0), Width::same(1));
    for desc in descs {
        let ty = Ty::parse(desc.abi_type())?;
        let ctx = name; // record-level context: no per-field allocation on the success path
        let shape = shape_of(ty, shapes, enums, ctx)?;
        let offset = if is_union {
            Width::same(0)
        } else {
            end.try_zip(shape.align, round_up, ctx)?
        };
        end = end.zip(
            offset.try_zip(shape.size, usize::checked_add, ctx)?,
            usize::max,
        );
        align = align.zip(shape.align, usize::max);
        fields.push(FieldLayout { desc, ty, offset });
    }
    let natural_align = align;
    let natural_size = end.try_zip(align, round_up, name)?;
    let (mut alignas, mut slab_align) = (None, None);
    if let Some(explicit) = explicit_align {
        if !explicit.is_power_of_two() {
            return Err(bad_type(name, "`align(N)` must be a non-zero power of two"));
        }
        if align.0.iter().any(|&n| explicit > n) {
            let over = align.zip(Width::same(explicit), usize::max);
            if explicit <= MAX_C_ALIGNAS {
                // C reproduces it exactly (Swift's ClangImporter imports `_Alignas` up to 16).
                alignas = Some(explicit);
                align = over;
            } else if natural_size.try_zip(over, round_up, name)? == natural_size
                && !embedded.contains(name)
            {
                // Larger alignments only align Rust-owned slabs (`NveRenderNode`'s 64-byte rows):
                // allowed when they change no size and no containing layout.
                slab_align = Some(explicit);
            } else {
                return Err(bad_type(
                    name,
                    &format!(
                        "`align({explicit})` changes the C layout, and C / Swift hosts cannot over-align past {MAX_C_ALIGNAS}"
                    ),
                ));
            }
        }
    }
    let size = end.try_zip(align, round_up, name)?;
    Ok(RecordLayout {
        name,
        doc,
        is_union,
        size,
        align,
        fields,
        alignas,
        slab_align,
        natural_align,
    })
}

/// Largest `align(N)` C reproduces with `NVE_ALIGNAS(N)` (Swift's ClangImporter rejects more).
pub const MAX_C_ALIGNAS: usize = 16;

/// `n` rounded up to `align` (a power of two, so never 0), or `None` on overflow.
fn round_up(n: usize, align: usize) -> Option<usize> {
    n.div_ceil(align).checked_mul(align)
}

/// `rustc`'s offsets are those of the running build; every declared contract must hold too.
fn check_against_rustc(
    record: &AbiRecordDescriptor,
    layout: &RecordLayout<'_>,
) -> Result<(), CodegenError> {
    let (size, align, expected_size, expected_size_64) = match record {
        AbiRecordDescriptor::Struct(s) => (s.size, s.align, s.expected_size, s.expected_size_64),
        AbiRecordDescriptor::Union(u) => (u.size, u.align, u.expected_size, u.expected_size_64),
    };
    let host = Model::host();
    let fail = |what: String| {
        Err(CodegenError::validation(format!(
            "`{}` C layout drifted from `rustc`: {what}",
            layout.name
        )))
    };
    if layout.size.at(host) != size {
        return fail(format!("size {} != rustc {size}", layout.size.at(host)));
    }
    // C reproduces Rust's alignment, except a slab-only `align(N)` (`slab_align`).
    let c_align = layout.align.at(host);
    if c_align.max(layout.slab_align.unwrap_or(1)) != align {
        return fail(format!("alignment {c_align} != rustc {align}"));
    }
    if expected_size.is_some_and(|n| layout.size.fixed() != Some(n)) {
        return fail(format!(
            "size {:?} breaks the `size = {expected_size:?}` contract",
            layout.size
        ));
    }
    if expected_size_64.is_some_and(|n| layout.size.p64() != n) {
        return fail(format!(
            "64-bit size {} breaks `size_64 = {expected_size_64:?}`",
            layout.size.p64()
        ));
    }
    for f in &layout.fields {
        let d = f.desc;
        if f.offset.at(host) != d.offset {
            return fail(format!(
                "`{}` offset {} != rustc {}",
                d.name,
                f.offset.at(host),
                d.offset
            ));
        }
        if d.expected_offset
            .is_some_and(|n| f.offset.fixed() != Some(n))
        {
            return fail(format!(
                "`{}` offset {:?} breaks `offset = {:?}`",
                d.name, f.offset, d.expected_offset
            ));
        }
        if d.expected_offset_64.is_some_and(|n| f.offset.p64() != n) {
            return fail(format!(
                "`{}` 64-bit offset {} breaks `offset64`",
                d.name,
                f.offset.p64()
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
