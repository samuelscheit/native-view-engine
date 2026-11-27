use crate::writer::CodeWriter;

use super::*;
use super::type_helpers::*;

pub(super) fn emit_kotlin_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "enum class {}(val rawValue: Int) {{", e.name);
    for (idx, v) in e.variants.iter().enumerate() {
        let sep = if idx + 1 == e.variants.len() { ";" } else { "," };
        writeln!(out, "    {}({}){sep}", v.name, kotlin_int_literal(v.value));
    }
    writeln!(out, "    companion object {{");
    writeln!(out, "        fun fromRaw(raw: Int): {} = when (raw) {{", e.name);
    for v in &e.variants {
        writeln!(out, "            {} -> {}", kotlin_int_literal(v.value), v.name);
    }
    writeln!(
        out,
        "            else -> throw IllegalArgumentException(\"invalid {} discriminant: $raw\")",
        e.name
    );
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_kotlin_struct(out: &mut CodeWriter, s: &HybridStructSpec, spec: &HybridFileSpec) {
    writeln!(out, "data class {} (", s.name);
    for (idx, f) in s.fields.iter().enumerate() {
        let fname = kotlin_ident(&f.name);
        let comma = if idx + 1 == s.fields.len() { "" } else { "," };
        let def = kotlin_default_value(&f.ty, None, spec);
        writeln!(out, "    val {fname}: {} = {def}{comma}", kotlin_type(&f.ty));
    }
    writeln!(out, ") {{");
    // Fields are read through `this.`, so a field named like the writer (`w`) never resolves to it.
    writeln!(out, "    fun encodeBin(w: BinWriter) {{");
    writeln!(out, "        val _m = w.beginMap({})", s.fields.len());
    for f in &s.fields {
        let fname = kotlin_ident(&f.name);
        let key_lit = escape_kotlin_string(&f.name);
        writeln!(out, "        w.writeMapKey(\"{key_lit}\")");
        emit_kotlin_bin_write(out, "        ", "w", &format!("this.{fname}"), &f.ty, 0);
    }
    writeln!(out, "        w.end(_m)");
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    companion object {{");
    writeln!(out, "        fun decodeBin(r: BinReader): {} {{", s.name);
    // `_f_<field>` never collides with the codec's `_a<d>` / `_el<d>` / `_ov<d>` locals; it uses
    // the unescaped name (`_f_object`, never `` _f_`object` ``).
    for f in &s.fields {
        let def = kotlin_default_value(&f.ty, None, spec);
        writeln!(out, "            var {}: {} = {def}", field_local(&f.name), kotlin_type(&f.ty));
    }
    writeln!(out, "            val _m = r.beginMap()");
    writeln!(out, "            repeat(_m.count) {{");
    writeln!(out, "                when (r.readMapKey()) {{");
    for f in &s.fields {
        let key_lit = escape_kotlin_string(&f.name);
        writeln!(out, "                    \"{key_lit}\" -> {{");
        emit_kotlin_bin_read(out, "                        ", "r", &field_local(&f.name), &f.ty, 0);
        writeln!(out, "                    }}");
    }
    writeln!(out, "                    else -> r.skipValue()");
    writeln!(out, "                }}");
    writeln!(out, "            }}");
    writeln!(out, "            r.end(_m)");
    write!(out, "            return {}(", s.name);
    for (idx, f) in s.fields.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ");
        }
        write!(out, "{} = {}", kotlin_ident(&f.name), field_local(&f.name));
    }
    writeln!(out, ")");
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_kotlin_module(out: &mut CodeWriter, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    writeln!(out, "interface Hybrid{pascal}Spec {{");
    for method in &m.methods {
        writeln!(out, "    {}", kotlin_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_kotlin_client(out, &pascal, m.interface_id(), m.schema_hash(spec), &m.methods, None);
}

pub(super) fn emit_kotlin_view(out: &mut CodeWriter, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);

    writeln!(out, "data class {pascal}Props(");
    for (idx, p) in v.props.iter().enumerate() {
        let pname = kotlin_ident(&p.name);
        let comma = if idx + 1 == v.props.len() { "" } else { "," };
        let def = kotlin_default_value(&p.ty, p.default_value.as_ref(), spec);
        writeln!(out, "    val {pname}: {} = {def}{comma}", kotlin_type(&p.ty));
    }
    writeln!(out, ")");
    writeln!(out);

    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "data class {pascal}{ev_pascal}Event(");
        for (idx, param) in ev.params.iter().enumerate() {
            let pname = kotlin_ident(&param.name);
            let comma = if idx + 1 == ev.params.len() { "" } else { "," };
            writeln!(out, "    val {pname}: {}{comma}", kotlin_type(&param.ty));
        }
        writeln!(out, ")");
        writeln!(out);
    }

    writeln!(out, "interface Hybrid{pascal}Spec {{");
    writeln!(out, "    fun setProps(props: {pascal}Props)");
    for method in &v.methods {
        writeln!(out, "    {}", kotlin_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_kotlin_client(out, &pascal, v.interface_id(), v.schema_hash(spec), &v.methods, Some(v));
}

pub(super) fn emit_kotlin_client(
    out: &mut CodeWriter,
    pascal: &str,
    interface_id: u32,
    schema_hash: u64,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
) {
    writeln!(
        out,
        "class {pascal}HybridClient private constructor(private var handle: Long) : Hybrid{pascal}Spec, AutoCloseable {{"
    );
    writeln!(out, "    companion object {{");
    writeln!(
        out,
        "        const val INTERFACE_ID: Int = {} // 0x{interface_id:08X}",
        kotlin_int_literal(interface_id)
    );
    writeln!(
        out,
        "        const val SCHEMA_HASH: Long = {} // 0x{schema_hash:016X}",
        kotlin_long_literal(schema_hash)
    );
    writeln!(out);
    writeln!(out, "        fun create(): {pascal}HybridClient = {pascal}HybridClient(nativeCreate())");
    writeln!(out, "        @JvmStatic private external fun nativeCreate(): Long");
    // `_nveHandle`: a user parameter named `handle` must not redeclare it.
    writeln!(out, "        @JvmStatic private external fun nativeDestroy(_nveHandle: Long)");
    if view.is_some() {
        writeln!(
            out,
            "        @JvmStatic private external fun nativeSetProps(_nveHandle: Long, _nveProps: ByteBuffer)"
        );
    }
    for m in methods {
        let jni_name = format!("native{}", to_pascal_case(&m.name));
        let mut params = vec!["_nveHandle: Long".to_string()];
        for p in &m.params {
            let pname = kotlin_ident(&p.name);
            params.push(format!("{pname}: {}", kotlin_jni_param_type(&p.ty)));
        }
        let ret = kotlin_jni_return_type(&m.return_type);
        let ret_suffix = if ret == "Unit" {
            String::new()
        } else {
            format!(": {ret}")
        };
        writeln!(
            out,
            "        @JvmStatic private external fun {jni_name}({}){ret_suffix}",
            params.join(", ")
        );
    }
    writeln!(out, "    }}");
    writeln!(out);

    // Clear the handle before destroying it, so no path can use or free it twice.
    writeln!(out, "    override fun close() {{");
    writeln!(out, "        val h = this.handle");
    writeln!(out, "        this.handle = 0L");
    writeln!(out, "        if (h != 0L) nativeDestroy(h)");
    writeln!(out, "    }}");
    writeln!(out);

    if let Some(v) = view {
        writeln!(out, "    override fun setProps(props: {pascal}Props) {{");
        writeln!(out, "        val _w = BinWriter()");
        writeln!(out, "        val _m = _w.beginMap({})", v.props.len());
        for p in &v.props {
            let pname = kotlin_ident(&p.name);
            let key_lit = escape_kotlin_string(&p.name);
            writeln!(out, "        _w.writeMapKey(\"{key_lit}\")");
            emit_kotlin_bin_write(out, "        ", "_w", &format!("props.{pname}"), &p.ty, 0);
        }
        writeln!(out, "        _w.end(_m)");
        writeln!(out, "        nativeSetProps(this.handle, _w.toDirectBuffer())");
        writeln!(out, "    }}");
        writeln!(out);
    }

    for m in methods {
        let jni_name = format!("native{}", to_pascal_case(&m.name));
        writeln!(out, "    override {} {{", kotlin_method_sig(m));
        // `this.handle`: a parameter named `handle` never shadows the native handle.
        let mut args = vec!["this.handle".to_string()];
        for (idx, p) in m.params.iter().enumerate() {
            let pname = kotlin_ident(&p.name);
            match &p.ty {
                HybridType::Bool
                | HybridType::U8
                | HybridType::U16
                | HybridType::Usize
                | HybridType::I32
                | HybridType::U32
                | HybridType::I64
                | HybridType::U64
                | HybridType::F32
                | HybridType::F64
                | HybridType::String => args.push(pname),
                HybridType::Enum(_) => args.push(format!("{pname}.rawValue")),
                HybridType::Bytes => args.push(format!("HybridRuntime.directBuffer({pname})")),
                HybridType::Value => args.push(format!("{pname}.toDirectBuffer()")),
                other => {
                    writeln!(out, "        val _w{idx} = BinWriter()");
                    emit_kotlin_bin_write(out, "        ", &format!("_w{idx}"), &pname, other, 0);
                    args.push(format!("_w{idx}.toDirectBuffer()"));
                }
            }
        }
        let call = format!("{jni_name}({})", args.join(", "));
        let effective_ret = match &m.return_type {
            HybridType::Result { ok, .. } => ok.as_ref(),
            other => other,
        };
        match effective_ret {
            HybridType::Void => writeln!(out, "        {call}"),
            HybridType::Bool
            | HybridType::U8
            | HybridType::U16
            | HybridType::Usize
            | HybridType::I32
            | HybridType::U32
            | HybridType::I64
            | HybridType::U64
            | HybridType::F32
            | HybridType::F64
            | HybridType::String
            | HybridType::Bytes => writeln!(out, "        return {call}"),
            HybridType::Enum(e) => writeln!(out, "        return {e}.fromRaw({call})"),
            // Exactly one encoded value (a `DynamicValue` too): trailing bytes are an error.
            other => {
                writeln!(out, "        val _r = BinReader({call})");
                writeln!(out, "        val _out: {}", kotlin_type(other));
                emit_kotlin_bin_read(out, "        ", "_r", "_out", other, 0);
                writeln!(out, "        _r.expectEnd()");
                writeln!(out, "        return _out");
            }
        }
        writeln!(out, "    }}");
        writeln!(out);
    }

    writeln!(out, "}}");
    writeln!(out);
}

