//! Swift hybrid enums, structs (C ABI + `NveBinValue` codecs), module / view protocols, and the
//! vtable clients over the static `NveHost` / `NveHostAbi` runtime.

use crate::writer::CodeWriter;

use super::codec_emitter::{
    emit_swift_bin_read, emit_swift_bin_write, emit_swift_nve_value, swift_default_value,
    swift_type,
};
use super::methods_emitter::{emit_swift_client_method, swift_method_sig};
use super::*;

pub(super) fn emit_swift_enum(out: &mut CodeWriter, e: &HybridEnumSpec) {
    writeln!(out, "public enum {}: UInt32, HybridEnum {{", e.name);
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
        // Throws when a `[T]` standing for a spec `[T; N]` holds another count (`.truncated` /
        // `.tooLarge`, like the binary codec). `self.` keeps a field named `self` from meaning
        // the receiver.
        writeln!(out, "    public func toAbi() throws -> NveHybrid{} {{", s.name);
        writeln!(out, "        var _nveRaw = NveHybrid{}()", s.name);
        for f in &s.fields {
            let field = format!("_nveRaw.{}", swift_member(&c_field_ident(&f.name)));
            let fname = swift_ident(&f.name);
            match &f.ty {
                HybridType::Enum(_) => writeln!(out, "        {field} = self.{fname}.rawValue"),
                HybridType::Struct(_) => writeln!(out, "        {field} = try self.{fname}.toAbi()"),
                HybridType::FixedArray { .. } => {
                    emit_swift_fixed_to_abi(out, &field, &format!("self.{fname}"), &f.ty);
                }
                _ => writeln!(out, "        {field} = self.{fname}"),
            }
        }
        writeln!(out, "        return _nveRaw");
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
            let field = swift_member(&c_field_ident(&f.name));
            let label = swift_label(&f.name);
            match &f.ty {
                HybridType::Enum(e) => write!(out, "{label}: try {e}.decode(raw.{field})"),
                HybridType::Struct(st) => {
                    write!(out, "{label}: try {st}.fromAbi(raw.{field})")
                }
                HybridType::FixedArray { .. } => {
                    write!(out, "{label}: {}", swift_fixed_from_abi(&format!("raw.{field}"), &f.ty))
                }
                _ => write!(out, "{label}: raw.{field}"),
            }
        }
        writeln!(out, ")");
        writeln!(out, "    }}");
    }

    // `NveBinValue` map codec (field names reach the codec only through `self.` / `_nveOut.`).
    writeln!(out);
    writeln!(out, "    public func encodeBin(into w: inout BinWriter) throws(BinError) {{");
    writeln!(out, "        let _nveMap = w.beginMap({})", s.fields.len());
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "        try w.writeMapKey(\"{key_lit}\")");
        emit_swift_bin_write(out, "        ", "w", &format!("self.{fname}"), &f.ty, 0);
    }
    writeln!(out, "        try w.end(_nveMap)");
    writeln!(out, "    }}");
    writeln!(out);

    writeln!(
        out,
        "    public static func decodeBin(from r: inout BinReader) throws -> {} {{",
        s.name
    );
    // Open the map first: malformed input throws before any default (e.g. `[T; N]`) is built.
    writeln!(out, "        let _nveCount = try r.beginMap()");
    writeln!(out, "        var _nveOut = {}()", s.name);
    writeln!(out, "        for _ in 0..<_nveCount {{");
    writeln!(out, "            switch try r.readMapKey() {{");
    for f in &s.fields {
        let fname = swift_ident(&f.name);
        let key_lit = escape_c_like_string(&f.name);
        writeln!(out, "            case \"{key_lit}\":");
        emit_swift_bin_read(out, "                ", "r", &format!("_nveOut.{fname}"), &f.ty, 0);
    }
    writeln!(out, "            default:");
    writeln!(out, "                try r.skipValue()");
    writeln!(out, "            }}");
    writeln!(out, "        }}");
    writeln!(out, "        return _nveOut");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

/// A POD `[T; N]` (nested too) in C is a homogeneous tuple of its innermost element: that
/// element's imported Swift type and the dimensions, outermost first.
fn swift_c_fixed_layout(ty: &HybridType) -> (&HybridType, String, Vec<usize>) {
    let (dims, elem) = ty.fixed_array_dims();
    let c_ty = match elem {
        HybridType::Enum(name) | HybridType::Struct(name) => format!("NveHybrid{name}"),
        other => swift_type(other),
    };
    (elem, c_ty, dims)
}

/// The flat C index of `_nveI0.._nveI{k}` in an array of `dims` (row-major).
fn swift_flat_index(dims: &[usize]) -> String {
    let mut flat = "_nveI0".to_string();
    for (k, len) in dims.iter().enumerate().skip(1) {
        flat = format!("({flat} * {len} + _nveI{k})");
    }
    flat
}

/// Copies the `[T]` (`[[T]]`, ...) `src` into the C tuple `dst` element by element, after checking
/// every level holds exactly its spec length.
fn emit_swift_fixed_to_abi(out: &mut CodeWriter, dst: &str, src: &str, ty: &HybridType) {
    let (elem, c_ty, dims) = swift_c_fixed_layout(ty);
    writeln!(out, "        try withUnsafeMutableBytes(of: &{dst}) {{ _nveBuf in");
    let mut indent = "            ".to_string();
    let mut value = src.to_string();
    for (k, len) in dims.iter().enumerate() {
        writeln!(out, "{indent}try BinError.requireCount({value}.count, exactly: {len})");
        writeln!(out, "{indent}for _nveI{k} in 0..<{len} {{");
        value = format!("{value}[_nveI{k}]");
        indent.push_str("    ");
    }
    let converted = match elem {
        HybridType::Enum(_) => format!("{value}.rawValue"),
        HybridType::Struct(_) => format!("try {value}.toAbi()"),
        _ => value,
    };
    let flat = swift_flat_index(&dims);
    writeln!(
        out,
        "{indent}_nveBuf.storeBytes(of: {converted}, toByteOffset: {flat} * MemoryLayout<{c_ty}>.stride, as: {c_ty}.self)"
    );
    for _ in &dims {
        indent.truncate(indent.len() - 4);
        writeln!(out, "{indent}}}");
    }
    writeln!(out, "        }}");
}

/// The `[T]` (`[[T]]`, ...) read element by element out of the C tuple `src`.
fn swift_fixed_from_abi(src: &str, ty: &HybridType) -> String {
    let (elem, c_ty, dims) = swift_c_fixed_layout(ty);
    let load = format!(
        "_nveBuf.load(fromByteOffset: {} * MemoryLayout<{c_ty}>.stride, as: {c_ty}.self)",
        swift_flat_index(&dims)
    );
    // Only enum and struct elements validate (`try`); scalars copy as they are.
    let (try_, mut body) = match elem {
        HybridType::Enum(e) => ("try ", format!("try {e}.decode({load})")),
        HybridType::Struct(s) => ("try ", format!("try {s}.fromAbi({load})")),
        _ => ("", load),
    };
    for (k, len) in dims.iter().enumerate().rev() {
        body = format!("{try_}(0..<{len}).map {{ _nveI{k} in {body} }}");
    }
    format!("{try_}withUnsafeBytes(of: {src}) {{ _nveBuf in {body} }}")
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

    writeln!(out, "public struct {pascal}Props: Equatable {{");
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
    let vtable = format!("NveHybrid{pascal}VTable");
    writeln!(out, "public final class {pascal}HybridClient: Hybrid{pascal}Spec {{");
    writeln!(out, "    public static let interfaceId: UInt32 = 0x{interface_id:08X}");
    writeln!(out, "    public static let schemaHash: UInt64 = 0x{schema_hash:016X}");
    writeln!(out);
    writeln!(out, "    private let vtable: UnsafePointer<{vtable}>");
    writeln!(out, "    private let handle: NveAppHandle");
    writeln!(out);
    writeln!(
        out,
        "    public static func resolve(getInterface: HybridInterfaceResolver?) throws -> {pascal}HybridClient {{"
    );
    writeln!(out, "        guard let getInterface else {{ throw HybridError.nullInterfaceResolver }}");
    writeln!(
        out,
        "        guard let raw = getInterface(interfaceId) else {{ throw HybridError.interfaceNotFound(interfaceId) }}"
    );
    writeln!(
        out,
        "        return try {pascal}HybridClient(vtable: raw.assumingMemoryBound(to: {vtable}.self))"
    );
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    public init(vtable: UnsafePointer<{vtable}>) throws {{");
    writeln!(
        out,
        "        try HybridError.checkAbiVersion(expected: {}HybridFile.abiVersion, actual: vtable.pointee.abi_version)",
        spec.stem_pascal()
    );
    writeln!(
        out,
        "        try HybridError.checkSchema(expected: Key64(Self.schemaHash), actual: Key64(vtable.pointee.schema_hash))"
    );
    writeln!(
        out,
        "        guard let create = vtable.pointee.create else {{ throw HybridError.nullFunction(\"create\") }}"
    );
    writeln!(out, "        guard let handle = create() else {{ throw HybridError.nullHandle }}");
    writeln!(out, "        self.vtable = vtable");
    writeln!(out, "        self.handle = handle");
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    deinit {{");
    writeln!(out, "        self.vtable.pointee.destroy?(self.handle)");
    writeln!(out, "    }}");
    writeln!(out);

    if let Some(v) = view {
        emit_swift_set_props(out, pascal, v);
    }
    for method in methods {
        emit_swift_client_method(out, method, spec);
    }

    // Owned buffers the peer returns: copied out, then released through its own allocator.
    writeln!(out, "    private func takeBytes(_ raw: NveHybridOwnedBytes) -> [UInt8] {{");
    writeln!(out, "        defer {{ self.vtable.pointee.free_bytes?(raw) }}");
    writeln!(out, "        guard let ptr = raw.ptr, raw.len > 0 else {{ return [] }}");
    writeln!(out, "        return Array(UnsafeBufferPointer(start: ptr, count: Int(raw.len)))");
    writeln!(out, "    }}");
    writeln!(out);
    writeln!(out, "    private func takeString(_ raw: NveHybridOwnedBytes) -> String {{");
    writeln!(out, "        String(decoding: self.takeBytes(raw), as: UTF8.self)");
    writeln!(out, "    }}");
    writeln!(out, "}}");
    writeln!(out);
}

/// `setProps`: every prop becomes one `NveMapEntry` whose `NveValue` lives in an `NveValueTree`
/// kept alive across `set_props_direct` (keys are borrowed `StaticString`s).
fn emit_swift_set_props(out: &mut CodeWriter, pascal: &str, v: &HybridViewSpec) {
    writeln!(out, "    public func setProps(_ props: {pascal}Props) throws {{");
    writeln!(
        out,
        "        guard let _nveFn = self.vtable.pointee.set_props_direct else {{ throw HybridError.nullFunction(\"set_props_direct\") }}"
    );
    writeln!(out, "        let _nveTree = NveValueTree()");
    for (idx, p) in v.props.iter().enumerate() {
        let pname = swift_ident(&p.name);
        emit_swift_nve_value(
            out,
            "        ",
            "_nveTree",
            &format!("_nveProp{idx}"),
            &format!("props.{pname}"),
            &p.ty,
        );
    }
    writeln!(out, "        let _nveEntries: [NveMapEntry] = [");
    for (idx, p) in v.props.iter().enumerate() {
        let key_lit = escape_c_like_string(&p.name);
        writeln!(out, "            _nveTree.entry(\"{key_lit}\", _nveProp{idx}),");
    }
    writeln!(out, "        ]");
    writeln!(out, "        let _nveStatus = withExtendedLifetime(_nveTree) {{");
    writeln!(
        out,
        "            _nveEntries.withUnsafeBufferPointer {{ _nveFn(self.handle, $0.baseAddress, UInt32($0.count)) }}"
    );
    writeln!(out, "        }}");
    writeln!(out, "        try HybridStatus.check(_nveStatus)");
    writeln!(out, "    }}");
    writeln!(out);
}
