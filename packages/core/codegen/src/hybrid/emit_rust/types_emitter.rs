use std::fmt::Write as _;

use super::*;
use super::codec_helpers::*;
use super::vtable_emitter::*;

pub(super) fn emit_rust_enum(out: &mut String, e: &HybridEnumSpec) {
    emit_doc_comments(out, "", e.doc.as_deref());
    writeln!(out, "#[repr(u32)]").unwrap();
    writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]").unwrap();
    writeln!(out, "pub enum {} {{", e.name).unwrap();
    for v in &e.variants {
        emit_doc_comments(out, "    ", v.doc.as_deref());
        writeln!(out, "    {} = {},", v.name, v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "impl Default for {} {{", e.name).unwrap();
    writeln!(out, "    #[inline]").unwrap();
    writeln!(out, "    fn default() -> Self {{").unwrap();
    writeln!(out, "        Self::{}", e.variants[0].name).unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "impl {} {{", e.name).unwrap();
    writeln!(out, "    #[inline]").unwrap();
    writeln!(out, "    pub const fn to_u32(self) -> u32 {{ self as u32 }}").unwrap();
    writeln!(out, "    #[inline]").unwrap();
    writeln!(out, "    pub const fn from_u32(raw: u32) -> Option<Self> {{").unwrap();
    writeln!(out, "        match raw {{").unwrap();
    for v in &e.variants {
        writeln!(out, "            {} => Some(Self::{}),", v.value, v.name).unwrap();
    }
    writeln!(out, "            _ => None,").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_rust_struct(out: &mut String, s: &HybridStructSpec, spec: &HybridFileSpec) {
    let is_pod = s.is_pod(spec);
    emit_doc_comments(out, "", s.doc.as_deref());
    if is_pod {
        writeln!(out, "#[repr(C)]").unwrap();
        writeln!(out, "#[derive(Debug, Clone, Copy, PartialEq, Default)]").unwrap();
    } else {
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Default)]").unwrap();
    }
    writeln!(out, "pub struct {} {{", s.name).unwrap();
    for f in &s.fields {
        emit_doc_comments(out, "    ", f.doc.as_deref());
        let fname = rust_ident(&f.name);
        let fty = rust_owned_type(&f.ty);
        writeln!(out, "    pub {fname}: {fty},").unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Emit compact binary `NveBinValue` encode/decode and `NveValueRef` decode helpers on every struct
    // so both non-POD structs and POD structs nested inside Vec/Option/Props/Events work zero-JSON.
    writeln!(out, "impl {} {{", s.name).unwrap();
    writeln!(out, "    pub fn encode_bin(&self, out: &mut Vec<u8>) {{").unwrap();
    writeln!(
        out,
        "        nve_bin_codec::write_map(out, {}, |out| {{",
        s.fields.len()
    )
    .unwrap();
    for f in &s.fields {
        let fname = rust_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            nve_bin_codec::write_map_key(out, \"{key_lit}\");").unwrap();
        emit_bin_write_expr(out, "            ", "out", &format!("&self.{fname}"), &f.ty);
    }
    writeln!(out, "        }});").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "    pub fn decode_bin(view: BinValueView<'_>) -> Result<Self, DecodeError> {{").unwrap();
    writeln!(out, "        if view.tag() != NveValueTag::Map {{").unwrap();
    writeln!(out, "            return Err(DecodeError::InvalidEnum(\"NveValueTag\", view.tag() as u32));").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        Ok(Self {{").unwrap();
    for f in &s.fields {
        let fname = rust_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        let decode_expr = bin_view_decode_expr("fv", &f.ty);
        if matches!(f.ty, HybridType::Optional(_)) {
            writeln!(
                out,
                "            {fname}: match view.get(\"{key_lit}\") {{ Some(fv) => {decode_expr}, None => None }},"
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "            {fname}: {{ let fv = view.get(\"{key_lit}\").ok_or(DecodeError::Truncated)?; {decode_expr} }},"
            )
            .unwrap();
        }
    }
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "    pub fn decode_value_ref(value: NveValueRef<'_>) -> Result<Self, DecodeError> {{").unwrap();
    writeln!(out, "        let map = value.as_map().ok_or(DecodeError::Truncated)?;").unwrap();
    writeln!(out, "        Ok(Self {{").unwrap();
    for f in &s.fields {
        let fname = rust_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        let decode_expr = value_ref_decode_expr("fv", &f.ty);
        if matches!(f.ty, HybridType::Optional(_)) {
            writeln!(
                out,
                "            {fname}: match map.get(\"{key_lit}\") {{ Some(fv) => {decode_expr}, None => None }},"
            )
            .unwrap();
        } else {
            writeln!(
                out,
                "            {fname}: {{ let fv = map.get(\"{key_lit}\").ok_or(DecodeError::Truncated)?; {decode_expr} }},"
            )
            .unwrap();
        }
    }
    writeln!(out, "        }})").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_pod_option_structs(out: &mut String, spec: &HybridFileSpec) {
    let opts = spec.collect_pod_option_types();
    for inner in &opts {
        let suffix = inner.pod_opt_suffix().unwrap();
        let ffi_ty = rust_c_abi_pod_type(inner);
        writeln!(out, "#[repr(C)]").unwrap();
        writeln!(out, "#[derive(Debug, Clone, Copy)]").unwrap();
        writeln!(out, "pub struct NveHybridOpt{suffix} {{").unwrap();
        writeln!(out, "    pub is_some: bool,").unwrap();
        writeln!(out, "    pub value: {ffi_ty},").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }
}

pub(super) fn emit_rust_module(out: &mut String, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let upper = to_snake_case(&m.name).to_ascii_uppercase();
    let snake = to_snake_case(&m.name);
    let pascal = to_pascal_case(&m.name);
    let iface_id = m.interface_id();
    let schema_hash = m.schema_hash(spec);

    writeln!(out, "// ===========================================================================").unwrap();
    writeln!(out, "// Hybrid Module: {pascal}").unwrap();
    writeln!(out, "// ===========================================================================").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub const NVE_HYBRID_{upper}_INTERFACE_ID: u32 = 0x{iface_id:08X};").unwrap();
    writeln!(out, "pub const NVE_HYBRID_{upper}_SCHEMA_HASH: u64 = 0x{schema_hash:016X};").unwrap();
    writeln!(out).unwrap();

    emit_doc_comments(out, "", m.doc.as_deref());
    writeln!(out, "pub trait {pascal}Hybrid {{").unwrap();
    for method in &m.methods {
        emit_doc_comments(out, "    ", method.doc.as_deref());
        emit_rust_trait_method(out, method);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_rust_vtable_struct(out, &pascal, &m.methods, false, spec);
    emit_rust_vtable_builder(out, &pascal, &snake, &upper, &m.methods, false, spec);
}

pub(super) fn emit_rust_view(out: &mut String, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let upper = to_snake_case(&v.name).to_ascii_uppercase();
    let snake = to_snake_case(&v.name);
    let pascal = to_pascal_case(&v.name);
    let iface_id = v.interface_id();
    let schema_hash = v.schema_hash(spec);

    writeln!(out, "// ===========================================================================").unwrap();
    writeln!(out, "// Hybrid View: {pascal}").unwrap();
    writeln!(out, "// ===========================================================================").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "pub const NVE_HYBRID_{upper}_INTERFACE_ID: u32 = 0x{iface_id:08X};").unwrap();
    writeln!(out, "pub const NVE_HYBRID_{upper}_SCHEMA_HASH: u64 = 0x{schema_hash:016X};").unwrap();
    writeln!(out).unwrap();

    // Props struct + Default + zero-copy `decode_props_direct(&[NveMapEntry])`
    writeln!(out, "#[derive(Debug, Clone, PartialEq)]").unwrap();
    writeln!(out, "pub struct {pascal}Props {{").unwrap();
    for p in &v.props {
        emit_doc_comments(out, "    ", p.doc.as_deref());
        let pname = rust_ident(&p.name);
        writeln!(out, "    pub {pname}: {},", rust_owned_type(&p.ty)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "impl Default for {pascal}Props {{").unwrap();
    writeln!(out, "    fn default() -> Self {{").unwrap();
    writeln!(out, "        Self {{").unwrap();
    for p in &v.props {
        let pname = rust_ident(&p.name);
        let def_expr = rust_prop_default_expr(&p.ty, p.default_value.as_ref());
        writeln!(out, "            {pname}: {def_expr},").unwrap();
    }
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "impl {pascal}Props {{").unwrap();
    writeln!(
        out,
        "    /// Zero-copy prop decoder reading directly from FFI `NveMapEntry` slices (no JSON)."
    )
    .unwrap();
    writeln!(
        out,
        "    /// # Safety"
    )
    .unwrap();
    writeln!(
        out,
        "    /// All nested pointers inside `entries` must remain valid for the duration of this call."
    )
    .unwrap();
    writeln!(
        out,
        "    pub unsafe fn decode_props_direct(entries: &[NveMapEntry]) -> Result<Self, DecodeError> {{"
    )
    .unwrap();
    writeln!(out, "        let mut props = Self::default();").unwrap();
    writeln!(out, "        let map = unsafe {{ NveMapRef::from_slice(entries) }};").unwrap();
    writeln!(out, "        for (key, val) in map.iter() {{").unwrap();
    writeln!(out, "            match key {{").unwrap();
    for p in &v.props {
        let pname = rust_ident(&p.name);
        let key_lit = escape_c_like_string(&p.name);
        let decode_expr = value_ref_decode_expr("val", &p.ty);
        writeln!(out, "                \"{key_lit}\" => props.{pname} = {decode_expr},").unwrap();
    }
    writeln!(out, "                _ => {{}}").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        Ok(props)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Typed events + zero-copy `NveDirectEvent` emitters + compact binary `NveBinValue` encoders
    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        let ev_snake = to_snake_case(&ev.name);
        let ev_struct = format!("{pascal}{ev_pascal}Event");
        emit_doc_comments(out, "", ev.doc.as_deref());
        writeln!(out, "#[derive(Debug, Clone, PartialEq, Default)]").unwrap();
        writeln!(out, "pub struct {ev_struct} {{").unwrap();
        for param in &ev.params {
            let pname = rust_ident(&param.name);
            writeln!(out, "    pub {pname}: {},", rust_owned_type(&param.ty)).unwrap();
        }
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        writeln!(out, "impl {ev_struct} {{").unwrap();
        // Compact binary `NveBinValue` encoder
        writeln!(
            out,
            "    /// Encodes this typed event payload into compact binary `NveBinValue` (zero JSON)."
        )
        .unwrap();
        writeln!(out, "    pub fn encode_bin(&self) -> Vec<u8> {{").unwrap();
        writeln!(out, "        let mut out = Vec::with_capacity(64);").unwrap();
        writeln!(
            out,
            "        nve_bin_codec::write_map(&mut out, {}, |out| {{",
            ev.params.len()
        )
        .unwrap();
        for param in &ev.params {
            let pname = rust_ident(&param.name);
            let key_lit = escape_c_like_string(&param.name);
            writeln!(out, "            nve_bin_codec::write_map_key(out, \"{key_lit}\");").unwrap();
            emit_bin_write_expr(out, "            ", "out", &format!("&self.{pname}"), &param.ty);
        }
        writeln!(out, "        }});").unwrap();
        writeln!(out, "        out").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();

        // Zero-copy stack-allocated `NveDirectEvent` emitter
        writeln!(
            out,
            "    /// Emits a zero-copy `NveDirectEvent` backed by stack-allocated `NveMapEntry` values."
        )
        .unwrap();
        writeln!(
            out,
            "    pub fn emit_direct<F: FnOnce(&NveDirectEvent)>(&self, surface_id: u32, node_id: u32, timestamp_us: u64, sink: F) {{"
        )
        .unwrap();
        // Pre-encode any complex event fields into local `Vec<u8>` so stack `NveValue` references stay alive.
        for (idx, param) in ev.params.iter().enumerate() {
            if !is_direct_nve_value_scalar_or_slice(&param.ty) {
                let pname = rust_ident(&param.name);
                writeln!(out, "        let mut _bin_{idx} = Vec::with_capacity(32);").unwrap();
                emit_bin_write_expr(
                    out,
                    "        ",
                    &format!("&mut _bin_{idx}"),
                    &format!("&self.{pname}"),
                    &param.ty,
                );
            }
        }
        writeln!(out, "        let entries: [NveMapEntry; {}] = [", ev.params.len()).unwrap();
        for (idx, param) in ev.params.iter().enumerate() {
            let pname = rust_ident(&param.name);
            let key_lit = escape_c_like_string(&param.name);
            let val_expr = direct_nve_value_expr(&format!("self.{pname}"), &param.ty, idx);
            writeln!(out, "            NveMapEntry::new(\"{key_lit}\", {val_expr}),").unwrap();
        }
        writeln!(out, "        ];").unwrap();
        let ev_name_lit = escape_c_like_string(&ev.name);
        writeln!(
            out,
            "        let direct = NveDirectEvent::custom(surface_id, node_id, timestamp_us, \"{ev_name_lit}\", NveValue::map(&entries));"
        )
        .unwrap();
        writeln!(out, "        sink(&direct);").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();

        // Convenience top-level `emit_<event>` helper function
        writeln!(
            out,
            "/// Convenience zero-copy `NveDirectEvent` emitter for `{pascal}::{}`.",
            sanitize_single_line_comment(&ev.name)
        )
        .unwrap();
        write!(
            out,
            "pub fn emit_{snake}_{ev_snake}<F: FnOnce(&NveDirectEvent)>(surface_id: u32, node_id: u32, timestamp_us: u64"
        )
        .unwrap();
        for param in &ev.params {
            let pname = rust_ident(&param.name);
            write!(out, ", {pname}: {}", rust_owned_type(&param.ty)).unwrap();
        }
        writeln!(out, ", sink: F) {{").unwrap();
        write!(out, "    let payload = {ev_struct} {{").unwrap();
        for param in &ev.params {
            let pname = rust_ident(&param.name);
            write!(out, " {pname},").unwrap();
        }
        writeln!(out, " }};").unwrap();
        writeln!(
            out,
            "    payload.emit_direct(surface_id, node_id, timestamp_us, sink);"
        )
        .unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    emit_doc_comments(out, "", v.doc.as_deref());
    writeln!(out, "pub trait {pascal}Hybrid {{").unwrap();
    writeln!(out, "    fn apply_props(&mut self, props: {pascal}Props);").unwrap();
    for method in &v.methods {
        emit_doc_comments(out, "    ", method.doc.as_deref());
        emit_rust_trait_method(out, method);
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_rust_vtable_struct(out, &pascal, &v.methods, true, spec);
    emit_rust_vtable_builder(out, &pascal, &snake, &upper, &v.methods, true, spec);
}

