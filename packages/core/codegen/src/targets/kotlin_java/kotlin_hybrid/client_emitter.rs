use crate::writer::CodeWriter;

use super::*;
use super::type_helpers::*;

pub(super) fn emit_kotlin_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "enum class {}(val rawValue: Int) {{", e.name);
    for (idx, v) in e.variants.iter().enumerate() {
        let sep = if idx + 1 == e.variants.len() { ";" } else { "," };
        writeln!(out, "    {}({}){sep}", v.name, v.value);
    }
    writeln!(out, "    companion object {{");
    writeln!(out, "        fun fromRaw(raw: Int): {} = when (raw) {{", e.name);
    for v in &e.variants {
        writeln!(out, "            {} -> {}", v.value, v.name);
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
    writeln!(out, "    fun encodeBin(w: NveBinWriter) {{");
    writeln!(out, "        w.writeMap({}) {{ mw ->", s.fields.len());
    for f in &s.fields {
        let fname = kotlin_ident(&f.name);
        let key_lit = escape_kotlin_string(&f.name);
        writeln!(out, "            mw.writeMapKey(\"{key_lit}\")");
        emit_kotlin_bin_write(out, "            ", "mw", &fname, &f.ty);
    }
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    companion object {{");
    writeln!(out, "        fun decodeBin(r: NveBinReader): {} {{", s.name);
    for f in &s.fields {
        let fname = kotlin_ident(&f.name);
        let def = kotlin_default_value(&f.ty, None, spec);
        writeln!(out, "            var _{fname}: {} = {def}", kotlin_type(&f.ty));
    }
    writeln!(out, "            val count = r.beginMap()");
    writeln!(out, "            repeat(count) {{");
    writeln!(out, "                when (r.readMapKey()) {{");
    for f in &s.fields {
        let fname = kotlin_ident(&f.name);
        let key_lit = escape_kotlin_string(&f.name);
        writeln!(out, "                    \"{key_lit}\" -> {{");
        emit_kotlin_bin_read(out, "                        ", "r", &format!("_{fname}"), &f.ty);
        writeln!(out, "                    }}");
    }
    writeln!(out, "                    else -> r.skipValue()");
    writeln!(out, "                }}");
    writeln!(out, "            }}");
    write!(out, "            return {}(", s.name);
    for (idx, f) in s.fields.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ");
        }
        let fname = kotlin_ident(&f.name);
        write!(out, "{fname} = _{fname}");
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
    writeln!(out, "        const val INTERFACE_ID: Int = 0x{interface_id:08X}");
    writeln!(
        out,
        "        const val SCHEMA_HASH: Long = 0x{schema_hash:016X}UL.toLong()"
    );
    writeln!(out);
    writeln!(out, "        fun create(): {pascal}HybridClient = {pascal}HybridClient(nativeCreate())");
    writeln!(out, "        @JvmStatic private external fun nativeCreate(): Long");
    writeln!(out, "        @JvmStatic private external fun nativeDestroy(handle: Long)");
    if view.is_some() {
        writeln!(
            out,
            "        @JvmStatic private external fun nativeSetProps(handle: Long, propsDirectBuf: ByteBuffer)"
        );
    }
    for m in methods {
        let jni_name = format!("native{}", to_pascal_case(&m.name));
        let mut params = vec!["handle: Long".to_string()];
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

    writeln!(out, "    override fun close() {{");
    writeln!(out, "        if (handle != 0L) {{");
    writeln!(out, "            nativeDestroy(handle)");
    writeln!(out, "            handle = 0L");
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out);

    if let Some(v) = view {
        writeln!(out, "    override fun setProps(props: {pascal}Props) {{");
        writeln!(out, "        val w = NveBinWriter()");
        writeln!(out, "        w.writeMap({}) {{ mw ->", v.props.len());
        for p in &v.props {
            let pname = kotlin_ident(&p.name);
            let key_lit = escape_kotlin_string(&p.name);
            writeln!(out, "            mw.writeMapKey(\"{key_lit}\")");
            emit_kotlin_bin_write(out, "            ", "mw", &format!("props.{pname}"), &p.ty);
        }
        writeln!(out, "        }}");
        writeln!(out, "        nativeSetProps(handle, w.toDirectByteBuffer())");
        writeln!(out, "    }}");
        writeln!(out);
    }

    for m in methods {
        let jni_name = format!("native{}", to_pascal_case(&m.name));
        writeln!(out, "    override {} {{", kotlin_method_sig(m));
        let mut args = vec!["handle".to_string()];
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
                HybridType::Bytes => {
                    writeln!(out, "        val _buf{idx} = ByteBuffer.allocateDirect({pname}.size)");
                    writeln!(out, "        _buf{idx}.put({pname})");
                    writeln!(out, "        _buf{idx}.flip()");
                    args.push(format!("_buf{idx}"));
                }
                HybridType::Value => {
                    writeln!(out, "        val _buf{idx} = ByteBuffer.allocateDirect({pname}.binBytes.size)");
                    writeln!(out, "        _buf{idx}.put({pname}.binBytes)");
                    writeln!(out, "        _buf{idx}.flip()");
                    args.push(format!("_buf{idx}"));
                }
                other => {
                    writeln!(out, "        val _w{idx} = NveBinWriter()");
                    emit_kotlin_bin_write(out, "        ", &format!("_w{idx}"), &pname, other);
                    args.push(format!("_w{idx}.toDirectByteBuffer()"));
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
            HybridType::Value => writeln!(out, "        return NveHybridDynamicValue({call})"),
            other => {
                writeln!(out, "        val _r = NveBinReader({call})");
                writeln!(out, "        val _out: {}", kotlin_type(other));
                emit_kotlin_bin_read(out, "        ", "_r", "_out", other);
                writeln!(out, "        return _out");
            }
        }
        writeln!(out, "    }}");
        writeln!(out);
    }

    writeln!(out, "}}");
    writeln!(out);
}

