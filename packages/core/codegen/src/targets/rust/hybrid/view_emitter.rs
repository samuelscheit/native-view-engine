//! Modules and views: their `<Name>Hybrid` traits, view props, and view events.

use crate::naming::{
    escape_c_like_string, rust_ident, sanitize_single_line_comment, to_pascal_case, to_snake_case,
};
use crate::schema::hybrid::{
    HybridEventSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridType, HybridViewSpec,
};
use crate::writer::CodeWriter;

use super::emit_doc_comments;
use super::type_map::{
    DEFAULT, RESULT, rust_owned_type, rust_param_type, rust_prop_default_expr,
};
use super::types_emitter::{RecordField, emit_record, emit_struct_codec};
use super::vtable_emitter::{VtableOwner, emit_rust_vtable};

pub(super) fn emit_rust_module(out: &mut CodeWriter, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let owner = VtableOwner::new(&m.name, false);
    emit_section_header(out, "Module", &owner, m.interface_id(), m.schema_hash(spec));
    emit_doc_comments(out, "", m.doc.as_deref());
    emit_trait(out, &owner, &m.methods);
    emit_rust_vtable(out, &owner, &m.methods, spec);
}

pub(super) fn emit_rust_view(out: &mut CodeWriter, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let owner = VtableOwner::new(&v.name, true);
    emit_section_header(out, "View", &owner, v.interface_id(), v.schema_hash(spec));
    emit_props(out, &owner.pascal, v);
    for ev in &v.events {
        emit_event(out, &owner, ev);
    }
    emit_doc_comments(out, "", v.doc.as_deref());
    emit_trait(out, &owner, &v.methods);
    emit_rust_vtable(out, &owner, &v.methods, spec);
}

fn emit_section_header(
    out: &mut CodeWriter,
    kind: &str,
    owner: &VtableOwner,
    iface_id: nve_abi::InterfaceId,
    schema_hash: u64,
) {
    let (pascal, upper) = (&owner.pascal, &owner.upper);
    writeln!(
        out,
        "// ==========================================================================="
    );
    writeln!(out, "// Hybrid {kind}: {pascal}");
    writeln!(
        out,
        "// ==========================================================================="
    );
    writeln!(out);
    writeln!(
        out,
        "pub const NVE_HYBRID_{upper}_INTERFACE_ID: u32 = 0x{iface_id:08X};"
    );
    writeln!(
        out,
        "pub const NVE_HYBRID_{upper}_SCHEMA_HASH: u64 = 0x{schema_hash:016X};"
    );
    writeln!(out);
}

fn emit_trait(out: &mut CodeWriter, owner: &VtableOwner, methods: &[HybridMethodSpec]) {
    let pascal = &owner.pascal;
    writeln!(out, "pub trait {pascal}Hybrid {{");
    if owner.is_view {
        writeln!(out, "    fn apply_props(&mut self, props: {pascal}Props);");
    }
    for method in methods {
        emit_doc_comments(out, "    ", method.doc.as_deref());
        write!(out, "    fn {}(&mut self", rust_ident(&method.name));
        for p in &method.params {
            write!(out, ", {}: {}", rust_ident(&p.name), rust_param_type(&p.ty));
        }
        match &method.return_type {
            HybridType::Void => writeln!(out, ");"),
            ret => writeln!(out, ") -> {};", rust_owned_type(ret)),
        }
    }
    writeln!(out, "}}");
    writeln!(out);
}

fn emit_props(out: &mut CodeWriter, pascal: &str, v: &HybridViewSpec) {
    writeln!(out, "#[derive(Debug, Clone, PartialEq)]");
    writeln!(out, "pub struct {pascal}Props {{");
    for p in &v.props {
        emit_doc_comments(out, "    ", p.doc.as_deref());
        writeln!(
            out,
            "    pub {}: {},",
            rust_ident(&p.name),
            rust_owned_type(&p.ty)
        );
    }
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "impl {DEFAULT} for {pascal}Props {{");
    writeln!(out, "    fn default() -> Self {{");
    writeln!(out, "        Self {{");
    for p in &v.props {
        let def = rust_prop_default_expr(&p.ty, p.default_value.as_ref());
        writeln!(out, "            {}: {def},", rust_ident(&p.name));
    }
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);

    writeln!(out, "impl {pascal}Props {{");
    writeln!(
        out,
        "    /// Decodes direct props by key: unknown keys are ignored, absent props keep their defaults."
    );
    writeln!(
        out,
        "    /// A key that is not valid UTF-8 is an error, never silently skipped."
    );
    writeln!(
        out,
        "    pub fn decode_props(map: nve_abi::NveMapRef<'_>) -> {RESULT}<Self, nve_abi::DecodeError> {{"
    );
    let binding = if v.props.is_empty() {
        "props"
    } else {
        "mut props"
    };
    writeln!(out, "        let {binding} = Self::default();");
    writeln!(out, "        for entry in map.try_iter() {{");
    if v.props.is_empty() {
        writeln!(out, "            entry?;");
    } else {
        writeln!(out, "            let (key, value) = entry?;");
        writeln!(out, "            match key {{");
        for p in &v.props {
            writeln!(
                out,
                "                \"{}\" => props.{} = nve_abi::hybrid::HybridCodec::decode_value_ref(value)?,",
                escape_c_like_string(&p.name),
                rust_ident(&p.name)
            );
        }
        writeln!(out, "                _ => {{}}");
        writeln!(out, "            }}");
    }
    writeln!(out, "        }}");
    writeln!(out, "        Ok(props)");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

/// Typed payload struct (an `NveBinValue` map codec), its `EventBatchBuf` emitter, and a free
/// `emit_<view>_<event>`. The event crosses as one `nve.custom` `NveEvent` named `ev.name` whose
/// byte-arena payload is the encoded map.
fn emit_event(out: &mut CodeWriter, owner: &VtableOwner, ev: &HybridEventSpec) {
    let pascal = &owner.pascal;
    let ev_struct = format!("{pascal}{}Event", to_pascal_case(&ev.name));
    emit_doc_comments(out, "", ev.doc.as_deref());
    let fields: Vec<_> = ev
        .params
        .iter()
        .map(|p| RecordField {
            ident: rust_ident(&p.name),
            ty: rust_owned_type(&p.ty),
            hybrid: &p.ty,
            doc: None,
        })
        .collect();
    emit_record(out, &[], "Debug, Clone, PartialEq", &ev_struct, &fields);
    let keys: Vec<&str> = ev.params.iter().map(|p| p.name.as_str()).collect();
    emit_struct_codec(out, &ev_struct, &keys);

    writeln!(out, "impl {ev_struct} {{");
    writeln!(
        out,
        "    /// Appends this event to `out` as one `nve.custom` event carrying its `NveBinValue` map."
    );
    writeln!(
        out,
        "    pub fn push_to(&self, surface: nve_abi::SurfaceId, node: nve_abi::NodeId, timestamp: nve_abi::TimestampUs, out: &mut nve_abi::EventBatchBuf) -> {RESULT}<(), nve_abi::DecodeError> {{"
    );
    writeln!(
        out,
        "        let data = nve_abi::hybrid::encode_to_vec(self)?;"
    );
    writeln!(
        out,
        "        out.try_push_custom(surface, node, timestamp, \"{}\", &data).map_err(nve_abi::DecodeError::InvalidEvent)",
        escape_c_like_string(&ev.name)
    );
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);

    writeln!(
        out,
        "/// Emits `{pascal}::{}` without building a `{ev_struct}` first.",
        sanitize_single_line_comment(&ev.name)
    );
    write!(
        out,
        "pub fn emit_{}_{}(_nve_surface: nve_abi::SurfaceId, _nve_node: nve_abi::NodeId, _nve_timestamp: nve_abi::TimestampUs",
        owner.snake,
        to_snake_case(&ev.name)
    );
    for p in &ev.params {
        write!(out, ", {}: {}", rust_ident(&p.name), rust_owned_type(&p.ty));
    }
    writeln!(
        out,
        ", _nve_out: &mut nve_abi::EventBatchBuf) -> {RESULT}<(), nve_abi::DecodeError> {{"
    );
    write!(out, "    {ev_struct} {{");
    for p in &ev.params {
        write!(out, " {},", rust_ident(&p.name));
    }
    writeln!(
        out,
        " }}.push_to(_nve_surface, _nve_node, _nve_timestamp, _nve_out)"
    );
    writeln!(out, "}}");
    writeln!(out);
}
