//! Enums, structs, and POD `NveHybridOpt*` wrappers.

use crate::naming::{escape_c_like_string, rust_ident};
use crate::schema::hybrid::{HybridEnumSpec, HybridFileSpec, HybridStructSpec};
use crate::writer::CodeWriter;

use super::emit_doc_comments;
use super::type_map::{CodecSupport, pod_c_type, rust_owned_type};

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
    emit_doc_comments(out, "", s.doc.as_deref());
    if s.is_pod(spec) {
        writeln!(out, "#[repr(C)]");
        writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Default)]");
    } else {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Default)]");
    }
    writeln!(out, "pub struct {name} {{");
    for f in &s.fields {
        emit_doc_comments(out, "    ", f.doc.as_deref());
        writeln!(
            out,
            "    pub {}: {},",
            rust_ident(&f.name),
            rust_owned_type(&f.ty)
        );
    }
    writeln!(out, "}}");
    writeln!(out);
    if !codec.encodes_struct(name) {
        return;
    }

    // `NveBinValue` map keyed by field name; the runtime derives the full codec from these two.
    writeln!(out, "impl HybridStruct for {name} {{");
    writeln!(
        out,
        "    fn encode_fields(&self, m: &mut BinMapWriter<'_>) {{"
    );
    for f in &s.fields {
        writeln!(
            out,
            "        HybridCodec::encode_bin(&self.{}, m.key(\"{}\"));",
            rust_ident(&f.name),
            escape_c_like_string(&f.name)
        );
    }
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(
        out,
        "    fn decode_fields(fields: &impl HybridFields) -> Result<Self, DecodeError> {{"
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

pub(super) fn emit_pod_option_structs(out: &mut CodeWriter, spec: &HybridFileSpec) {
    for (suffix, inner) in spec.collect_pod_option_types() {
        writeln!(out, "#[repr(C)]");
        writeln!(out, "#[derive(Debug, Clone, Copy, Default)]");
        writeln!(out, "pub struct NveHybridOpt{suffix} {{");
        writeln!(out, "    pub is_some: bool,");
        writeln!(out, "    pub value: {},", pod_c_type(&inner));
        writeln!(out, "}}");
        writeln!(out);
    }
}
