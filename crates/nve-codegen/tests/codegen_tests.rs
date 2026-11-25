use std::path::Path;

use nve_codegen::{
    compute_scene_schema_hash, core_abi_schema_hash, export_all_targets, export_compose,
    export_flutter, export_rn_spec, export_swiftui, export_target, export_web_component,
    generate_all_abi_files, generate_all_core_abi_files, generate_all_scene_abi_files,
    import_rn_file, import_rn_source, parse_rn_spec, parse_rn_spec_file, scene_abi_spec,
    CommandSpec, DefaultValue, EventBubbling, EventSpec, ExportBackend, ExportTarget,
    NveExportSpec, ObjectField, ParamSpec, ParsedRnSpec, PropSpec, PropType,
    NVE_CORE_ABI_SCHEMA_HASH, NVE_SCENE_SCHEMA_HASH,
};

const RN_LIBRARIES_DIR: &str =
    "/Users/user/developer/developer/respond/react-native/react-native/packages/react-native/Libraries";

/// Embedded fallback copy of RN's real `SwitchNativeComponent.js` for environments without the local RN checkout.
const EMBEDDED_SWITCH_SPEC: &str = r#"
/**
 * Copyright (c) Meta Platforms, Inc. and affiliates.
 * @flow strict-local
 */
import type {ViewProps} from '../../../../../Libraries/Components/View/ViewPropTypes';
import type {ColorValue} from '../../../../../Libraries/StyleSheet/StyleSheet';
import type {BubblingEventHandler, Int32, WithDefault} from '../../../../../Libraries/Types/CodegenTypes';
import codegenNativeCommands from '../../../../../Libraries/Utilities/codegenNativeCommands';
import codegenNativeComponent from '../../../../../Libraries/Utilities/codegenNativeComponent';

type NativeSwitchChangeEvent = Readonly<{
  value: boolean,
  target: Int32,
}>;

type SwitchNativeProps = Readonly<{
  ...ViewProps,
  disabled?: WithDefault<boolean, false>,
  value?: WithDefault<boolean, false>,
  tintColor?: ?ColorValue,
  onTintColor?: ?ColorValue,
  thumbTintColor?: ?ColorValue,
  onChange?: ?BubblingEventHandler<NativeSwitchChangeEvent>,
}>;

interface NativeCommands {
  readonly setValue: (viewRef: React.ElementRef<ComponentType>, value: boolean) => void;
}

export const Commands: NativeCommands = codegenNativeCommands<NativeCommands>({
  supportedCommands: ['setValue'],
});

export default codegenNativeComponent<SwitchNativeProps>('Switch', {
  paperComponentName: 'RCTSwitch',
  excludedPlatforms: ['android'],
  interfaceOnly: true,
}) as ComponentType;
"#;

fn assert_balanced_delimiters(label: &str, code: &str) {
    let mut braces = 0i32;
    let mut parens = 0i32;
    let mut in_single = false;
    let mut in_double = false;
    let mut in_line_comment = false;
    let mut in_block_comment = false;
    let chars: Vec<char> = code.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();

        if in_line_comment {
            if c == '\n' {
                in_line_comment = false;
            }
            i += 1;
            continue;
        }
        if in_block_comment {
            if c == '*' && next == Some('/') {
                in_block_comment = false;
                i += 2;
                continue;
            }
            i += 1;
            continue;
        }
        if in_single {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '\'' {
                in_single = false;
            }
            i += 1;
            continue;
        }
        if in_double {
            if c == '\\' {
                i += 2;
                continue;
            }
            if c == '"' {
                in_double = false;
            }
            i += 1;
            continue;
        }

        if c == '/' && next == Some('/') {
            in_line_comment = true;
            i += 2;
            continue;
        }
        if c == '/' && next == Some('*') {
            in_block_comment = true;
            i += 2;
            continue;
        }
        if c == '\'' && !label.ends_with(".rs") {
            // Skip Rust lifetimes like `'static`
            in_single = true;
            i += 1;
            continue;
        }
        if c == '"' {
            in_double = true;
            i += 1;
            continue;
        }

        match c {
            '{' => braces += 1,
            '}' => braces -= 1,
            '(' => parens += 1,
            ')' => parens -= 1,
            _ => {}
        }
        assert!(
            braces >= 0,
            "{label}: negative brace balance near char {i}"
        );
        assert!(
            parens >= 0,
            "{label}: negative paren balance near char {i}"
        );
        i += 1;
    }

    assert_eq!(braces, 0, "{label}: unbalanced curly braces");
    assert_eq!(parens, 0, "{label}: unbalanced parentheses");
}

fn counter_card_export_spec(backend: ExportBackend) -> NveExportSpec {
    NveExportSpec {
        name: "NveCounterCard".to_owned(),
        backend,
        rust_vtable_symbol: Some("nve_counter_card_vtable".to_owned()),
        android_package: Some("dev.nve.components".to_owned()),
        doc: Some("Interactive counter card rendered natively by NVE.".to_owned()),
        props: vec![
            PropSpec {
                name: "initialCount".to_owned(),
                prop_type: PropType::Int32,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Int(0)),
                doc: Some("Initial counter value.".to_owned()),
            },
            PropSpec {
                name: "accentColor".to_owned(),
                prop_type: PropType::Color,
                optional: true,
                nullable: true,
                default_value: None,
                doc: Some("Optional accent tint color.".to_owned()),
            },
            PropSpec {
                name: "variant".to_owned(),
                prop_type: PropType::StringEnum {
                    variants: vec!["solid".into(), "outline".into(), "ghost".into()],
                },
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::String("solid".into())),
                doc: Some("Visual button style variant.".to_owned()),
            },
            PropSpec {
                name: "stepScale".to_owned(),
                prop_type: PropType::Float,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Float(1.0)),
                doc: None,
            },
            PropSpec {
                name: "disabled".to_owned(),
                prop_type: PropType::Bool,
                optional: true,
                nullable: false,
                default_value: Some(DefaultValue::Bool(false)),
                doc: None,
            },
        ],
        events: vec![
            EventSpec {
                name: "onCountChange".to_owned(),
                bubbling: EventBubbling::Direct,
                optional: true,
                paper_top_level_name: None,
                payload: vec![
                    ObjectField {
                        name: "count".to_owned(),
                        field_type: PropType::Int32,
                        optional: false,
                        nullable: false,
                    },
                    ObjectField {
                        name: "delta".to_owned(),
                        field_type: PropType::Int32,
                        optional: false,
                        nullable: false,
                    },
                ],
                payload_type_name: Some("CountChangeEvent".to_owned()),
                doc: Some("Emitted whenever the counter value changes.".to_owned()),
            },
            EventSpec {
                name: "onReset".to_owned(),
                bubbling: EventBubbling::Bubbling,
                optional: true,
                paper_top_level_name: None,
                payload: Vec::new(),
                payload_type_name: None,
                doc: None,
            },
        ],
        commands: vec![CommandSpec {
            name: "setCount".to_owned(),
            params: vec![ParamSpec {
                name: "value".to_owned(),
                param_type: PropType::Int32,
                optional: false,
            }],
        }],
    }
}

#[test]
fn parses_real_rn_component_specs_and_follows_reexports() {
    let switch_path = Path::new(RN_LIBRARIES_DIR).join("Components/Switch/SwitchNativeComponent.js");
    let switch_spec = if switch_path.exists() {
        parse_rn_spec_file(&switch_path).expect("parse real SwitchNativeComponent.js")
    } else {
        parse_rn_spec(EMBEDDED_SWITCH_SPEC).expect("parse embedded Switch spec")
    };

    let ParsedRnSpec::Component(sw) = switch_spec else {
        panic!("expected Component spec for Switch");
    };
    assert_eq!(sw.component_name, "Switch");
    assert!(sw.extends_view_props);
    assert_eq!(
        sw.options.paper_component_name.as_deref(),
        Some("RCTSwitch")
    );
    assert_eq!(sw.options.excluded_platforms, vec!["android"]);
    assert!(sw.options.interface_only);

    let disabled = sw.props.iter().find(|p| p.name == "disabled").unwrap();
    assert_eq!(disabled.prop_type, PropType::Bool);
    assert_eq!(disabled.default_value, Some(DefaultValue::Bool(false)));

    let tint = sw.props.iter().find(|p| p.name == "tintColor").unwrap();
    assert_eq!(tint.prop_type, PropType::Color);
    assert!(tint.nullable);

    assert_eq!(sw.events.len(), 1);
    assert_eq!(sw.events[0].name, "onChange");
    assert_eq!(sw.events[0].bubbling, EventBubbling::Bubbling);
    assert_eq!(sw.events[0].payload.len(), 2);
    assert_eq!(sw.events[0].payload[0].name, "value");
    assert_eq!(sw.events[0].payload[0].field_type, PropType::Bool);
    assert_eq!(sw.events[0].payload[1].name, "target");
    assert_eq!(sw.events[0].payload[1].field_type, PropType::Int32);

    assert_eq!(sw.commands.len(), 1);
    assert_eq!(sw.commands[0].name, "setValue");
    assert_eq!(sw.commands[0].params.len(), 1);
    assert_eq!(sw.commands[0].params[0].name, "value");
    assert_eq!(sw.commands[0].params[0].param_type, PropType::Bool);

    // Additional real RN specs if the local RN checkout is present
    let activity_path = Path::new(RN_LIBRARIES_DIR)
        .join("Components/ActivityIndicator/ActivityIndicatorViewNativeComponent.js");
    if activity_path.exists() {
        let out = import_rn_file(&activity_path).expect("import ActivityIndicatorView");
        let ParsedRnSpec::Component(act) = &out.spec else {
            panic!("expected component");
        };
        assert_eq!(act.component_name, "ActivityIndicatorView");
        let size = act.props.iter().find(|p| p.name == "size").unwrap();
        assert_eq!(
            size.prop_type,
            PropType::StringEnum {
                variants: vec!["small".into(), "large".into()]
            }
        );
        assert_eq!(
            size.default_value,
            Some(DefaultValue::String("small".into()))
        );
        assert_balanced_delimiters(&out.rust_module.relative_path, &out.rust_module.content);
        assert_balanced_delimiters(&out.cpp_adapter.relative_path, &out.cpp_adapter.content);
    }

    let modal_path = Path::new(RN_LIBRARIES_DIR).join("Modal/RCTModalHostViewNativeComponent.js");
    if modal_path.exists() {
        let out = import_rn_file(&modal_path).expect("import ModalHostView");
        let ParsedRnSpec::Component(modal) = &out.spec else {
            panic!("expected component");
        };
        assert_eq!(modal.component_name, "ModalHostView");
        assert!(modal.events.iter().any(|e| e.name == "onRequestClose" && e.payload.is_empty()));
        let orient_ev = modal
            .events
            .iter()
            .find(|e| e.name == "onOrientationChange")
            .expect("onOrientationChange");
        assert_eq!(orient_ev.bubbling, EventBubbling::Direct);
        assert_eq!(orient_ev.payload.len(), 1);
        assert_eq!(orient_ev.payload[0].name, "orientation");
        assert_balanced_delimiters(&out.rust_module.relative_path, &out.rust_module.content);
        assert_balanced_delimiters(&out.cpp_adapter.relative_path, &out.cpp_adapter.content);
    }

    let refresh_path = Path::new(RN_LIBRARIES_DIR)
        .join("Components/RefreshControl/PullToRefreshViewNativeComponent.js");
    if refresh_path.exists() {
        let out = import_rn_file(&refresh_path).expect("import PullToRefreshView");
        let ParsedRnSpec::Component(ptr) = &out.spec else {
            panic!("expected component");
        };
        assert_eq!(ptr.component_name, "PullToRefreshView");
        let refreshing = ptr.props.iter().find(|p| p.name == "refreshing").unwrap();
        assert!(!refreshing.optional);
        let offset = ptr
            .props
            .iter()
            .find(|p| p.name == "progressViewOffset")
            .unwrap();
        assert_eq!(offset.prop_type, PropType::Float);
        assert_eq!(offset.default_value, Some(DefaultValue::Int(0)));
        assert_eq!(ptr.commands.len(), 1);
        assert_eq!(ptr.commands[0].name, "setNativeRefreshing");
    }
}

#[test]
fn parses_real_rn_turbomodule_specs_and_ts_turbomodules() {
    let appearance_path = Path::new(RN_LIBRARIES_DIR).join("Utilities/NativeAppearance.js");
    if appearance_path.exists() {
        let out = import_rn_file(&appearance_path).expect("import NativeAppearance");
        let ParsedRnSpec::TurboModule(tm) = &out.spec else {
            panic!("expected TurboModule");
        };
        assert_eq!(tm.module_name, "Appearance");
        assert!(!tm.enforcing);
        assert!(tm.methods.iter().any(|m| m.name == "getColorScheme"));
        assert!(tm.methods.iter().any(|m| m.name == "setColorScheme"));
        assert!(out.rust_module.content.contains("pub trait AppearanceModule"));
        assert!(out
            .cpp_adapter
            .content
            .contains("class AppearanceTurboModuleHost"));
    }

    let ts_tm = r#"
    import type { TurboModule } from 'react-native';
    import { TurboModuleRegistry } from 'react-native';

    export interface Spec extends TurboModule {
      /** Fetch user score asynchronously. */
      fetchScore(userId: string, force?: boolean): Promise<Double>;
      ping(count: Int32): boolean;
      subscribe(callback: (code: Int32, message: string) => void): void;
    }

    export default TurboModuleRegistry.getEnforcing<Spec>('NveAnalytics');
    "#;

    let out = import_rn_source(ts_tm).expect("import TS TurboModule");
    let ParsedRnSpec::TurboModule(tm) = &out.spec else {
        panic!("expected TurboModule");
    };
    assert_eq!(tm.module_name, "NveAnalytics");
    assert!(tm.enforcing);
    assert_eq!(tm.methods.len(), 3);
    assert_eq!(
        tm.methods[0].return_type,
        PropType::Promise {
            inner: Box::new(PropType::Double)
        }
    );
    assert!(tm.methods[0].params[1].optional);
    assert_balanced_delimiters(&out.rust_module.relative_path, &out.rust_module.content);
    assert_balanced_delimiters(&out.cpp_adapter.relative_path, &out.cpp_adapter.content);
}

#[test]
fn import_rn_handles_ts_edge_cases_comments_keywords_unions_and_negative_defaults() {
    let ts_spec = r#"
    import type { ViewProps, ColorValue } from 'react-native';
    import type {
      BubblingEventHandler,
      DirectEventHandler,
      Double,
      Float,
      Int32,
      WithDefault,
    } from 'react-native/Libraries/Types/CodegenTypes';
    import codegenNativeComponent from 'react-native/Libraries/Utilities/codegenNativeComponent';

    type MetricPayload = Readonly<{
      /* inline block comment */
      latencyMs: Double;
      droppedFrames: Int32;
    }>;

    export interface NativeProps extends ViewProps {
      // Negative integer & float defaults
      offset?: WithDefault<Int32, -1>;
      bias?: WithDefault<Float, -0.5>;
      // Reserved keyword prop names across Rust / C++ / Kotlin / Dart / Swift
      type?: WithDefault<'bar' | 'line' | 'area', 'line'>;
      default?: WithDefault<0 | 1 | 2, 1>;
      tint?: ColorValue | null;
      tags?: ReadonlyArray<string>;
      onMetrics?: DirectEventHandler<MetricPayload, 'topMetrics'>;
      onBubbleTap?: BubblingEventHandler<{ x: Float; y: Float }>;
    }

    export default codegenNativeComponent<NativeProps>('NveSparkline');
    "#;

    let out = import_rn_source(ts_spec).expect("import NveSparkline");
    let ParsedRnSpec::Component(comp) = &out.spec else {
        panic!("expected component");
    };
    assert_eq!(comp.component_name, "NveSparkline");
    assert_eq!(comp.props.len(), 6);
    assert_eq!(comp.events.len(), 2);

    let offset = comp.props.iter().find(|p| p.name == "offset").unwrap();
    assert_eq!(offset.default_value, Some(DefaultValue::Int(-1)));
    let bias = comp.props.iter().find(|p| p.name == "bias").unwrap();
    assert_eq!(bias.default_value, Some(DefaultValue::Float(-0.5)));

    let metrics_ev = comp.events.iter().find(|e| e.name == "onMetrics").unwrap();
    assert_eq!(metrics_ev.bubbling, EventBubbling::Direct);
    assert_eq!(
        metrics_ev.paper_top_level_name.as_deref(),
        Some("topMetrics")
    );
    assert_eq!(metrics_ev.payload.len(), 2);

    // Rust keywords escaped properly (`r#type`) & RawPropValue encoding present
    assert!(out.rust_module.content.contains("pub r#type:"));
    assert!(out.rust_module.content.contains("RawPropValue::I64"));
    assert!(out.rust_module.content.contains("RawPropValue::F64"));
    assert!(out.rust_module.content.contains("pub fn decode_event"));
    assert_balanced_delimiters(&out.rust_module.relative_path, &out.rust_module.content);

    // C++20 adapter aligns with Slice 2's `nve_rn_component.hpp`
    assert!(out
        .cpp_adapter
        .content
        .contains("#include \"nve_rn_component.hpp\""));
    assert!(out
        .cpp_adapter
        .content
        .contains("nve::fabric_host_register_custom_component"));
    assert!(out
        .cpp_adapter
        .content
        .contains("nve::fabric_host_enqueue_custom_event"));
    assert!(out.cpp_adapter.content.contains("default_"));
    assert_balanced_delimiters(&out.cpp_adapter.relative_path, &out.cpp_adapter.content);
}

#[test]
fn export_rn_generates_valid_five_file_package_and_roundtrips_through_import_rn() {
    let spec = counter_card_export_spec(ExportBackend::Fabric);
    let json = spec.to_json_pretty().expect("serialize NveExportSpec");
    let parsed_spec = NveExportSpec::from_json(&json).expect("deserialize NveExportSpec");
    let out = export_rn_spec(&parsed_spec).expect("export_rn_spec");

    assert_eq!(
        out.native_component_ts.relative_path,
        "NveCounterCardNativeComponent.generated.ts"
    );
    assert_eq!(
        out.cpp_bridge_hpp.relative_path,
        "NveCounterCardNveBridge.generated.hpp"
    );
    assert_eq!(
        out.cpp_bridge_cpp.relative_path,
        "NveCounterCardNveBridge.generated.cpp"
    );
    assert_eq!(
        out.ios_component_view_mm.relative_path,
        "NveCounterCardComponentView.generated.mm"
    );
    assert_eq!(
        out.android_view_manager_kt.relative_path,
        "NveCounterCardViewManager.generated.kt"
    );
    assert_eq!(
        out.windows_component_view_hpp.relative_path,
        "NveCounterCardRnwComponentView.generated.hpp"
    );
    assert_eq!(out.files().len(), 6);

    // 1. TypeScript spec assertions
    let ts = &out.native_component_ts.content;
    assert!(ts.contains("export interface NativeProps extends ViewProps"));
    assert!(ts.contains("initialCount?: WithDefault<Int32, 0>;"));
    assert!(ts.contains("accentColor?: ColorValue | null;"));
    assert!(ts.contains("variant?: WithDefault<'solid' | 'outline' | 'ghost', 'solid'>;"));
    assert!(ts.contains("onCountChange?: DirectEventHandler<CountChangeEvent>;"));
    assert!(ts.contains("onReset?: BubblingEventHandler<null>;"));
    assert!(ts.contains("export default codegenNativeComponent<NativeProps>('NveCounterCard');"));
    assert_balanced_delimiters("NveCounterCardNativeComponent.generated.ts", ts);

    // 2. C++20 glue assertions (verbatim match with Slice 2 `nve_rn_component.hpp`, zero JSON)
    let hpp = &out.cpp_bridge_hpp.content;
    let cpp = &out.cpp_bridge_cpp.content;
    assert!(hpp.contains("#include \"nve_rn_component.hpp\""));
    assert!(hpp.contains("std::unique_ptr<nve::NveRnComponentBridge> bridge_;"));
    assert!(hpp.contains("const void* nve_counter_card_vtable(uint32_t interface_id);"));
    assert!(hpp.contains("update_props_direct"));
    assert!(!hpp.contains("update_props_json"));
    assert!(!hpp.contains("payload_json"));
    assert!(cpp.contains("bridge_->update_props(to_prop_map(props), auto_render)"));
    assert!(cpp.contains("bridge_->update_props_direct(entries, count, auto_render)"));
    assert!(cpp.contains("bridge_->drain_emitted_events()"));
    assert!(cpp.contains("ev.payload_bytes"));
    assert_balanced_delimiters("NveCounterCardNveBridge.generated.hpp", hpp);
    assert_balanced_delimiters("NveCounterCardNveBridge.generated.cpp", cpp);

    // 3. iOS ObjC++ assertions (verbatim match with Slice 2 `RCTNveComponentView.h`)
    let mm = &out.ios_component_view_mm.content;
    assert!(mm.contains("#import \"RCTNveComponentView.h\""));
    assert!(mm.contains("@interface NveCounterCardComponentView : RCTNveComponentView"));
    assert!(mm.contains("- (nullable NveGetInterfaceFn)defaultNveGetInterface"));
    assert!(mm.contains("[self applyPropsDictionary:dict];"));
    assert!(mm.contains("[self drainNveEmittedEvents];"));
    assert!(mm.contains("- (void)handleNveEvent:(NSString *)eventName payload:(NSDictionary<NSString *, id> *)payload"));
    assert_balanced_delimiters("NveCounterCardComponentView.generated.mm", mm);

    // 4. Android Kotlin assertions (zero-JSON binary NveBinValue wrapping `dev.nve.host.NveNativeBridge`)
    let kt = &out.android_view_manager_kt.content;
    assert!(kt.contains("package dev.nve.components"));
    assert!(kt.contains("import dev.nve.host.NveNativeBridge"));
    assert!(kt.contains("import dev.nve.compose.NveBinValue"));
    assert!(kt.contains("import dev.nve.compose.NveBinValueView"));
    assert!(!kt.contains("JSONObject"));
    assert!(!kt.contains("org.json"));
    assert!(kt.contains("NveNativeBridge.nativeCreateHost"));
    assert!(kt.contains("@ReactProp(name = \"initialCount\", defaultInt = 0)"));
    assert!(kt.contains("@ReactProp(name = \"accentColor\", customType = \"Color\")"));
    assert!(kt.contains("getExportedCustomDirectEventTypeConstants"));
    assert!(kt.contains("getExportedCustomBubblingEventTypeConstants"));
    assert_balanced_delimiters("NveCounterCardViewManager.generated.kt", kt);

    // 4b. Windows RNW Fabric C++20 header assertions (targeting `NveRnwComponentView.hpp`)
    let rnw = &out.windows_component_view_hpp.content;
    assert!(rnw.contains("#include \"NveRnwComponentView.hpp\""));
    assert!(rnw.contains("class NveCounterCardRnwComponentView"));
    assert!(rnw.contains("nve::win::NveRnwComponentHost"));
    assert!(rnw.contains("register_nve_counter_card_rnw_component"));
    assert!(rnw.contains("nve::win::register_nve_rnw_component"));
    assert!(rnw.contains("L\"NveCounterCard\""));
    assert_balanced_delimiters("NveCounterCardRnwComponentView.generated.hpp", rnw);

    // 5. Full bi-directional round-trip: parse our own generated TS right back via `import_rn`!
    let roundtrip = import_rn_source(ts).expect("round-trip parse generated NativeComponent.ts");
    let ParsedRnSpec::Component(rt) = &roundtrip.spec else {
        panic!("expected component on round-trip");
    };
    assert_eq!(rt.component_name, "NveCounterCard");
    assert_eq!(rt.props.len(), spec.props.len());
    assert_eq!(rt.events.len(), spec.events.len());
    assert_eq!(rt.commands.len(), spec.commands.len());
    assert_eq!(rt.events[0].name, "onCountChange");
    assert_eq!(rt.events[0].payload.len(), 2);
}

#[test]
fn multi_target_export_generates_flutter_swiftui_compose_and_web_component() {
    let skia_spec = counter_card_export_spec(ExportBackend::Skia);
    let fabric_spec = counter_card_export_spec(ExportBackend::Fabric);

    // 1. Flutter (Skia -> NveTextureHostWidget; Fabric -> NvePlatformViewWidget)
    let fl_skia = export_flutter(&skia_spec).expect("export_flutter skia");
    assert_eq!(
        fl_skia.dart_widget.relative_path,
        "nve_counter_card.generated.dart"
    );
    let dart_s = &fl_skia.dart_widget.content;
    assert!(dart_s.contains("import 'dart:ffi' as ffi;"));
    assert!(dart_s.contains("import 'package:nve_flutter/nve_flutter.dart';"));
    assert!(dart_s.contains("class NveCounterCard extends StatefulWidget"));
    assert!(dart_s.contains("return NveTextureHostWidget("));
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
    assert!(fl_fabric
        .dart_widget
        .content
        .contains("return NvePlatformViewWidget("));

    // 2. SwiftUI (`NveSwiftUI.NveSurfaceRepresentable` via zero-boxing `NveDirectEncodable`)
    let sw_out = export_swiftui(&skia_spec).expect("export_swiftui");
    assert_eq!(
        sw_out.swift_view.relative_path,
        "NveCounterCardView.generated.swift"
    );
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

    // 3. Jetpack Compose (`dev.nve.compose.NveSurface` via zero-JSON `NveBinValue` direct writer)
    let cmp_out = export_compose(&fabric_spec).expect("export_compose");
    assert_eq!(
        cmp_out.compose_kt.relative_path,
        "NveCounterCardComposable.generated.kt"
    );
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

    // 4. Web Component (`<nve-counter-card>` + `.d.ts` wrapping `wasm-bindgen`, zero JSON)
    let web_out = export_web_component(&skia_spec).expect("export_web_component");
    assert_eq!(web_out.tag_name, "nve-counter-card");
    assert_eq!(
        web_out.element_js.relative_path,
        "nve-counter-card.generated.js"
    );
    assert_eq!(
        web_out.types_dts.relative_path,
        "nve-counter-card.generated.d.ts"
    );
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

    // Non-`Nve`-prefixed component name gets prefixed with `nve-` automatically
    let plain_spec = NveExportSpec::new("SparkChart", ExportBackend::Skia);
    let plain_web = export_web_component(&plain_spec).expect("plain web component");
    assert_eq!(plain_web.tag_name, "nve-spark-chart");

    // 5. Unified multi-target export + write_to_dir verification
    let all = export_all_targets(&fabric_spec, ExportTarget::ALL).expect("export all targets");
    assert_eq!(all.len(), 5);
    let total_files: usize = all.iter().map(|o| o.files.len()).sum();
    assert_eq!(total_files, 11); // 6 RN (incl. RNW) + 1 Flutter + 1 SwiftUI + 1 Compose + 2 Web

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
    assert!(NveExportSpec::from_json("{invalid json").is_err());
    assert!(parse_rn_spec("export const x = 42;").is_err());
    assert!(ExportTarget::from_name("unknown-platform").is_err());
}

#[test]
fn all_generated_import_and_export_bindings_are_strictly_zero_json() {
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
    // 1. Minimal spec (zero props, zero events, zero commands)
    let empty = NveExportSpec::new("EmptyCanvas", ExportBackend::Skia);
    let empty_all = export_all_targets(&empty, ExportTarget::ALL).expect("export minimal spec");
    assert_eq!(empty_all.len(), 5);
    for bundle in &empty_all {
        for file in &bundle.files {
            assert!(!file.content.is_empty());
            assert_balanced_delimiters(&file.relative_path, &file.content);
        }
    }

    // 2. Complex spec with IntEnum, Double, Point, EdgeInsets, Array, Object, and optional payload fields
    let mut complex = NveExportSpec::new("DataGrid", ExportBackend::Fabric);
    complex.props = vec![
        PropSpec {
            name: "density".to_owned(),
            prop_type: PropType::IntEnum {
                variants: vec![0, 1, 2],
            },
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
            prop_type: PropType::Array {
                item: Box::new(PropType::String),
            },
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
fn generated_cpp20_compiles_cleanly_against_real_nve_rn_component_header() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.join("../../cxx/nve-fabric-bridge/include");
    if !include_dir.join("nve_rn_component.hpp").exists() {
        return;
    }

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

    // 1. Check Direction 1 (`import_rn`) C++20 Fabric adapter header
    let imported = import_rn_source(EMBEDDED_SWITCH_SPEC).expect("import Switch");
    let adapter_path = tmp.join(&imported.cpp_adapter.relative_path);
    std::fs::write(&adapter_path, &imported.cpp_adapter.content).unwrap();

    let import_harness = tmp.join("check_import.cpp");
    std::fs::write(
        &import_harness,
        format!(
            "#include \"{}\"\nvoid instantiate(NveFabricHostHandle h) {{\n  nve::generated::register_switch_adapter(h, [](uint32_t, const nve::generated::SwitchProps&) {{}});\n}}\n",
            imported.cpp_adapter.relative_path
        ),
    )
    .unwrap();

    let status_import = std::process::Command::new("clang++")
        .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(&include_dir)
        .arg("-I")
        .arg(&tmp)
        .arg(&import_harness)
        .output()
        .expect("run clang++ on import_rn adapter");
    assert!(
        status_import.status.success(),
        "import_rn C++20 adapter failed to compile:\n{}",
        String::from_utf8_lossy(&status_import.stderr)
    );

    // 2. Check Direction 2 (`export_rn`) C++20 bridge `.hpp` + `.cpp`
    let spec = counter_card_export_spec(ExportBackend::Fabric);
    let exported = export_rn_spec(&spec).expect("export NveCounterCard");
    exported.write_to_dir(&tmp).unwrap();

    let status_export = std::process::Command::new("clang++")
        .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
        .arg("-I")
        .arg(&include_dir)
        .arg("-I")
        .arg(&tmp)
        .arg(tmp.join(&exported.cpp_bridge_cpp.relative_path))
        .output()
        .expect("run clang++ on export_rn bridge");
    assert!(
        status_export.status.success(),
        "export_rn C++20 bridge failed to compile:\n{}",
        String::from_utf8_lossy(&status_export.stderr)
    );

    // 3. Check RNW Fabric C++20 header compiles cleanly against real `hosts/windows-shell/NveRnwComponentView.hpp`
    let windows_include_dir = manifest_dir.join("../../hosts/windows-shell");
    if windows_include_dir.join("NveRnwComponentView.hpp").exists() {
        let rnw_harness = tmp.join("check_rnw.cpp");
        std::fs::write(
            &rnw_harness,
            format!(
                "#include \"{}\"\nvoid instantiate_rnw() {{\n  nve::exported::NveCounterCardRnwComponentView view;\n  (void)view;\n}}\n",
                exported.windows_component_view_hpp.relative_path
            ),
        )
        .unwrap();
        let status_rnw = std::process::Command::new("clang++")
            .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(&include_dir)
            .arg("-I")
            .arg(&windows_include_dir)
            .arg("-I")
            .arg(&tmp)
            .arg(&rnw_harness)
            .output()
            .expect("run clang++ on export_rn RNW component view");
        assert!(
            status_rnw.status.success(),
            "export_rn RNW C++20 component view failed to compile:\n{}",
            String::from_utf8_lossy(&status_rnw.stderr)
        );
    }

    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn unified_core_and_scene_abi_generated_files_match_checked_in_artifacts_byte_for_byte() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let core_files = generate_all_core_abi_files();
    let scene_files = generate_all_scene_abi_files();
    let all_files = generate_all_abi_files();

    assert_eq!(core_files.len(), 5);
    assert_eq!(scene_files.len(), 5);
    assert_eq!(all_files.len(), 15);

    // Cross-check both deterministic FNV-1a schema hashes against `crates/nve-abi`.
    assert_ne!(NVE_CORE_ABI_SCHEMA_HASH, 0);
    assert_eq!(core_abi_schema_hash(), NVE_CORE_ABI_SCHEMA_HASH);
    assert_eq!(NVE_CORE_ABI_SCHEMA_HASH, nve_abi::NVE_CORE_ABI_SCHEMA_HASH);

    assert_ne!(NVE_SCENE_SCHEMA_HASH, 0);
    assert_eq!(compute_scene_schema_hash(&scene_abi_spec()), NVE_SCENE_SCHEMA_HASH);
    assert_eq!(NVE_SCENE_SCHEMA_HASH, nve_abi::NVE_SCENE_SCHEMA_HASH);

    // C and Swift headers must be byte-for-byte identical copies.
    assert_eq!(core_files[0].content, core_files[1].content);
    assert_eq!(scene_files[0].content, scene_files[1].content);

    // Verify NVE_IFACE_HOST = 5, NveHostStatus, NveHostCallbacks, and NveHostApi across all 5 Core ABI targets.
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

    let update_mode =
        std::env::var("NVE_UPDATE_ABI").is_ok() || std::env::var("NVE_UPDATE_SCENE_ABI").is_ok();
    for file in &all_files {
        let fname = Path::new(&file.relative_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap();
        if file.relative_path.ends_with(".java") {
            assert!(
                file.relative_path.contains("/generated/"),
                "generated Java file must live inside a `generated/` package directory: {}",
                file.relative_path
            );
            assert!(
                file.content.contains("package dev.nve.android.generated;"),
                "generated Java file must declare `package dev.nve.android.generated;`: {}",
                file.relative_path
            );
        } else {
            assert!(
                fname.contains(".generated."),
                "generated non-Java file must contain `.generated.` in filename: {}",
                file.relative_path
            );
        }
        let full_path = repo_root.join(&file.relative_path);
        if update_mode {
            if let Some(parent) = full_path.parent() {
                std::fs::create_dir_all(parent).expect("create parent dirs");
            }
            std::fs::write(&full_path, &file.content).expect("write generated ABI file");
        }
        let on_disk = std::fs::read_to_string(&full_path)
            .unwrap_or_else(|e| {
                panic!(
                    "missing generated ABI file {}: {e}",
                    full_path.display()
                )
            })
            .replace("\r\n", "\n");
        assert_eq!(
            on_disk, file.content,
            "ABI schema drift detected in {}! Re-run with NVE_UPDATE_ABI=1 or `cargo nve codegen abi` to regenerate.",
            file.relative_path
        );
    }
}

#[test]
fn naming_escapers_neutralize_interpolation_quotes_and_comment_breakout() {
    use nve_codegen::naming::{
        escape_c_like_string, escape_dart_single_quoted_string, escape_kotlin_string,
        escape_ts_single_quoted, format_float_literal, sanitize_block_comment,
        sanitize_single_line_comment,
    };

    assert_eq!(
        escape_kotlin_string("$HOME/${expr}\"\\a\n\r\t\0"),
        "\\$HOME/\\${expr}\\\"\\\\a\\n\\r\\t\\u0000"
    );
    assert_eq!(
        escape_dart_single_quoted_string("$HOME/${expr}'\\a\n\r\t\0"),
        "\\$HOME/\\${expr}\\'\\\\a\\n\\r\\t\\u0000"
    );
    assert_eq!(
        escape_c_like_string("\\(swift)\"\\a\n\r\t\0"),
        "\\\\(swift)\\\"\\\\a\\n\\r\\t\\0"
    );
    assert_eq!(
        escape_ts_single_quoted("it's\\a\n\r\t\0\u{2028}\u{2029}"),
        "it\\'s\\\\a\\n\\r\\t\\u0000\\u2028\\u2029"
    );

    let sanitized_line = sanitize_single_line_comment("tag\r\nevil();\u{2028}more\u{2029}end");
    assert!(!sanitized_line.contains('\n'));
    assert!(!sanitized_line.contains('\r'));
    assert!(!sanitized_line.contains('\u{2028}'));
    assert!(!sanitized_line.contains('\u{2029}'));

    let sanitized_block = sanitize_block_comment("hello */ evil();\r\n/* ok");
    assert!(!sanitized_block.contains("*/"));
    assert!(!sanitized_block.contains('\r'));

    assert_eq!(format_float_literal(2.0), "2.0");
    assert_eq!(format_float_literal(f64::NAN), "0.0");
    assert_eq!(format_float_literal(f64::INFINITY), "0.0");
}

#[test]
fn export_and_import_neutralize_string_interpolation_and_comment_breakout() {
    let mut spec = NveExportSpec::new("AdversarialWidget", ExportBackend::Skia);
    spec.doc = Some("Top doc */ evil();\nsecond line\r\nthird".to_owned());
    spec.props = vec![
        PropSpec {
            name: "templateText".to_owned(),
            prop_type: PropType::String,
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::String(
                "$HOME/${System.exit(1)} it's \\ \"quoted\" \\(swift)\nnext\r\tline".to_owned(),
            )),
            doc: Some("Prop doc line1\n*/ evil();\n// breakout".to_owned()),
        },
        PropSpec {
            name: "mode".to_owned(),
            prop_type: PropType::StringEnum {
                variants: vec![
                    "$HOME".to_owned(),
                    "${expr}".to_owned(),
                    "it's\\quoted\"mode\nbreak".to_owned(),
                ],
            },
            optional: true,
            nullable: false,
            default_value: Some(DefaultValue::String("$HOME".to_owned())),
            doc: None,
        },
    ];
    spec.events = vec![EventSpec {
        name: "onPayload".to_owned(),
        bubbling: EventBubbling::Direct,
        optional: true,
        paper_top_level_name: Some("top$HOME'\"\\".to_owned()),
        payload: vec![ObjectField {
            name: "detail".to_owned(),
            field_type: PropType::String,
            optional: false,
            nullable: false,
        }],
        payload_type_name: None,
        doc: Some("Event doc */ breakout\nnext".to_owned()),
    }];

    // 1. Jetpack Compose (Kotlin) — `$` must be escaped so `$HOME` / `${...}` never interpolate.
    let compose = export_compose(&spec).expect("export_compose");
    let kt = &compose.compose_kt.content;
    assert!(kt.contains("\\$HOME/\\${System.exit(1)}"));
    assert!(!kt.replace("\\$", "").contains("$HOME"));
    assert!(!kt.replace("\\$", "").contains("${System.exit(1)}"));
    assert!(!kt.contains("*/ evil();"));
    assert_balanced_delimiters(&compose.compose_kt.relative_path, kt);

    // 2. React Native Android ViewManager (Kotlin) — `$` in top-level event name escaped.
    let rn = export_rn_spec(&spec).expect("export_rn_spec");
    let rn_kt = &rn.android_view_manager_kt.content;
    assert!(rn_kt.contains("top\\$HOME"));
    assert!(!rn_kt.replace("\\$", "").contains("top$HOME"));
    assert!(!rn.native_component_ts.content.contains("*/ evil();"));
    assert_balanced_delimiters(&rn.native_component_ts.relative_path, &rn.native_component_ts.content);
    assert_balanced_delimiters(&rn.cpp_bridge_hpp.relative_path, &rn.cpp_bridge_hpp.content);
    assert_balanced_delimiters(&rn.cpp_bridge_cpp.relative_path, &rn.cpp_bridge_cpp.content);
    assert_balanced_delimiters(&rn.ios_component_view_mm.relative_path, &rn.ios_component_view_mm.content);
    assert_balanced_delimiters(&rn.android_view_manager_kt.relative_path, rn_kt);
    assert_balanced_delimiters(&rn.windows_component_view_hpp.relative_path, &rn.windows_component_view_hpp.content);

    // 3. Flutter (Dart) — `$` and `'` escaped inside single-quoted literals; multiline doc kept commented.
    let flutter = export_flutter(&spec).expect("export_flutter");
    let dart = &flutter.dart_widget.content;
    assert!(dart.contains("\\$HOME/\\${System.exit(1)} it\\'s \\\\ \"quoted\""));
    assert!(!dart.replace("\\$", "").contains("'$HOME"));
    assert!(!dart.replace("\\$", "").contains("${System.exit(1)}"));
    for line in dart.lines() {
        assert!(
            !line.trim_start().starts_with("*/ evil();"),
            "comment breakout in Dart output: {line}"
        );
    }
    assert_balanced_delimiters(&flutter.dart_widget.relative_path, dart);

    // 4. SwiftUI (Swift) — `\` escaped so `\(swift)` becomes `\\(swift)` and never interpolates.
    let swiftui = export_swiftui(&spec).expect("export_swiftui");
    let swift = &swiftui.swift_view.content;
    assert!(swift.contains("\\\\(swift)"));
    for line in swift.lines() {
        assert!(
            !line.trim_start().starts_with("*/ evil();"),
            "comment breakout in Swift output: {line}"
        );
    }
    assert_balanced_delimiters(&swiftui.swift_view.relative_path, swift);

    // 5. Web Component (JS + d.ts) — single quotes & newlines escaped; header comments single-line.
    let web = export_web_component(&spec).expect("export_web_component");
    assert!(web.element_js.content.contains("it\\'s"));
    assert!(web.element_js.content.contains("\\nnext\\r\\tline"));
    assert_balanced_delimiters(&web.element_js.relative_path, &web.element_js.content);
    assert_balanced_delimiters(&web.types_dts.relative_path, &web.types_dts.content);

    // 6. Real C++20 compiler verification of adversarial `export_rn` C++ bridge!
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let include_dir = manifest_dir.join("../../cxx/nve-fabric-bridge/include");
    if include_dir.join("nve_rn_component.hpp").exists()
        && std::process::Command::new("clang++")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    {
        let tmp = std::env::temp_dir().join(format!(
            "nve_codegen_adversarial_cpp_{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();
        rn.write_to_dir(&tmp).unwrap();
        let out = std::process::Command::new("clang++")
            .args(["-std=c++20", "-fsyntax-only", "-Wall", "-Wextra", "-Werror"])
            .arg("-I")
            .arg(&include_dir)
            .arg("-I")
            .arg(&tmp)
            .arg(tmp.join(&rn.cpp_bridge_cpp.relative_path))
            .output()
            .expect("clang++ adversarial export_rn");
        assert!(
            out.status.success(),
            "adversarial export_rn C++20 failed:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    // 7. `import_rn` with comment-breakout component name & adversarial default string.
    let adversarial_rn_spec = r#"
    import type { ViewProps } from 'react-native';
    import type { WithDefault } from 'react-native/Libraries/Types/CodegenTypes';
    import codegenNativeComponent from 'react-native/Libraries/Utilities/codegenNativeComponent';

    export interface NativeProps extends ViewProps {
      label?: WithDefault<string, '$HOME/${x} it\'s "quoted"\nline2'>;
    }

    export default codegenNativeComponent<NativeProps>('EvilTag\nfn injected() {} //');
    "#;
    let imported = import_rn_source(adversarial_rn_spec).expect("import adversarial RN spec");
    for line in imported.rust_module.content.lines() {
        assert!(
            !line.trim_start().starts_with("fn injected()"),
            "comment breakout in import_rn Rust output: {line}"
        );
    }
    for line in imported.cpp_adapter.content.lines() {
        assert!(
            !line.trim_start().starts_with("fn injected()"),
            "comment breakout in import_rn C++ output: {line}"
        );
    }
    assert_balanced_delimiters(&imported.rust_module.relative_path, &imported.rust_module.content);
    assert_balanced_delimiters(&imported.cpp_adapter.relative_path, &imported.cpp_adapter.content);
}

#[test]
fn fuzz_rn_lexer_and_parser_never_panic_on_adversarial_and_truncated_inputs() {
    use nve_codegen::lexer::tokenize;

    // 1. Every UTF-8 prefix truncation of the embedded Switch spec
    for (byte_idx, _) in EMBEDDED_SWITCH_SPEC.char_indices() {
        let prefix = &EMBEDDED_SWITCH_SPEC[..byte_idx];
        let _ = tokenize(prefix);
        let _ = parse_rn_spec(prefix);
    }

    // 2. Deeply nested recursive constructs (must error cleanly instead of stack-overflowing)
    let deep_cases = [
        format!(
            "interface P {{ a: {}string{}; }} export default codegenNativeComponent<P>('X');",
            "(".repeat(256),
            ")".repeat(256)
        ),
        format!(
            "interface P {{ a: {}string; }} export default codegenNativeComponent<P>('X');",
            "?".repeat(256)
        ),
        format!(
            "interface P {{ a: {}string{}; }} export default codegenNativeComponent<P>('X');",
            "Readonly<".repeat(256),
            ">".repeat(256)
        ),
        format!(
            "interface P {{ {}x: string{} }} export default codegenNativeComponent<P>('X');",
            "a: { ".repeat(200),
            " }".repeat(200)
        ),
        "/* unterminated block".to_owned(),
        "'unterminated string\\".to_owned(),
        "\"unterminated".to_owned(),
        "`unterminated".to_owned(),
        "// comment without newline".to_owned(),
        "interface A extends A { a: A } export default codegenNativeComponent<A>('A');".to_owned(),
        "type A = A; interface P { x: A } export default codegenNativeComponent<P>('A');".to_owned(),
    ];
    for case in &deep_cases {
        let _ = tokenize(case);
        let _ = parse_rn_spec(case);
    }

    // 3. 1,200 deterministic pseudo-random adversarial token/character streams
    let alphabet: &[&str] = &[
        "export", "default", "interface", "type", "const", "extends", "readonly",
        "codegenNativeComponent", "codegenNativeCommands", "TurboModuleRegistry",
        "getEnforcing", "get", "WithDefault", "DirectEventHandler", "BubblingEventHandler",
        "Readonly", "ViewProps", "string", "boolean", "Int32", "Float", "Double",
        "null", "undefined", "void", "true", "false", "{", "}", "{|", "|}", "(", ")",
        "[", "]", "<", ">", ":", ";", ",", ".", "...", "=>", "?", "|", "&", "=", "-", "+",
        "'str'", "\"$HOME/${x}\"", "'unterm", "/*", "*/", "//\n", "123", "-99999999999999999999",
        "0xZZ", " ", "\n", "\t", "🦀",
    ];
    let mut seed: u64 = 0x9E37_79B9_7F4A_7C15;
    let mut next_u64 = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };

    for i in 0..1200 {
        let len = (next_u64() as usize % 36) + 1;
        let mut s = String::with_capacity(len * 8);
        for _ in 0..len {
            let tok = alphabet[(next_u64() as usize) % alphabet.len()];
            s.push_str(tok);
            if (next_u64() & 1) == 0 {
                s.push(' ');
            }
        }
        let _ = tokenize(&s);
        let _ = parse_rn_spec(&s);
        // Also test raw byte-level arbitrary unicode slices every 4th iteration
        if i % 4 == 0 && !s.is_empty() {
            let cut = s.char_indices().nth((next_u64() as usize) % s.chars().count()).map(|(b, _)| b).unwrap_or(0);
            let _ = parse_rn_spec(&s[..cut]);
        }
    }
}
