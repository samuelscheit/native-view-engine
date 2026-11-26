//! Proves that NVE's Option 2 zero-serialization schemas (`NveDrawOp`/`NveDrawListView` and
//! `NveThemeSnapshot`/`NveSensorSample`/`NveCameraFrameDescriptor`/host-service vtables) can be
//! authored purely as typed Rust `HybridFileSpec` and `RelocSchema` values — colocated in their
//! owning packages with ZERO centralized `.nve.rs` text parsing — and compile cleanly under
//! `clang -std=c11 -Wall -Wextra -Werror`, `clang++ -std=c++20 -Wall -Wextra -Werror`, and
//! `rustc --edition=2024 -D warnings`.

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{check_c11_syntax, compile_and_run_cpp20, tool_available};

use nve_codegen::{
    generate_hybrid_target, generate_reloc_all, HybridEnumSpec, HybridEnumVariant,
    HybridFieldSpec, HybridFileSpec, HybridMethodSpec, HybridModuleSpec, HybridParamSpec,
    HybridStructSpec, HybridTarget, HybridType, RelocEnum, RelocEnumRepr, RelocEnumVariant,
    RelocField, RelocSchema, RelocStruct, RelocType,
};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .and_then(Path::parent)
        .expect("workspace root 3 levels above CARGO_MANIFEST_DIR")
        .to_path_buf()
}

fn temp_test_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "nve_option2_schema_test_{tag}_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp dir");
    dir
}

fn field(name: &str, ty: HybridType, offset: Option<usize>, offset_64: Option<usize>) -> HybridFieldSpec {
    HybridFieldSpec {
        name: name.to_string(),
        ty,
        doc: None,
        offset,
        offset_64,
    }
}

fn variant(name: &str, value: u32) -> HybridEnumVariant {
    HybridEnumVariant {
        name: name.to_string(),
        value,
        doc: None,
    }
}

fn param(name: &str, ty: HybridType) -> HybridParamSpec {
    HybridParamSpec {
        name: name.to_string(),
        ty,
    }
}

fn c_fn(name: &str, params: Vec<HybridParamSpec>, return_type: HybridType) -> HybridMethodSpec {
    HybridMethodSpec {
        name: name.to_string(),
        params,
        return_type,
        doc: None,
        has_self: false,
        optional: false,
    }
}

fn method(name: &str, params: Vec<HybridParamSpec>, return_type: HybridType) -> HybridMethodSpec {
    HybridMethodSpec {
        name: name.to_string(),
        params,
        return_type,
        doc: None,
        has_self: true,
        optional: false,
    }
}

/// Option 2 flat 64-byte cache-line `NveDrawOp` + `NveDrawOpKind` + `NveDrawListView` schema
/// built directly via Rust `HybridFileSpec` (ready to replace `NVE_DRAW1` byte-stream codec).
fn draw_ops_hybrid_spec() -> HybridFileSpec {
    HybridFileSpec {
        namespace: Some("NveDrawOps".to_string()),
        enums: vec![HybridEnumSpec {
            name: "NveDrawOpKind".to_string(),
            repr: Some("u32".to_string()),
            c_prefix: Some("NVE_DRAW_OP_".to_string()),
            variants: vec![
                variant("FillRect", 0),
                variant("StrokeRect", 1),
                variant("FillRoundedRect", 2),
                variant("StrokeRoundedRect", 3),
                variant("FillCircle", 4),
                variant("StrokeCircle", 5),
                variant("DrawLine", 6),
                variant("StrokePolyline", 7),
                variant("FillPath", 8),
                variant("DrawText", 9),
                variant("PushClip", 10),
                variant("PopClip", 11),
                variant("DrawImage", 12),
            ],
            ..Default::default()
        }],
        structs: vec![
            HybridStructSpec {
                name: "NveDrawOp".to_string(),
                align: Some(64),
                expected_size: Some(64),
                fields: vec![
                    field("kind", HybridType::Enum("NveDrawOpKind".to_string()), Some(0), None),
                    field("flags", HybridType::U32, Some(4), None),
                    field("color_rgba", HybridType::U32, Some(8), None),
                    field("secondary_rgba", HybridType::U32, Some(12), None),
                    field("x", HybridType::F32, Some(16), None),
                    field("y", HybridType::F32, Some(20), None),
                    field("width", HybridType::F32, Some(24), None),
                    field("height", HybridType::F32, Some(28), None),
                    field("param0", HybridType::F32, Some(32), None),
                    field("param1", HybridType::F32, Some(36), None),
                    field("param2", HybridType::F32, Some(40), None),
                    field("param3", HybridType::F32, Some(44), None),
                    field("data_offset", HybridType::U32, Some(48), None),
                    field("data_len", HybridType::U32, Some(52), None),
                    field("text_offset", HybridType::U32, Some(56), None),
                    field("text_len", HybridType::U32, Some(60), None),
                ],
                ..Default::default()
            },
            HybridStructSpec {
                name: "NveDrawListView".to_string(),
                expected_size_64: Some(48),
                fields: vec![
                    field(
                        "ops",
                        HybridType::ConstPtr(Box::new(HybridType::Struct("NveDrawOp".to_string()))),
                        Some(0),
                        None,
                    ),
                    field("op_count", HybridType::U32, None, Some(8)),
                    field(
                        "vertices",
                        HybridType::ConstPtr(Box::new(HybridType::F32)),
                        None,
                        Some(16),
                    ),
                    field("vertex_count", HybridType::U32, None, Some(24)),
                    field(
                        "text_buf",
                        HybridType::ConstPtr(Box::new(HybridType::U8)),
                        None,
                        Some(32),
                    ),
                    field("text_len", HybridType::U32, None, Some(40)),
                ],
                ..Default::default()
            },
        ],
        // Default statically linked direct `extern "C"` call (zero vtable, LTO-inlinable).
        c_functions: vec![c_fn(
            "nve_draw_submit_list",
            vec![param(
                "view",
                HybridType::ConstPtr(Box::new(HybridType::Struct(
                    "NveDrawListView".to_string(),
                ))),
            )],
            HybridType::I32,
        )],
        ..Default::default()
    }
}

/// Companion `RelocSchema` representation of `NveDrawOp` (64-byte pure POD) for mmap/IPC slabs.
fn draw_ops_reloc_schema() -> RelocSchema {
    RelocSchema {
        name: "NveDrawSlab".to_string(),
        magic: *b"NVEDRAW2",
        root: "DrawSlabRoot".to_string(),
        enums: vec![RelocEnum {
            name: "NveDrawOpKind".to_string(),
            repr: RelocEnumRepr::U32,
            variants: vec![
                RelocEnumVariant::new("FillRect", 0),
                RelocEnumVariant::new("StrokeRect", 1),
                RelocEnumVariant::new("FillRoundedRect", 2),
                RelocEnumVariant::new("StrokeRoundedRect", 3),
                RelocEnumVariant::new("FillCircle", 4),
                RelocEnumVariant::new("StrokeCircle", 5),
                RelocEnumVariant::new("DrawLine", 6),
                RelocEnumVariant::new("StrokePolyline", 7),
                RelocEnumVariant::new("FillPath", 8),
                RelocEnumVariant::new("DrawText", 9),
                RelocEnumVariant::new("PushClip", 10),
                RelocEnumVariant::new("PopClip", 11),
                RelocEnumVariant::new("DrawImage", 12),
            ],
            doc: None,
        }],
        structs: vec![
            RelocStruct {
                name: "NveDrawOp".to_string(),
                align: Some(64),
                fields: vec![
                    RelocField::new("kind", RelocType::Enum("NveDrawOpKind".to_string())),
                    RelocField::new("flags", RelocType::U32),
                    RelocField::new("color_rgba", RelocType::U32),
                    RelocField::new("secondary_rgba", RelocType::U32),
                    RelocField::new("x", RelocType::F32),
                    RelocField::new("y", RelocType::F32),
                    RelocField::new("width", RelocType::F32),
                    RelocField::new("height", RelocType::F32),
                    RelocField::new("param0", RelocType::F32),
                    RelocField::new("param1", RelocType::F32),
                    RelocField::new("param2", RelocType::F32),
                    RelocField::new("param3", RelocType::F32),
                    RelocField::new("data_offset", RelocType::U32),
                    RelocField::new("data_len", RelocType::U32),
                    RelocField::new("text_offset", RelocType::U32),
                    RelocField::new("text_len", RelocType::U32),
                ],
                doc: None,
            },
            RelocStruct {
                name: "DrawSlabRoot".to_string(),
                align: None,
                fields: vec![
                    RelocField::new(
                        "ops",
                        RelocType::RelocSlice(Box::new(RelocType::Struct("NveDrawOp".to_string()))),
                    ),
                    RelocField::new(
                        "vertices",
                        RelocType::RelocSlice(Box::new(RelocType::F32)),
                    ),
                    RelocField::new("text_buf", RelocType::RelocStr),
                ],
                doc: None,
            },
        ],
    }
}

/// Option 2 typed `#[repr(C)]` POD structs & vtables replacing `NVT2`/`NVL1`/`NVH1`/`BinWriter`
/// across `NVE_IFACE_HOST = 5`, constructed directly via Rust `HybridFileSpec`.
fn host_services_hybrid_spec() -> HybridFileSpec {
    HybridFileSpec {
        namespace: Some("NveHostServices".to_string()),
        enums: vec![
            HybridEnumSpec {
                name: "NveColorScheme".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![
                    variant("Unspecified", 0),
                    variant("Light", 1),
                    variant("Dark", 2),
                ],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveContrastLevel".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![variant("Standard", 0), variant("High", 1)],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveTextDirection".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![variant("Ltr", 0), variant("Rtl", 1)],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveImpactStyle".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![
                    variant("Light", 1),
                    variant("Medium", 2),
                    variant("Heavy", 3),
                    variant("Soft", 4),
                    variant("Rigid", 5),
                ],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveNotificationHaptic".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![
                    variant("Success", 1),
                    variant("Warning", 2),
                    variant("Error", 3),
                ],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveSensorKind".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![
                    variant("Accelerometer", 0),
                    variant("Gyroscope", 1),
                    variant("Magnetometer", 2),
                    variant("Barometer", 3),
                    variant("AmbientLight", 4),
                ],
                ..Default::default()
            },
            HybridEnumSpec {
                name: "NveCameraPixelFormat".to_string(),
                repr: Some("u8".to_string()),
                variants: vec![
                    variant("Nv12", 0),
                    variant("Bgra8888", 1),
                    variant("Rgba8888", 2),
                    variant("Yuv420p", 3),
                ],
                ..Default::default()
            },
        ],
        structs: vec![
            // 16-byte #[repr(C)] NveThemeSnapshot (replaces NVT2)
            HybridStructSpec {
                name: "NveThemeSnapshot".to_string(),
                expected_size: Some(16),
                fields: vec![
                    field("color_scheme", HybridType::U8, Some(0), None),
                    field("contrast", HybridType::U8, Some(1), None),
                    field("flags", HybridType::U8, Some(2), None),
                    field("_pad0", HybridType::U8, Some(3), None),
                    field("accent_rgba", HybridType::U32, Some(4), None),
                    field("font_scale_x1000", HybridType::U16, Some(8), None),
                    field("scale_factor_x100", HybridType::U16, Some(10), None),
                    field("reserved", HybridType::U32, Some(12), None),
                ],
                ..Default::default()
            },
            // Locale snapshot (replaces NVL1)
            HybridStructSpec {
                name: "NveLocaleSnapshot".to_string(),
                fields: vec![
                    field("tag", HybridType::String, None, None),
                    field("language", HybridType::String, None, None),
                    field("region", HybridType::String, None, None),
                    field(
                        "text_direction",
                        HybridType::Enum("NveTextDirection".to_string()),
                        None,
                        None,
                    ),
                    field("uses_24h_clock", HybridType::Bool, None, None),
                    field("timezone", HybridType::String, None, None),
                ],
                ..Default::default()
            },
            // 24-byte #[repr(C)] NveSensorSample (matches packages/framework/sdk/src/sensors.rs)
            HybridStructSpec {
                name: "NveSensorSample".to_string(),
                expected_size: Some(24),
                fields: vec![
                    field("x", HybridType::F32, Some(0), None),
                    field("y", HybridType::F32, Some(4), None),
                    field("z", HybridType::F32, Some(8), None),
                    field("_pad", HybridType::U32, Some(12), None),
                    field("timestamp_ns", HybridType::U64, Some(16), None),
                ],
                ..Default::default()
            },
            // 40-byte #[repr(C)] NveCameraFrameDescriptor (matches NVCF wire layout byte-for-byte)
            HybridStructSpec {
                name: "NveCameraFrameDescriptor".to_string(),
                expected_size: Some(40),
                fields: vec![
                    field(
                        "magic",
                        HybridType::FixedArray {
                            elem: Box::new(HybridType::U8),
                            len: 4,
                        },
                        Some(0),
                        None,
                    ),
                    field("version", HybridType::U8, Some(4), None),
                    field("pixel_format", HybridType::U8, Some(5), None),
                    field("rotation_deg", HybridType::U16, Some(6), None),
                    field("width", HybridType::U32, Some(8), None),
                    field("height", HybridType::U32, Some(12), None),
                    field("stride_bytes", HybridType::U32, Some(16), None),
                    field("reserved", HybridType::U32, Some(20), None),
                    field("timestamp_ns", HybridType::U64, Some(24), None),
                    field("native_handle_id", HybridType::U64, Some(32), None),
                ],
                ..Default::default()
            },
            HybridStructSpec {
                name: "NveStorageOpenInfo".to_string(),
                expected_size: Some(32),
                fields: vec![
                    field("handle", HybridType::U64, Some(0), None),
                    field("key_count", HybridType::U64, Some(8), None),
                    field("log_bytes", HybridType::U64, Some(16), None),
                    field("recovered_torn_tail", HybridType::Bool, Some(24), None),
                    field(
                        "_pad",
                        HybridType::FixedArray {
                            elem: Box::new(HybridType::U8),
                            len: 7,
                        },
                        Some(25),
                        None,
                    ),
                ],
                ..Default::default()
            },
            HybridStructSpec {
                name: "NveCompactStats".to_string(),
                expected_size: Some(24),
                fields: vec![
                    field("live_keys", HybridType::U64, Some(0), None),
                    field("bytes_before", HybridType::U64, Some(8), None),
                    field("bytes_after", HybridType::U64, Some(16), None),
                ],
                ..Default::default()
            },
            HybridStructSpec {
                name: "NveStorageEntry".to_string(),
                fields: vec![
                    field("key", HybridType::Bytes, None, None),
                    field("value", HybridType::Bytes, None, None),
                ],
                ..Default::default()
            },
        ],
        // Default statically linked direct `extern "C"` API (zero vtable, LTO-inlinable).
        c_functions: vec![
            c_fn(
                "nve_clipboard_write_text",
                vec![param("text", HybridType::String)],
                HybridType::I32,
            ),
            c_fn(
                "nve_clipboard_read_text",
                vec![
                    param("out_buf", HybridType::MutPtr(Box::new(HybridType::U8))),
                    param("out_cap", HybridType::Usize),
                    param("out_len", HybridType::MutPtr(Box::new(HybridType::Usize))),
                ],
                HybridType::I32,
            ),
            c_fn("nve_clipboard_has_text", vec![], HybridType::Bool),
            c_fn("nve_clipboard_clear", vec![], HybridType::I32),
            c_fn(
                "nve_system_env_get_theme",
                vec![param(
                    "out_theme",
                    HybridType::MutPtr(Box::new(HybridType::Struct(
                        "NveThemeSnapshot".to_string(),
                    ))),
                )],
                HybridType::I32,
            ),
            c_fn(
                "nve_haptics_impact",
                vec![param("style", HybridType::Enum("NveImpactStyle".to_string()))],
                HybridType::I32,
            ),
            c_fn(
                "nve_haptics_notification",
                vec![param(
                    "kind",
                    HybridType::Enum("NveNotificationHaptic".to_string()),
                )],
                HybridType::I32,
            ),
            c_fn("nve_haptics_selection", vec![], HybridType::I32),
            c_fn(
                "nve_storage_open",
                vec![
                    param("namespace_name", HybridType::String),
                    param(
                        "out_info",
                        HybridType::MutPtr(Box::new(HybridType::Struct(
                            "NveStorageOpenInfo".to_string(),
                        ))),
                    ),
                ],
                HybridType::I32,
            ),
            c_fn(
                "nve_storage_put",
                vec![
                    param("namespace_name", HybridType::String),
                    param("key", HybridType::Bytes),
                    param("value", HybridType::Bytes),
                ],
                HybridType::I32,
            ),
            c_fn(
                "nve_storage_delete",
                vec![
                    param("namespace_name", HybridType::String),
                    param("key", HybridType::Bytes),
                    param("out_deleted", HybridType::MutPtr(Box::new(HybridType::Bool))),
                ],
                HybridType::I32,
            ),
            c_fn(
                "nve_storage_compact",
                vec![
                    param("namespace_name", HybridType::String),
                    param(
                        "out_stats",
                        HybridType::MutPtr(Box::new(HybridType::Struct(
                            "NveCompactStats".to_string(),
                        ))),
                    ),
                ],
                HybridType::I32,
            ),
            c_fn(
                "nve_sensors_is_available",
                vec![param("kind", HybridType::Enum("NveSensorKind".to_string()))],
                HybridType::Bool,
            ),
            c_fn(
                "nve_sensors_latest_sample",
                vec![
                    param("kind", HybridType::Enum("NveSensorKind".to_string())),
                    param(
                        "out_sample",
                        HybridType::MutPtr(Box::new(HybridType::Struct(
                            "NveSensorSample".to_string(),
                        ))),
                    ),
                ],
                HybridType::I32,
            ),
            c_fn(
                "nve_camera_latest_frame",
                vec![param(
                    "out_desc",
                    HybridType::MutPtr(Box::new(HybridType::Struct(
                        "NveCameraFrameDescriptor".to_string(),
                    ))),
                )],
                HybridType::I32,
            ),
        ],
        // Optional dynamic `dlopen`/`LoadLibrary` `cdylib` vtable surfaces (for hot-reload plugins only).
        modules: vec![
            HybridModuleSpec {
                name: "Clipboard".to_string(),
                trait_name: "NveClipboardVtable".to_string(),
                c_name: Some("NveClipboardVtable".to_string()),
                methods: vec![
                    method(
                        "read_text",
                        vec![],
                        HybridType::Result {
                            ok: Box::new(HybridType::Optional(Box::new(HybridType::String))),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "write_text",
                        vec![param("text", HybridType::String)],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method("has_text", vec![], HybridType::Bool),
                    method(
                        "clear",
                        vec![],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                ],
                ..Default::default()
            },
            HybridModuleSpec {
                name: "SystemEnv".to_string(),
                trait_name: "NveSystemEnvVtable".to_string(),
                c_name: Some("NveSystemEnvVtable".to_string()),
                methods: vec![
                    method(
                        "get_theme",
                        vec![],
                        HybridType::Struct("NveThemeSnapshot".to_string()),
                    ),
                    method(
                        "get_locale",
                        vec![],
                        HybridType::Struct("NveLocaleSnapshot".to_string()),
                    ),
                ],
                ..Default::default()
            },
            HybridModuleSpec {
                name: "Haptics".to_string(),
                trait_name: "NveHapticsVtable".to_string(),
                c_name: Some("NveHapticsVtable".to_string()),
                methods: vec![
                    method(
                        "impact",
                        vec![param("style", HybridType::Enum("NveImpactStyle".to_string()))],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "notification",
                        vec![param(
                            "kind",
                            HybridType::Enum("NveNotificationHaptic".to_string()),
                        )],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "selection",
                        vec![],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                ],
                ..Default::default()
            },
            HybridModuleSpec {
                name: "Storage".to_string(),
                trait_name: "NveStorageVtable".to_string(),
                c_name: Some("NveStorageVtable".to_string()),
                methods: vec![
                    method(
                        "open",
                        vec![param("namespace_name", HybridType::String)],
                        HybridType::Result {
                            ok: Box::new(HybridType::Struct("NveStorageOpenInfo".to_string())),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "get",
                        vec![
                            param("namespace_name", HybridType::String),
                            param("key", HybridType::Bytes),
                        ],
                        HybridType::Result {
                            ok: Box::new(HybridType::Optional(Box::new(HybridType::Bytes))),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "put",
                        vec![
                            param("namespace_name", HybridType::String),
                            param("key", HybridType::Bytes),
                            param("value", HybridType::Bytes),
                        ],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "delete",
                        vec![
                            param("namespace_name", HybridType::String),
                            param("key", HybridType::Bytes),
                        ],
                        HybridType::Result {
                            ok: Box::new(HybridType::Bool),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "scan_prefix",
                        vec![
                            param("namespace_name", HybridType::String),
                            param("prefix", HybridType::Bytes),
                            param("limit", HybridType::U32),
                        ],
                        HybridType::Result {
                            ok: Box::new(HybridType::Vec(Box::new(HybridType::Struct(
                                "NveStorageEntry".to_string(),
                            )))),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "compact",
                        vec![param("namespace_name", HybridType::String)],
                        HybridType::Result {
                            ok: Box::new(HybridType::Struct("NveCompactStats".to_string())),
                            err: Box::new(HybridType::String),
                        },
                    ),
                ],
                ..Default::default()
            },
            HybridModuleSpec {
                name: "Sensors".to_string(),
                trait_name: "NveSensorsVtable".to_string(),
                c_name: Some("NveSensorsVtable".to_string()),
                methods: vec![
                    method(
                        "is_available",
                        vec![param("kind", HybridType::Enum("NveSensorKind".to_string()))],
                        HybridType::Bool,
                    ),
                    method(
                        "subscribe",
                        vec![
                            param("kind", HybridType::Enum("NveSensorKind".to_string())),
                            param("rate_hz", HybridType::U16),
                        ],
                        HybridType::Result {
                            ok: Box::new(HybridType::U64),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "unsubscribe",
                        vec![param("subscription_id", HybridType::U64)],
                        HybridType::Result {
                            ok: Box::new(HybridType::Void),
                            err: Box::new(HybridType::String),
                        },
                    ),
                    method(
                        "latest_sample",
                        vec![param("kind", HybridType::Enum("NveSensorKind".to_string()))],
                        HybridType::Optional(Box::new(HybridType::Struct(
                            "NveSensorSample".to_string(),
                        ))),
                    ),
                ],
                ..Default::default()
            },
        ],
        ..Default::default()
    }
}

#[test]
fn option2_draw_ops_and_host_services_validate_and_hash_deterministically() {
    let draw_spec = draw_ops_hybrid_spec();
    draw_spec.validate().expect("NveDrawOps spec must validate");
    assert_eq!(draw_spec.file_schema_hash(), draw_spec.file_schema_hash());
    assert_ne!(draw_spec.file_schema_hash(), 0);
    assert!(draw_spec.find_struct("NveDrawOp").unwrap().is_pod(&draw_spec));

    let draw_reloc = draw_ops_reloc_schema();
    let reloc_layouts = draw_reloc.compute_layouts().expect("NveDrawSlab reloc layout");
    assert_eq!(reloc_layouts["NveDrawOp"].size, 64);
    assert_eq!(reloc_layouts["NveDrawOp"].align, 64);
    assert!(reloc_layouts["NveDrawOp"].is_pure_pod);

    let host_spec = host_services_hybrid_spec();
    host_spec
        .validate()
        .expect("NveHostServices spec must validate");
    assert_eq!(host_spec.file_schema_hash(), host_spec.file_schema_hash());
    assert_ne!(host_spec.file_schema_hash(), draw_spec.file_schema_hash());
    assert!(host_spec
        .find_struct("NveThemeSnapshot")
        .unwrap()
        .is_pod(&host_spec));
    assert!(host_spec
        .find_struct("NveSensorSample")
        .unwrap()
        .is_pod(&host_spec));
    assert!(host_spec
        .find_struct("NveCameraFrameDescriptor")
        .unwrap()
        .is_pod(&host_spec));
    assert_eq!(draw_spec.c_functions.len(), 1);
    assert_eq!(host_spec.c_functions.len(), 15);
    assert_eq!(host_spec.modules.len(), 5);
}

#[test]
fn option2_draw_ops_and_host_services_compile_under_c11_cpp20_and_rust2024() {
    assert!(tool_available("rustc"), "rustc must be available");

    let dir = temp_test_dir("compile");
    common::write_generated_c_abi_headers(&dir);
    let nve_inc = workspace_root().join("cxx/nve-fabric-bridge/include");

    let draw_spec = draw_ops_hybrid_spec();
    let host_spec = host_services_hybrid_spec();
    let draw_cpp = generate_hybrid_target(&draw_spec, HybridTarget::Cpp).unwrap();
    let host_cpp = generate_hybrid_target(&host_spec, HybridTarget::Cpp).unwrap();
    draw_cpp.write_to_dir(&dir).unwrap();
    host_cpp.write_to_dir(&dir).unwrap();

    // Also emit & compile the companion RelocSchema for NveDrawOp
    let draw_reloc_files = generate_reloc_all(&draw_ops_reloc_schema()).unwrap();
    for f in &draw_reloc_files {
        std::fs::write(dir.join(&f.relative_path), &f.content).unwrap();
    }

    // 1. Verify BOTH pure C11 headers compile under `clang -std=c11 -Wall -Wextra -Werror`
    let c_driver = dir.join("check_c11.c");
    std::fs::write(
        &c_driver,
        r#"
#include "NveDrawOpsHybridAbi.generated.h"
#include "NveHostServicesHybridAbi.generated.h"

NVE_STATIC_ASSERT(sizeof(NveHybridNveDrawOp) == 64u, "NveDrawOp must be 64 bytes");
NVE_STATIC_ASSERT(sizeof(NveHybridNveThemeSnapshot) == 16u, "NveThemeSnapshot must be 16 bytes");
NVE_STATIC_ASSERT(sizeof(NveHybridNveSensorSample) == 24u, "NveSensorSample must be 24 bytes");
NVE_STATIC_ASSERT(sizeof(NveHybridNveCameraFrameDescriptor) == 40u, "NveCameraFrameDescriptor must be 40 bytes");

uint64_t _check_hashes(void) {
  return NVE_HYBRID_NVE_DRAW_OPS_FILE_SCHEMA_HASH ^
         NVE_HYBRID_NVE_HOST_SERVICES_FILE_SCHEMA_HASH ^
         NVE_HYBRID_CLIPBOARD_SCHEMA_HASH ^
         NVE_HYBRID_SYSTEM_ENV_SCHEMA_HASH ^
         NVE_HYBRID_HAPTICS_SCHEMA_HASH ^
         NVE_HYBRID_STORAGE_SCHEMA_HASH ^
         NVE_HYBRID_SENSORS_SCHEMA_HASH;
}
"#,
    )
    .unwrap();

    check_c11_syntax(&c_driver, &[&dir, &nve_inc], &dir);

    // 2. Compile AND execute a C++20 binary exercising both Hybrid C++20 headers + DrawSlab Reloc header
    let cpp_driver = dir.join("check_cpp20.cpp");
    std::fs::write(
        &cpp_driver,
        r#"
#include "NveDrawOpsHybridSpec.generated.hpp"
#include "NveHostServicesHybridSpec.generated.hpp"
#include "NveDrawSlabReloc.generated.hpp"

#include <cassert>

// Direct static `extern "C"` implementations (zero vtable, LTO-inlinable default path).
extern "C" int32_t nve_clipboard_write_text(NveStrSlice text) {
  return (text.ptr && text.len > 0u) ? 0 : -4;
}

extern "C" int32_t nve_system_env_get_theme(NveHybridNveThemeSnapshot* out_theme) {
  if (!out_theme) return -4;
  out_theme->color_scheme = 2u;
  out_theme->contrast = 1u;
  out_theme->flags = 1u;
  out_theme->pad0 = 0u;
  out_theme->accent_rgba = 0x007AFFFFu;
  out_theme->font_scale_x1000 = 1250u;
  out_theme->scale_factor_x100 = 200u;
  out_theme->reserved = 0u;
  return 0;
}

extern "C" int32_t nve_draw_submit_list(const NveHybridNveDrawListView* view) {
  return (view && view->ops && view->op_count > 0u) ? 0 : -4;
}

int main() {
  // Exercise direct static `extern "C"` calls with zero vtable indirection
  const char* msg = "hello-nve";
  NveStrSlice text_slice{reinterpret_cast<const uint8_t*>(msg), 9u};
  assert(nve_clipboard_write_text(text_slice) == 0);

  NveHybridNveThemeSnapshot direct_theme{};
  assert(nve_system_env_get_theme(&direct_theme) == 0);
  assert(direct_theme.color_scheme == 2u && direct_theme.font_scale_x1000 == 1250u);

  // Verify NveDrawOp POD round-trip + NveDrawListView direct submission
  nve::hybrid::NveDrawOp op{};
  op.kind = nve::hybrid::NveDrawOpKind::FillRoundedRect;
  op.colorRgba = 0xFF00AAFFu;
  op.x = 10.0f;
  op.y = 20.0f;
  op.width = 120.0f;
  op.height = 48.0f;
  op.param0 = 8.0f;
  const NveHybridNveDrawOp raw_op = op.to_abi();
  static_assert(sizeof(raw_op) == 64u);
  NveHybridNveDrawListView draw_view{&raw_op, 1u, nullptr, 0u, nullptr, 0u};
  assert(nve_draw_submit_list(&draw_view) == 0);
  const auto op_back = nve::hybrid::NveDrawOp::from_abi(raw_op);
  assert(op_back.kind == nve::hybrid::NveDrawOpKind::FillRoundedRect);
  assert(op_back.colorRgba == 0xFF00AAFFu);

  // Verify NveCameraFrameDescriptor 40-byte NVCF round-trip (including FixedArray [u8; 4])
  nve::hybrid::NveCameraFrameDescriptor cam{};
  cam.magic = {'N', 'V', 'C', 'F'};
  cam.version = 1u;
  cam.pixelFormat = 1u;
  cam.rotationDeg = 90u;
  cam.width = 1920u;
  cam.height = 1080u;
  cam.strideBytes = 7680u;
  cam.timestampNs = UINT64_C(42000000);
  cam.nativeHandleId = UINT64_C(0xDEADBEEF);
  const NveHybridNveCameraFrameDescriptor raw_cam = cam.to_abi();
  static_assert(sizeof(raw_cam) == 40u);
  const auto cam_back = nve::hybrid::NveCameraFrameDescriptor::from_abi(raw_cam);
  assert(cam_back.magic[0] == 'N' && cam_back.magic[3] == 'F');
  assert(cam_back.width == 1920u && cam_back.nativeHandleId == UINT64_C(0xDEADBEEF));

  // Verify NveThemeSnapshot (16B) & NveSensorSample (24B)
  nve::hybrid::NveThemeSnapshot theme{};
  theme.colorScheme = 2u;
  theme.accentRgba = 0x007AFFFFu;
  theme.fontScaleX1000 = 1250u;
  theme.scaleFactorX100 = 200u;
  const NveHybridNveThemeSnapshot raw_theme = theme.to_abi();
  static_assert(sizeof(raw_theme) == 16u);
  assert(nve::hybrid::NveThemeSnapshot::from_abi(raw_theme).fontScaleX1000 == 1250u);

  nve::hybrid::NveSensorSample sample{};
  sample.x = 0.1f;
  sample.y = 9.81f;
  sample.z = -0.2f;
  sample.timestampNs = UINT64_C(999999);
  const NveHybridNveSensorSample raw_sample = sample.to_abi();
  static_assert(sizeof(raw_sample) == 24u);
  assert(nve::hybrid::NveSensorSample::from_abi(raw_sample).timestampNs == UINT64_C(999999));

  // Verify Reloc NveDrawOp is 64-byte aligned & has as_bytes()
  nve::reloc::nve_draw_slab::NveDrawOp reloc_op{};
  reloc_op.kind = nve::reloc::nve_draw_slab::NveDrawOpKind::DrawText;
  static_assert(sizeof(reloc_op) == 64u);
  static_assert(alignof(nve::reloc::nve_draw_slab::NveDrawOp) == 64u);
  assert(reloc_op.as_bytes().size() == 64u);

  return 0;
}
"#,
    )
    .unwrap();

    compile_and_run_cpp20(&cpp_driver, "check_cpp20", &[&dir, &nve_inc], &dir);

    // 3. Also verify the companion NveDrawSlab Rust 2024 module compiles with `-D warnings`
    let rust_check = Command::new("rustc")
        .args([
            "--edition=2024",
            "--crate-type=lib",
            "--crate-name=nve_draw_slab_reloc",
            "-D",
            "warnings",
        ])
        .arg(dir.join("nve_draw_slab_reloc.generated.rs"))
        .arg("-o")
        .arg(dir.join("libnve_draw_slab_reloc.rlib"))
        .output()
        .expect("run rustc on NveDrawSlab reloc");
    assert!(
        rust_check.status.success(),
        "rustc on NveDrawSlab reloc failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&rust_check.stdout),
        String::from_utf8_lossy(&rust_check.stderr)
    );

    let _ = std::fs::remove_dir_all(&dir);
}
