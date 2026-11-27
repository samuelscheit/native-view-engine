mod common;

use std::path::Path;

use common::specs::{assert_balanced_delimiters, counter_card_export_spec, EMBEDDED_SWITCH_SPEC};
use nve_codegen::{
    compute_scene_schema_hash, core_abi_schema_hash, export_all_targets, export_compose,
    export_flutter, export_rn_spec, export_swiftui, export_target, export_web_component,
    generate_all_abi_files, generate_all_core_abi_files, generate_all_scene_abi_files,
    import_rn_source, parse_rn_spec, scene_abi_spec, DefaultValue, EventBubbling, EventSpec,
    ExportBackend, ExportTarget, NveExportSpec, ObjectField, PropSpec, PropType,
    NVE_CORE_ABI_SCHEMA_HASH, NVE_SCENE_SCHEMA_HASH,
};

#[test]
fn multi_target_export_generates_flutter_swiftui_compose_and_web_component() {
    let skia_spec = counter_card_export_spec(ExportBackend::Skia);
    let fabric_spec = counter_card_export_spec(ExportBackend::Fabric);

    let fl_skia = export_flutter(&skia_spec).expect("export_flutter skia");
    assert_eq!(fl_skia.dart_widget.relative_path, "nve_counter_card.generated.dart");
    let dart_s = &fl_skia.dart_widget.content;
    assert!(dart_s.contains("import 'dart:ffi' as ffi;"));
    assert!(dart_s.contains("import 'package:nve_flutter/nve_canvas.dart';"));
    assert!(dart_s.contains("import 'package:nve_flutter/nve_flutter.dart';"));
    assert!(dart_s.contains("class NveCounterCard extends StatefulWidget"));
    assert!(dart_s.contains("child: NveCanvasView("));
    assert!(dart_s.contains("NveSurfaceController"));
    assert!(dart_s.contains("NveEventKind.custom"));
    assert!(dart_s.contains("NveCounterCardCountChangeEvent.fromBinView"));
    assert!(!dart_s.contains("fromJson"));
    assert!(!dart_s.contains("payloadJson"));
    assert!(dart_s.contains("if (Platform.isIOS)"));
    assert!(!dart_s.contains("Platform.isIOS || Platform.isMacOS"));
    assert!(dart_s.contains(
        "Platform.isWindows ? 'nve_counter_card.dll' : Platform.isMacOS ? 'libnve_counter_card.dylib' : 'libnve_counter_card.so'"
    ));
    assert_balanced_delimiters("nve_counter_card.generated.dart", dart_s);

    let fl_fabric = export_flutter(&fabric_spec).expect("export_flutter fabric");
    assert!(fl_fabric.dart_widget.content.contains("child: NveCanvasView("));

    let sw_out = export_swiftui(&skia_spec).expect("export_swiftui");
    assert_eq!(sw_out.swift_view.relative_path, "NveCounterCardView.generated.swift");
    let swift = &sw_out.swift_view.content;
    assert!(swift.contains("import SwiftUI"));
    assert!(swift.contains("import NveSwiftUI"));
    assert!(swift.contains("public struct NveCounterCardDirectProps: NveDirectEncodable, Equatable, Sendable"));
    assert!(swift.contains("public struct NveCounterCardView: View"));
    assert!(swift.contains("NveSurfaceRepresentable("));
    assert!(swift.contains("backend: .skia"));
    assert!(swift.contains("directProps: makeDirectProps()"));
    assert!(swift.contains("event.withBinValueView"));
    assert!(!swift.contains("[String: Any]"));
    assert!(swift.contains("NveSurfaceController"));
    assert_balanced_delimiters("NveCounterCardView.generated.swift", swift);

    let cmp_out = export_compose(&fabric_spec).expect("export_compose");
    assert_eq!(cmp_out.compose_kt.relative_path, "NveCounterCardComposable.generated.kt");
    let compose = &cmp_out.compose_kt.content;
    assert!(compose.contains("import dev.nve.compose.NveSurface"));
    assert!(compose.contains("import dev.nve.compose.NveBinValue"));
    assert!(compose.contains("import dev.nve.compose.NveBinValueView"));
    assert!(compose.contains("import dev.nve.compose.NveRenderBackend"));
    assert!(compose.contains("import dev.nve.compose.rememberNveSurfaceController"));
    assert!(compose.contains("object NveCounterCardDirectPropsWriter"));
    assert!(compose.contains("updatePropsBinary(encodedPropsBin)"));
    assert!(compose.contains("@Composable\nfun NveCounterCard("));
    assert!(compose.contains("backend = NveRenderBackend.FABRIC"));
    assert!(compose.contains("NveEventKind.CUSTOM"));
    assert!(!compose.contains("JSONObject"));
    assert!(!compose.contains("org.json"));
    assert!(!compose.contains("buildMap<String, Any?>"));
    assert_balanced_delimiters("NveCounterCardComposable.generated.kt", compose);

    let web_out = export_web_component(&skia_spec).expect("export_web_component");
    assert_eq!(web_out.tag_name, "nve-counter-card");
    assert_eq!(web_out.element_js.relative_path, "nve-counter-card.generated.js");
    assert_eq!(web_out.types_dts.relative_path, "nve-counter-card.generated.d.ts");
    let js = &web_out.element_js.content;
    let dts = &web_out.types_dts.content;
    assert!(js.contains("export class NveCounterCardElement extends HTMLElement"));
    assert!(js.contains("static tagName = 'nve-counter-card';"));
    assert!(js.contains("this.attachShadow({ mode: 'open' })"));
    assert!(js.contains("mod.mount_nve_component("));
    assert!(js.contains("encodeNveBinValue"));
    assert!(js.contains("decodeNveBinValue"));
    assert!(!js.contains("JSON.stringify"));
    assert!(!js.contains("JSON.parse"));
    assert!(!js.contains("set_props_json"));
    assert!(js.contains("new CustomEvent('count-change'"));
    assert!(js.contains("customElements.define(tagName, NveCounterCardElement)"));
    assert_balanced_delimiters("nve-counter-card.generated.js", js);

    assert!(dts.contains("export interface NveCounterCardCountChangeDetail"));
    assert!(dts.contains("export interface NveCounterCardEventMap"));
    assert!(dts.contains("'nve-counter-card': NveCounterCardElement;"));
    assert_balanced_delimiters("nve-counter-card.generated.d.ts", dts);

    let plain_spec = NveExportSpec::new("SparkChart", ExportBackend::Skia);
    let plain_web = export_web_component(&plain_spec).expect("plain web component");
    assert_eq!(plain_web.tag_name, "nve-spark-chart");

    let all = export_all_targets(&fabric_spec, ExportTarget::ALL).expect("export all targets");
    assert_eq!(all.len(), 5);
    let total_files: usize = all.iter().map(|o| o.files.len()).sum();
    assert_eq!(total_files, 11);

    let single = export_target(&fabric_spec, ExportTarget::from_name("swiftui").unwrap()).unwrap();
    assert_eq!(single.files.len(), 1);

    let tmp = std::env::temp_dir().join("nve_codegen_test_multi_export");
    let _ = std::fs::remove_dir_all(&tmp);
    for bundle in &all {
        let dir = tmp.join(bundle.target.dir_name());
        let written = bundle.write_to_dir(&dir).expect("write generated files");
        assert_eq!(written.len(), bundle.files.len());
        for path in written {
            assert!(path.is_file(), "expected generated file at {}", path.display());
        }
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn validates_invalid_specs_cleanly() {
    assert!(export_rn_spec(&NveExportSpec::new("", ExportBackend::Fabric)).is_err());
    assert!(export_rn_spec(&NveExportSpec::new("123BadName", ExportBackend::Fabric)).is_err());
    assert!(serde_json::from_str::<NveExportSpec>("{invalid json").is_err());
    assert!(parse_rn_spec("export const x = 42;").is_err());
    assert!(ExportTarget::from_name("unknown-platform").is_err());
}

#[test]
fn all_generated_import_and_export_bindings_are_strictly_binary() {
    let imported = import_rn_source(EMBEDDED_SWITCH_SPEC).expect("import Switch");
    for file in [&imported.rust_module, &imported.cpp_adapter] {
        assert!(!file.content.contains("serde_json"), "{} must not use serde_json", file.relative_path);
        assert!(!file.content.contains("payload_json"), "{} must not use payload_json", file.relative_path);
    }

    let spec = counter_card_export_spec(ExportBackend::Fabric);
    for bundle in export_all_targets(&spec, ExportTarget::ALL).expect("export all targets") {
        for file in &bundle.files {
            assert!(!file.content.contains("serde_json"), "{} must not use serde_json", file.relative_path);
            assert!(!file.content.contains("JSONObject"), "{} must not use JSONObject", file.relative_path);
            assert!(!file.content.contains("org.json"), "{} must not use org.json", file.relative_path);
            assert!(!file.content.contains("[String: Any]"), "{} must not use [String: Any]", file.relative_path);
            assert!(!file.content.contains("buildMap<String, Any?>"), "{} must not use buildMap<String, Any?>", file.relative_path);
            assert!(!file.content.contains("JSON.stringify"), "{} must not use JSON.stringify", file.relative_path);
            assert!(!file.content.contains("JSON.parse"), "{} must not use JSON.parse", file.relative_path);
            assert!(!file.content.contains("update_props_json"), "{} must not use update_props_json", file.relative_path);
        }
    }
}

#[test]
fn minimal_and_complex_specs_export_cleanly_across_all_five_targets() {
    let empty = NveExportSpec::new("EmptyCanvas", ExportBackend::Skia);
    let empty_all = export_all_targets(&empty, ExportTarget::ALL).expect("export minimal spec");
    assert_eq!(empty_all.len(), 5);
    for bundle in &empty_all {
        for file in &bundle.files {
            assert!(!file.content.is_empty());
            assert_balanced_delimiters(&file.relative_path, &file.content);
        }
    }

    let mut complex = NveExportSpec::new("DataGrid", ExportBackend::Fabric);
    complex.props = vec![
        PropSpec {
            name: "density".to_owned(),
            prop_type: PropType::IntEnum { variants: vec![0, 1, 2] },
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Int(1)),
            doc: None,
        },
        PropSpec {
            name: "rowHeight".to_owned(),
            prop_type: PropType::Double,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::Float(44.5)),
            doc: None,
        },
        PropSpec {
            name: "columns".to_owned(),
            prop_type: PropType::Array { item: Box::new(PropType::String) },
            optional: false,
            nullable: false,
            default_value: None,
            doc: None,
        },
        PropSpec {
            name: "scrollOffset".to_owned(),
            prop_type: PropType::Point,
            optional: true,
            nullable: true,
            default_value: None,
            doc: None,
        },
    ];
    complex.events = vec![EventSpec {
        name: "onRowSelect".to_owned(),
        bubbling: EventBubbling::Direct,
        optional: true,
        paper_top_level_name: Some("topRowSelect".to_owned()),
        payload: vec![
            ObjectField {
                name: "rowIndex".to_owned(),
                field_type: PropType::Int32,
                optional: false,
                nullable: false,
            },
            ObjectField {
                name: "note".to_owned(),
                field_type: PropType::String,
                optional: true,
                nullable: true,
            },
        ],
        payload_type_name: None,
        doc: None,
    }];

    let all = export_all_targets(&complex, ExportTarget::ALL).expect("export complex spec");
    for bundle in &all {
        for file in &bundle.files {
            assert_balanced_delimiters(&file.relative_path, &file.content);
        }
    }
}

#[test]
fn generated_c_and_cpp20_abi_headers_compile_cleanly() {
    let clang_ok = std::process::Command::new("clang++")
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success());
    if !clang_ok {
        return;
    }

    let tmp = std::env::temp_dir().join("nve_codegen_cpp20_syntax_check");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).unwrap();
    common::write_generated_c_abi_headers(&tmp);

    // Compile `nve_runtime_abi.generated.h` standalone AND in all 6 include-order permutations
    // with `nve_abi.generated.h` and `nve_scene_abi.generated.h` under strict C11 and C++20.
    let cases: &[(&str, &str, &str)] = &[
        (
            "runtime_standalone",
            "#include \"nve_runtime_abi.generated.h\"\n",
            "return (int)(sizeof(NveRuntimeApi) + sizeof(NveRuntimeSceneView) + sizeof(NveTableView));",
        ),
        (
            "core_scene_runtime",
            "#include \"nve_abi.generated.h\"\n#include \"nve_scene_abi.generated.h\"\n#include \"nve_runtime_abi.generated.h\"\n",
            "return (int)(sizeof(NveRenderNode) + sizeof(NveSceneView) + sizeof(NveRuntimeSceneView) + sizeof(NveTableView));",
        ),
        (
            "core_runtime_scene",
            "#include \"nve_abi.generated.h\"\n#include \"nve_runtime_abi.generated.h\"\n#include \"nve_scene_abi.generated.h\"\n",
            "return (int)(sizeof(NveRenderNode) + sizeof(NveRuntimeApi) + sizeof(NveTableView));",
        ),
        (
            "scene_core_runtime",
            "#include \"nve_scene_abi.generated.h\"\n#include \"nve_abi.generated.h\"\n#include \"nve_runtime_abi.generated.h\"\n",
            "return (int)(sizeof(NveSceneView) + sizeof(NveRuntimeSceneView) + sizeof(NveTableView));",
        ),
        (
            "scene_runtime_core",
            "#include \"nve_scene_abi.generated.h\"\n#include \"nve_runtime_abi.generated.h\"\n#include \"nve_abi.generated.h\"\n",
            "return (int)(sizeof(NveSceneView) + sizeof(NveRuntimeApi) + sizeof(NveBufferSlice));",
        ),
        (
            "runtime_core_scene",
            "#include \"nve_runtime_abi.generated.h\"\n#include \"nve_abi.generated.h\"\n#include \"nve_scene_abi.generated.h\"\n",
            "return (int)(sizeof(NveRuntimeSceneView) + sizeof(NveRenderNode) + sizeof(NveEventBatch));",
        ),
        (
            "runtime_scene_core",
            "#include \"nve_runtime_abi.generated.h\"\n#include \"nve_scene_abi.generated.h\"\n#include \"nve_abi.generated.h\"\n",
            "return (int)(sizeof(NveRuntimeApi) + sizeof(NveSceneView) + sizeof(NveStrSlice));",
        ),
    ];

    for (name, includes, body) in cases {
        let c11_harness = tmp.join(format!("check_{name}.c"));
        std::fs::write(
            &c11_harness,
            format!("{includes}int check_c11_{name}(void) {{ {body} }}\n"),
        )
        .unwrap();
        let status_c11 = std::process::Command::new("clang")
            .args(["-std=c11", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(&tmp)
            .arg(&c11_harness)
            .output()
            .expect("run clang on generated C11 ABI headers");
        assert!(
            status_c11.status.success(),
            "Generated C11 ABI headers failed for `{name}`:\n{}",
            String::from_utf8_lossy(&status_c11.stderr)
        );

        let cpp_harness = tmp.join(format!("check_{name}.cpp"));
        std::fs::write(
            &cpp_harness,
            format!(
                "{includes}static_assert(NVE_RUNTIME_SCHEMA_HASH == 0x{:016X}ULL);\nint check_cpp_{name}() {{ {body} }}\n",
                nve_abi::NVE_RUNTIME_SCHEMA_HASH
            ),
        )
        .unwrap();
        let status_cpp = std::process::Command::new("clang++")
            .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(&tmp)
            .arg(&cpp_harness)
            .output()
            .expect("run clang++ on generated C++20 ABI headers");
        assert!(
            status_cpp.status.success(),
            "Generated C++20 ABI headers failed for `{name}`:\n{}",
            String::from_utf8_lossy(&status_cpp.stderr)
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn unified_core_and_scene_abi_generated_files_match_checked_in_artifacts_byte_for_byte() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    let core_files = generate_all_core_abi_files();
    let scene_files = generate_all_scene_abi_files();
    let all_files = generate_all_abi_files();

    assert_eq!(core_files.len(), 5);
    assert_eq!(scene_files.len(), 4);
    assert_eq!(all_files.len(), 17);

    assert_ne!(NVE_CORE_ABI_SCHEMA_HASH, 0);
    assert_eq!(core_abi_schema_hash(), NVE_CORE_ABI_SCHEMA_HASH);
    assert_eq!(NVE_CORE_ABI_SCHEMA_HASH, nve_abi::NVE_CORE_ABI_SCHEMA_HASH);

    assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
    assert_eq!(compute_scene_schema_hash(&scene_abi_spec()), NVE_SCENE_SCHEMA_HASH);
    assert_eq!(NVE_SCENE_SCHEMA_HASH, nve_abi::NVE_SCENE_SCHEMA_HASH);

    assert_eq!(core_files[0].content, core_files[1].content);
    assert!(scene_files[0].relative_path.ends_with("nve_scene_abi.generated.h"));

    assert!(core_files[0].content.contains("#define NVE_IFACE_HOST 5u"));
    assert!(core_files[0].content.contains("typedef enum NveHostStatus"));
    assert!(core_files[0].content.contains("typedef struct NveHostCallbacks"));
    assert!(core_files[0].content.contains("typedef struct NveHostApi"));
    assert!(core_files[2].content.contains("const int nveIfaceHost = 5;"));
    assert!(core_files[2].content.contains("enum NveHostStatus"));
    assert!(core_files[2].content.contains("final class NveHostApi extends ffi.Struct"));
    assert!(core_files[3].content.contains("public static final int IFACE_HOST = 5;"));
    assert!(core_files[3].content.contains("public static final int HOST_BUFFER_TOO_SMALL = -1;"));
    assert!(core_files[4].content.contains("HOST: 5,"));
    assert!(core_files[4].content.contains("export const NVE_HOST_STATUS ="));

    assert!(core_files[0].content.contains("typedef struct NvePropEntry"));
    assert!(core_files[0].content.contains("typedef struct NvePropSlice"));
    assert!(core_files[0].content.contains("typedef struct NveCommandSlice"));
    assert!(core_files[0].content.contains("sizeof(NvePropEntry) == 32u"));
    assert!(core_files[0].content.contains("sizeof(NvePropSlice) == 16u"));
    assert!(core_files[0].content.contains("sizeof(NveCommandSlice) == 16u"));
    assert!(core_files[2].content.contains("final class NvePropEntry extends ffi.Struct"));
    assert!(core_files[2].content.contains("final class NvePropSlice extends ffi.Struct"));
    assert!(core_files[2].content.contains("final class NveCommandSlice extends ffi.Struct"));
    assert!(core_files[3].content.contains("public static final int SIZEOF_PROP_ENTRY_64 = 32;"));
    assert!(core_files[3].content.contains("public static final int SIZEOF_PROP_SLICE_64 = 16;"));
    assert!(core_files[3].content.contains("public static final int SIZEOF_COMMAND_SLICE_64 = 16;"));

    assert_eq!(all_files, generate_all_abi_files(), "ABI codegen must be deterministic");

    let tmp_abi = std::env::temp_dir().join(format!(
        "nve_codegen_abi_materialize_{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&tmp_abi);
    for file in &all_files {
        let out_path = tmp_abi.join(&file.relative_path);
        if let Some(parent) = out_path.parent() {
            std::fs::create_dir_all(parent).expect("create temp ABI parent dirs");
        }
        std::fs::write(&out_path, &file.content).expect("write temp ABI file");
        assert_eq!(std::fs::read_to_string(&out_path).unwrap(), file.content);
    }
    let _ = std::fs::remove_dir_all(&tmp_abi);

    let update_mode =
        std::env::var("NVE_UPDATE_ABI").is_ok() || std::env::var("NVE_UPDATE_SCENE_ABI").is_ok();
    common::assert_generated_file_naming_and_optional_disk_parity(
        &all_files,
        &repo_root,
        update_mode,
    );
}
