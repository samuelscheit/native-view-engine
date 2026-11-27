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

use std::collections::BTreeMap;

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
    let mut shapes: BTreeMap<&'a str, Shape> = BTreeMap::new();
    for file in files {
        for alias in file.aliases {
            let shape = shape_of(Ty::parse(alias.target)?, &shapes, alias.name)?;
            shapes.insert(alias.name, shape);
        }
        for e in file.enums {
            let repr = Scalar::parse(e.repr).ok_or_else(|| bad_type(e.repr, "enum repr"))?;
            shapes.insert(e.name, repr.shape());
        }
    }
    let mut out = Vec::new();
    for file in files {
        for record in file.records {
            let layout = layout_record(record, &shapes)?;
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

fn shape_of(ty: Ty<'_>, shapes: &BTreeMap<&str, Shape>, ctx: &str) -> Result<Shape, CodegenError> {
    Ok(match ty {
        Ty::Ptr { .. } => Shape::POINTER,
        Ty::Scalar(s) => s.shape(),
        Ty::Array { elem, len } => {
            let e = shape_of(Ty::parse(elem)?, shapes, ctx)?;
            Shape {
                size: e.size.zip(e.size, |n, _| n * len),
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

fn layout_record<'a>(
    record: &'a AbiRecordDescriptor,
    shapes: &BTreeMap<&str, Shape>,
) -> Result<RecordLayout<'a>, CodegenError> {
    let (name, doc, is_union, descs) = match record {
        AbiRecordDescriptor::Struct(s) => (s.name, s.doc, false, s.fields),
        AbiRecordDescriptor::Union(u) => (u.name, u.doc, true, u.fields),
    };
    let mut fields = Vec::with_capacity(descs.len());
    let (mut end, mut align) = (Width::same(0), Width::same(1));
    for desc in descs {
        let ty = Ty::parse(desc.abi_type())?;
        let shape = shape_of(ty, shapes, name)?;
        let offset = if is_union {
            Width::same(0)
        } else {
            end.zip(shape.align, round_up)
        };
        end = end.zip(offset.zip(shape.size, |o, s| o + s), usize::max);
        align = align.zip(shape.align, usize::max);
        fields.push(FieldLayout { desc, ty, offset });
    }
    let size = end.zip(align, round_up);
    Ok(RecordLayout {
        name,
        doc,
        is_union,
        size,
        align,
        fields,
    })
}

const fn round_up(n: usize, align: usize) -> usize {
    n.div_ceil(align) * align
}

/// `rustc`'s offsets are those of the running build; every declared contract must hold too.
fn check_against_rustc(
    record: &AbiRecordDescriptor,
    layout: &RecordLayout<'_>,
) -> Result<(), CodegenError> {
    let (size, expected_size, expected_size_64) = match record {
        AbiRecordDescriptor::Struct(s) => (s.size, s.expected_size, s.expected_size_64),
        AbiRecordDescriptor::Union(u) => (u.size, u.expected_size, u.expected_size_64),
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
mod tests {
    use nve_abi::schema::{
        AbiFieldDescriptor, AbiFileDescriptor, AbiRecordDescriptor, AbiStructDescriptor,
    };

    use super::{Model, Ty, layout_records};

    const fn field(
        name: &'static str,
        ty: &'static str,
        offset: usize,
        expected: Option<usize>,
    ) -> AbiFieldDescriptor {
        AbiFieldDescriptor {
            name,
            ty,
            c_type: None,
            doc: None,
            offset,
            expected_offset: expected,
            expected_offset_64: None,
        }
    }

    const fn file(records: &'static [AbiRecordDescriptor]) -> AbiFileDescriptor {
        AbiFileDescriptor {
            namespace: None,
            consts: &[],
            aliases: &[],
            enums: &[],
            records,
            modules: &[],
            c_functions: &[],
        }
    }

    const fn record(
        fields: &'static [AbiFieldDescriptor],
        size: usize,
        expected_size: Option<usize>,
    ) -> AbiRecordDescriptor {
        AbiRecordDescriptor::Struct(AbiStructDescriptor {
            name: "S",
            doc: None,
            size,
            align: 8,
            explicit_align: None,
            expected_size,
            expected_size_64: None,
            fields,
        })
    }

    // `{ u8 tag; u64 value; const void* ptr; }` as `rustc` lays it out on this 64-bit host.
    static OK: [AbiRecordDescriptor; 1] = [record(
        &[
            field("tag", "u8", 0, Some(0)),
            field("value", "u64", 8, None),
            field("ptr", "*const c_void", 16, None),
        ],
        24,
        None,
    )];

    #[test]
    fn three_data_models_from_one_descriptor() {
        assert_eq!(
            Model::host(),
            Model::P64,
            "the codegen tests run on a 64-bit host"
        );
        let ok = file(&OK);
        let r = layout_records(&[&ok]).expect("valid layout");
        let [tag, value, ptr] = r[0].fields.as_slice() else {
            panic!("three fields")
        };
        assert_eq!(tag.offset.fixed(), Some(0));
        assert_eq!(
            value.offset.0,
            [8, 8, 4],
            "i386 System V aligns u64 to 4 inside structs"
        );
        assert_eq!(ptr.offset.0, [16, 16, 12]);
        assert_eq!(r[0].size.0, [24, 24, 16]);
        assert!(!r[0].is_fixed());
    }

    #[test]
    fn drift_from_rustc_or_a_contract_is_an_error() {
        static WRONG_OFFSET: [AbiRecordDescriptor; 1] = [record(
            &[field("tag", "u8", 0, None), field("value", "u64", 4, None)],
            16,
            None,
        )];
        let err = layout_records(&[&file(&WRONG_OFFSET)])
            .unwrap_err()
            .to_string();
        assert!(err.contains("`value` offset 8 != rustc 4"), "{err}");

        static BROKEN_CONTRACT: [AbiRecordDescriptor; 1] = [record(
            &[
                field("tag", "u8", 0, None),
                field("value", "u64", 8, Some(8)),
            ],
            16,
            None,
        )];
        let err = layout_records(&[&file(&BROKEN_CONTRACT)])
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("breaks `offset = Some(8)`"),
            "differs on i386: {err}"
        );

        static WRONG_SIZE: [AbiRecordDescriptor; 1] =
            [record(&[field("a", "u32", 0, None)], 4, Some(8))];
        assert!(
            layout_records(&[&file(&WRONG_SIZE)])
                .unwrap_err()
                .to_string()
                .contains("`size =")
        );

        static UNDECLARED: [AbiRecordDescriptor; 1] =
            [record(&[field("a", "Later", 0, None)], 4, None)];
        assert!(
            layout_records(&[&file(&UNDECLARED)])
                .unwrap_err()
                .to_string()
                .contains("before it is declared")
        );
    }

    #[test]
    fn type_spellings() {
        assert_eq!(
            Ty::parse("*mut NveFrameStats").ok(),
            Some(Ty::Ptr {
                mutable: true,
                pointee: "NveFrameStats"
            })
        );
        assert_eq!(
            Ty::parse("[u32; 5]").ok(),
            Some(Ty::Array {
                elem: "u32",
                len: 5
            })
        );
        assert_eq!(Ty::parse("()").ok(), Some(Ty::Unit));
        assert!(Ty::parse("Option<fn()>").is_err());
        assert!(Ty::parse("[u8; x]").is_err());
    }
}
