//! Cross-model layout verification for ABI vtable header fields and slot tables (`P64`, `P32`, `I386`).

use std::collections::{BTreeMap, BTreeSet};

use nve_abi::schema::{AbiFileDescriptor, AbiVtableDescriptor};

use super::{
    FieldLayout, Model, RecordLayout, Shape, Ty, Width, round_up, shape_of,
};
use crate::error::CodegenError;

/// Validated layout of one vtable's leading header fields and first function-pointer slot offset.
#[derive(Debug, Clone)]
pub struct VtableHeaderLayout<'a> {
    pub name: &'a str,
    pub header_fields: Vec<FieldLayout<'a>>,
    /// Byte offset of the first function-pointer slot on `[P64, P32, I386]`.
    pub slots_offset: Width,
}

/// Validates every vtable's `header_fields`, `slots_offset`, alignment, and total size across
/// `[P64, P32, I386]` and against the running host `rustc` layout.
pub fn layout_vtable_headers_on<'a>(
    base: &[RecordLayout<'a>],
    base_files: &[&'a AbiFileDescriptor],
    files: &[&'a AbiFileDescriptor],
    records: &[RecordLayout<'a>],
) -> Result<Vec<VtableHeaderLayout<'a>>, CodegenError> {
    let mut shapes: BTreeMap<&'a str, Shape> = base
        .iter()
        .chain(records)
        .map(|r| (r.name, Shape { size: r.size, align: r.align }))
        .collect();
    let mut enums: BTreeSet<&'a str> = BTreeSet::new();
    let mut callbacks: BTreeSet<&'a str> = BTreeSet::new();

    for e in base_files.iter().chain(files).flat_map(|f| f.enums) {
        enums.insert(e.name);
    }
    for cb in base_files.iter().chain(files).flat_map(|f| f.callbacks) {
        callbacks.insert(cb.name);
        shapes.insert(cb.name, Shape::POINTER);
    }
    for alias in base_files.iter().chain(files).flat_map(|f| f.aliases) {
        let shape = shape_of(Ty::parse(alias.target)?, &shapes, &enums, &callbacks, alias.name)?;
        shapes.insert(alias.name, shape);
        if let Some(rn) = alias.rust_name {
            shapes.insert(rn, shape);
        }
    }

    let mut out = Vec::new();
    for v in files.iter().flat_map(|f| f.vtables) {
        out.push(layout_one_vtable(v, &shapes, &enums, &callbacks)?);
    }
    Ok(out)
}

fn layout_one_vtable<'a>(
    v: &'a AbiVtableDescriptor,
    shapes: &BTreeMap<&str, Shape>,
    enums: &BTreeSet<&str>,
    callbacks: &BTreeSet<&str>,
) -> Result<VtableHeaderLayout<'a>, CodegenError> {
    let host = Model::host();
    let fail = |what: String| {
        Err(CodegenError::validation(format!(
            "vtable `{}` C layout drifted from `rustc`: {what}",
            v.name
        )))
    };

    let mut header_fields = Vec::with_capacity(v.header_fields.len());
    let (mut end, mut align) = (Width::same(0), Width::same(1));
    for desc in v.header_fields {
        let ty = Ty::parse(desc.abi_type())?;
        if let Ty::Nullable(cb) = ty {
            return Err(CodegenError::validation(format!(
                "vtable `{}` header field `{}` uses callback typedef `Option<{cb}>`; vtable slots must be inline `extern \"C\" fn(...)` fields after header fields",
                v.name, desc.name
            )));
        }
        if let Ty::Named(name) = ty
            && callbacks.contains(name)
        {
            return Err(CodegenError::validation(format!(
                "vtable `{}` header field `{}` uses callback typedef `{name}`; vtable slots must be inline `extern \"C\" fn(...)` fields after header fields",
                v.name, desc.name
            )));
        }
        let shape = shape_of(ty, shapes, enums, callbacks, v.name)?;
        let offset = end.try_zip(shape.align, round_up, v.name)?;
        end = offset.try_zip(shape.size, usize::checked_add, v.name)?;
        align = align.zip(shape.align, usize::max);

        if offset.at(host) != desc.offset {
            return fail(format!(
                "header `{}` offset {} != rustc {}",
                desc.name,
                offset.at(host),
                desc.offset
            ));
        }
        if desc
            .expected_offset
            .is_some_and(|n| offset.fixed() != Some(n))
        {
            return fail(format!(
                "header `{}` offset {:?} breaks `offset = {:?}`",
                desc.name, offset, desc.expected_offset
            ));
        }
        if desc.expected_offset_64.is_some_and(|n| offset.p64() != n) {
            return fail(format!(
                "header `{}` 64-bit offset {} breaks `offset64`",
                desc.name,
                offset.p64()
            ));
        }
        header_fields.push(FieldLayout { desc, ty, offset });
    }

    let slots_offset = end.try_zip(Shape::POINTER.align, round_up, v.name)?;
    if slots_offset.at(host) != v.slots_offset {
        return fail(format!(
            "slots_offset {} != rustc {}",
            slots_offset.at(host),
            v.slots_offset
        ));
    }
    let vtable_align = align.zip(Shape::POINTER.align, usize::max);
    let slots_bytes = Shape::POINTER
        .size
        .try_zip(Width::same(v.methods.len()), usize::checked_mul, v.name)?;
    let raw_size = slots_offset.try_zip(slots_bytes, usize::checked_add, v.name)?;
    let size = raw_size.try_zip(vtable_align, round_up, v.name)?;
    if vtable_align.at(host) != v.align {
        return fail(format!(
            "alignment {} != rustc {}",
            vtable_align.at(host),
            v.align
        ));
    }
    if size.at(host) != v.size {
        return fail(format!("size {} != rustc {}", size.at(host), v.size));
    }

    Ok(VtableHeaderLayout {
        name: v.name,
        header_fields,
        slots_offset,
    })
}
