//! Deep integration tests for `nve-dioxus` Trait Environment (`TraitProvider`, `TraitScope`,
//! `TraitValueScope`, `use_trait`, `use_environment`, `PlatformOnly`, `TraitSwitch`,
//! breakpoint-only re-rendering) and First-Class `<NativeView>` escape hatches (`NativeView`,
//! `NativeViewProvider`, `NativeViewHandle`, `WebElement` `"web.*"` wildcards, graceful fallback).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

use nve_abi::{
    Command, CommandDecoder, ComponentKind, Dimension, EventPayload, NativeEvent, NativeEventKind,
    NodeId, RawPropValue,
};
use nve_dioxus::native_view::{
    DecodeNativeCommand, DecodeNativeProps, DecodedWebElementProps, NATIVE_CMD_MAGIC,
    NATIVE_PROPS_MAGIC, NATIVE_VIEW_KIND_PROP_KEY, NATIVE_VIEW_PROPS_PROP_KEY,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;

// Marker custom TraitKeys with NO Clone/PartialEq derives (verifies ergonomic TraitKey bounds).
struct AccentToken;
impl TraitKey for AccentToken {
    type Value = String;
    fn default_value() -> String {
        "default-blue".to_owned()
    }
}

struct SectionDepth;
impl TraitKey for SectionDepth {
    type Value = u32;
    fn default_value() -> u32 {
        0
    }
}

fn collect_texts(bytes: &[u8]) -> Vec<String> {
    let (_, cmds) = CommandDecoder::new(bytes)
        .expect("valid header")
        .decode_all()
        .expect("valid commands");
    cmds.into_iter()
        .filter_map(|c| match c {
            Command::SetText { text, .. } => Some(text),
            _ => None,
        })
        .collect()
}

fn find_test_id(bytes: &[u8], target: &str) -> Option<NodeId> {
    let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
    cmds.into_iter().find_map(|c| match c {
        Command::SetRawProp {
            node_id,
            key,
            value: RawPropValue::String(v),
        } if key == "testID" && v == target => Some(node_id),
        _ => None,
    })
}

// =============================================================================
// 1. Nested <TraitScope> shadowing, restoration, custom keys, PlatformOnly & TraitSwitch
// =============================================================================

#[component]
fn ScopeProbe(label: &'static str) -> Element {
    let scheme = use_trait::<ColorScheme>();
    let density = use_trait::<UiDensity>();
    let accent = use_trait::<AccentToken>();
    let depth = use_trait::<SectionDepth>();
    let env = use_environment();
    let resolved = env
        .resolve::<&'static str>()
        .when_dark("dark-mode")
        .when_light("light-mode")
        .default("other");
    rsx! {
        text { "{label}:{scheme:?}:{density:?}:{accent}:{depth}:{resolved}" }
    }
}

#[component]
fn NestedTraitScopeApp() -> Element {
    let _store = use_provide_trait_store(|| {
        let mut s = TraitStore::new();
        s.set::<PlatformOs>(PlatformOs::MacOs);
        s.set::<HostToolkit>(HostToolkit::AppKit);
        s.set::<ColorScheme>(ColorScheme::Light);
        s.set::<UiDensity>(UiDensity::Comfortable);
        s.set::<SizeClass>(SizeClass::Expanded);
        s.set::<AccentToken>("root-cobalt".to_owned());
        s
    });

    rsx! {
        view {
            ScopeProbe { label: "root" }
            TraitScope {
                overrides: TraitOverrides::new()
                    .with::<ColorScheme>(ColorScheme::Dark)
                    .with::<AccentToken>("outer-violet".to_owned())
                    .with::<SectionDepth>(1),
                ScopeProbe { label: "outer-before" }
                TraitScope {
                    overrides: TraitOverrides::new()
                        .with::<UiDensity>(UiDensity::Compact)
                        .with::<SectionDepth>(2),
                    ScopeProbe { label: "inner" }
                    TraitValueScope::<AccentToken> {
                        value: "leaf-emerald".to_owned(),
                        ScopeProbe { label: "leaf" }
                    }
                }
                // Sibling after inner scope must see outer scope restored cleanly!
                ScopeProbe { label: "outer-after" }
            }
            // Sibling after outer scope must see root scope restored cleanly!
            ScopeProbe { label: "root-after" }

            // PlatformOnly matching & fallback
            PlatformOnly {
                os: PlatformOs::MacOs,
                toolkit: HostToolkit::AppKit,
                fallback: rsx! { text { "plat:wrong" } },
                text { "plat:macos-appkit" }
            }
            PlatformOnly {
                os: PlatformOs::Ios,
                fallback: rsx! { text { "plat:ios-fallback" } },
                text { "plat:ios-only" }
            }

            // TraitSwitch for both built-in SizeClass and custom AccentToken
            TraitSwitch::<SizeClass> {
                cases: vec![
                    (SizeClass::Compact, rsx! { text { "switch:compact" } }),
                    (SizeClass::Expanded, rsx! { text { "switch:expanded" } }),
                ],
                fallback: rsx! { text { "switch:fallback" } },
            }
            TraitSwitch::<AccentToken> {
                cases: vec![
                    ("root-cobalt".to_owned(), rsx! { text { "custom-switch:cobalt" } }),
                ],
                fallback: rsx! { text { "custom-switch:miss" } },
            }
        }
    }
}

#[test]
fn nested_trait_scope_shadowing_restoration_platform_only_and_trait_switch() {
    let mut surface = DioxusSurface::new(1, NestedTraitScopeApp);
    let (_, _, frame) = surface.render_frame(1);
    let texts = collect_texts(&frame);

    assert!(texts.contains(&"root:Light:Comfortable:root-cobalt:0:light-mode".to_owned()));
    assert!(texts.contains(&"outer-before:Dark:Comfortable:outer-violet:1:dark-mode".to_owned()));
    assert!(texts.contains(&"inner:Dark:Compact:outer-violet:2:dark-mode".to_owned()));
    assert!(texts.contains(&"leaf:Dark:Compact:leaf-emerald:2:dark-mode".to_owned()));
    assert!(texts.contains(&"outer-after:Dark:Comfortable:outer-violet:1:dark-mode".to_owned()));
    assert!(texts.contains(&"root-after:Light:Comfortable:root-cobalt:0:light-mode".to_owned()));

    assert!(texts.contains(&"plat:macos-appkit".to_owned()));
    assert!(!texts.contains(&"plat:wrong".to_owned()));
    assert!(texts.contains(&"plat:ios-fallback".to_owned()));
    assert!(!texts.contains(&"plat:ios-only".to_owned()));

    assert!(texts.contains(&"switch:expanded".to_owned()));
    assert!(!texts.contains(&"switch:compact".to_owned()));
    assert!(texts.contains(&"custom-switch:cobalt".to_owned()));
}

// =============================================================================
// 2. Breakpoint-Only Reactivity inside Dioxus VirtualDom (1px drag = 0 re-renders!)
// =============================================================================

thread_local! {
    static SIZE_CLASS_RENDERS: Cell<usize> = const { Cell::new(0) };
    static ORIENTATION_RENDERS: Cell<usize> = const { Cell::new(0) };
    static CONTAINER_RENDERS: Cell<usize> = const { Cell::new(0) };
}

#[component]
fn SizeClassSubscriber() -> Element {
    let sc = use_trait::<SizeClass>();
    SIZE_CLASS_RENDERS.with(|c| c.set(c.get() + 1));
    rsx! { text { "sc:{sc:?}" } }
}

#[component]
fn OrientationSubscriber() -> Element {
    let ori = use_trait::<Orientation>();
    ORIENTATION_RENDERS.with(|c| c.set(c.get() + 1));
    rsx! { text { "ori:{ori:?}" } }
}

#[component]
fn ContainerSubscriber() -> Element {
    let cs = use_trait::<ContainerSize>();
    CONTAINER_RENDERS.with(|c| c.set(c.get() + 1));
    rsx! { text { "cs:{cs.width:.0}x{cs.height:.0}" } }
}

#[component]
fn BreakpointReactivityApp() -> Element {
    let store = use_provide_trait_store(|| {
        let mut s = TraitStore::new();
        s.update_viewport(1200.0, 800.0);
        s
    });
    let s1 = store.clone();
    let s2 = store.clone();
    let s3 = store.clone();

    rsx! {
        view {
            SizeClassSubscriber {}
            OrientationSubscriber {}
            ContainerSubscriber {}
            pressable {
                test_id: "resize-1201",
                onpress: move |_| {
                    s1.update_viewport(1201.0, 800.0);
                },
            }
            pressable {
                test_id: "resize-599",
                onpress: move |_| {
                    s2.update_viewport(599.0, 800.0);
                },
            }
            pressable {
                test_id: "resize-600",
                onpress: move |_| {
                    s3.update_viewport(600.0, 800.0);
                },
            }
        }
    }
}

#[test]
fn breakpoint_only_dioxus_reactivity_skips_rerenders_on_1px_drag() {
    SIZE_CLASS_RENDERS.with(|c| c.set(0));
    ORIENTATION_RENDERS.with(|c| c.set(0));
    CONTAINER_RENDERS.with(|c| c.set(0));

    let mut surface = DioxusSurface::new(1, BreakpointReactivityApp);
    let (_, _, f1) = surface.render_frame(1);
    let t1 = collect_texts(&f1);
    assert!(t1.contains(&"sc:Expanded".to_owned()));
    assert!(t1.contains(&"ori:Landscape".to_owned()));
    assert!(t1.contains(&"cs:1200x800".to_owned()));
    assert_eq!(SIZE_CLASS_RENDERS.with(Cell::get), 1);
    assert_eq!(ORIENTATION_RENDERS.with(Cell::get), 1);
    assert_eq!(CONTAINER_RENDERS.with(Cell::get), 1);

    let btn_1201 = find_test_id(&f1, "resize-1201").unwrap();
    let btn_599 = find_test_id(&f1, "resize-599").unwrap();
    let btn_600 = find_test_id(&f1, "resize-600").unwrap();

    // 1px drag 1200x800 -> 1201x800: SizeClass & Orientation subscribers MUST NOT re-render!
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: btn_1201,
        timestamp_us: 100,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let (_, _, f2) = surface.render_frame(2);
    assert!(collect_texts(&f2).contains(&"cs:1201x800".to_owned()));
    assert_eq!(
        SIZE_CLASS_RENDERS.with(Cell::get),
        1,
        "1px drag within Expanded must NOT re-render SizeClass subscriber"
    );
    assert_eq!(
        ORIENTATION_RENDERS.with(Cell::get),
        1,
        "1px drag within Landscape must NOT re-render Orientation subscriber"
    );
    assert_eq!(CONTAINER_RENDERS.with(Cell::get), 2);

    // Resize to 599x800 -> crosses SizeClass (Expanded->Compact) AND Orientation (Landscape->Portrait)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: btn_599,
        timestamp_us: 200,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let (_, _, f3) = surface.render_frame(3);
    let t3 = collect_texts(&f3);
    assert!(t3.contains(&"sc:Compact".to_owned()));
    assert!(t3.contains(&"ori:Portrait".to_owned()));
    assert_eq!(SIZE_CLASS_RENDERS.with(Cell::get), 2);
    assert_eq!(ORIENTATION_RENDERS.with(Cell::get), 2);

    // Cross 599 -> 600 -> bumps SizeClass (Compact->Medium), does NOT bump Orientation (Portrait->Portrait)
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: btn_600,
        timestamp_us: 300,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let (_, _, f4) = surface.render_frame(4);
    assert!(collect_texts(&f4).contains(&"sc:Medium".to_owned()));
    assert_eq!(SIZE_CLASS_RENDERS.with(Cell::get), 3);
    assert_eq!(
        ORIENTATION_RENDERS.with(Cell::get),
        2,
        "599x800 -> 600x800 stays Portrait so Orientation subscriber must not re-render"
    );
}

// =============================================================================
// 3. <NativeView> Native Emission, Binary Props, Handle Auto-Binding/Unbinding,
//    WebElement "web.*" Wildcards & Graceful Fallback Rendering
// =============================================================================

thread_local! {
    static ACTIVE_REGISTRY: RefCell<NativeViewRegistry> = RefCell::new(NativeViewRegistry::new());
    static CAPTURED_WV_HANDLE: RefCell<Option<NativeViewHandle<WebView>>> = const { RefCell::new(None) };
    static CAPTURED_VP_HANDLE: RefCell<Option<NativeViewHandle<VideoPlayer>>> = const { RefCell::new(None) };
}

#[component]
fn NativeEscapeHatchApp() -> Element {
    let mut show_webview = use_signal(|| true);
    let mut reg_sig = use_provide_native_view_registry(|| {
        ACTIVE_REGISTRY.with(|r| r.borrow().clone())
    });
    let wv_handle = use_native_view_handle::<WebView>();
    let vp_handle = use_native_view_handle::<VideoPlayer>();

    CAPTURED_WV_HANDLE.with(|slot| *slot.borrow_mut() = Some(wv_handle.clone()));
    CAPTURED_VP_HANDLE.with(|slot| *slot.borrow_mut() = Some(vp_handle.clone()));

    rsx! {
        view {
            NativeView::<BlurSurface> {
                props: BlurSurfaceProps {
                    material: BlurMaterial::Thick,
                    intensity: 0.9,
                    tint: Some(0x0F17_2ACC),
                },
                test_id: "native-blur",
                style: "width: 320px; height: 140px;",
                fallback: rsx! {
                    view {
                        test_id: "blur-fallback",
                        text { "BlurUnavailableFallback" }
                    }
                },
            }
            if show_webview() {
                NativeView::<WebView> {
                    props: WebViewProps {
                        url: "https://nve.dev/home".into(),
                        allows_back_forward: true,
                    },
                    test_id: "native-webview",
                    handle: wv_handle.clone(),
                    fallback: rsx! {
                        text { "WebViewFallback" }
                    },
                }
            }
            NativeView::<VideoPlayer> {
                props: VideoPlayerProps::new("https://cdn.nve.dev/clip.mp4"),
                test_id: "native-video",
                handle: vp_handle.clone(),
                fallback: rsx! {
                    text { "VideoFallback" }
                },
            }
            NativeView::<WebElement> {
                props: WebElementProps::new("video")
                    .with_attr("playsinline", "true")
                    .with_attr("preload", "metadata"),
                test_id: "web-video-el",
                fallback: rsx! {
                    text { "WebVideoFallback" }
                },
            }
            NativeView::<WebElement> {
                props: WebElementProps::new("svg").with_attr("viewBox", "0 0 24 24"),
                test_id: "web-svg-el",
                fallback: rsx! {
                    text { "WebSvgFallback" }
                },
            }
            pressable {
                test_id: "toggle-webview",
                onpress: move |_| show_webview.toggle(),
            }
            pressable {
                test_id: "switch-to-svg-only",
                onpress: move |_| {
                    reg_sig.set(NativeViewRegistry::new().with("web.svg"));
                },
            }
        }
    }
}

#[derive(Default)]
struct CustomNodeInfo {
    custom_name: Option<String>,
    raw_props: HashMap<String, RawPropValue>,
    width: Dimension,
}

fn collect_custom_nodes(bytes: &[u8]) -> HashMap<NodeId, CustomNodeInfo> {
    let (_, cmds) = CommandDecoder::new(bytes).unwrap().decode_all().unwrap();
    let mut map: HashMap<NodeId, CustomNodeInfo> = HashMap::new();
    for cmd in cmds {
        match cmd {
            Command::CreateNode {
                node_id,
                kind: ComponentKind::Custom,
                custom_name,
            } => {
                map.insert(
                    node_id,
                    CustomNodeInfo {
                        custom_name,
                        raw_props: HashMap::new(),
                        width: Dimension::Auto,
                    },
                );
            }
            Command::SetRawProp {
                node_id,
                key,
                value,
            } => {
                if let Some(info) = map.get_mut(&node_id) {
                    info.raw_props.insert(key, value);
                }
            }
            Command::SetLayout { node_id, layout } => {
                if let Some(info) = map.get_mut(&node_id) {
                    info.width = layout.width;
                }
            }
            _ => {}
        }
    }
    map
}

#[test]
fn native_view_emits_binary_custom_nodes_binds_handles_and_unmounts_cleanly() {
    let mut reg = NativeViewRegistry::apple();
    reg.register("web.*");
    ACTIVE_REGISTRY.with(|r| *r.borrow_mut() = reg);

    let mut surface = DioxusSurface::new(9, NativeEscapeHatchApp);
    let (_, _, frame1) = surface.render_frame(1);

    let custom_nodes = collect_custom_nodes(&frame1);
    let texts = collect_texts(&frame1);
    assert!(!texts.contains(&"BlurUnavailableFallback".to_owned()));
    assert!(!texts.contains(&"WebViewFallback".to_owned()));
    assert!(!texts.contains(&"VideoFallback".to_owned()));
    assert!(!texts.contains(&"WebVideoFallback".to_owned()));
    assert!(!texts.contains(&"WebSvgFallback".to_owned()));

    // Verify BlurSurface emitted ComponentKind::Custom with custom_name="nve.blur_surface",
    // binary NVNP RawPropValue::Bytes, and parsed layout width=320px.
    let blur_id = find_test_id(&frame1, "native-blur").expect("blur mounted");
    let blur_node = &custom_nodes[&blur_id];
    assert_eq!(blur_node.custom_name.as_deref(), Some("nve.blur_surface"));
    assert_eq!(blur_node.width, Dimension::Points(320.0));
    assert_eq!(
        blur_node.raw_props.get(NATIVE_VIEW_KIND_PROP_KEY),
        Some(&RawPropValue::String("nve.blur_surface".to_owned()))
    );
    let Some(RawPropValue::Bytes(blur_bytes)) =
        blur_node.raw_props.get(NATIVE_VIEW_PROPS_PROP_KEY)
    else {
        panic!("expected RawPropValue::Bytes for nve.native_props");
    };
    assert!(blur_bytes.starts_with(&NATIVE_PROPS_MAGIC));
    let decoded_blur = BlurSurfaceProps::from_prop_bytes(blur_bytes).unwrap();
    assert_eq!(decoded_blur.material, BlurMaterial::Thick);
    assert_eq!(decoded_blur.intensity, 0.9);
    assert_eq!(decoded_blur.tint, Some(0x0F17_2ACC));

    // Verify WebElement `"web.*"` wildcard emitted `"web.video"` and `"web.svg"` custom nodes.
    let web_video_id = find_test_id(&frame1, "web-video-el").expect("web.video mounted");
    let web_video_node = &custom_nodes[&web_video_id];
    assert_eq!(web_video_node.custom_name.as_deref(), Some("web.video"));
    let Some(RawPropValue::Bytes(we_bytes)) =
        web_video_node.raw_props.get(NATIVE_VIEW_PROPS_PROP_KEY)
    else {
        panic!("expected binary NVNP props on WebElement");
    };
    let decoded_we = DecodedWebElementProps::from_prop_bytes(we_bytes).unwrap();
    assert_eq!(decoded_we.tag, "video");
    assert_eq!(
        decoded_we.attrs,
        vec![
            ("playsinline".to_owned(), "true".to_owned()),
            ("preload".to_owned(), "metadata".to_owned()),
        ]
    );

    let web_svg_id = find_test_id(&frame1, "web-svg-el").expect("web.svg mounted");
    assert_eq!(custom_nodes[&web_svg_id].custom_name.as_deref(), Some("web.svg"));

    // Verify WebViewHandle auto-bound to exact NodeId on mount and emits binary DispatchCommand
    // without re-rendering RSX!
    let wv_id = find_test_id(&frame1, "native-webview").expect("webview mounted");
    let wv_handle = CAPTURED_WV_HANDLE.with(|s| s.borrow().clone().unwrap());
    assert_eq!(wv_handle.surface_id(), Some(9));
    assert_eq!(wv_handle.node_id(), Some(wv_id));

    let cmd_frame = wv_handle
        .encode_command_frame(42, WebViewCommand::EvaluateScript("1 + 1".into()))
        .unwrap();
    let (hdr, cmds) = CommandDecoder::new(&cmd_frame)
        .unwrap()
        .decode_all()
        .unwrap();
    assert_eq!(hdr.surface_id, 9);
    assert_eq!(hdr.frame_id, 42);
    match &cmds[0] {
        Command::DispatchCommand {
            node_id,
            command_name,
            payload,
        } => {
            assert_eq!(*node_id, wv_id);
            assert_eq!(command_name, "evaluate_script");
            assert!(payload.starts_with(&NATIVE_CMD_MAGIC));
            assert_eq!(
                WebViewCommand::from_command_bytes(payload).unwrap(),
                WebViewCommand::EvaluateScript("1 + 1".into())
            );
        }
        other => panic!("expected DispatchCommand, got {other:?}"),
    }

    // Unmounting `<NativeView<WebView>>` must automatically unbind its handle!
    let toggle_id = find_test_id(&frame1, "toggle-webview").unwrap();
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 9,
        node_id: toggle_id,
        timestamp_us: 500,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = surface.render_frame(2);
    assert!(
        !wv_handle.is_bound(),
        "unmounting NativeView must unbind its NativeViewHandle"
    );
    assert_eq!(
        wv_handle.dispatch(WebViewCommand::Reload),
        Err(NativeViewCodecError::UnboundHandle)
    );

    // Switching registry dynamically to `"web.svg"` only renders fallbacks for Blur/Video/web.video
    // while keeping `"web.svg"` mounted natively!
    let switch_id = find_test_id(&frame1, "switch-to-svg-only").unwrap();
    assert!(surface.dispatch_events(&[NativeEvent {
        surface_id: 9,
        node_id: switch_id,
        timestamp_us: 600,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let (_, _, frame3) = surface.render_frame(3);
    let texts3 = collect_texts(&frame3);
    assert!(texts3.contains(&"BlurUnavailableFallback".to_owned()));
    assert!(texts3.contains(&"VideoFallback".to_owned()));
    assert!(texts3.contains(&"WebVideoFallback".to_owned()));
    assert!(!texts3.contains(&"WebSvgFallback".to_owned()));

    let vp_handle = CAPTURED_VP_HANDLE.with(|s| s.borrow().clone().unwrap());
    assert!(!vp_handle.is_bound());
}

#[test]
fn unregistered_native_views_render_graceful_fallbacks_with_zero_custom_nodes() {
    ACTIVE_REGISTRY.with(|r| *r.borrow_mut() = NativeViewRegistry::headless());

    let mut surface = DioxusSurface::new(1, NativeEscapeHatchApp);
    let (_, _, frame) = surface.render_frame(1);

    let custom_nodes = collect_custom_nodes(&frame);
    assert!(
        custom_nodes.is_empty(),
        "headless/empty registry must emit zero ComponentKind::Custom nodes, got {}",
        custom_nodes.len()
    );

    let texts = collect_texts(&frame);
    assert!(texts.contains(&"BlurUnavailableFallback".to_owned()));
    assert!(texts.contains(&"WebViewFallback".to_owned()));
    assert!(texts.contains(&"VideoFallback".to_owned()));
    assert!(texts.contains(&"WebVideoFallback".to_owned()));
    assert!(texts.contains(&"WebSvgFallback".to_owned()));

    let wv_handle = CAPTURED_WV_HANDLE.with(|s| s.borrow().clone().unwrap());
    assert!(!wv_handle.is_bound());
}
