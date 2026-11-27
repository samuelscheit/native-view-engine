//! The `syn`-based hybrid `.rs` spec frontend: every declaration form it accepts, and where it
//! reports errors. (Output was verified byte-identical to the retired hand-written parser on these
//! and every `hybrid_codegen_tests.rs` fixture.)

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
    assert!(location(&deep).2.contains("maximum type nesting depth"));
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
