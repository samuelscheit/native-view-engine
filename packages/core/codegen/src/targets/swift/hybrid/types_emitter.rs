use crate::writer::CodeWriter;

use super::*;
use super::methods_emitter::*;

pub(super) fn emit_swift_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "public enum {}: UInt32, Equatable, Hashable {{", e.name);
    for v in &e.variants {
        let vname = swift_ident(&v.name);
        writeln!(out, "    case {vname} = {}", v.value);
    }
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_swift_struct(out: &mut CodeWriter, s: &HybridStructSpec, spec: &HybridFileSpec) {
    writeln!(out, "public struct {}: Equatable {{", s.name);
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        writeln!(out, "    public var {fname}: {}", swift_type(&f.ty));
    }
    writeln!(out);

    // Public memberwise init
    write!(out, "    public init(");
    for (idx, f) in s.fields.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ");
        }
        let fname = swift_ident(&f.name);
        write!(
            out,
            "{fname}: {} = {}",
            swift_type(&f.ty),
            swift_default_value(&f.ty, None, spec)
        );
    }
    writeln!(out, ") {{");
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        writeln!(out, "        self.{fname} = {fname}");
    }
    writeln!(out, "    }}");

    if s.is_pod(spec) {
        writeln!(out);
        writeln!(out, "    public func toAbi() -> NveHybrid{} {{", s.name);
        write!(out, "        NveHybrid{}(", s.name);
        for (idx, f) in s.fields.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ");
            }
            let cname = c_field_ident(&f.name);
            let fname = swift_ident(&f.name);
            match &f.ty {
                HybridType::Enum(_) => write!(out, "{cname}: {fname}.rawValue"),
                HybridType::Struct(_) => write!(out, "{cname}: {fname}.toAbi()"),
                _ => write!(out, "{cname}: {fname}"),
            }
        }
        writeln!(out, ")");
        writeln!(out, "    }}");
        writeln!(out);

        writeln!(
            out,
            "    public static func fromAbi(_ raw: NveHybrid{}) throws -> {} {{",
            s.name, s.name
        );
        write!(out, "        {}(", s.name);
        for (idx, f) in s.fields.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ");
            }
            let cname = c_field_ident(&f.name);
            let fname = swift_ident(&f.name);
            match &f.ty {
                HybridType::Enum(e) => write!(
                    out,
                    "{fname}: try {e}(rawValue: raw.{cname}).unwrapOrThrow(\"{e}\", raw.{cname})"
                ),
                HybridType::Struct(st) => {
                    write!(out, "{fname}: try {st}.fromAbi(raw.{cname})")
                }
                _ => write!(out, "{fname}: raw.{cname}"),
            }
        }
        writeln!(out, ")");
        writeln!(out, "    }}");
    }

    // Binary encode/decode
    writeln!(out);
    writeln!(out, "    public func encodeBin(into w: inout NveHybridBinWriter) {{");
    writeln!(out, "        w.writeMap(count: {}) {{ w in", s.fields.len());
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            w.writeMapKey(\"{key_lit}\")");
        emit_swift_bin_write(out, "            ", "w", &format!("self.{fname}"), &f.ty);
    }
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out);

    writeln!(
        out,
        "    public static func decodeBin(from r: inout NveHybridBinReader) throws -> {} {{",
        s.name
    );
    writeln!(out, "        var out = {}()", s.name);
    writeln!(out, "        let count = try r.beginMap()");
    writeln!(out, "        for _ in 0 ..< count {{");
    writeln!(out, "            let key = try r.readMapKey()");
    writeln!(out, "            switch key {{");
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            case \"{key_lit}\":");
        emit_swift_bin_read(out, "                ", "r", &format!("out.{fname}"), &f.ty);
    }
    writeln!(out, "            default:");
    writeln!(out, "                try r.skipValue()");
    writeln!(out, "            }}");
    writeln!(out, "        }}");
    writeln!(out, "        return out");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_swift_module(out: &mut CodeWriter, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    writeln!(out, "public protocol Hybrid{pascal}Spec: AnyObject {{");
    for method in &m.methods {
        writeln!(out, "    {}", swift_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_swift_client(out, &pascal, m.interface_id(), m.schema_hash(spec), &m.methods, None, spec);
}

pub(super) fn emit_swift_view(out: &mut CodeWriter, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);

    // Props struct (conforms to `NveDirectEncodable` for zero-alloc stack-slab arena encoding)
    writeln!(out, "public struct {pascal}Props: Equatable, NveDirectEncodable {{");
    for p in &v.props {
        let pname = swift_ident(&p.name);
        writeln!(out, "    public var {pname}: {}", swift_type(&p.ty));
    }
    writeln!(out);
    write!(out, "    public init(");
    for (idx, p) in v.props.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ");
        }
        let pname = swift_ident(&p.name);
        let def = swift_default_value(&p.ty, p.default_value.as_ref(), spec);
        write!(out, "{pname}: {} = {def}", swift_type(&p.ty));
    }
    writeln!(out, ") {{");
    for p in &v.props {
        let pname = swift_ident(&p.name);
        writeln!(out, "        self.{pname} = {pname}");
    }
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    public var directPropCount: Int {{ {} }}", v.props.len());
    writeln!(out);
    writeln!(out, "    public func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {{");
    for p in &v.props {
        let key_lit = escape_c_like_string(&p.name);
        let pname = swift_ident(&p.name);
        emit_swift_direct_prop_write(out, &key_lit, &format!("self.{pname}"), &p.ty);
    }
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);

    // Event structs
    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "public struct {pascal}{ev_pascal}Event: Equatable {{");
        for param in &ev.params {
            let pname = swift_ident(&param.name);
            writeln!(out, "    public var {pname}: {}", swift_type(&param.ty));
        }
        writeln!(out);
        write!(out, "    public init(");
        for (idx, param) in ev.params.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ");
            }
            let pname = swift_ident(&param.name);
            write!(out, "{pname}: {}", swift_type(&param.ty));
        }
        writeln!(out, ") {{");
        for param in &ev.params {
            let pname = swift_ident(&param.name);
            writeln!(out, "        self.{pname} = {pname}");
        }
        writeln!(out, "    }}");
        writeln!(out, "}}");
        writeln!(out);
    }

    writeln!(out, "public protocol Hybrid{pascal}Spec: AnyObject {{");
    writeln!(out, "    func setProps(_ props: {pascal}Props) throws");
    for method in &v.methods {
        writeln!(out, "    {}", swift_method_sig(method));
    }
    writeln!(out, "}}");
    writeln!(out);

    emit_swift_client(out, &pascal, v.interface_id(), v.schema_hash(spec), &v.methods, Some(v), spec);
}

pub(super) fn emit_swift_client(
    out: &mut CodeWriter,
    pascal: &str,
    interface_id: nve_abi::InterfaceId,
    schema_hash: u64,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
    spec: &HybridFileSpec,
) {
    writeln!(out, "public final class {pascal}HybridClient: Hybrid{pascal}Spec {{");
    writeln!(out, "    public static let interfaceId: UInt32 = 0x{interface_id:08X}");
    writeln!(out, "    public static let schemaHash: UInt64 = 0x{schema_hash:016X}");
    writeln!(out);
    writeln!(out, "    private let vtable: UnsafePointer<NveHybrid{pascal}VTable>");
    writeln!(out, "    private var handle: NveAppHandle?");
    writeln!(out);

    writeln!(
        out,
        "    public static func resolve(getInterface: NveGetInterfaceFn?) throws -> {pascal}HybridClient {{"
    );
    writeln!(out, "        guard let resolver = getInterface else {{ throw NveHybridError.nullInterfaceResolver }}");
    writeln!(out, "        guard let raw = resolver(interfaceId) else {{ throw NveHybridError.interfaceNotFound(interfaceId) }}");
    writeln!(
        out,
        "        return try {pascal}HybridClient(vtable: raw.assumingMemoryBound(to: NveHybrid{pascal}VTable.self))"
    );
    writeln!(out, "    }}");
    writeln!(out);

    writeln!(
        out,
        "    public init(vtable: UnsafePointer<NveHybrid{pascal}VTable>) throws {{"
    );
    writeln!(out, "        if vtable.pointee.abi_version != nveHybridAbiVersion || vtable.pointee.schema_hash != Self.schemaHash {{");
    writeln!(out, "            throw NveHybridError.schemaMismatch(expected: Self.schemaHash, actual: vtable.pointee.schema_hash)");
    writeln!(out, "        }}");
    writeln!(out, "        guard let createFn = vtable.pointee.create, let h = createFn() else {{");
    writeln!(out, "            throw NveHybridError.nullHandle");
    writeln!(out, "        }}");
    writeln!(out, "        self.vtable = vtable");
    writeln!(out, "        self.handle = h");
    writeln!(out, "    }}");
    writeln!(out);

    writeln!(out, "    deinit {{");
    writeln!(out, "        if let h = handle, let destroyFn = vtable.pointee.destroy {{");
    writeln!(out, "            destroyFn(h)");
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out);

    if let Some(v) = view {
        emit_swift_set_props(out, pascal, v);
    }

    for method in methods {
        emit_swift_client_method(out, method, spec);
    }

    writeln!(out, "    private func takeString(_ raw: NveHybridOwnedBytes) -> String {{");
    writeln!(out, "        defer {{ if let freeFn = vtable.pointee.free_bytes {{ freeFn(raw) }} }}");
    writeln!(out, "        guard let ptr = raw.ptr, raw.len > 0 else {{ return \"\" }}");
    writeln!(out, "        let buf = UnsafeBufferPointer(start: ptr, count: Int(raw.len))");
    writeln!(out, "        return String(decoding: buf, as: UTF8.self)");
    writeln!(out, "    }}");
    writeln!(out);

    writeln!(out, "    private func takeBytes(_ raw: NveHybridOwnedBytes) -> [UInt8] {{");
    writeln!(out, "        defer {{ if let freeFn = vtable.pointee.free_bytes {{ freeFn(raw) }} }}");
    writeln!(out, "        guard let ptr = raw.ptr, raw.len > 0 else {{ return [] }}");
    writeln!(out, "        let buf = UnsafeBufferPointer(start: ptr, count: Int(raw.len))");
    writeln!(out, "        return Array(buf)");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

pub(super) fn emit_swift_direct_prop_write(out: &mut CodeWriter, key_lit: &str, expr: &str, ty: &HybridType) {
    match ty {
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
        | HybridType::Bytes => {
            writeln!(out, "        builder.write(\"{key_lit}\", {expr})");
        }
        HybridType::Enum(_) => {
            writeln!(out, "        builder.write(\"{key_lit}\", enum: {expr})");
        }
        HybridType::Optional(inner) => match inner.as_ref() {
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
            | HybridType::Bytes => {
                writeln!(out, "        builder.write(\"{key_lit}\", {expr})");
            }
            HybridType::Enum(_) => {
                writeln!(out, "        builder.write(\"{key_lit}\", enum: {expr})");
            }
            other => {
                writeln!(out, "        if let _optProp = {expr} {{");
                writeln!(out, "            var _w = NveHybridBinWriter()");
                emit_swift_bin_write(out, "            ", "_w", "_optProp", other);
                writeln!(out, "            builder.write(\"{key_lit}\", _w.bytes)");
                writeln!(out, "        }} else {{");
                writeln!(out, "            builder.writeNull(\"{key_lit}\")");
                writeln!(out, "        }}");
            }
        },
        other => {
            writeln!(out, "        do {{");
            writeln!(out, "            var _w = NveHybridBinWriter()");
            emit_swift_bin_write(out, "            ", "_w", expr, other);
            writeln!(out, "            builder.write(\"{key_lit}\", _w.bytes)");
            writeln!(out, "        }}");
        }
    }
}

pub(super) fn emit_swift_set_props(out: &mut CodeWriter, pascal: &str, _v: &HybridViewSpec) {
    writeln!(out, "    public func setProps(_ props: {pascal}Props) throws {{");
    writeln!(out, "        guard let h = handle, let setFn = vtable.pointee.set_props_direct else {{ throw NveHybridError.nullHandle }}");
    writeln!(out, "        let status: Int32 = try NveDirectPropsWriter.withEncodedProps(props) {{ ptr, count in");
    writeln!(out, "            setFn(h, ptr, count)");
    writeln!(out, "        }}");
    writeln!(out, "        if status != Int32(NVE_HYBRID_OK.rawValue) {{");
    writeln!(out, "            throw NveHybridError.remoteError(\"setProps failed\")");
    writeln!(out, "        }}");
    writeln!(out, "    }}");
    writeln!(out);
}

