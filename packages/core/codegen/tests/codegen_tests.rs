mod common;

use common::specs::{assert_balanced_delimiters, counter_card_export_spec, EMBEDDED_SWITCH_SPEC};
use nve_codegen::{
    export_rn_spec, import_rn_file, import_rn_source, parse_rn_spec, parse_rn_spec_file,
    DefaultValue, EventBubbling, ExportBackend, NveExportSpec, ParsedRnSpec, PropType,
};

/// `rel` inside an optional checkout of React Native's `packages/react-native/Libraries`, named by
/// `NVE_RN_LIBRARIES_DIR`. Unset (or empty) returns `None`, and the real-spec checks are skipped
/// or use the embedded Switch spec. A set variable must name a real checkout: a typo fails loudly
/// instead of silently skipping every real-spec check.
fn rn_spec(rel: &str) -> Option<std::path::PathBuf> {
    let dir = std::path::PathBuf::from(std::env::var_os("NVE_RN_LIBRARIES_DIR").filter(|v| !v.is_empty())?);
    assert!(dir.is_dir(), "NVE_RN_LIBRARIES_DIR={} is not a directory", dir.display());
    let path = dir.join(rel);
    assert!(path.is_file(), "NVE_RN_LIBRARIES_DIR has no {rel}");
    Some(path)
}

#[test]
fn parses_real_rn_component_specs_and_follows_reexports() {
    let switch_spec = match rn_spec("Components/Switch/SwitchNativeComponent.js") {
        Some(path) => parse_rn_spec_file(&path).expect("parse real SwitchNativeComponent.js"),
        None => parse_rn_spec(EMBEDDED_SWITCH_SPEC).expect("parse embedded Switch spec"),
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
    if let Some(activity_path) = rn_spec("Components/ActivityIndicator/ActivityIndicatorViewNativeComponent.js") {
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

    if let Some(modal_path) = rn_spec("Modal/RCTModalHostViewNativeComponent.js") {
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

    if let Some(refresh_path) = rn_spec("Components/RefreshControl/PullToRefreshViewNativeComponent.js") {
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
    if let Some(appearance_path) = rn_spec("Utilities/NativeAppearance.js") {
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

    // Rust keywords escaped properly (`r#type`) & NveValue binary encoding present
    assert!(out.rust_module.content.contains("pub r#type:"));
    assert!(out.rust_module.content.contains("NveValue::i64"));
    assert!(out.rust_module.content.contains("NveValue::f64"));
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
fn import_rn_array_of_objects_preencodes_buffers_and_propagates_decode_error() {
    let ts_spec = r#"
    import type { ViewProps } from 'react-native';
    import type { Float } from 'react-native/Libraries/Types/CodegenTypes';
    import codegenNativeComponent from 'react-native/Libraries/Utilities/codegenNativeComponent';

    export interface NativeProps extends ViewProps {
      points: ReadonlyArray<{ label: string; score: Float }>;
      optPoints?: ReadonlyArray<{ label: string; score: Float }>;
    }

    export default codegenNativeComponent<NativeProps>('NveSeries');
    "#;

    let out = import_rn_source(ts_spec).expect("import NveSeries");
    let rs = &out.rust_module.content;
    assert!(rs.contains("pub fn encode_props(&self) -> Result<Vec<u8>, DecodeError>"));
    assert!(rs.contains("pub fn to_bin_bytes(&self) -> Result<Vec<u8>, DecodeError>"));
    assert!(
        rs.contains("let encoded_items: Vec<Vec<u8>> ="),
        "Array<Object> must pre-encode element buffers before building Vec<NveValue>: {rs}"
    );
    assert!(
        !rs.contains("encode_bin_value_to_vec(unsafe { NveValueRef::from_raw(&map_val) }).unwrap_or_default()"),
        "import_rn must propagate DecodeError instead of swallowing via unwrap_or_default"
    );
    assert_balanced_delimiters(&out.rust_module.relative_path, rs);
}

#[test]
fn export_rn_generates_valid_five_file_package_and_roundtrips_through_import_rn() {
    let spec = counter_card_export_spec(ExportBackend::Fabric);
    let json = serde_json::to_string_pretty(&spec).expect("serialize NveExportSpec");
    let parsed_spec: NveExportSpec = serde_json::from_str(&json).expect("deserialize NveExportSpec");
    parsed_spec.validate().expect("round-tripped spec validates");
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

    // 2. C++20 glue assertions (verbatim match with Slice 2 `nve_rn_component.hpp`, binary FFI)
    let hpp = &out.cpp_bridge_hpp.content;
    let cpp = &out.cpp_bridge_cpp.content;
    assert!(hpp.contains("#include \"nve_rn_component.hpp\""));
    assert!(hpp.contains("std::unique_ptr<nve::NveRnComponentBridge> bridge_;"));
    assert!(hpp.contains("const void* nve_counter_card_vtable(uint32_t interface_id);"));
    assert!(hpp.contains("const NveMapEntry* entries"));
    assert!(!hpp.contains("to_prop_map"));
    assert!(!hpp.contains("update_props_json"));
    assert!(!hpp.contains("payload_json"));
    assert!(cpp.contains("std::array<NveMapEntry, 5u> entries{};"));
    assert!(cpp.contains("bridge_->update_props(entries, count, auto_render)"));
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

    // 4. Android Kotlin assertions (binary NveBinValue wrapping `dev.nve.host.NveNativeBridge`)
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
    assert!(rnw.contains("std::shared_ptr<nve::win::NveWinHost> host_;"));
    assert!(rnw.contains("struct NveCounterCardRnwEventCallbacks"));
    assert!(!rnw.contains("NveRnwComponentHost"));
    assert!(!rnw.contains("NveBridge.generated.hpp"));  // never pulls the RN Fabric C++ bridge in
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
