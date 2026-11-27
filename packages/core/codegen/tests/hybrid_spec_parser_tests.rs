//! The `syn`-based hybrid `.rs` spec frontend: every declaration form it accepts (pinned by a
//! golden snapshot), every Rust form it rejects (each with its location), and its nesting bound.

use nve_codegen::{CodegenError, HybridDefaultValue, HybridType, parse_hybrid_rust_spec};

const EVERY_FORM: &str = r#"
//! Inner docs are ignored.
use nve_abi::NveValue;

#[nve_hybrid_namespace(name = "acme_media")]
/// Max frames.
#[nve_hybrid_const]
pub const MAX_FRAMES: u32 = 0x40;

#[nve_hybrid_const]
pub const LIMIT: u64 = 1000;

/// Opaque handle.
#[nve_hybrid_alias]
pub type CameraHandle = u64;

/// Modes.
#[nve_hybrid_enum(c_prefix = "NVE_MODE_", dart_fallback = "Off")]
#[repr(u8)]
pub enum Mode {
    /// Off doc.
    Off,
    On = 7,
    Auto,
}

#[nve_hybrid_struct(size = 16, size_64 = 24, align = 8)]
#[repr(C)]
pub struct Packet {
    /// The id.
    #[offset(0)]
    #[offset64(0)]
    pub id: u32,
    #[offset(8)]
    pub data: *const u8,
    pub len: usize,
    pub arr: [f32; 4],
}

#[nve_hybrid_union]
#[repr(C, align(8))]
pub union Num { pub i: i64, pub f: f64 }

/// Unannotated declarations are ignored.
pub struct Ignored { pub x: u8 }

#[nve_hybrid_c_fn]
pub extern "C" fn nve_cam_open(handle: CameraHandle, mode: Mode, out: *mut Packet) -> i32;

/// Camera.
/// Second line.
#[nve_hybrid_module(name = "Camera", interface_id = 4097, c_name = "NveCameraApi")]
pub trait CameraSpec {
    /// Opens.
    fn open(&mut self, handle: CameraHandle) -> Result<Packet, String>;
    #[optional]
    fn torch(&self, on: bool) -> Option<Vec<Mode>>;
    fn data(&self, buf: &[u8], tags: &[String]) -> Vec<u8>;
    fn dynamic(&self, v: NveValue) -> NveValue;
    fn nothing(&self) -> ();
}

#[nve_hybrid_view(interface_id = 4098)]
pub trait PreviewSpec {
    #[prop(default = Mode::Auto)] fn mode(&self) -> Mode;
    #[prop(default = -1.5)] fn bias(&self) -> f64;
    #[prop(default = -9223372036854775808)] fn offset(&self) -> i64;
    #[prop(default = 3)] fn count(&self) -> u32;
    #[prop(default = "hi \"there\"\n")] fn label(&self) -> String;
    #[prop(default = None)] fn poster(&self) -> Option<String>;
    #[prop(default = true)] fn live(&self) -> bool;
    #[prop] fn plain(&self) -> i32;
    #[event] fn on_frame(&self, ts: u64, size: u32);
    #[method] fn snap(&self) -> Result<Vec<u8>, String>;
    fn implicit_method(&mut self, x: f32);
}
"#;

#[test]
fn parses_every_declaration_form() {
    let spec = parse_hybrid_rust_spec(EVERY_FORM).expect("every form parses");
    assert_eq!(spec.namespace.as_deref(), Some("acme_media"));

    let [max, limit] = spec.consts.as_slice() else {
        panic!("two consts")
    };
    assert_eq!(
        (max.name.as_str(), max.value, max.hex, max.doc.as_deref()),
        ("MAX_FRAMES", 64, true, Some("Max frames."))
    );
    assert_eq!(
        (limit.value, limit.hex, &limit.ty),
        (1000, false, &HybridType::U64)
    );
    assert_eq!(spec.aliases[0].target, HybridType::U64);

    let mode = &spec.enums[0];
    assert_eq!(
        (
            mode.repr.as_deref(),
            mode.c_prefix.as_deref(),
            mode.dart_fallback.as_deref()
        ),
        (Some("u8"), Some("NVE_MODE_"), Some("Off"))
    );
    let values: Vec<_> = mode
        .variants
        .iter()
        .map(|v| (v.name.as_str(), v.value))
        .collect();
    assert_eq!(values, [("Off", 0), ("On", 7), ("Auto", 8)]);
    assert_eq!(mode.variants[0].doc.as_deref(), Some("Off doc."));

    assert_eq!(spec.structs.len(), 2, "the unannotated struct is ignored");
    let packet = &spec.structs[0];
    assert_eq!(
        (packet.expected_size, packet.expected_size_64, packet.align),
        (Some(16), Some(24), Some(8))
    );
    assert_eq!(
        (
            packet.fields[0].offset,
            packet.fields[0].offset_64,
            packet.fields[1].offset
        ),
        (Some(0), Some(0), Some(8))
    );
    assert_eq!(
        packet.fields[1].ty,
        HybridType::ConstPtr(Box::new(HybridType::U8))
    );
    assert_eq!(
        packet.fields[3].ty,
        HybridType::FixedArray {
            elem: Box::new(HybridType::F32),
            len: 4
        }
    );
    let num = &spec.structs[1];
    assert!(num.is_union);
    assert_eq!(num.align, Some(8), "align from #[repr(C, align(8))]");

    let open = &spec.c_functions[0];
    assert!(!open.has_self);
    assert_eq!(open.params[0].ty, HybridType::Handle("CameraHandle".into()));
    assert_eq!(open.params[1].ty, HybridType::Enum("Mode".into()));
    assert_eq!(
        open.params[2].ty,
        HybridType::MutPtr(Box::new(HybridType::Struct("Packet".into())))
    );

    let camera = &spec.modules[0];
    assert_eq!(
        (
            camera.name.as_str(),
            camera.explicit_interface_id,
            camera.c_name.as_deref()
        ),
        ("Camera", Some(4097), Some("NveCameraApi"))
    );
    assert_eq!(camera.doc.as_deref(), Some("Camera.\nSecond line."));
    let [open_m, torch, data, dynamic, nothing] = camera.methods.as_slice() else {
        panic!("five methods")
    };
    assert!(open_m.has_self && !open_m.optional && torch.optional);
    assert_eq!(
        torch.return_type,
        HybridType::Optional(Box::new(HybridType::Vec(Box::new(HybridType::Enum(
            "Mode".into()
        )))))
    );
    assert_eq!(data.params[0].ty, HybridType::Bytes);
    assert_eq!(
        data.params[1].ty,
        HybridType::Vec(Box::new(HybridType::String))
    );
    assert_eq!(data.return_type, HybridType::Bytes);
    assert_eq!(dynamic.return_type, HybridType::Value);
    assert_eq!(nothing.return_type, HybridType::Void);

    let preview = &spec.views[0];
    assert_eq!(
        (preview.name.as_str(), preview.explicit_interface_id),
        ("Preview", Some(4098)),
        "name defaults to the trait minus `Spec`"
    );
    let defaults: Vec<_> = preview
        .props
        .iter()
        .map(|p| p.default_value.clone())
        .collect();
    assert_eq!(
        defaults,
        [
            Some(HybridDefaultValue::EnumVariant {
                enum_name: "Mode".into(),
                variant: "Auto".into()
            }),
            Some(HybridDefaultValue::Float(-1.5)),
            Some(HybridDefaultValue::Int(i64::MIN)),
            Some(HybridDefaultValue::UInt(3)),
            Some(HybridDefaultValue::String("hi \"there\"\n".into())),
            Some(HybridDefaultValue::None),
            Some(HybridDefaultValue::Bool(true)),
            None,
        ]
    );
    assert_eq!(preview.events[0].params.len(), 2);
    assert_eq!(
        preview.methods.len(),
        2,
        "#[method] and unannotated fns are both methods"
    );
}

/// `(line, col, message)` of the parse error `source` must raise, or the unexpected outcome.
fn try_location(source: &str) -> Result<(usize, usize, String), String> {
    match parse_hybrid_rust_spec(source) {
        Err(CodegenError::Parse { line, col, message }) => Ok((line, col, message)),
        other => Err(format!("expected a parse error, got {other:?}")),
    }
}

#[test]
fn reports_parse_errors_at_their_source_location() {
    let location = |source: &str| try_location(source).expect("parse error");
    let (line, col, message) =
        location("#[nve_hybrid_struct]\npub struct S {\n    pub x: HashMap<u8, u8>,\n}\n");
    assert_eq!((line, col), (3, 12), "{message}");
    assert!(message.contains("unsupported generic type 'HashMap<...>'"));

    let (line, _, message) = location("\n\nimpl Foo {}\n");
    assert_eq!(line, 3, "{message}");
    assert!(message.contains("unexpected item at top level"));

    let (line, _, message) =
        location("#[nve_hybrid_module]\npub trait T {\n    #[prop] fn x(&self) -> i32;\n}\n");
    assert_eq!(line, 3);
    assert!(message.contains("only valid inside #[nve_hybrid_view]"));

    let (_, _, message) = location("#[nve_hybrid_c_fn]\npub extern \"C\" fn f(&self) -> i32;\n");
    assert!(message.contains("cannot take `self`"), "{message}");

    let deep = format!(
        "#[nve_hybrid_module] pub trait T {{ fn f(&self) -> {}u8; }}",
        "*const ".repeat(200)
    );
    assert!(
        location(&deep)
            .2
            .contains("maximum type nesting depth exceeded")
    );
    assert!(
        location("#[nve_hybrid_enum] pub enum E { A = 4294967296 }")
            .2
            .contains("exceeds u32::MAX")
    );
    assert!(
        location("#[nve_hybrid_view] pub trait V { #[prop(default = 1 + 2)] fn x(&self) -> i32; }")
            .2
            .contains("unsupported #[prop(default = ...)] value")
    );
}

/// `EVERY_FORM` as parsed, pinned. Regenerate with `NVE_UPDATE_GOLDEN=1` after a deliberate change.
#[test]
fn every_form_matches_its_golden_snapshot() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/golden/hybrid_spec_every_form.txt");
    let actual = format!(
        "{:#?}\n",
        parse_hybrid_rust_spec(EVERY_FORM).expect("every form parses")
    );
    if std::env::var_os("NVE_UPDATE_GOLDEN").is_some() {
        std::fs::write(&path, &actual).expect("write golden snapshot");
    }
    let golden = std::fs::read_to_string(&path)
        .expect("read golden snapshot (NVE_UPDATE_GOLDEN=1 writes it)");
    assert_eq!(
        actual,
        golden.replace("\r\n", "\n"),
        "parser output drifted from {}",
        path.display()
    );
}

const EXTRAS: &str = r#"
#![nve_hybrid_namespace(name = "acme")]

/** Block doc. */
#[nve_hybrid_enum]
#[repr(C)]
#[repr(u32)]
pub enum Edge { Low = 7, High = 4294967295 }

#[nve_hybrid_struct]
#[repr(C)]
#[repr(align(16))]
pub struct Aligned { pub bytes: &'static mut [u8] }

extern "C" {
    /// In a block.
    #[nve_hybrid_c_fn]
    pub fn nve_block_fn(a: u32) -> Edge;
}

#[nve_hybrid_namespace(name = "acme")]
#[nve_hybrid_c_fn]
pub unsafe extern "C" fn nve_free_fn(e: *mut Aligned);

#[nve::nve_hybrid_module(name = "Path", interface_id = 4294967295)]
pub trait PathSpec {
    fn ping(&self);
}
"#;

#[test]
fn extern_blocks_path_attributes_multi_repr_and_file_input() {
    let spec = parse_hybrid_rust_spec(EXTRAS).expect("extras parse");
    // The file-level namespace and the same one repeated on an `extern fn` agree.
    assert_eq!(spec.namespace.as_deref(), Some("acme"));
    let edge = &spec.enums[0];
    assert_eq!(edge.doc.as_deref(), Some("Block doc."));
    assert_eq!(
        edge.repr.as_deref(),
        Some("u32"),
        "every #[repr(...)] is read"
    );
    assert_eq!(
        edge.variants[1].value,
        u32::MAX,
        "u32::MAX is a valid last discriminant"
    );
    assert_eq!(spec.structs[0].align, Some(16));
    assert_eq!(spec.structs[0].fields[0].ty, HybridType::Bytes);
    let names: Vec<_> = spec.c_functions.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["nve_block_fn", "nve_free_fn"]);
    assert_eq!(
        spec.c_functions[0].return_type,
        HybridType::Enum("Edge".into())
    );
    assert_eq!(spec.c_functions[0].doc.as_deref(), Some("In a block."));
    assert_eq!(spec.modules[0].explicit_interface_id, Some(u32::MAX));

    let dir = std::env::temp_dir().join(format!("nve_hybrid_spec_file_{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let file = dir.join("extras.nve.rs");
    std::fs::write(&file, format!("\u{feff}{EXTRAS}")).expect("write spec");
    assert_eq!(
        nve_codegen::parse_hybrid_rust_spec_file(&file).expect("file parses"),
        spec,
        "BOM is skipped"
    );
    assert!(matches!(
        nve_codegen::parse_hybrid_rust_spec_file(&dir.join("missing.nve.rs")),
        Err(CodegenError::Io { .. })
    ));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Every Rust form the IR cannot represent fails with a located parse error, never silently.
#[test]
fn unsupported_forms_are_located_errors() {
    let module = |body: &str| format!("#[nve_hybrid_module]\npub trait T {{\n{body}\n}}\n");
    let view = |body: &str| format!("#[nve_hybrid_view]\npub trait V {{\n{body}\n}}\n");
    let field = |ty: &str| format!("#[nve_hybrid_struct]\npub struct S {{\n    pub x: {ty},\n}}\n");
    let cases: Vec<(String, &str)> = vec![
        (
            "#[nve_hybrid_c_fn]\npub extern \"C\" fn log(fmt: *const u8, ...);".into(),
            "variadic",
        ),
        (
            "#[nve_hybrid_c_fn]\npub fn f();".into(),
            "must be declared `extern \"C\"`",
        ),
        (
            "#[nve_hybrid_c_fn]\npub extern \"system\" fn f();".into(),
            "only the `extern \"C\"` ABI",
        ),
        (
            "extern \"system\" {\n    #[nve_hybrid_c_fn] fn f();\n}".into(),
            "only the `extern \"C\"` ABI",
        ),
        (
            "extern \"C\" {\n    static X: u32;\n}".into(),
            "may only declare functions",
        ),
        (
            "pub extern \"C\" fn stray();".into(),
            "without #[nve_hybrid_c_fn]",
        ),
        ("type Opaque;".into(), "unsupported item"),
        (module("    fn f<U>(&self);"), "generic parameters"),
        (
            module("    fn f(&self) where Self: Sized;"),
            "`where` clauses",
        ),
        (module("    async fn f(&self);"), "`async fn`"),
        (module("    const fn f(&self);"), "`const fn`"),
        (
            module("    extern \"C\" fn f(&self);"),
            "cannot declare an `extern` ABI",
        ),
        (module("    fn f(&self) {}"), "no default body"),
        (module("    type Out;"), "may only declare `fn` members"),
        (
            "#[nve_hybrid_module]\npub trait T<U> {}".into(),
            "generic parameters",
        ),
        (
            "#[nve_hybrid_module]\npub trait T: Clone {}".into(),
            "supertraits",
        ),
        (
            "#[nve_hybrid_enum]\npub enum E { A(u32) }".into(),
            "unit variants",
        ),
        (
            "#[nve_hybrid_enum]\npub enum E { B { x: u8 } }".into(),
            "unit variants",
        ),
        (
            "#[nve_hybrid_enum]\npub enum E { A = 4294967295, B }".into(),
            "overflows u32",
        ),
        (
            "#[nve_hybrid_enum]\n#[repr(u8)]\n#[repr(u32)]\npub enum E { A }".into(),
            "conflicting integer reprs",
        ),
        (
            "#[nve_hybrid_struct]\npub struct S<T> { pub x: T }".into(),
            "generic parameters",
        ),
        (
            "#[nve_hybrid_union]\npub union U<T> { pub x: T }".into(),
            "generic parameters",
        ),
        (
            "#[nve_hybrid_alias]\npub type A<T> = u32;".into(),
            "generic parameters",
        ),
        (
            "#[nve_hybrid_struct]\npub struct S(u32);".into(),
            "named fields",
        ),
        (field("Vec<>"), "empty `<>`"),
        (field("Foo<u8>::Bar"), "last path segment"),
        (field("Option"), "`Option` needs its type arguments"),
        (field("Result"), "`Result` needs its type arguments"),
        (field("u32<u8>"), "unsupported generic type"),
        (
            "#[nve_hybrid_struct]\npub struct S {\n    pub r#type: u32,\n}".into(),
            "raw identifier `r#type`",
        ),
        (
            "#[nve_hybrid_struct]\n#[repr(C, packed)]\npub struct S { pub x: u8 }".into(),
            "packed layouts",
        ),
        (
            "#[nve_hybrid_struct]\n#[repr(C, align(1 +))]\npub struct S { pub x: u8 }".into(),
            "align(N)",
        ),
        (
            "#[nve_hybrid_struct]\n#[repr(align(3))]\npub struct S { pub x: u8 }".into(),
            "power of two",
        ),
        (
            "#[nve_hybrid_struct]\n#[repr(transparent)]\npub struct S { pub x: u8 }".into(),
            "unsupported `#[repr(transparent)]`",
        ),
        (
            "#[nve_hybrid_struct = 16]\npub struct S { pub x: u8 }".into(),
            "not `#[attr = value]`",
        ),
        (
            "#[nve_hybrid_struct(size)]\npub struct S { pub x: u8 }".into(),
            "`size` needs a value",
        ),
        (
            "#[nve_hybrid_struct(sizee = 4)]\npub struct S { pub x: u8 }".into(),
            "unknown key `sizee`",
        ),
        (
            "#[nve_hybrid_struct(size = 1, size = 2)]\npub struct S { pub x: u8 }".into(),
            "given twice",
        ),
        (
            "#[nve_hybrid_module(interface_id)]\npub trait T {}".into(),
            "`interface_id` needs a value",
        ),
        (
            "#[nve_hybrid_module(interface_id = 4294967296)]\npub trait T {}".into(),
            "exceeds u32::MAX",
        ),
        (
            "#[nve_hybrid_module(name = 5)]\npub trait T {}".into(),
            "expected a string for `name`",
        ),
        (
            "#[nve_hybrid_view(c_name = \"X\")]\npub trait V {}".into(),
            "unknown key `c_name`",
        ),
        (
            view("    #[prop(default)] fn x(&self) -> i32;"),
            "`default` needs a value",
        ),
        (
            view("    #[prop(default = 1e400)] fn x(&self) -> f64;"),
            "must be finite",
        ),
        (
            view("    #[prop(default = Mode::<u8>::Auto)] fn x(&self) -> u32;"),
            "without `::` prefix or generics",
        ),
        (
            view("    #[prop(default = ::None)] fn x(&self) -> Option<u32>;"),
            "without `::` prefix or generics",
        ),
        (
            "#[nve_hybrid_namespace]\n#[nve_hybrid_const]\npub const X: u32 = 1;".into(),
            "needs `name = \"...\"`",
        ),
        (
            "#[nve_hybrid_struc]\npub struct S { pub x: u8 }".into(),
            "unknown hybrid spec attribute",
        ),
        ("#[nve_hybrid_module]\npub trait T {".into(), "unbalanced delimiter"),
        ("#[nve_hybrid_struct]\npub struct".into(), "unexpected end of input"),
        // Markers: wrong item kind, duplicates, file scope, namespaces.
        ("#[nve_hybrid_struct]\npub enum E { A }".into(), "`#[nve_hybrid_struct]` is not valid on an enum"),
        ("#[nve_hybrid_c_fn]\npub const X: u32 = 1;".into(), "`#[nve_hybrid_c_fn]` is not valid on a const"),
        ("#[nve_hybrid_struct]\n#[nve_hybrid_struct]\npub struct S { pub x: u8 }".into(), "is given twice"),
        ("#![nve_hybrid_struct]\n#[nve_hybrid_const]\npub const X: u32 = 1;".into(), "not valid on the file"),
        ("#![nve_hybrid_namespace(name = \"a\")]\n#[nve_hybrid_namespace(name = \"b\")]\n#[nve_hybrid_const]\npub const X: u32 = 1;".into(), "namespace `b` conflicts with `a`"),
        // Member and field attributes.
        (view("    #[porp] fn x(&self) -> i32;"), "unexpected attribute `#[porp]`"),
        (view("    #[optional] fn x(&self);"), "#[optional] is only valid inside #[nve_hybrid_module]"),
        (view("    #[event(bubbles)] fn on_x(&self);"), "this attribute takes no arguments"),
        (view("    #[prop] #[prop] fn x(&self) -> i32;"), "is given twice"),
        (module("    #[optional(x)] fn f(&self);"), "this attribute takes no arguments"),
        (module("    unsafe fn f(&self);"), "cannot be `unsafe fn`"),
        (module("    fn f(self);"), "`&self` or `&mut self`"),
        (module("    fn f(self: Box<Self>);"), "`&self` or `&mut self`"),
        (module("    fn f(&self, mut a: u32);"), "no `ref`, `mut`, or `@`"),
        (module("    fn f(&self, ref a: u32);"), "no `ref`, `mut`, or `@`"),
        ("#[nve_hybrid_struct]\npub struct S {\n    #[offset(0)] #[offset(0)] pub x: u8,\n}".into(), "is given twice"),
        ("#[nve_hybrid_struct]\npub struct S {\n    #[ofset(0)] pub x: u8,\n}".into(), "unexpected attribute `#[ofset]`"),
        ("#[nve_hybrid_struct]\npub struct S {\n    #[offset(-4)] pub x: u8,\n}".into(), "non-negative integer"),
        ("#[nve_hybrid_enum]\npub enum E {\n    #[prop] A,\n}".into(), "unexpected attribute `#[prop]` on an enum variant"),
        // Argument syntax.
        ("#[nve_hybrid_const(x = 1)]\npub const X: u32 = 1;".into(), "this attribute takes no arguments"),
        ("#[nve_hybrid_struct(a::b = 1)]\npub struct S { pub x: u8 }".into(), "plain identifiers"),
        // `repr` / `align` consistency and ranges.
        ("#[nve_hybrid_enum]\n#[repr(align(8))]\npub enum E { A }".into(), "not valid on an enum"),
        ("#[nve_hybrid_struct]\n#[repr(u8)]\npub struct S { pub x: u8 }".into(), "not valid on a struct or union"),
        ("#[nve_hybrid_struct(align = 8)]\n#[repr(C, align(8))]\npub struct S { pub x: u8 }".into(), "give the alignment once"),
        ("#[nve_hybrid_struct]\n#[repr(align(8))]\n#[repr(align(16))]\npub struct S { pub x: u8 }".into(), "`align(N)` is given twice"),
        ("#[nve_hybrid_enum]\n#[repr(u8)]\npub enum E { A = 255, B }".into(), "does not fit `#[repr(u8)]`"),
        ("extern \"system\" {}".into(), "only the `extern \"C\"` ABI"),
        // `cfg` is never evaluated, so it is rejected wherever it appears.
        ("#[cfg(test)]\n#[nve_hybrid_const]\npub const X: u32 = 1;".into(), "`#[cfg]` is not evaluated"),
        ("#![cfg_attr(x, y)]\n#[nve_hybrid_const]\npub const X: u32 = 1;".into(), "`#[cfg_attr]` is not evaluated"),
        (module("    #[cfg(x)] fn f(&self);"), "`#[cfg]` is not evaluated"),
        ("#[nve_hybrid_struct]\npub struct S {\n    #[cfg(x)] pub x: u8,\n}".into(), "`#[cfg]` is not evaluated"),
        ("#[nve_hybrid_enum]\npub enum E {\n    #[cfg_attr(x, y)] A,\n}".into(), "`#[cfg_attr]` is not evaluated"),
        // Only the namespace may be an inner attribute.
        ("#[nve_hybrid_module]\npub trait T {\n    #![nve_hybrid_view]\n}".into(), "must be an outer attribute"),
        // Member attributes are single identifiers; tool attributes stay harmless.
        (view("    #[doc::prop] fn x(&self) -> i32;"), "unexpected attribute `#[doc::prop]`"),
        (view("    #[a::prop] fn x(&self) -> i32;"), "unexpected attribute `#[a::prop]`"),
        (view("    #[event = 1] fn on_x(&self);"), "this attribute takes no arguments"),
        // Tool attributes spelled like member attributes are not member attributes.
        (view("    #[clippy::prop] fn x(&self) -> i32;"), "unexpected attribute `#[clippy::prop]`"),
        (view("    #[rustfmt::event] fn on_x(&self);"), "unexpected attribute `#[rustfmt::event]`"),
        (module("    #[clippy::optional] fn f(&self);"), "unexpected attribute `#[clippy::optional]`"),
        ("#[nve_hybrid_struct]\npub struct S {\n    #[clippy::offset(0)] pub x: u8,\n}".into(), "unexpected attribute `#[clippy::offset]`"),
        // The first offending attribute in source order is reported.
        (module("    #[cfg(test)] #[prop] fn x(&self) -> i32;"), "`#[cfg]` is not evaluated"),
        (module("    #[prop] #[cfg(test)] fn x(&self) -> i32;"), "only valid inside #[nve_hybrid_view]"),
        (module("    #[method] fn f(&self);"), "#[method] is only valid inside #[nve_hybrid_view]"),
        // Parameters and receivers carry no attributes `cfg` could drop.
        (module("    fn f(&self, #[cfg(x)] a: u32);"), "`#[cfg]` is not evaluated"),
        (module("    fn f(#[cfg_attr(x, y)] &self);"), "`#[cfg_attr]` is not evaluated"),
        (module("    fn f(&self, #[prop] a: u32);"), "unexpected attribute `#[prop]` on a parameter"),
        (module("    fn f(&'static self);"), "no lifetime"),
        (module("    fn f(&'a mut self);"), "no lifetime"),
        (module("    fn f(&self, a @ _: u32);"), "no `ref`, `mut`, or `@`"),
        ("#[nve_hybrid_enum]\n#[repr(u8)]\n#[repr(u8)]\npub enum E { A }".into(), "`repr(u8)` is given twice"),
        ("#[nve_hybrid_enum]\n#[repr(u8)]\n#[repr(u16)]\npub enum E { A }".into(), "conflicting integer reprs"),
    ];
    for (source, expected) in &cases {
        let (line, col, message) =
            try_location(source).unwrap_or_else(|e| panic!("{source}\n=> {e}"));
        assert!(
            message.contains(expected),
            "{source}\n=> {line}:{col} {message}"
        );
        assert!(
            line >= 1 && col >= 1,
            "{source}\n=> unlocated error {line}:{col} {message}"
        );
    }
    // An unclosed brace and an early end of input are located at the open brace / end of input.
    assert_eq!(
        try_location("#[nve_hybrid_module]\npub trait T {")
            .expect("error")
            .0,
        2
    );
    assert_eq!(
        try_location("#[nve_hybrid_struct]\npub struct")
            .expect("error")
            .0,
        2
    );
}

/// Runs `f` on a thread with a 2 MiB stack (the default for spawned threads): the parser never
/// recurses on its caller's stack, so a small one must be enough.
fn on_small_stack<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("no stack overflow")
}

/// Every prefix / infix / postfix / keyword / group form, 100k deep, is rejected by the path bound
/// before `syn` runs, whatever the caller's stack (here 2 MiB).
#[test]
fn deep_nesting_of_every_form_is_rejected_on_a_small_stack() {
    let deep = 100_000;
    let rep = |s: &str| s.repeat(deep);
    let types = [
        rep("*const ") + "u8",
        rep("&") + "u8",
        rep("Option<") + "u8" + &rep(">"),
        rep("(") + "u8" + &rep(")"),
        rep("[") + "u8" + &rep("; 1]"),
        rep("fn() -> ") + "u8",
        rep("extern \"C\" fn() -> ") + "u8",
    ];
    let exprs = [
        rep("-") + "1",
        rep("!") + "1",
        rep("return ") + "1",
        rep("|| ") + "1",
        rep("|x,| ") + "1",
        rep("a = ") + "1",
        rep("a += ") + "1",
        "1".to_owned() + &rep(" + 1"),
        "a".to_owned() + &rep(".b"),
        "f".to_owned() + &rep("()"),
        "a".to_owned() + &rep("[0]"),
        "a".to_owned() + &rep("?"),
        "1".to_owned() + &rep(" as u8"),
        rep("if a { 1 } else ") + "{ 1 }",
        rep("match a { _ => ") + "1" + &rep(" }"),
        rep("loop { break ") + "1" + &rep(" }"),
        rep("while a { ") + &rep(" }"),
        rep("for x in a { ") + &rep(" }"),
        rep("{ let a = ") + "1" + &rep("; a }"),
        rep("become ") + "1",
        rep("..") + "1",
        rep("1..=") + "1",
        rep("x @ ") + "1",
        rep("move || ") + "1",
    ];
    let mut sources: Vec<String> = types
        .iter()
        .map(|ty| format!("#[nve_hybrid_struct]\npub struct S {{ pub x: {ty} }}"))
        .collect();
    sources.extend(
        exprs
            .iter()
            .map(|e| format!("#[nve_hybrid_const]\npub const X: u32 = {e};")),
    );
    let results = on_small_stack(move || {
        sources
            .iter()
            .map(|s| try_location(s).map(|(_, _, m)| m))
            .collect::<Vec<_>>()
    });
    for (i, result) in results.into_iter().enumerate() {
        let message = result.unwrap_or_else(|e| panic!("form #{i}: {e}"));
        assert!(message.contains("nesting too deep"), "form #{i}: {message}");
    }
}

/// Realistic inputs never trip the bound: forms nested just under it, long runs of `//!` docs
/// and of flat consts (a segment ends at `;`), and hundreds of items.
#[test]
fn nesting_under_the_bound_and_long_flat_specs_still_parse() {
    let ok = |source: String| {
        on_small_stack(move || {
            parse_hybrid_rust_spec(&source)
                .map(|_| ())
                .map_err(|e| e.to_string())
        })
    };
    ok(format!(
        "#[nve_hybrid_struct]\npub struct S {{ pub x: {}u8{} }}",
        "Vec<".repeat(40),
        ">".repeat(40)
    ))
    .expect("40 Vec<");
    ok(format!(
        "#[nve_hybrid_struct]\npub struct S {{ pub x: {}u8 }}",
        "*const ".repeat(60)
    ))
    .expect("60 *const");
    let not_nesting = |e: String| assert!(!e.contains("nesting too deep"), "{e}");
    not_nesting(
        ok(format!(
            "#[nve_hybrid_struct]\npub struct S {{ pub x: [u8; {}1{}] }}",
            "(".repeat(100),
            ")".repeat(100)
        ))
        .expect_err("a parenthesized array length is not a literal"),
    );
    let docs = "//! doc line\n".repeat(150);
    ok(format!("{docs}#[nve_hybrid_const]\npub const X: u32 = 1;")).expect("150 //! lines");
    let consts: String = (0..70)
        .map(|i| format!("#[nve_hybrid_const]\npub const F{i}: u32 = 1 << {i};\n"))
        .collect();
    not_nesting(ok(consts).expect_err("`1 << i` is not an unsigned literal"));
    // The heaviest recursive forms just under the bound: accepted by the pre-check (the value
    // itself is then rejected), and `syn` parses, spans, and drops them on the 2 MiB stack.
    // `(form, tokens per repetition)`: the bound counts tokens.
    for (form, tokens) in [
        ("-", 1),
        ("!", 1),
        ("return ", 1),
        ("|| ", 2),
        ("&", 1),
        ("a = ", 2),
        ("move || ", 3),
    ] {
        not_nesting(
            ok(format!(
                "#[nve_hybrid_const]\npub const X: u32 = {}1;",
                form.repeat(115 / tokens)
            ))
            .expect_err("not an unsigned literal"),
        );
    }
    not_nesting(
        ok(format!(
            "#[nve_hybrid_const]\npub const X: u32 = 1{};",
            " + 1".repeat(55)
        ))
        .expect_err("not a literal"),
    );
    let structs: String = (0..300)
        .map(|i| format!("/// Doc.\n#[nve_hybrid_struct]\npub struct S{i} {{ pub x: u32 }}\n"))
        .collect();
    ok(structs).expect("300 items");
    let attrs: String = (0..200).map(|i| format!("#[doc = \"{i}\"]\n")).collect();
    ok(format!("{attrs}#[nve_hybrid_const]\npub const X: u32 = 1;")).expect("200 outer attributes");
}

#[test]
fn block_docs_and_prop_default_ranges() {
    let spec = parse_hybrid_rust_spec(
        "/**\n * First line.\n * Second line.\n */\n#[nve_hybrid_const]\npub const X: u32 = 1;",
    )
    .expect("block doc");
    assert_eq!(
        spec.consts[0].doc.as_deref(),
        Some("First line.\nSecond line.")
    );
    for (ty, default) in [
        ("u8", "300"),
        ("i32", "4294967295"),
        ("f32", "1e300"),
        ("u32", "-1"),
    ] {
        let source = format!(
            "#[nve_hybrid_view]\npub trait V {{ #[prop(default = {default})] fn x(&self) -> {ty}; }}"
        );
        assert!(
            matches!(
                parse_hybrid_rust_spec(&source),
                Err(CodegenError::Validation(_))
            ),
            "default {default} must not fit {ty}"
        );
    }
    assert!(
        parse_hybrid_rust_spec(
            "#[nve_hybrid_view]\npub trait V { #[prop(default = 255)] fn x(&self) -> u8; }"
        )
        .is_ok()
    );
}

/// Largest path bound the frontend accepts (`MAX_PATH_TOKENS` in `frontend/hybrid_spec/stack.rs`).
const MAX_PATH_TOKENS: usize = 2048;

/// `source` is valid Rust: `syn` parses all of it (on a generous stack), so the frontend's own
/// parse of it recursed through the whole nest too.
fn assert_valid_rust(name: &str, source: &str) {
    let src = source.to_owned();
    let valid = std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(move || syn::parse_file(&src).map(drop).map_err(|e| e.to_string()))
        .expect("spawn syn checker")
        .join()
        .expect("syn checker");
    assert!(valid.is_ok(), "{name} is not valid Rust: {valid:?}");
}

/// The frontend's outcome for `source` (on the caller's 2 MiB stack): accepted, or a located
/// parse error that is not the depth cap (never a thread / validation failure).
fn parse_without_depth_error(name: &str, source: &str) {
    let src = source.to_owned();
    let result = on_small_stack(move || parse_hybrid_rust_spec(&src));
    if let Err(err) = result {
        assert!(
            matches!(err, CodegenError::Parse { .. }),
            "{name}: expected Ok or a parse error, got {err}"
        );
        if let CodegenError::Parse { message, .. } = err {
            assert!(!message.contains("nesting too deep"), "{name}: {message}");
        }
    }
}

/// Every recursive form, nested to about 1,950 path tokens (just under the cap), parses, lowers,
/// and drops on the parse thread's stack: the stack size is sound for the worst forms (`&&..&u8`,
/// `((u8))`, `{{1}}`, `&&&1`, `let &&&x`, ...), whatever their grammar. Forms that cross the
/// count restarts (`;`, `,`, file-level items) nest that deep *per* statement / element / item.
#[test]
fn every_recursive_form_parses_on_its_sized_stack() {
    // (form, path tokens per repetition, prefix repeated, suffix repeated, innermost)
    let types: [(&str, usize, &str, &str, &str); 14] = [
        ("*const", 2, "*const ", "", "u8"),
        ("&", 1, "&", "", "u8"),
        ("&'struct", 3, "&'struct ", "", "u8"),
        ("&'union mut", 4, "&'union mut ", "", "u8"),
        ("Option<", 3, "Option<", ">", "u8"),
        ("(", 1, "(", ")", "u8"),
        ("(u8,", 1, "(u8, ", ")", "u8"),
        ("[; 1]", 1, "[", "; 1]", "u8"),
        ("[&&&; 1]", 4, "[&&& ", "; 1]", "u8"),
        ("fn() ->", 4, "fn() -> ", "", "u8"),
        ("fn(", 2, "fn(", ")", "u8"),
        ("extern C fn", 6, "extern \"C\" fn() -> ", "", "u8"),
        ("for<'a, 'b> fn", 14, "for<'a, 'b> fn() -> ", "", "u8"),
        ("dyn Fn() ->", 5, "dyn Fn() -> ", "", "u8"),
    ];
    let exprs: [(&str, usize, &str, &str, &str); 43] = [
        ("-", 1, "-", "", "1"),
        ("!", 1, "!", "", "1"),
        ("*", 1, "*", "", "1"),
        ("&", 1, "&", "", "1"),
        ("&mut", 2, "&mut ", "", "1"),
        ("{", 1, "{", "}", "1"),
        ("(", 1, "(", ")", "1"),
        ("[", 1, "[", "]", "1"),
        ("return", 1, "return ", "", "1"),
        ("||", 2, "|| ", "", "1"),
        ("|x,|", 4, "|x,| ", "", "1"),
        ("|| |x,|", 6, "|| |x,| ", "", "1"),
        ("|&_,|-", 6, "|&_,|- ", "", "1"),
        ("move ||", 3, "move || ", "", "1"),
        ("|| {", 3, "|| { ", "}", "1"),
        ("a =", 2, "a = ", "", "1"),
        ("union =", 2, "union = ", "", "1"),
        ("union.union =", 4, "union.union = ", "", "1"),
        ("f::<u8, u8> =", 9, "f::<u8, u8> = ", "", "1"),
        ("a +=", 3, "a += ", "", "1"),
        ("a.b", 2, "", ".b", "a"),
        ("()", 1, "", "()", "f"),
        ("f(", 2, "f(", ")", "1"),
        ("f(1, 2,", 2, "f(1, 2, ", ")", "1"),
        ("[1,", 1, "[1, ", "]", "1"),
        ("[0]", 1, "", "[0]", "a"),
        ("?", 1, "", "?", "a"),
        (".await", 2, "", ".await", "x"),
        ("as u8", 2, "", " as u8", "1"),
        ("if else", 4, "if a { 1 } else ", "", "{ 1 }"),
        ("if union", 5, "if union {} else ", "", "{}"),
        ("match", 6, "match a { _ => ", " }", "1"),
        ("loop break", 3, "loop { break ", " }", "1"),
        ("'a: loop {", 5, "'a: loop { ", "}", ""),
        ("while", 3, "while a { ", " }", ""),
        ("for x in a {", 5, "for x in a { ", " }", ""),
        ("{ let a =", 4, "{ let a = ", "; a }", "1"),
        ("unsafe", 2, "unsafe { ", "}", ""),
        ("const {", 2, "const { ", " }", "1"),
        ("S { x:", 4, "S { x: ", " }", "1"),
        ("a::<", 5, "a::<", ">", "u8"),
        ("become", 1, "become ", "", "1"),
        ("yield", 1, "yield ", "", "1"),
    ];
    // Inside a statement / pattern: `let &&&x`, `let [[[x]]]`, `let (((x,),),)`.
    let pats: [(&str, usize, &str, &str, &str); 3] = [
        ("let &", 1, "&", "", "x"),
        ("let [", 1, "[", "]", "x"),
        ("let (,)", 1, "(", ",)", "x"),
    ];
    let nest = |(_, per, prefix, suffix, inner): (&str, usize, &str, &str, &str)| {
        let n = (MAX_PATH_TOKENS - 100) / per;
        format!("{}{inner}{}", prefix.repeat(n), suffix.repeat(n))
    };
    let mut sources: Vec<(String, String)> = Vec::new();
    for form in types {
        sources.push((
            form.0.into(),
            format!(
                "#[nve_hybrid_struct]\npub struct S {{ pub x: {} }}",
                nest(form)
            ),
        ));
    }
    for form in exprs {
        sources.push((
            form.0.into(),
            format!("#[nve_hybrid_const]\npub const X: u32 = {};", nest(form)),
        ));
    }
    for form in pats {
        sources.push((
            form.0.into(),
            format!(
                "#[nve_hybrid_const]\npub const X: u32 = {{ let {} = 1; 1 }};",
                nest(form)
            ),
        ));
    }
    sources.push((
        "mod m {".into(),
        nest(("mod m {", 3, "mod m { ", "}", "")),
    ));
    // Restart-crossing forms: each statement / element / item nests to the cap on its own.
    let deep_neg = "-".repeat(MAX_PATH_TOKENS - 100);
    sources.push((
        "3 statements".into(),
        format!(
            "#[nve_hybrid_const]\npub const X: u32 = {{ {deep_neg}x; {deep_neg}x; {deep_neg}x }};"
        ),
    ));
    sources.push(("let-else".into(), format!("#[nve_hybrid_const]\npub const X: u32 = {{ let Some(x) = {deep_neg}a else {{ return }}; {deep_neg}x }};")));
    sources.push((
        "3 tuple elements".into(),
        format!("#[nve_hybrid_const]\npub const X: u32 = ({deep_neg}1, {deep_neg}1, {deep_neg}1);"),
    ));
    let deep_ref = "&".repeat(MAX_PATH_TOKENS - 100);
    sources.push((
        "3 items".into(),
        (0..3)
            .map(|i| format!("#[nve_hybrid_struct]\npub struct S{i} {{ pub x: {deep_ref}u8 }}\n"))
            .collect(),
    ));
    sources.push(("3 fields".into(), format!("#[nve_hybrid_struct]\npub struct S {{ pub a: [{deep_ref}u8; 1], pub b: {deep_ref}u8, pub c: {deep_ref}u8 }}")));
    for (name, source) in &sources {
        assert_valid_rust(name, source);
        parse_without_depth_error(name, source);
    }
}

/// The path bound is exact at its cap: at the cap the spec reaches `syn`, one token more is a
/// located error before it. (`#[..]` restarts; `pub const X: u32 = -..-1;` is 8 + k tokens.)
#[test]
fn the_path_bound_cap_is_exact() {
    let spec = |k: usize| {
        format!(
            "#[nve_hybrid_const]\npub const X: u32 = {}1;",
            "-".repeat(k)
        )
    };
    let at_cap = spec(MAX_PATH_TOKENS - 8);
    assert_valid_rust("at cap", &at_cap);
    match parse_hybrid_rust_spec(&at_cap) {
        Err(CodegenError::Parse { message, .. }) => {
            assert!(!message.contains("nesting too deep"), "{message}");
        }
        other => panic!("expected the lowering's parse error, got {other:?}"),
    }
    match parse_hybrid_rust_spec(&spec(MAX_PATH_TOKENS - 7)) {
        Err(CodegenError::Parse { line, message, .. }) => {
            assert_eq!(line, 2);
            assert!(message.contains("nesting too deep"), "{message}");
        }
        other => panic!("expected the depth error, got {other:?}"),
    }
}

/// Long flat specs never trip the bound: items, members, fields, variants, and doc lines restart
/// the count.
#[test]
fn long_flat_specs_parse() {
    let blocks: String = (0..300)
        .map(|i| {
            format!(
                "extern \"C\" {{\n    #[nve_hybrid_c_fn]\n    pub fn f{i}(a: u32) -> u32;\n}}\n"
            )
        })
        .collect();
    assert_eq!(
        parse_hybrid_rust_spec(&blocks)
            .expect("300 extern blocks")
            .c_functions
            .len(),
        300
    );
    let consts: String = (0..3000)
        .map(|i| {
            format!("/// Doc {i}.\n/// More.\n#[nve_hybrid_const]\npub const C{i}: u32 = {i};\n")
        })
        .collect();
    assert_eq!(
        parse_hybrid_rust_spec(&consts)
            .expect("3000 consts")
            .consts
            .len(),
        3000
    );
    let members: String = (0..3000)
        .map(|i| format!("    /// Doc.\n    fn m{i}(&self, a: u32) -> u32;\n"))
        .collect();
    let spec = parse_hybrid_rust_spec(&format!(
        "#[nve_hybrid_module]\npub trait T {{\n{members}}}"
    ))
    .expect("3000 members");
    assert_eq!(spec.modules[0].methods.len(), 3000);
    let fields: String = (0..1000)
        .map(|i| format!("    /// Doc.\n    pub f{i}: Option<Vec<u8>>,\n"))
        .collect();
    let spec = parse_hybrid_rust_spec(&format!(
        "#[nve_hybrid_struct]\npub struct S {{\n{fields}}}"
    ))
    .expect("1000 fields");
    assert_eq!(spec.structs[0].fields.len(), 1000);
    let variants: String = (0..1000).map(|i| format!("    V{i} = {i},\n")).collect();
    let spec = parse_hybrid_rust_spec(&format!("#[nve_hybrid_enum]\npub enum E {{\n{variants}}}"))
        .expect("1000 variants");
    assert_eq!(spec.enums[0].variants.len(), 1000);
    let header = "//! Header line.\n".repeat(2000);
    parse_hybrid_rust_spec(&format!(
        "{header}#[nve_hybrid_const]\npub const X: u32 = 1;"
    ))
    .expect("2000 //! lines");
}

/// `///` Markdown survives verbatim; only `/** */` line decoration is stripped.
#[test]
fn line_docs_keep_markdown_and_block_docs_keep_indentation() {
    let spec = parse_hybrid_rust_spec(
        "/// * bullet\n/// **bold** and *em*\n#[nve_hybrid_const]\npub const X: u32 = 1;\n\
         /**\n * Title.\n *\n *     code();\n * * nested bullet\n */\n#[nve_hybrid_const]\npub const Y: u32 = 2;",
    )
    .expect("docs");
    assert_eq!(
        spec.consts[0].doc.as_deref(),
        Some("* bullet\n**bold** and *em*")
    );
    assert_eq!(
        spec.consts[1].doc.as_deref(),
        Some("Title.\n\n    code();\n* nested bullet")
    );
    // A block doc that starts with an indented code line keeps that indentation.
    let spec = parse_hybrid_rust_spec(
        "/**\n *     indented();\n */\n#[nve_hybrid_const]\npub const Z: u32 = 3;",
    )
    .expect("indented block doc");
    assert_eq!(spec.consts[0].doc.as_deref(), Some("    indented();"));
}

/// `usize` defaults fit `u32` (32-bit hosts); negative defaults never fit unsigned props.
#[test]
fn unsigned_and_usize_prop_defaults_are_range_checked() {
    let prop = |ty: &str, default: &str| {
        parse_hybrid_rust_spec(&format!(
            "#[nve_hybrid_view]\npub trait V {{ #[prop(default = {default})] fn x(&self) -> {ty}; }}"
        ))
    };
    for (ty, default) in [
        ("usize", "4294967295"),
        ("usize", "0"),
        ("u8", "255"),
        ("u16", "65535"),
    ] {
        prop(ty, default).unwrap_or_else(|e| panic!("{ty} = {default}: {e}"));
    }
    for (ty, default) in [
        ("usize", "4294967296"),
        ("usize", "-1"),
        ("u8", "-1"),
        ("u16", "-1"),
        ("u8", "256"),
    ] {
        assert!(
            matches!(prop(ty, default), Err(CodegenError::Validation(_))),
            "{ty} = {default} must be rejected"
        );
    }
}

/// `Option<Option<T>>` / `Option<Value>`: `None` and `Some(null)` would both encode as `Null`.
#[test]
fn ambiguous_nested_options_are_rejected() {
    for ty in [
        "Option<Option<u32>>",
        "Option<NveValue>",
        "Vec<Option<Option<String>>>",
    ] {
        let source = format!("#[nve_hybrid_module]\npub trait T {{ fn f(&self, a: {ty}); }}");
        match parse_hybrid_rust_spec(&source) {
            Err(CodegenError::Validation(message)) => {
                assert!(message.contains("ambiguous on the wire"), "{ty}: {message}");
            }
            other => panic!("{ty}: expected a validation error, got {other:?}"),
        }
    }
    parse_hybrid_rust_spec(
        "#[nve_hybrid_module]\npub trait T { fn f(&self, a: Option<Vec<Option<u32>>>); }",
    )
    .expect("an option inside a sequence is unambiguous");
}
