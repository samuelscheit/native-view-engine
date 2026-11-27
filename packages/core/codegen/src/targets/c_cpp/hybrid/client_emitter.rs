use crate::writer::CodeWriter;

use super::*;
use super::type_helpers::*;

pub(super) fn emit_cpp_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "enum class {} : uint32_t {{", e.name);
    for v in &e.variants {
        writeln!(out, "  {} = {}u,", v.name, v.value);
    }
    writeln!(out, "}};");
    writeln!(out);
}

pub(super) fn emit_cpp_struct(out: &mut CodeWriter, s: &HybridStructSpec, spec: &HybridFileSpec) {
    writeln!(out, "struct {} {{", s.name);
    for f in &s.fields {
        let fname = cpp_ident(&f.name);
        writeln!(out, "  {} {fname}{{}};", cpp_owned_type(&f.ty));
    }
    writeln!(out);

    if s.is_pod(spec) {
        writeln!(out, "  NveHybrid{} to_abi() const noexcept {{", s.name);
        writeln!(out, "    NveHybrid{} raw{{}};", s.name);
        for f in &s.fields {
            let fname = cpp_ident(&f.name);
            let cname = c_field_ident(&f.name);
            match &f.ty {
                HybridType::Enum(_) => {
                    writeln!(out, "    raw.{cname} = static_cast<uint32_t>({fname});");
                }
                HybridType::Struct(_) => {
                    writeln!(out, "    raw.{cname} = {fname}.to_abi();");
                }
                HybridType::FixedArray { .. } => {
                    writeln!(
                        out,
                        "    std::memcpy(raw.{cname}, {fname}.data(), sizeof(raw.{cname}));"
                    );
                }
                _ => {
                    writeln!(out, "    raw.{cname} = {fname};");
                }
            }
        }
        writeln!(out, "    return raw;");
        writeln!(out, "  }}");
        writeln!(out);

        writeln!(out, "  static {} from_abi(const NveHybrid{}& raw) noexcept {{", s.name, s.name);
        writeln!(out, "    {} out{{}};", s.name);
        for f in &s.fields {
            let fname = cpp_ident(&f.name);
            let cname = c_field_ident(&f.name);
            match &f.ty {
                HybridType::Enum(e) => {
                    writeln!(out, "    out.{fname} = static_cast<{e}>(raw.{cname});");
                }
                HybridType::Struct(st) => {
                    writeln!(out, "    out.{fname} = {st}::from_abi(raw.{cname});");
                }
                HybridType::FixedArray { .. } => {
                    writeln!(
                        out,
                        "    std::memcpy(out.{fname}.data(), raw.{cname}, sizeof(raw.{cname}));"
                    );
                }
                _ => {
                    writeln!(out, "    out.{fname} = raw.{cname};");
                }
            }
        }
        writeln!(out, "    return out;");
        writeln!(out, "  }}");
        writeln!(out);
    }

    // Compact binary NveBinValue encode/decode for every struct
    writeln!(out, "  void encode_bin(BinWriter& w) const {{");
    writeln!(out, "    w.write_map({}u, [&](BinWriter& w) {{", s.fields.len());
    for f in &s.fields {
        let fname = cpp_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "      w.write_map_key(\"{key_lit}\");");
        emit_cpp_bin_write(out, "      ", "w", &fname, &f.ty);
    }
    writeln!(out, "    }});");
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(out, "  static {} decode_bin(BinReader& r) {{", s.name);
    writeln!(out, "    {} out{{}};", s.name);
    writeln!(out, "    const uint32_t count = r.begin_map();");
    writeln!(out, "    for (uint32_t i = 0; i < count; ++i) {{");
    writeln!(out, "      const auto key = r.read_map_key();");
    for (idx, f) in s.fields.iter().enumerate() {
        let fname = cpp_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        let cond = if idx == 0 { "if" } else { "else if" };
        writeln!(out, "      {cond} (key == \"{key_lit}\") {{");
        emit_cpp_bin_read(out, "        ", "r", &format!("out.{fname}"), &f.ty);
        writeln!(out, "      }}");
    }
    writeln!(out, "      else {{");
    writeln!(out, "        r.skip_value();");
    writeln!(out, "      }}");
    writeln!(out, "    }}");
    writeln!(out, "    return out;");
    writeln!(out, "  }}");

    writeln!(out, "}};");
    writeln!(out);
}

pub(super) fn emit_cpp_module(out: &mut CodeWriter, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    let upper = to_snake_case(&m.name).to_ascii_uppercase();

    // Nitro-style abstract Spec base class
    writeln!(out, "/// Nitro-style compile-time-enforced C++20 contract for `{pascal}`.");
    writeln!(out, "class Hybrid{pascal}Spec {{");
    writeln!(out, " public:");
    writeln!(out, "  virtual ~Hybrid{pascal}Spec() = default;");
    for method in &m.methods {
        writeln!(out, "  virtual {} = 0;", cpp_method_signature(method));
    }
    writeln!(out, "}};");
    writeln!(out);

    emit_cpp_client(out, &pascal, &upper, &m.methods, None, spec);
}

pub(super) fn emit_cpp_view(out: &mut CodeWriter, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);
    let upper = to_snake_case(&v.name).to_ascii_uppercase();

    // Props struct
    writeln!(out, "struct {pascal}Props {{");
    for p in &v.props {
        let pname = cpp_ident(&p.name);
        let def = cpp_prop_default_initializer(&p.ty, p.default_value.as_ref());
        writeln!(out, "  {} {pname} = {def};", cpp_owned_type(&p.ty));
    }
    writeln!(out, "}};");
    writeln!(out);

    // Event structs
    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "struct {pascal}{ev_pascal}Event {{");
        for param in &ev.params {
            let pname = cpp_ident(&param.name);
            writeln!(out, "  {} {pname}{{}};", cpp_owned_type(&param.ty));
        }
        writeln!(out, "}};");
        writeln!(out);
    }

    // Nitro-style abstract Spec base class
    writeln!(out, "/// Nitro-style compile-time-enforced C++20 view contract for `{pascal}`.");
    writeln!(out, "class Hybrid{pascal}Spec {{");
    writeln!(out, " public:");
    writeln!(out, "  virtual ~Hybrid{pascal}Spec() = default;");
    writeln!(out, "  virtual void setProps(const {pascal}Props& props) = 0;");
    for method in &v.methods {
        writeln!(out, "  virtual {} = 0;", cpp_method_signature(method));
    }
    writeln!(out, "}};");
    writeln!(out);

    emit_cpp_client(out, &pascal, &upper, &v.methods, Some(v), spec);
}

pub(super) fn emit_cpp_client(
    out: &mut CodeWriter,
    pascal: &str,
    upper: &str,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
    spec: &HybridFileSpec,
) {
    writeln!(
        out,
        "/// Zero-overhead RAII C++20 client wrapping `const NveHybrid{pascal}VTable*`."
    );
    writeln!(out, "class {pascal}Client final : public Hybrid{pascal}Spec {{");
    writeln!(out, " public:");
    writeln!(out, "  static constexpr uint32_t kInterfaceId = NVE_HYBRID_{upper}_INTERFACE_ID;");
    writeln!(out, "  static constexpr uint64_t kSchemaHash = NVE_HYBRID_{upper}_SCHEMA_HASH;");
    writeln!(out);

    writeln!(
        out,
        "  static {pascal}Client resolve(NveGetInterfaceFn get_iface) {{"
    );
    writeln!(out, "    if (!get_iface) {{");
    writeln!(out, "      throw std::runtime_error(\"nve_get_interface pointer is null\");");
    writeln!(out, "    }}");
    writeln!(
        out,
        "    const auto* vt = static_cast<const NveHybrid{pascal}VTable*>(get_iface(kInterfaceId));"
    );
    writeln!(out, "    return {pascal}Client(vt);");
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(out, "  explicit {pascal}Client(const NveHybrid{pascal}VTable* vtable) : vtable_(vtable) {{");
    writeln!(out, "    if (!vtable_) {{");
    writeln!(out, "      throw std::runtime_error(\"hybrid vtable for {pascal} is null\");");
    writeln!(out, "    }}");
    writeln!(out, "    if (vtable_->abi_version != NVE_HYBRID_ABI_VERSION || vtable_->schema_hash != kSchemaHash) {{");
    writeln!(out, "      throw std::runtime_error(\"hybrid schema hash mismatch for {pascal}\");");
    writeln!(out, "    }}");
    writeln!(out, "    handle_ = vtable_->create();");
    writeln!(out, "    if (!handle_) {{");
    writeln!(out, "      throw std::runtime_error(\"failed to create {pascal} hybrid instance\");");
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);

    writeln!(out, "  {pascal}Client(const {pascal}Client&) = delete;");
    writeln!(out, "  {pascal}Client& operator=(const {pascal}Client&) = delete;");
    writeln!(out);
    writeln!(out, "  {pascal}Client({pascal}Client&& other) noexcept : vtable_(other.vtable_), handle_(other.handle_) {{");
    writeln!(out, "    other.vtable_ = nullptr;");
    writeln!(out, "    other.handle_ = nullptr;");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  {pascal}Client& operator=({pascal}Client&& other) noexcept {{");
    writeln!(out, "    if (this != &other) {{");
    writeln!(out, "      reset();");
    writeln!(out, "      vtable_ = other.vtable_;");
    writeln!(out, "      handle_ = other.handle_;");
    writeln!(out, "      other.vtable_ = nullptr;");
    writeln!(out, "      other.handle_ = nullptr;");
    writeln!(out, "    }}");
    writeln!(out, "    return *this;");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  ~{pascal}Client() override {{ reset(); }}");
    writeln!(out);
    writeln!(out, "  void reset() noexcept {{");
    writeln!(out, "    if (vtable_ && handle_) {{");
    writeln!(out, "      vtable_->destroy(handle_);");
    writeln!(out, "    }}");
    writeln!(out, "    handle_ = nullptr;");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  const NveHybrid{pascal}VTable* vtable() const noexcept {{ return vtable_; }}");
    writeln!(out, "  NveAppHandle handle() const noexcept {{ return handle_; }}");
    writeln!(out);

    if let Some(v) = view {
        emit_cpp_set_props(out, pascal, v);
    }

    for method in methods {
        emit_cpp_client_method(out, method, spec);
    }

    writeln!(out, " private:");
    writeln!(out, "  std::string take_string(NveHybridOwnedBytes raw) const {{");
    writeln!(out, "    if (!raw.ptr || raw.len == 0) {{");
    writeln!(out, "      if (raw.ptr) {{ vtable_->free_bytes(raw); }}");
    writeln!(out, "      return {{}};");
    writeln!(out, "    }}");
    writeln!(out, "    std::string out(reinterpret_cast<const char*>(raw.ptr), raw.len);");
    writeln!(out, "    vtable_->free_bytes(raw);");
    writeln!(out, "    return out;");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  std::vector<uint8_t> take_bytes(NveHybridOwnedBytes raw) const {{");
    writeln!(out, "    if (!raw.ptr || raw.len == 0) {{");
    writeln!(out, "      if (raw.ptr) {{ vtable_->free_bytes(raw); }}");
    writeln!(out, "      return {{}};");
    writeln!(out, "    }}");
    writeln!(out, "    std::vector<uint8_t> out(raw.ptr, raw.ptr + raw.len);");
    writeln!(out, "    vtable_->free_bytes(raw);");
    writeln!(out, "    return out;");
    writeln!(out, "  }}");
    writeln!(out);
    writeln!(out, "  const NveHybrid{pascal}VTable* vtable_ = nullptr;");
    writeln!(out, "  NveAppHandle handle_ = nullptr;");
    writeln!(out, "}};");
    writeln!(out);
}

pub(super) fn emit_cpp_set_props(out: &mut CodeWriter, pascal: &str, v: &HybridViewSpec) {
    writeln!(out, "  void setProps(const {pascal}Props& props) override {{");
    if v.props.is_empty() {
        writeln!(out, "    (void)props;");
        writeln!(out, "    vtable_->set_props_direct(handle_, nullptr, 0u);");
        writeln!(out, "  }}");
        writeln!(out);
        return;
    }
    writeln!(out, "    std::vector<NveMapEntry> entries;");
    writeln!(out, "    entries.reserve({}u);", v.props.len());
    // Keep any scratch BinWriters alive for the duration of set_props_direct.
    for (idx, p) in v.props.iter().enumerate() {
        if !cpp_prop_is_direct_nve_value(&p.ty) {
            let pname = cpp_ident(&p.name);
            let wname = format!("_prop_bin_{idx}");
            writeln!(out, "    BinWriter {wname};");
            emit_cpp_bin_write(out, "    ", &wname, &format!("props.{pname}"), &p.ty);
        }
    }
    for (idx, p) in v.props.iter().enumerate() {
        let pname = cpp_ident(&p.name);
        let key_lit = escape_c_like_string(&p.name);
        writeln!(out, "    {{");
        writeln!(out, "      NveMapEntry entry{{}};");
        writeln!(
            out,
            "      entry.key = NveStrSlice{{reinterpret_cast<const uint8_t*>(\"{key_lit}\"), {}u}};",
            p.name.len()
        );
        emit_cpp_fill_nve_value(out, "      ", "entry.value", &format!("props.{pname}"), &p.ty, idx);
        writeln!(out, "      entries.push_back(entry);");
        writeln!(out, "    }}");
    }
    writeln!(
        out,
        "    const int32_t status = vtable_->set_props_direct(handle_, entries.data(), static_cast<uint32_t>(entries.size()));"
    );
    writeln!(out, "    if (status != NVE_HYBRID_OK) {{");
    writeln!(out, "      throw std::runtime_error(\"setProps failed on {pascal}\");");
    writeln!(out, "    }}");
    writeln!(out, "  }}");
    writeln!(out);
}

pub(super) fn emit_cpp_client_method(out: &mut CodeWriter, method: &HybridMethodSpec, spec: &HybridFileSpec) {
    let cname = c_field_ident(&method.name);
    writeln!(out, "  {} override {{", cpp_method_signature(method));

    // Prepare arguments
    let mut call_args = vec!["handle_".to_string()];
    for (idx, p) in method.params.iter().enumerate() {
        let pname = cpp_ident(&p.name);
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
            | HybridType::F64 => call_args.push(pname),
            HybridType::Enum(_) => call_args.push(format!("static_cast<uint32_t>({pname})")),
            HybridType::Struct(_) if p.ty.is_pod(spec) => {
                call_args.push(format!("{pname}.to_abi()"));
            }
            HybridType::String => {
                writeln!(
                    out,
                    "    const NveStrSlice _arg_{idx}{{reinterpret_cast<const uint8_t*>({pname}.data()), {pname}.size()}};"
                );
                call_args.push(format!("_arg_{idx}"));
            }
            HybridType::Bytes => {
                writeln!(
                    out,
                    "    const NveBufferSlice _arg_{idx}{{{pname}.data(), {pname}.size()}};"
                );
                call_args.push(format!("_arg_{idx}"));
            }
            HybridType::Value => call_args.push(pname),
            HybridType::Optional(inner) => {
                if let Some(suffix) = inner.pod_opt_suffix_for(spec) {
                    writeln!(out, "    NveHybridOpt{suffix} _arg_{idx}{{false, {{}}}};");
                    writeln!(out, "    if ({pname}.has_value()) {{");
                    writeln!(out, "      _arg_{idx}.is_some = true;");
                    match inner.as_ref() {
                        HybridType::Enum(_) => {
                            writeln!(out, "      _arg_{idx}.value = static_cast<uint32_t>(*{pname});");
                        }
                        HybridType::Struct(_) => {
                            writeln!(out, "      _arg_{idx}.value = {pname}->to_abi();");
                        }
                        _ => {
                            writeln!(out, "      _arg_{idx}.value = *{pname};");
                        }
                    }
                    writeln!(out, "    }}");
                    call_args.push(format!("_arg_{idx}"));
                } else {
                    let wname = format!("_arg_w_{idx}");
                    writeln!(out, "    BinWriter {wname};");
                    emit_cpp_bin_write(out, "    ", &wname, &pname, &p.ty);
                    call_args.push(format!("{wname}.slice()"));
                }
            }
            other => {
                let wname = format!("_arg_w_{idx}");
                writeln!(out, "    BinWriter {wname};");
                emit_cpp_bin_write(out, "    ", &wname, &pname, other);
                call_args.push(format!("{wname}.slice()"));
            }
        }
    }

    match &method.return_type {
        HybridType::Void => {
            writeln!(out, "    vtable_->{cname}({});", call_args.join(", "));
        }
        HybridType::Result { ok, .. } => {
            if !matches!(ok.as_ref(), HybridType::Void) {
                writeln!(out, "    {} _raw_ok{{}};", c_return_type(ok, spec));
                call_args.push("&_raw_ok".to_string());
            }
            writeln!(out, "    NveHybridOwnedBytes _raw_err{{nullptr, 0, 0}};");
            call_args.push("&_raw_err".to_string());
            writeln!(
                out,
                "    const int32_t _status = vtable_->{cname}({});",
                call_args.join(", ")
            );
            writeln!(out, "    if (_status != NVE_HYBRID_OK) {{");
            writeln!(out, "      const std::string _msg = take_string(_raw_err);");
            writeln!(
                out,
                "      throw std::runtime_error(_msg.empty() ? \"hybrid call failed\" : _msg);"
            );
            writeln!(out, "    }}");
            if !matches!(ok.as_ref(), HybridType::Void) {
                emit_cpp_return_decode(out, "    ", "_raw_ok", ok, spec);
            }
        }
        other => {
            writeln!(
                out,
                "    const auto _raw_ret = vtable_->{cname}({});",
                call_args.join(", ")
            );
            emit_cpp_return_decode(out, "    ", "_raw_ret", other, spec);
        }
    }

    writeln!(out, "  }}");
    writeln!(out);
}

pub(super) fn emit_cpp_return_decode(
    out: &mut CodeWriter,
    indent: &str,
    var: &str,
    ty: &HybridType,
    spec: &HybridFileSpec,
) {
    match ty {
        HybridType::Void => {}
        HybridType::Bool
        | HybridType::U8
        | HybridType::U16
        | HybridType::Usize
        | HybridType::I32
        | HybridType::U32
        | HybridType::I64
        | HybridType::U64
        | HybridType::F32
        | HybridType::F64 => {
            writeln!(out, "{indent}return {var};");
        }
        HybridType::Enum(e) => {
            writeln!(out, "{indent}return static_cast<{e}>({var});");
        }
        HybridType::Struct(s) if ty.is_pod(spec) => {
            writeln!(out, "{indent}return {s}::from_abi({var});");
        }
        HybridType::String => {
            writeln!(out, "{indent}return take_string({var});");
        }
        HybridType::Bytes => {
            writeln!(out, "{indent}return take_bytes({var});");
        }
        HybridType::Value => {
            writeln!(out, "{indent}return DynamicValue{{take_bytes({var})}};");
        }
        HybridType::Optional(inner) if inner.is_pod(spec) => {
            writeln!(out, "{indent}if (!{var}.is_some) {{ return std::nullopt; }}");
            match inner.as_ref() {
                HybridType::Enum(e) => {
                    writeln!(out, "{indent}return static_cast<{e}>({var}.value);");
                }
                HybridType::Struct(s) => {
                    writeln!(out, "{indent}return {s}::from_abi({var}.value);");
                }
                _ => {
                    writeln!(out, "{indent}return {var}.value;");
                }
            }
        }
        other => {
            writeln!(out, "{indent}const auto _bytes = take_bytes({var});");
            writeln!(out, "{indent}BinReader _r(_bytes);");
            writeln!(out, "{indent}{} _out{{}};", cpp_owned_type(other));
            emit_cpp_bin_read(out, indent, "_r", "_out", other);
            writeln!(out, "{indent}return _out;");
        }
    }
}

