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
    assert!(location(&deep).2.contains("nests deeper than 128 levels"));
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
#![nve_hybrid_namespace(name = "file_level")]

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

#[nve_hybrid_namespace(name = "from_extern_fn")]
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
    // A later item's namespace wins; the file-level one is applied first.
    assert_eq!(spec.namespace.as_deref(), Some("from_extern_fn"));
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
        ("#[nve_hybrid_module]\npub trait T {".into(), ""),
        ("#[nve_hybrid_struct]\npub struct".into(), ""),
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

/// 100k-level nesting is rejected before `syn` recurses, on a thread with a 2 MiB stack.
#[test]
fn deep_nesting_is_rejected_without_overflowing_a_small_stack() {
    let deep = 100_000;
    let fields = [
        "*const ".repeat(deep) + "u8",
        "&".repeat(deep) + "u8",
        "Option<".repeat(deep) + "u8" + &">".repeat(deep),
        "(".repeat(deep) + "u8" + &")".repeat(deep),
        "[".repeat(deep) + "u8" + &"; 1]".repeat(deep),
        "fn() -> ".repeat(deep) + "u8",
    ];
    let mut sources: Vec<String> = fields
        .iter()
        .map(|ty| format!("#[nve_hybrid_struct]\npub struct S {{ pub x: {ty} }}"))
        .collect();
    sources.push(format!(
        "#[nve_hybrid_const]\npub const X: u32 = {}1;",
        "-".repeat(deep)
    ));
    sources.push(format!(
        "#[nve_hybrid_const]\npub const X: u32 = {}1;",
        "return ".repeat(deep)
    ));
    sources.push(format!(
        "#[nve_hybrid_const]\npub const X: u32 = {}1;",
        "|| ".repeat(deep)
    ));
    let worker = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(move || {
            sources
                .iter()
                .map(|s| try_location(s).map(|(_, _, m)| m))
                .collect::<Vec<_>>()
        });
    for result in worker.expect("spawn").join().expect("no stack overflow") {
        let message = result.expect("a parse error");
        assert!(
            message.contains("nests deeper than 128 levels"),
            "{message}"
        );
    }
    // Realistic nesting well under the bound still parses on the same small stack.
    let ok = std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(|| {
            parse_hybrid_rust_spec(&format!(
                "#[nve_hybrid_struct]\npub struct S {{ pub x: {}u8{} }}",
                "Option<".repeat(40),
                ">".repeat(40)
            ))
            .is_ok()
        })
        .expect("spawn");
    assert!(ok.join().expect("no stack overflow"));
}
