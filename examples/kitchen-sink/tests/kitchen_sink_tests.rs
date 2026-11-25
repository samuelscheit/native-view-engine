use std::path::PathBuf;

use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_engine::loader::HotAppInstance;
use nve_engine::{HotPatchMsg, NveApp, RenderBackend, Viewport};
use nve_example_kitchen_sink::{
    create_surface, decode_kitchen_sink_state, encode_kitchen_sink_state, DensityPreset,
    GallerySection, KitchenSinkModule, KitchenSinkState, LocalePreset, SpringPreset, ThemeMode,
    KITCHEN_SINK_STATE_MAGIC,
};
use nve_i18n::CalendarDate;
use nve_ui::DateRange;

fn find_node_by_test_id(backend: &HeadlessBackend, test_id: &str) -> NodeId {
    backend
        .nodes()
        .values()
        .find(|n| matches!(n.raw_props.get("testID"), Some(RawPropValue::String(s)) if s == test_id))
        .unwrap_or_else(|| panic!("node with testID='{test_id}' not found"))
        .id
}

fn press_test_id(
    surface: &mut nve_dioxus::DioxusSurface,
    backend: &mut HeadlessBackend,
    test_id: &str,
    frame_id: u64,
) {
    let id = find_node_by_test_id(backend, test_id);
    let rect = backend.layout_of(id).unwrap_or_default();
    let handled = surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: id,
        timestamp_us: frame_id * 16_000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point {
            x: rect.x + 2.0,
            y: rect.y + 2.0,
        },
    }]);
    assert!(handled, "expected press on '{test_id}' to be handled");
    let (_, _, patch) = surface.render_frame(frame_id);
    backend.submit_frame(&patch).unwrap();
}

#[test]
fn binary_state_codec_roundtrips_all_fields_and_rejects_corruption() {
    let state = KitchenSinkState {
        active_section: GallerySection::Motion,
        theme_mode: ThemeMode::Light,
        density: DensityPreset::Spacious,
        locale: LocalePreset::ArEg,
        pseudo_locale: true,
        reduced_motion: true,
        counter: -42,
        checkbox_terms: false,
        checkbox_indeterminate: false,
        switch_notifications: false,
        switch_telemetry: true,
        combobox_single: "swift".to_owned(),
        combobox_multi: vec!["skia".to_owned(), "i18n".to_owned(), "router".to_owned()],
        selected_date: CalendarDate::new(2026, 7, 4).ok(),
        selected_range: CalendarDate::new(2026, 7, 1)
            .ok()
            .zip(CalendarDate::new(2026, 7, 15).ok())
            .map(|(s, e)| DateRange::new(s, e)),
        notes: "Binary round-trip UTF-8: 日本語 / العربية / Deutsch".to_owned(),
        spring_preset: SpringPreset::Bouncy,
        motion_toggled: true,
        active_tab: "binary".to_owned(),
    };

    let bytes = encode_kitchen_sink_state(&state);
    assert!(bytes.starts_with(&KITCHEN_SINK_STATE_MAGIC));
    let decoded = decode_kitchen_sink_state(&bytes).expect("valid state must decode");
    assert_eq!(decoded, state);

    // Reject bad magic, bad version, truncated bytes, trailing garbage, and invalid enums.
    assert!(decode_kitchen_sink_state(&[]).is_none());
    assert!(decode_kitchen_sink_state(&bytes[..bytes.len() - 1]).is_none());
    let mut trailing = bytes.clone();
    trailing.push(0xAA);
    assert!(decode_kitchen_sink_state(&trailing).is_none());

    let mut bad_magic = bytes.clone();
    bad_magic[0] = b'X';
    assert!(decode_kitchen_sink_state(&bad_magic).is_none());

    let mut bad_ver = bytes.clone();
    bad_ver[KITCHEN_SINK_STATE_MAGIC.len()] = 99;
    assert!(decode_kitchen_sink_state(&bad_ver).is_none());

    let mut bad_section = bytes.clone();
    bad_section[KITCHEN_SINK_STATE_MAGIC.len() + 2] = 250;
    assert!(decode_kitchen_sink_state(&bad_section).is_none());
}

#[test]
fn single_command_buffer_drives_headless_skia_and_web_with_events() {
    let mut app = create_surface(1);
    let mut headless = HeadlessBackend::new();
    let mut skia = SkiaBackend::new();
    let mut web = WebBackend::new();

    let vp = Viewport {
        width: 1280.0,
        height: 860.0,
        scale_factor: 1.0,
    };
    headless.set_viewport(1, vp);
    skia.set_viewport(1, vp);
    web.set_viewport(1, vp);

    let (_, _, frame1) = app.render_frame(1);
    headless.submit_frame(&frame1).unwrap();
    skia.submit_frame(&frame1).unwrap();
    web.submit_frame(&frame1).unwrap();

    assert!(headless.render_ascii_tree().contains("Count: 0"));
    assert!(web.render_html().contains("Count: 0"));
    assert!(!skia.display_list().ops.is_empty());

    // Press Increment + twice via testID lookup.
    for frame_id in 2..=3 {
        let inc_id = find_node_by_test_id(&headless, "ks-counter-inc");
        let handled = app.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: inc_id,
            timestamp_us: frame_id * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 10.0, y: 10.0 },
        }]);
        assert!(handled);
        let (_, _, patch) = app.render_frame(frame_id);
        headless.submit_frame(&patch).unwrap();
        skia.submit_frame(&patch).unwrap();
        web.submit_frame(&patch).unwrap();
    }

    assert!(headless.render_ascii_tree().contains("Count: 2"));
    assert!(web.render_html().contains("Count: 2"));

    // Tier-1 hot patch changes live style without recompile.
    let patched = app.apply_hot_patch(&HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "kitchen-sink".into(),
        target_test_id: None,
        node_path: vec![],
        attr_updates: vec![("opacity".into(), "0.9".into())],
        text_updates: vec![],
        text_replacements: vec![],
    });
    assert!(patched);
}

#[test]
fn gallery_navigates_all_sections_switches_controls_and_recovers_error_boundary() {
    let mut app = create_surface(1);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        1,
        Viewport {
            width: 1440.0,
            height: 900.0,
            scale_factor: 1.0,
        },
    );

    let (_, _, f1) = app.render_frame(1);
    headless.submit_frame(&f1).unwrap();
    assert!(headless.render_ascii_tree().contains("Overview & Primitives"));

    // Toggle theme, density, locale, pseudo, and reduced motion.
    press_test_id(&mut app, &mut headless, "ks-theme-toggle", 2);
    assert!(headless.render_ascii_tree().contains("Theme: Light"));
    press_test_id(&mut app, &mut headless, "ks-density-spacious", 3);
    press_test_id(&mut app, &mut headless, "ks-locale-de-DE", 4);
    press_test_id(&mut app, &mut headless, "ks-reduced-motion-toggle", 5);

    // Navigate through Forms, Selection, Overlays, Layout, Motion.
    press_test_id(&mut app, &mut headless, "ks-nav-forms", 6);
    assert!(headless.render_ascii_tree().contains("FormState Validation"));
    press_test_id(&mut app, &mut headless, "ks-nav-selection", 7);
    assert!(headless.render_ascii_tree().contains("Locale-Aware DatePicker"));
    assert!(headless.render_ascii_tree().contains("Monday"));
    press_test_id(&mut app, &mut headless, "ks-nav-overlays", 8);
    assert!(headless.render_ascii_tree().contains("ToastProvider"));
    press_test_id(&mut app, &mut headless, "ks-toast-success", 9);
    assert!(headless.render_ascii_tree().contains("Build Verified"));

    press_test_id(&mut app, &mut headless, "ks-nav-layout", 10);
    assert!(headless.render_ascii_tree().contains("1,000 Variable-Height Rows"));
    press_test_id(&mut app, &mut headless, "ks-vl-jump-500", 11);
    assert!(headless.render_ascii_tree().contains("Row #500"));

    // Motion + Canvas + ErrorBoundary trigger & Retry recovery!
    press_test_id(&mut app, &mut headless, "ks-nav-motion", 12);
    assert!(headless.render_ascii_tree().contains("UiErrorBoundary Subtree Isolation"));
    press_test_id(&mut app, &mut headless, "ks-trigger-error", 13);
    let err_tree = headless.render_ascii_tree();
    assert!(
        err_tree.contains("Subtree Error Caught"),
        "expected error boundary fallback, got:\n{err_tree}"
    );
    assert!(err_tree.contains("Origin:"));
    // Sibling gallery chrome stayed alive.
    assert!(err_tree.contains("NVE Kitchen Sink"));

    press_test_id(&mut app, &mut headless, "ks-error-boundary-retry", 14);
    let recovered_tree = headless.render_ascii_tree();
    assert!(recovered_tree.contains("Protected subtree is mounted and operational."));
    assert!(recovered_tree.contains("Recoveries: 1"));
}

#[test]
fn module_snapshot_and_restore_preserves_full_gallery_state() {
    let mut m1 = KitchenSinkModule::create(1);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(1, Viewport::default());
    let f1 = m1.render(1).to_vec();
    headless.submit_frame(&f1).unwrap();

    let inc_id = find_node_by_test_id(&headless, "ks-counter-inc");
    let press_bytes = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: inc_id,
        timestamp_us: 1_000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 4.0, y: 4.0 },
    }]);
    assert!(m1.dispatch_events(&press_bytes));
    let _ = m1.render(2);
    assert_eq!(m1.count(), 1);

    let snap = m1.snapshot_state().to_vec();
    let mut m2 = KitchenSinkModule::create(2);
    assert!(m2.try_restore_state(&snap));
    assert_eq!(m2.count(), 1);
    let mut h2 = HeadlessBackend::new();
    h2.set_viewport(2, Viewport::default());
    h2.submit_frame(m2.render(1)).unwrap();
    assert!(h2.render_ascii_tree().contains("Count: 1"));
}

#[test]
fn tier2_cdylib_hotswap_preserves_state_across_generations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let dylib_path = workspace_root.join(format!(
        "target/debug/{}nve_example_kitchen_sink.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    ));
    if !dylib_path.exists() {
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "nve-example-kitchen-sink"])
            .current_dir(workspace_root)
            .status()
            .expect("build cdylib");
        assert!(status.success());
    }

    let mut host = HeadlessBackend::new();
    host.set_viewport(
        1,
        Viewport {
            width: 1280.0,
            height: 860.0,
            scale_factor: 1.0,
        },
    );

    let mut inst = HotAppInstance::load(&dylib_path, 1, 1, None).expect("load gen 1");
    let frame1 = inst.render(1);
    host.submit_frame(frame1).unwrap();
    assert!(host.render_ascii_tree().contains("Count: 0"));

    let inc_node = find_node_by_test_id(&host, "ks-counter-inc");
    let press_bytes = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: inc_node,
        timestamp_us: 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    }]);

    assert!(inst.dispatch_events(&press_bytes));
    let f2 = inst.render(2);
    host.submit_frame(f2).unwrap();
    assert!(inst.dispatch_events(&press_bytes));
    let f3 = inst.render(3);
    host.submit_frame(f3).unwrap();
    assert!(host.render_ascii_tree().contains("Count: 2"));

    // Live Tier-2 dylib hot-swap to generation 2: count == 2 MUST survive!
    let outcome = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome.previous_generation, 1);
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.generation(), 2);

    let mut host2 = HeadlessBackend::new();
    host2.set_viewport(
        1,
        Viewport {
            width: 1280.0,
            height: 860.0,
            scale_factor: 1.0,
        },
    );
    let f4 = inst.render(4);
    host2.submit_frame(f4).unwrap();
    assert!(
        host2.render_ascii_tree().contains("Count: 2"),
        "restored tree was:\n{}",
        host2.render_ascii_tree()
    );
}
