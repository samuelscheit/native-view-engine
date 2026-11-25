//! Intermediate Representation (IR), `#[repr(C)]` layout computation, semantic validation,
//! and deterministic FNV-1a64 schema hashing for NVE's zero-copy `RelocSlice` code generator.
//!
//! Schemas are constructed directly as typed Rust values (e.g. from `trait NveSchema` /
//! `#[derive(NveAbi)]` in each owning crate) with zero `.nve.rs` text parsing and direct binary FFI.

use std::collections::{BTreeMap, BTreeSet};

use crate::error::CodegenError;
use crate::schema::hybrid::{fnv1a_64_update, FNV1A_64_OFFSET_BASIS};

/// Fixed byte size of the 8-byte-aligned `RelocHeader` (`magic + schema_hash + total_len +
/// flags + root_offset + _pad`).
pub const RELOC_HEADER_SIZE: usize = 32;

/// Required alignment of `RelocHeader`.
pub const RELOC_HEADER_ALIGN: usize = 8;

/// Fixed byte size of `RelocSlice<T>` and `RelocStr` (`offset: u32, len: u32`).
pub const RELOC_SLICE_SIZE: usize = 8;

/// Required alignment of `RelocSlice<T>` and `RelocStr`.
pub const RELOC_SLICE_ALIGN: usize = 4;

/// Default maximum recursion depth enforced by `VerifyMode::Full`.
pub const RELOC_MAX_VERIFY_DEPTH: usize = 32;

/// Backing integer representation for a `#[repr(u8/u16/u32)]` `RelocEnum`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RelocEnumRepr {
    U8,
    U16,
    U32,
}

impl RelocEnumRepr {
    pub const fn size(self) -> usize {
        match self {
            Self::U8 => 1,
            Self::U16 => 2,
            Self::U32 => 4,
        }
    }

    pub const fn align(self) -> usize {
        self.size()
    }

    pub const fn max_value(self) -> u32 {
        match self {
            Self::U8 => u8::MAX as u32,
            Self::U16 => u16::MAX as u32,
            Self::U32 => u32::MAX,
        }
    }

    pub const fn rust_type(self) -> &'static str {
        match self {
            Self::U8 => "u8",
            Self::U16 => "u16",
            Self::U32 => "u32",
        }
    }

    pub const fn cpp_type(self) -> &'static str {
        match self {
            Self::U8 => "uint8_t",
            Self::U16 => "uint16_t",
            Self::U32 => "uint32_t",
        }
    }
}

/// One named discriminant of a [`RelocEnum`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocEnumVariant {
    pub name: String,
    pub value: u32,
    pub doc: Option<String>,
}

impl RelocEnumVariant {
    pub fn new(name: impl Into<String>, value: u32) -> Self {
        Self {
            name: name.into(),
            value,
            doc: None,
        }
    }
}

/// Fixed-width C-ABI enum (`#[repr(u8)]`, `#[repr(u16)]`, or `#[repr(u32)]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocEnum {
    pub name: String,
    pub repr: RelocEnumRepr,
    pub variants: Vec<RelocEnumVariant>,
    pub doc: Option<String>,
}

/// Supported zero-copy field types inside a [`RelocStruct`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RelocType {
    Bool,
    U8,
    I8,
    U16,
    I16,
    U32,
    I32,
    F32,
    U64,
    I64,
    F64,
    FixedArray { elem: Box<RelocType>, len: usize },
    Enum(String),
    Struct(String),
    RelocSlice(Box<RelocType>),
    RelocStr,
}

impl RelocType {
    /// Deterministic canonical signature included in the FNV-1a64 schema hash.
    pub fn canonical_signature(&self) -> String {
        match self {
            Self::Bool => "bool".to_string(),
            Self::U8 => "u8".to_string(),
            Self::I8 => "i8".to_string(),
            Self::U16 => "u16".to_string(),
            Self::I16 => "i16".to_string(),
            Self::U32 => "u32".to_string(),
            Self::I32 => "i32".to_string(),
            Self::F32 => "f32".to_string(),
            Self::U64 => "u64".to_string(),
            Self::I64 => "i64".to_string(),
            Self::F64 => "f64".to_string(),
            Self::FixedArray { elem, len } => format!("[{};{len}]", elem.canonical_signature()),
            Self::Enum(name) => format!("enum:{name}"),
            Self::Struct(name) => format!("struct:{name}"),
            Self::RelocSlice(elem) => format!("reloc_slice<{}>", elem.canonical_signature()),
            Self::RelocStr => "reloc_str".to_string(),
        }
    }

    /// True if this type contains any `RelocSlice` or `RelocStr` indirection.
    pub fn has_reloc_indirection(&self, schema: &RelocSchema) -> bool {
        match self {
            Self::RelocSlice(_) | Self::RelocStr => true,
            Self::FixedArray { elem, .. } => elem.has_reloc_indirection(schema),
            Self::Struct(name) => schema.find_struct(name).is_some_and(|s| {
                s.fields
                    .iter()
                    .any(|f| f.ty.has_reloc_indirection(schema))
            }),
            _ => false,
        }
    }
}

/// One named field inside a [`RelocStruct`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocField {
    pub name: String,
    pub ty: RelocType,
    pub doc: Option<String>,
}

impl RelocField {
    pub fn new(name: impl Into<String>, ty: RelocType) -> Self {
        Self {
            name: name.into(),
            ty,
            doc: None,
        }
    }
}

/// `#[repr(C)]` struct definition inside a [`RelocSchema`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocStruct {
    pub name: String,
    pub fields: Vec<RelocField>,
    pub align: Option<usize>,
    pub doc: Option<String>,
}

/// Computed `#[repr(C)]` byte offset, size, and alignment of one field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FieldLayout {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub align: usize,
}

/// Computed `#[repr(C)]` layout metadata for one [`RelocStruct`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructLayout {
    pub name: String,
    pub size: usize,
    pub align: usize,
    pub fields: Vec<FieldLayout>,
    /// True if the compiler inserts any implicit interior or trailing padding bytes.
    pub has_implicit_padding: bool,
    /// True if the struct contains any `RelocSlice` or `RelocStr` field transitively.
    pub has_reloc_indirection: bool,
    /// True if the struct has no `RelocSlice`/`RelocStr` indirection and zero implicit padding,
    /// making zero-copy `as_bytes(&self) -> &[u8]` sound with zero padding UB.
    pub is_pure_pod: bool,
    /// True if `VerifyMode::Full` must inspect bytes of this struct (contains `bool`, `enum`,
    /// `RelocStr`, `RelocSlice`, or a nested struct that needs verification).
    pub needs_value_verify: bool,
}

/// Complete declarative zero-copy `Reloc` binary slab schema.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RelocSchema {
    pub name: String,
    pub magic: [u8; 8],
    pub root: String,
    pub enums: Vec<RelocEnum>,
    pub structs: Vec<RelocStruct>,
}

impl RelocSchema {
    pub fn find_enum(&self, name: &str) -> Option<&RelocEnum> {
        self.enums.iter().find(|e| e.name == name)
    }

    pub fn find_struct(&self, name: &str) -> Option<&RelocStruct> {
        self.structs.iter().find(|s| s.name == name)
    }

    /// Returns structs in topological order (direct by-value dependencies before dependents).
    pub fn structs_topological(&self) -> Vec<&RelocStruct> {
        let mut out = Vec::with_capacity(self.structs.len());
        let mut visited = BTreeSet::new();
        for s in &self.structs {
            self.visit_topo(s, &mut visited, &mut out);
        }
        out
    }

    fn visit_topo<'a>(
        &'a self,
        s: &'a RelocStruct,
        visited: &mut BTreeSet<&'a str>,
        out: &mut Vec<&'a RelocStruct>,
    ) {
        if !visited.insert(s.name.as_str()) {
            return;
        }
        for f in &s.fields {
            visit_direct_struct_deps(&f.ty, &mut |dep| {
                if let Some(ds) = self.find_struct(dep) {
                    self.visit_topo(ds, visited, out);
                }
            });
        }
        out.push(s);
    }

    /// Computes exact `#[repr(C)]` layouts for every struct in topological order.
    pub fn compute_layouts(&self) -> Result<BTreeMap<String, StructLayout>, CodegenError> {
        self.validate()?;
        let mut layouts: BTreeMap<String, StructLayout> = BTreeMap::new();
        for s in self.structs_topological() {
            let mut offset = 0usize;
            let mut max_align = s.align.unwrap_or(1);
            let mut field_layouts = Vec::with_capacity(s.fields.len());
            let mut sum_field_sizes = 0usize;
            let mut any_nested_padding = false;
            let mut any_indirection = false;
            let mut needs_verify = false;

            for f in &s.fields {
                let (f_size, f_align, f_pad, f_indir, f_verify) =
                    type_metrics(&f.ty, self, &layouts)?;
                offset = align_up(offset, f_align);
                field_layouts.push(FieldLayout {
                    name: f.name.clone(),
                    offset,
                    size: f_size,
                    align: f_align,
                });
                offset = offset.checked_add(f_size).ok_or_else(|| {
                    CodegenError::validation(format!("struct '{}' size overflow", s.name))
                })?;
                sum_field_sizes += f_size;
                max_align = max_align.max(f_align);
                any_nested_padding |= f_pad;
                any_indirection |= f_indir;
                needs_verify |= f_verify;
            }

            let total_size = align_up(offset, max_align);
            let has_implicit_padding = any_nested_padding || (sum_field_sizes != total_size);
            let is_pure_pod = !any_indirection && !has_implicit_padding;

            layouts.insert(
                s.name.clone(),
                StructLayout {
                    name: s.name.clone(),
                    size: total_size,
                    align: max_align,
                    fields: field_layouts,
                    has_implicit_padding,
                    has_reloc_indirection: any_indirection,
                    is_pure_pod,
                    needs_value_verify: needs_verify,
                },
            );
        }
        Ok(layouts)
    }

    /// Minimum alignment required for the base pointer of any slab encoding this schema.
    pub fn slab_align(&self, layouts: &BTreeMap<String, StructLayout>) -> usize {
        layouts
            .values()
            .map(|l| l.align)
            .fold(RELOC_HEADER_ALIGN, usize::max)
    }

    /// Deterministic 64-bit FNV-1a schema hash covering magic, root, enums (names, repr,
    /// variants), and every struct's name, size, alignment, field names, types, and byte offsets.
    pub fn schema_hash(&self) -> Result<u64, CodegenError> {
        let layouts = self.compute_layouts()?;
        Ok(self.schema_hash_with_layouts(&layouts))
    }

    pub(crate) fn schema_hash_with_layouts(&self, layouts: &BTreeMap<String, StructLayout>) -> u64 {
        let mut h = FNV1A_64_OFFSET_BASIS;
        h = hash_str(h, "nve.reloc.schema.v1");
        h = hash_str(h, &self.name);
        h = fnv1a_64_update(h, &self.magic);
        h = hash_str(h, &self.root);

        for e in &self.enums {
            h = hash_str(h, "enum");
            h = hash_str(h, &e.name);
            h = hash_str(h, e.repr.rust_type());
            for v in &e.variants {
                h = hash_str(h, &v.name);
                h = fnv1a_64_update(h, &v.value.to_le_bytes());
            }
        }

        for s in self.structs_topological() {
            let layout = &layouts[&s.name];
            h = hash_str(h, "struct");
            h = hash_str(h, &s.name);
            h = fnv1a_64_update(h, &(layout.size as u64).to_le_bytes());
            h = fnv1a_64_update(h, &(layout.align as u64).to_le_bytes());
            for (f, fl) in s.fields.iter().zip(layout.fields.iter()) {
                h = hash_str(h, &f.name);
                h = hash_str(h, &f.ty.canonical_signature());
                h = fnv1a_64_update(h, &(fl.offset as u64).to_le_bytes());
                h = fnv1a_64_update(h, &(fl.size as u64).to_le_bytes());
            }
        }
        h
    }
}

pub(crate) const fn align_up(offset: usize, align: usize) -> usize {
    if align <= 1 {
        return offset;
    }
    let rem = offset % align;
    if rem == 0 {
        offset
    } else {
        offset + (align - rem)
    }
}

/// Returns `(size, align, has_implicit_padding, has_indirection, needs_verify)`.
pub(crate) fn type_metrics(
    ty: &RelocType,
    schema: &RelocSchema,
    layouts: &BTreeMap<String, StructLayout>,
) -> Result<(usize, usize, bool, bool, bool), CodegenError> {
    match ty {
        RelocType::Bool => Ok((1, 1, false, false, true)),
        RelocType::U8 | RelocType::I8 => Ok((1, 1, false, false, false)),
        RelocType::U16 | RelocType::I16 => Ok((2, 2, false, false, false)),
        RelocType::U32 | RelocType::I32 | RelocType::F32 => Ok((4, 4, false, false, false)),
        RelocType::U64 | RelocType::I64 | RelocType::F64 => Ok((8, 8, false, false, false)),
        RelocType::Enum(name) => {
            let e = schema.find_enum(name).ok_or_else(|| {
                CodegenError::validation(format!("unknown enum '{name}'"))
            })?;
            Ok((e.repr.size(), e.repr.align(), false, false, true))
        }
        RelocType::Struct(name) => {
            let l = layouts.get(name).ok_or_else(|| {
                CodegenError::validation(format!("layout missing for struct '{name}'"))
            })?;
            Ok((
                l.size,
                l.align,
                l.has_implicit_padding,
                l.has_reloc_indirection,
                l.needs_value_verify,
            ))
        }
        RelocType::FixedArray { elem, len } => {
            let (es, ea, ep, ei, ev) = type_metrics(elem, schema, layouts)?;
            let total = es.checked_mul(*len).ok_or_else(|| {
                CodegenError::validation("fixed array byte size overflow")
            })?;
            Ok((total, ea, ep, ei, ev))
        }
        RelocType::RelocSlice(_) | RelocType::RelocStr => {
            Ok((RELOC_SLICE_SIZE, RELOC_SLICE_ALIGN, false, true, true))
        }
    }
}

fn hash_str(mut h: u64, s: &str) -> u64 {
    h = fnv1a_64_update(h, &(s.len() as u32).to_le_bytes());
    fnv1a_64_update(h, s.as_bytes())
}

pub(crate) fn visit_direct_struct_deps<'a, F: FnMut(&'a str)>(ty: &'a RelocType, f: &mut F) {
    match ty {
        RelocType::Struct(name) => f(name.as_str()),
        RelocType::FixedArray { elem, .. } => visit_direct_struct_deps(elem, f),
        // `RelocSlice<T>` is always 8 bytes (offset + len) and breaks value-size cycles.
        _ => {}
    }
}
