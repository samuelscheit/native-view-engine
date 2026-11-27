//! Enums, structs, their raw C-ABI carriers (`NveHybridRaw<Name>`), and POD `NveHybridOpt*`
//! options.

use crate::naming::{escape_c_like_string, rust_ident};
use crate::schema::hybrid::{HybridEnumSpec, HybridFileSpec, HybridStructSpec, HybridType};
use crate::writer::CodeWriter;

use super::emit_doc_comments;
use super::type_map::{
    CodecSupport, default_expr, needs_manual_default, needs_raw, raw_c_type, raw_decode_expr,
    raw_encode_expr, rust_owned_type,
};

/// One field of an emitted Rust struct.
pub(super) struct RecordField<'a> {
    pub(super) ident: String,
    pub(super) ty: String,
    pub(super) hybrid: &'a HybridType,
    pub(super) doc: Option<&'a str>,
}

/// Emits `pub struct name { fields }` with `derives`, plus `Default`: derived when every field
/// supports it, else written out (std has no `Default` for `[T; N > 32]`).
pub(super) fn emit_record(
    out: &mut CodeWriter,
    attrs: &[&str],
    derives: &str,
    name: &str,
    fields: &[RecordField<'_>],
) {
    let manual_default = fields.iter().any(|f| needs_manual_default(f.hybrid));
    for attr in attrs {
        writeln!(out, "{attr}");
    }
    if manual_default {
        writeln!(out, "#[derive({derives})]");
    } else {
        writeln!(out, "#[derive({derives}, Default)]");
    }
    writeln!(out, "pub struct {name} {{");
    for f in fields {
        emit_doc_comments(out, "    ", f.doc);
        writeln!(out, "    pub {}: {},", f.ident, f.ty);
    }
    writeln!(out, "}}");
    writeln!(out);
    if !manual_default {
        return;
    }
    writeln!(out, "impl Default for {name} {{");
    writeln!(out, "    fn default() -> Self {{");
    writeln!(out, "        Self {{");
    for f in fields {
        writeln!(out, "            {}: {},", f.ident, default_expr(f.hybrid));
    }
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_rust_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    let name = &e.name;
    emit_doc_comments(out, "", e.doc.as_deref());
    writeln!(out, "#[repr(u32)]");
    writeln!(
        out,
        "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]"
    );
    writeln!(out, "pub enum {name} {{");
    for (idx, v) in e.variants.iter().enumerate() {
        emit_doc_comments(out, "    ", v.doc.as_deref());
        if idx == 0 {
            writeln!(out, "    #[default]");
        }
        writeln!(out, "    {} = {},", v.name, v.value);
    }
    writeln!(out, "}}");
    writeln!(out);
    writeln!(out, "impl {name} {{");
    writeln!(out, "    #[inline]");
    writeln!(out, "    pub const fn to_u32(self) -> u32 {{");
    writeln!(out, "        self as u32");
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    #[inline]");
    writeln!(
        out,
        "    pub const fn from_u32(raw: u32) -> Option<Self> {{"
    );
    writeln!(out, "        match raw {{");
    for v in &e.variants {
        writeln!(out, "            {} => Some(Self::{}),", v.value, v.name);
    }
    writeln!(out, "            _ => None,");
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
    writeln!(out, "nve_abi::hybrid_enum_codec!({name});");
    writeln!(out);
}

pub(super) fn emit_rust_struct(
    out: &mut CodeWriter,
    s: &HybridStructSpec,
    spec: &HybridFileSpec,
    codec: &CodecSupport<'_>,
) {
    let name = &s.name;
    // C-only records (unions, raw pointers, handles) never cross a Rust vtable.
    if !codec.encodes_struct(name) {
        return;
    }
    emit_doc_comments(out, "", s.doc.as_deref());
    let fields: Vec<_> = s
        .fields
        .iter()
        .map(|f| RecordField {
            ident: rust_ident(&f.name),
            ty: rust_owned_type(&f.ty),
            hybrid: &f.ty,
            doc: f.doc.as_deref(),
        })
        .collect();
    let is_pod = s.is_pod(spec);
    if is_pod {
        emit_record(
            out,
            &["#[repr(C)]"],
            "Debug, Clone, Copy, PartialEq",
            name,
            &fields,
        );
    } else {
        emit_record(out, &[], "Debug, Clone, PartialEq", name, &fields);
    }
    let struct_ty = HybridType::Struct(name.clone());
    if is_pod && needs_raw(&struct_ty, spec) {
        emit_raw_carrier(out, s, spec);
    }

    // `NveBinValue` map keyed by field name; the runtime derives the full codec from these two.
    writeln!(out, "impl nve_abi::hybrid::HybridStruct for {name} {{");
    writeln!(
        out,
        "    fn encode_fields(&self, m: &mut nve_abi::BinMapWriter<'_>) {{"
    );
    for f in &s.fields {
        writeln!(
            out,
            "        nve_abi::hybrid::HybridCodec::encode_bin(&self.{}, m.key(\"{}\"));",
            rust_ident(&f.name),
            escape_c_like_string(&f.name)
        );
    }
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(
        out,
        "    fn decode_fields(fields: &impl nve_abi::hybrid::HybridFields) -> Result<Self, nve_abi::DecodeError> {{"
    );
    writeln!(out, "        Ok(Self {{");
    for f in &s.fields {
        writeln!(
            out,
            "            {}: fields.field(\"{}\")?,",
            rust_ident(&f.name),
            escape_c_like_string(&f.name)
        );
    }
    writeln!(out, "        }})");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
    writeln!(out, "nve_abi::hybrid_struct_codec!({name});");
    writeln!(out);
}

/// `NveHybridRaw<Name>`: the C layout of POD struct `Name` with `bool` as `u8` and enums as `u32`,
/// so any bits a C caller passes are valid on entry; `decode` checks them.
fn emit_raw_carrier(out: &mut CodeWriter, s: &HybridStructSpec, spec: &HybridFileSpec) {
    let name = &s.name;
    let raw = format!("NveHybridRaw{name}");
    let fields: Vec<_> = s
        .fields
        .iter()
        .map(|f| RecordField {
            ident: rust_ident(&f.name),
            ty: raw_c_type(&f.ty, spec),
            hybrid: &f.ty,
            doc: None,
        })
        .collect();
    writeln!(
        out,
        "/// C-ABI carrier of [`{name}`]; [`{raw}::decode`] validates it."
    );
    emit_record(out, &["#[repr(C)]"], "Debug, Clone, Copy", &raw, &fields);

    writeln!(out, "impl {raw} {{");
    writeln!(
        out,
        "    pub fn decode(self) -> Result<{name}, nve_abi::DecodeError> {{"
    );
    writeln!(out, "        Ok({name} {{");
    for f in &s.fields {
        let field = rust_ident(&f.name);
        let value = format!("self.{field}");
        match raw_decode_expr(&value, &f.ty, spec) {
            Some(expr) => writeln!(out, "            {field}: {expr}?,"),
            None => writeln!(out, "            {field}: {value},"),
        }
    }
    writeln!(out, "        }})");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "impl From<{name}> for {raw} {{");
    writeln!(out, "    fn from(value: {name}) -> Self {{");
    writeln!(out, "        Self {{");
    for f in &s.fields {
        let field = rust_ident(&f.name);
        let expr = raw_encode_expr(&format!("value.{field}"), &f.ty, spec);
        writeln!(out, "            {field}: {expr},");
    }
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

/// `NveHybridOpt<T>`: `is_some` crosses as a `u8` and `value` as its raw carrier.
pub(super) fn emit_pod_option_structs(out: &mut CodeWriter, spec: &HybridFileSpec) {
    for (suffix, inner) in spec.collect_pod_option_types() {
        let fields = [
            RecordField {
                ident: "is_some".to_owned(),
                ty: "u8".to_owned(),
                hybrid: &HybridType::U8,
                doc: None,
            },
            RecordField {
                ident: "value".to_owned(),
                ty: raw_c_type(&inner, spec),
                hybrid: &inner,
                doc: None,
            },
        ];
        let name = format!("NveHybridOpt{suffix}");
        emit_record(out, &["#[repr(C)]"], "Debug, Clone, Copy", &name, &fields);
    }
}
