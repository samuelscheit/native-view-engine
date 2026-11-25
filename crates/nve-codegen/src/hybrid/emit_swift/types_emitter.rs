use std::fmt::Write as _;

use super::*;
use super::methods_emitter::*;

pub(super) fn emit_swift_enum(out: &mut String, e: &HybridEnumSpec) {
    writeln!(out, "public enum {}: UInt32, Equatable, Hashable {{", e.name).unwrap();
    for v in &e.variants {
        let vname = swift_ident(&v.name);
        writeln!(out, "    case {vname} = {}", v.value).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_swift_struct(out: &mut String, s: &HybridStructSpec, spec: &HybridFileSpec) {
    writeln!(out, "public struct {}: Equatable {{", s.name).unwrap();
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        writeln!(out, "    public var {fname}: {}", swift_type(&f.ty)).unwrap();
    }
    writeln!(out).unwrap();

    // Public memberwise init
    write!(out, "    public init(").unwrap();
    for (idx, f) in s.fields.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ").unwrap();
        }
        let fname = swift_ident(&f.name);
        write!(
            out,
            "{fname}: {} = {}",
            swift_type(&f.ty),
            swift_default_value(&f.ty, None, spec)
        )
        .unwrap();
    }
    writeln!(out, ") {{").unwrap();
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        writeln!(out, "        self.{fname} = {fname}").unwrap();
    }
    writeln!(out, "    }}").unwrap();

    if s.is_pod(spec) {
        writeln!(out).unwrap();
        writeln!(out, "    public func toAbi() -> NveHybrid{} {{", s.name).unwrap();
        write!(out, "        NveHybrid{}(", s.name).unwrap();
        for (idx, f) in s.fields.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ").unwrap();
            }
            let cname = c_field_ident(&f.name);
            let fname = swift_ident(&f.name);
            match &f.ty {
                HybridType::Enum(_) => write!(out, "{cname}: {fname}.rawValue").unwrap(),
                HybridType::Struct(_) => write!(out, "{cname}: {fname}.toAbi()").unwrap(),
                _ => write!(out, "{cname}: {fname}").unwrap(),
            }
        }
        writeln!(out, ")").unwrap();
        writeln!(out, "    }}").unwrap();
        writeln!(out).unwrap();

        writeln!(
            out,
            "    public static func fromAbi(_ raw: NveHybrid{}) throws -> {} {{",
            s.name, s.name
        )
        .unwrap();
        write!(out, "        {}(", s.name).unwrap();
        for (idx, f) in s.fields.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ").unwrap();
            }
            let cname = c_field_ident(&f.name);
            let fname = swift_ident(&f.name);
            match &f.ty {
                HybridType::Enum(e) => write!(
                    out,
                    "{fname}: try {e}(rawValue: raw.{cname}).unwrapOrThrow(\"{e}\", raw.{cname})"
                )
                .unwrap(),
                HybridType::Struct(st) => {
                    write!(out, "{fname}: try {st}.fromAbi(raw.{cname})").unwrap()
                }
                _ => write!(out, "{fname}: raw.{cname}").unwrap(),
            }
        }
        writeln!(out, ")").unwrap();
        writeln!(out, "    }}").unwrap();
    }

    // Binary encode/decode
    writeln!(out).unwrap();
    writeln!(out, "    public func encodeBin(into w: inout NveHybridBinWriter) {{").unwrap();
    writeln!(out, "        w.writeMap(count: {}) {{ w in", s.fields.len()).unwrap();
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            w.writeMapKey(\"{key_lit}\")").unwrap();
        emit_swift_bin_write(out, "            ", "w", &format!("self.{fname}"), &f.ty);
    }
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "    public static func decodeBin(from r: inout NveHybridBinReader) throws -> {} {{",
        s.name
    )
    .unwrap();
    writeln!(out, "        var out = {}()", s.name).unwrap();
    writeln!(out, "        let count = try r.beginMap()").unwrap();
    writeln!(out, "        for _ in 0 ..< count {{").unwrap();
    writeln!(out, "            let key = try r.readMapKey()").unwrap();
    writeln!(out, "            switch key {{").unwrap();
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            case \"{key_lit}\":").unwrap();
        emit_swift_bin_read(out, "                ", "r", &format!("out.{fname}"), &f.ty);
    }
    writeln!(out, "            default:").unwrap();
    writeln!(out, "                try r.skipValue()").unwrap();
    writeln!(out, "            }}").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        return out").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_swift_module(out: &mut String, m: &HybridModuleSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&m.name);
    writeln!(out, "public protocol Hybrid{pascal}Spec: AnyObject {{").unwrap();
    for method in &m.methods {
        writeln!(out, "    {}", swift_method_sig(method)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_swift_client(out, &pascal, m.interface_id(), m.schema_hash(spec), &m.methods, None, spec);
}

pub(super) fn emit_swift_view(out: &mut String, v: &HybridViewSpec, spec: &HybridFileSpec) {
    let pascal = to_pascal_case(&v.name);

    // Props struct (conforms to `NveDirectEncodable` for zero-alloc stack-slab arena encoding)
    writeln!(out, "public struct {pascal}Props: Equatable, NveDirectEncodable {{").unwrap();
    for p in &v.props {
        let pname = swift_ident(&p.name);
        writeln!(out, "    public var {pname}: {}", swift_type(&p.ty)).unwrap();
    }
    writeln!(out).unwrap();
    write!(out, "    public init(").unwrap();
    for (idx, p) in v.props.iter().enumerate() {
        if idx > 0 {
            write!(out, ", ").unwrap();
        }
        let pname = swift_ident(&p.name);
        let def = swift_default_value(&p.ty, p.default_value.as_ref(), spec);
        write!(out, "{pname}: {} = {def}", swift_type(&p.ty)).unwrap();
    }
    writeln!(out, ") {{").unwrap();
    for p in &v.props {
        let pname = swift_ident(&p.name);
        writeln!(out, "        self.{pname} = {pname}").unwrap();
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    public var directPropCount: Int {{ {} }}", v.props.len()).unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    public func encodeDirect(into builder: inout NveDirectPropsBuilder) throws {{").unwrap();
    for p in &v.props {
        let key_lit = escape_c_like_string(&p.name);
        let pname = swift_ident(&p.name);
        emit_swift_direct_prop_write(out, &key_lit, &format!("self.{pname}"), &p.ty);
    }
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    // Event structs
    for ev in &v.events {
        let ev_pascal = to_pascal_case(&ev.name);
        writeln!(out, "public struct {pascal}{ev_pascal}Event: Equatable {{").unwrap();
        for param in &ev.params {
            let pname = swift_ident(&param.name);
            writeln!(out, "    public var {pname}: {}", swift_type(&param.ty)).unwrap();
        }
        writeln!(out).unwrap();
        write!(out, "    public init(").unwrap();
        for (idx, param) in ev.params.iter().enumerate() {
            if idx > 0 {
                write!(out, ", ").unwrap();
            }
            let pname = swift_ident(&param.name);
            write!(out, "{pname}: {}", swift_type(&param.ty)).unwrap();
        }
        writeln!(out, ") {{").unwrap();
        for param in &ev.params {
            let pname = swift_ident(&param.name);
            writeln!(out, "        self.{pname} = {pname}").unwrap();
        }
        writeln!(out, "    }}").unwrap();
        writeln!(out, "}}").unwrap();
        writeln!(out).unwrap();
    }

    writeln!(out, "public protocol Hybrid{pascal}Spec: AnyObject {{").unwrap();
    writeln!(out, "    func setProps(_ props: {pascal}Props) throws").unwrap();
    for method in &v.methods {
        writeln!(out, "    {}", swift_method_sig(method)).unwrap();
    }
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();

    emit_swift_client(out, &pascal, v.interface_id(), v.schema_hash(spec), &v.methods, Some(v), spec);
}

pub(super) fn emit_swift_client(
    out: &mut String,
    pascal: &str,
    interface_id: u32,
    schema_hash: u64,
    methods: &[HybridMethodSpec],
    view: Option<&HybridViewSpec>,
    spec: &HybridFileSpec,
) {
    writeln!(out, "public final class {pascal}HybridClient: Hybrid{pascal}Spec {{").unwrap();
    writeln!(out, "    public static let interfaceId: UInt32 = 0x{interface_id:08X}").unwrap();
    writeln!(out, "    public static let schemaHash: UInt64 = 0x{schema_hash:016X}").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "    private let vtable: UnsafePointer<NveHybrid{pascal}VTable>").unwrap();
    writeln!(out, "    private var handle: NveAppHandle?").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "    public static func resolve(getInterface: NveGetInterfaceFn?) throws -> {pascal}HybridClient {{"
    )
    .unwrap();
    writeln!(out, "        guard let resolver = getInterface else {{ throw NveHybridError.nullInterfaceResolver }}").unwrap();
    writeln!(out, "        guard let raw = resolver(interfaceId) else {{ throw NveHybridError.interfaceNotFound(interfaceId) }}").unwrap();
    writeln!(
        out,
        "        return try {pascal}HybridClient(vtable: raw.assumingMemoryBound(to: NveHybrid{pascal}VTable.self))"
    )
    .unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(
        out,
        "    public init(vtable: UnsafePointer<NveHybrid{pascal}VTable>) throws {{"
    )
    .unwrap();
    writeln!(out, "        if vtable.pointee.abi_version != nveHybridAbiVersion || vtable.pointee.schema_hash != Self.schemaHash {{").unwrap();
    writeln!(out, "            throw NveHybridError.schemaMismatch(expected: Self.schemaHash, actual: vtable.pointee.schema_hash)").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        guard let createFn = vtable.pointee.create, let h = createFn() else {{").unwrap();
    writeln!(out, "            throw NveHybridError.nullHandle").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        self.vtable = vtable").unwrap();
    writeln!(out, "        self.handle = h").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "    deinit {{").unwrap();
    writeln!(out, "        if let h = handle, let destroyFn = vtable.pointee.destroy {{").unwrap();
    writeln!(out, "            destroyFn(h)").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    if let Some(v) = view {
        emit_swift_set_props(out, pascal, v);
    }

    for method in methods {
        emit_swift_client_method(out, method, spec);
    }

    writeln!(out, "    private func takeString(_ raw: NveHybridOwnedBytes) -> String {{").unwrap();
    writeln!(out, "        defer {{ if let freeFn = vtable.pointee.free_bytes {{ freeFn(raw) }} }}").unwrap();
    writeln!(out, "        guard let ptr = raw.ptr, raw.len > 0 else {{ return \"\" }}").unwrap();
    writeln!(out, "        let buf = UnsafeBufferPointer(start: ptr, count: Int(raw.len))").unwrap();
    writeln!(out, "        return String(decoding: buf, as: UTF8.self)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();

    writeln!(out, "    private func takeBytes(_ raw: NveHybridOwnedBytes) -> [UInt8] {{").unwrap();
    writeln!(out, "        defer {{ if let freeFn = vtable.pointee.free_bytes {{ freeFn(raw) }} }}").unwrap();
    writeln!(out, "        guard let ptr = raw.ptr, raw.len > 0 else {{ return [] }}").unwrap();
    writeln!(out, "        let buf = UnsafeBufferPointer(start: ptr, count: Int(raw.len))").unwrap();
    writeln!(out, "        return Array(buf)").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out, "}}").unwrap();
    writeln!(out).unwrap();
}

pub(super) fn emit_swift_direct_prop_write(out: &mut String, key_lit: &str, expr: &str, ty: &HybridType) {
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
            writeln!(out, "        builder.write(\"{key_lit}\", {expr})").unwrap();
        }
        HybridType::Enum(_) => {
            writeln!(out, "        builder.write(\"{key_lit}\", enum: {expr})").unwrap();
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
                writeln!(out, "        builder.write(\"{key_lit}\", {expr})").unwrap();
            }
            HybridType::Enum(_) => {
                writeln!(out, "        builder.write(\"{key_lit}\", enum: {expr})").unwrap();
            }
            other => {
                writeln!(out, "        if let _optProp = {expr} {{").unwrap();
                writeln!(out, "            var _w = NveHybridBinWriter()").unwrap();
                emit_swift_bin_write(out, "            ", "_w", "_optProp", other);
                writeln!(out, "            builder.write(\"{key_lit}\", _w.bytes)").unwrap();
                writeln!(out, "        }} else {{").unwrap();
                writeln!(out, "            builder.writeNull(\"{key_lit}\")").unwrap();
                writeln!(out, "        }}").unwrap();
            }
        },
        other => {
            writeln!(out, "        do {{").unwrap();
            writeln!(out, "            var _w = NveHybridBinWriter()").unwrap();
            emit_swift_bin_write(out, "            ", "_w", expr, other);
            writeln!(out, "            builder.write(\"{key_lit}\", _w.bytes)").unwrap();
            writeln!(out, "        }}").unwrap();
        }
    }
}

pub(super) fn emit_swift_set_props(out: &mut String, pascal: &str, _v: &HybridViewSpec) {
    writeln!(out, "    public func setProps(_ props: {pascal}Props) throws {{").unwrap();
    writeln!(out, "        guard let h = handle, let setFn = vtable.pointee.set_props_direct else {{ throw NveHybridError.nullHandle }}").unwrap();
    writeln!(out, "        let status: Int32 = try NveDirectPropsWriter.withEncodedProps(props) {{ ptr, count in").unwrap();
    writeln!(out, "            setFn(h, ptr, count)").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "        if status != Int32(NVE_HYBRID_OK.rawValue) {{").unwrap();
    writeln!(out, "            throw NveHybridError.remoteError(\"setProps failed\")").unwrap();
    writeln!(out, "        }}").unwrap();
    writeln!(out, "    }}").unwrap();
    writeln!(out).unwrap();
}

