mod common;


use common::specs::{assert_balanced_delimiters, counter_card_export_spec, EMBEDDED_SWITCH_SPEC};
use nve_codegen::{
    export_all_targets, export_compose, export_flutter, export_rn_spec, export_swiftui,
    export_target, export_web_component, import_rn_source, parse_rn_spec, DefaultValue,
    EventBubbling, EventSpec, ExportBackend, ExportTarget, NveExportSpec, ObjectField, PropSpec,
    PropType,
};

#[test]
fn multi_target_export_generates_flutter_swiftui_compose_and_web_component() {
    let skia_spec = counter_card_export_spec(ExportBackend::Skia);
    let fabric_spec = counter_card_export_spec(ExportBackend::Fabric);

    let fl_skia = export_flutter(&skia_spec).expect("export_flutter skia");
    assert_eq!(fl_skia.dart_widget.relative_path, "nve_counter_card.generated.dart");
    let dart_s = &fl_skia.dart_widget.content;
    assert!(dart_s.contains("import 'dart:ffi' as ffi;"));
    assert!(dart_s.contains("import 'package:nve_flutter/nve_flutter.dart';"));
    assert_eq!(dart_s.matches("import 'package:").count(), 2, "flutter/widgets + nve_flutter only");
    assert!(dart_s.contains("class NveCounterCard extends StatefulWidget"));
    assert!(dart_s.contains("child: NveCanvasView(controller: _canvas)"));
    assert!(dart_s.contains("_canvas = NveCanvasController.open("));
    assert!(dart_s.contains("appSymbol: NveCounterCardNativeBindings.vtableSymbol"));
    assert!(dart_s.contains("static const String vtableSymbol = 'nve_counter_card_vtable';"));
    assert!(dart_s.contains("_canvas?.sendCommand('setCount', <String, Object?>{"));
    assert!(dart_s.contains("..onAppEvent = _handleEvent;"));
    assert!(dart_s.contains("void _syncProps() => _canvas.setProps(_encodeProps());"));
    assert!(dart_s.contains("NveCounterCardCountChangeEvent.fromValue(event.value)"));
    assert!(dart_s.contains("count: switch (m['count']) { final num v => v.toInt(), _ => 0 },"));
    assert!(!dart_s.contains("NveSurfaceController"));
    assert!(!dart_s.contains("NveChannelClient"));
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
    assert_eq!(total_files, 9);

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
    for file in imported.files() {
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

    let c11_harness = tmp.join("check_abi.c");
    std::fs::write(
        &c11_harness,
        "#include <nve/generated/nve_runtime_abi.generated.h>\n#include <nve/generated/nve_abi.generated.h>\nint check_abi_sizes(void) { return (int)(sizeof(NveSceneView) + sizeof(NveTableView) + sizeof(NveRuntimeApi) + sizeof(NveNodeRow)); }\n",
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
        "Generated C11 ABI headers failed to compile:\n{}",
        String::from_utf8_lossy(&status_c11.stderr)
    );

    let cpp_harness = tmp.join("check_abi.cpp");
    std::fs::write(
        &cpp_harness,
        format!(
            "#include <nve/generated/nve_abi.generated.h>\n#include <nve/generated/nve_runtime_abi.generated.h>\nstatic_assert(sizeof(NveLayoutRect) == NVE_LAYOUT_RECT_SIZE);\nstatic_assert(sizeof(NveNodeRow) == NVE_NODE_ROW_SIZE);\nstatic_assert(sizeof(NveEvent) == NVE_EVENT_SIZE);\nstatic_assert(NVE_RUNTIME_SCHEMA_HASH == 0x{:016X}ULL);\nint check_abi_cpp() {{ return static_cast<int>(NVE_PRIMARY_SLOT_NONE) + static_cast<int>(NVE_RUNTIME_OK); }}\n",
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
        "Generated C++20 ABI headers failed to compile:\n{}",
        String::from_utf8_lossy(&status_cpp.stderr)
    );

    let _ = std::fs::remove_dir_all(&tmp);
}
