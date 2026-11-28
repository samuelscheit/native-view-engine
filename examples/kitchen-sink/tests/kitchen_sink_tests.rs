use std::path::PathBuf;

use nve_abi::{EventBatchBuf, FrameId, NodeId, SurfaceId, TimestampUs};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_canvas_skia::CanvasSkiaSystem;
use nve_canvas_web::CanvasWebSystem;
use nve_tables::tables::DebugId;
use nve_engine::loader::HotAppInstance;
use nve_engine::{HotPatchMsg, NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_example_kitchen_sink::{
    create_surface, decode_kitchen_sink_state, encode_kitchen_sink_state, DensityPreset,
    GallerySection, KitchenSinkModule, KitchenSinkState, LocalePreset, SpringPreset, ThemeMode,
    KITCHEN_SINK_STATE_MAGIC,
};
use nve_i18n::CalendarDate;
use nve_store::NodeStore;
use nve_ui::DateRange;
use nve_web_html_web::register_web_html_systems;

fn find_node_by_test_id(store: &NodeStore, test_id: &str) -> NodeId {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if id != NodeId::ROOT
            && let Some(dbg) = store.try_row::<DebugId>(id)
            && dbg.test_id() == test_id
        {
            return id;
        }
        stack.extend_from_slice(store.children(id));
    }
    #[expect(clippy::panic, reason = "test helper fails when expected testID is missing")]
    {
        panic!("node with testID='{test_id}' not found")
    }
}

fn press_test_id(
    surface: &mut nve_dioxus::DioxusSurface,
    backend: &mut HeadlessBackend,
    test_id: &str,
    frame_id: u64,
) {
    let id = find_node_by_test_id(surface.store(), test_id);
    let rect = backend.layout_of(id).unwrap_or_default();
    let mut batch = EventBatchBuf::new();
    batch.push_press(
        SurfaceId::ROOT,
        id,
        TimestampUs(frame_id * 16_000),
        rect.x + 2.0,
        rect.y + 2.0,
    );
    let handled = surface.dispatch_events(batch.as_validated());
    assert!(handled, "expected press on '{test_id}' to be handled");
    surface
        .commit_to(FrameId(frame_id), backend)
        .expect("commit test press frame");
}

#[test]
fn binary_state_codec_roundtrips_all_fields_and_rejects_corruption() {
    let state = KitchenSinkState {
        active_section: GallerySection::Motion,
        theme_mode: ThemeMode::Light,
        os: nve_ui::Appearance::DARK,
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
    let mut app = create_surface(SurfaceId::ROOT);
    let mut headless = HeadlessBackend::new();
    let mut skia = SkiaBackend::new();
    skia.register_table_system(CanvasSkiaSystem::new());
    let mut web = WebBackend::new();
    register_web_html_systems(web.table_registry_mut());
    web.register_table_system(CanvasWebSystem::new());

    let vp = Viewport {
        width: 1280.0,
        height: 860.0,
        scale_factor: 1.0,
    };
    headless.set_viewport(SurfaceId::ROOT, vp);
    skia.set_viewport(SurfaceId::ROOT, vp);
    web.set_viewport(SurfaceId::ROOT, vp);

    app.reconcile(FrameId(1));
    headless.commit(app.store(), app.last_changes()).unwrap();
    skia.commit(app.store(), app.last_changes()).unwrap();
    web.commit(app.store(), app.last_changes()).unwrap();

    assert!(headless.render_ascii_tree(app.store()).contains("Count: 0"));
    assert!(web.render_html(app.store()).contains("Count: 0"));
    assert!(skia.scene().live_op_count() > 0);

    // Press Increment + twice via testID lookup.
    for frame_id in 2..=3 {
        let inc_id = find_node_by_test_id(app.store(), "ks-counter-inc");
        let mut batch = EventBatchBuf::new();
        batch.push_press(SurfaceId::ROOT, inc_id, TimestampUs(frame_id * 16_000), 10.0, 10.0);
        let handled = app.dispatch_events(batch.as_validated());
        assert!(handled);
        app.reconcile(FrameId(frame_id));
        headless.commit(app.store(), app.last_changes()).unwrap();
        skia.commit(app.store(), app.last_changes()).unwrap();
        web.commit(app.store(), app.last_changes()).unwrap();
    }

    assert!(headless.render_ascii_tree(app.store()).contains("Count: 2"));
    assert!(web.render_html(app.store()).contains("Count: 2"));

    // Tier-1 hot patch changes live style via binary RowPatch without recompile.
    let mut rp = nve_store::RowPatch::new();
    rp.set(nve_tables::tables::Visual::OPACITY, 0.9);
    let mut words = Vec::new();
    assert!(nve_store::FlatResource::write_words(&rp, &mut words));
    let patched = app.apply_hot_patch(&HotPatchMsg::TemplatePatch {
        generation: 1,
        template_key: 0,
        node_path: vec![0],
        target_test_id: None,
        row_patch_words: words,
        text_updates: vec![],
        text_replacements: vec![],
    });
    assert!(patched);
}

#[test]
fn gallery_navigates_all_sections_switches_controls_and_recovers_error_boundary() {
    let mut app = create_surface(SurfaceId::ROOT);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 1440.0,
            height: 900.0,
            scale_factor: 1.0,
        },
    );

    app.commit_to(FrameId(1), &mut headless).unwrap();
    assert!(headless.render_ascii_tree(app.store()).contains("Overview & Primitives"));

    // Toggle theme, density, locale, pseudo, and reduced motion.
    press_test_id(&mut app, &mut headless, "ks-theme-toggle", 2);
    assert!(headless.render_ascii_tree(app.store()).contains("Theme: Light"));
    press_test_id(&mut app, &mut headless, "ks-density-spacious", 3);
    press_test_id(&mut app, &mut headless, "ks-locale-de-DE", 4);
    press_test_id(&mut app, &mut headless, "ks-reduced-motion-toggle", 5);

    // Navigate through Forms, Selection, Overlays, Layout, Motion.
    press_test_id(&mut app, &mut headless, "ks-nav-forms", 6);
    assert!(headless.render_ascii_tree(app.store()).contains("FormState Validation"));
    press_test_id(&mut app, &mut headless, "ks-nav-selection", 7);
    assert!(headless.render_ascii_tree(app.store()).contains("Locale-Aware DatePicker"));
    assert!(headless.render_ascii_tree(app.store()).contains("Monday"));
    press_test_id(&mut app, &mut headless, "ks-nav-overlays", 8);
    assert!(headless.render_ascii_tree(app.store()).contains("ToastProvider"));
    press_test_id(&mut app, &mut headless, "ks-toast-success", 9);
    assert!(headless.render_ascii_tree(app.store()).contains("Build Verified"));

    press_test_id(&mut app, &mut headless, "ks-nav-layout", 10);
    assert!(headless.render_ascii_tree(app.store()).contains("1,000 Variable-Height Rows"));
    press_test_id(&mut app, &mut headless, "ks-vl-jump-500", 11);
    assert!(headless.render_ascii_tree(app.store()).contains("Row #500"));

    // Motion + Canvas + ErrorBoundary trigger & Retry recovery!
    press_test_id(&mut app, &mut headless, "ks-nav-motion", 12);
    assert!(headless.render_ascii_tree(app.store()).contains("UiErrorBoundary Subtree Isolation"));
    press_test_id(&mut app, &mut headless, "ks-trigger-error", 13);
    let err_tree = headless.render_ascii_tree(app.store());
    assert!(
        err_tree.contains("Subtree Error Caught"),
        "expected error boundary fallback, got:\n{err_tree}"
    );
    assert!(err_tree.contains("Origin:"));
    // Sibling gallery chrome stayed alive.
    assert!(err_tree.contains("NVE Kitchen Sink"));

    press_test_id(&mut app, &mut headless, "ks-error-boundary-retry", 14);
    let recovered_tree = headless.render_ascii_tree(app.store());
    assert!(recovered_tree.contains("Protected subtree is mounted and operational."));
    assert!(recovered_tree.contains("Recoveries: 1"));
}

#[test]
fn module_snapshot_and_restore_preserves_full_gallery_state() {
    let mut m1 = KitchenSinkModule::create(SurfaceId::ROOT);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(SurfaceId::ROOT, Viewport::default());
    let mut rep1 =
        StoreWireReplica::new(headless, nve_tables::tables::core_realization_order());
    let f1 = m1.render(FrameId(1)).to_vec();
    rep1.submit_nve2_frame(&f1).unwrap();

    let inc_id = find_node_by_test_id(rep1.store(), "ks-counter-inc");
    let mut press_ev = EventBatchBuf::new();
    press_ev.push_press(SurfaceId::ROOT, inc_id, TimestampUs(1_000), 4.0, 4.0);
    assert!(m1.dispatch_events(press_ev.as_validated()));
    let _ = m1.render(FrameId(2));
    assert_eq!(m1.count(), 1);

    let snap = m1.snapshot_state().to_vec();
    let mut m2 = KitchenSinkModule::create(SurfaceId::new(2, 0));
    assert!(m2.try_restore_state(&snap));
    assert_eq!(m2.count(), 1);
    let mut h2 = HeadlessBackend::new();
    h2.set_viewport(SurfaceId::new(2, 0), Viewport::default());
    let mut rep2 = StoreWireReplica::new(h2, nve_tables::tables::core_realization_order());
    rep2.submit_nve2_frame(m2.render(FrameId(1))).unwrap();
    assert!(rep2.backend().render_ascii_tree(rep2.store()).contains("Count: 1"));
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
    // Always rebuild (incremental no-op when fresh): a dylib left over from an older commit
    // speaks a stale wire format and fails spuriously.
    let status = std::process::Command::new("cargo")
        .args(["build", "-p", "nve-example-kitchen-sink"])
        .current_dir(workspace_root)
        .status()
        .expect("build cdylib");
    assert!(status.success());

    let mut host = HeadlessBackend::new();
    host.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 1280.0,
            height: 860.0,
            scale_factor: 1.0,
        },
    );
    let mut rep = StoreWireReplica::new(host, nve_tables::tables::core_realization_order());

    let mut inst = HotAppInstance::load(&dylib_path, SurfaceId::ROOT, 1, None).expect("load gen 1");
    let frame1 = inst.render(FrameId(1));
    rep.submit_nve2_frame(frame1).unwrap();
    assert!(rep.backend().render_ascii_tree(rep.store()).contains("Count: 0"));

    let inc_node = find_node_by_test_id(rep.store(), "ks-counter-inc");
    let mut press_ev = EventBatchBuf::new();
    press_ev.push_press(SurfaceId::ROOT, inc_node, TimestampUs(1000), 10.0, 10.0);

    assert!(inst.dispatch_events(press_ev.as_validated()));
    let f2 = inst.render(FrameId(2));
    rep.submit_nve2_frame(f2).unwrap();
    assert!(inst.dispatch_events(press_ev.as_validated()));
    let f3 = inst.render(FrameId(3));
    rep.submit_nve2_frame(f3).unwrap();
    assert!(rep.backend().render_ascii_tree(rep.store()).contains("Count: 2"));

    // Live Tier-2 dylib hot-swap to generation 2: count == 2 MUST survive!
    let outcome = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome.previous_generation, 1);
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.generation(), 2);

    let mut host2 = HeadlessBackend::new();
    host2.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 1280.0,
            height: 860.0,
            scale_factor: 1.0,
        },
    );
    let mut rep2 =
        StoreWireReplica::new(host2, nve_tables::tables::core_realization_order());
    let f4 = inst.render(FrameId(4));
    rep2.submit_nve2_frame(f4).unwrap();
    assert!(
        rep2.backend().render_ascii_tree(rep2.store()).contains("Count: 2"),
        "restored tree was:\n{}",
        rep2.backend().render_ascii_tree(rep2.store())
    );
}

#[test]
fn os_appearance_host_events_retheme_auto_mode_in_place_and_pinned_modes_ignore_the_os_scheme() {
    use nve_abi::{encode_single_push_into, ColorScheme, ContrastLevel, SERVICE_SYSTEM_ENV};
    use nve_tables::tables::Visual;
    use nve_system_env::{ThemeSnapshot, EVENT_THEME_CHANGED};

    let os = |scheme, contrast| {
        let raw = ThemeSnapshot {
            color_scheme: Some(scheme),
            contrast,
            ..ThemeSnapshot::default()
        }
        .to_repr_c_bytes();
        let mut batch = [0u8; 56];
        let encoded =
            encode_single_push_into(&mut batch, SERVICE_SYSTEM_ENV, EVENT_THEME_CHANGED, &raw)
                .expect("encode canonical theme push batch");
        assert_eq!(encoded.len(), 56);
        batch
    };
    let root_bg = |m: &KitchenSinkModule| {
        let store = m.surface().store();
        store.row::<Visual>(find_node_by_test_id(store, "kitchen-sink-root")).background()
    };
    let mut module = KitchenSinkModule::create(SurfaceId::ROOT);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(SurfaceId::ROOT, Viewport { width: 1440.0, height: 900.0, scale_factor: 1.0 });
    module.commit_to(FrameId(1), &mut headless).expect("frame 1");
    assert!(headless.render_ascii_tree(module.surface().store()).contains("Theme: Auto"));
    let dark = root_bg(&module);
    press_test_id(module.surface_mut(), &mut headless, "ks-counter-inc", 2);

    let raw_12 = ThemeSnapshot {
        color_scheme: Some(ColorScheme::Light),
        contrast: ContrastLevel::Standard,
        ..ThemeSnapshot::default()
    }
    .to_repr_c_bytes();
    assert!(!module.dispatch_host_events(&raw_12), "raw 12-byte slices without NVHB magic are ignored");
    assert!(module.dispatch_host_events(&os(ColorScheme::Light, ContrastLevel::Standard)));
    assert!(!module.dispatch_host_events(&os(ColorScheme::Light, ContrastLevel::Standard)), "same appearance is a no-op");
    assert!(!module.dispatch_host_events(b"not a theme snapshot"), "foreign batches are ignored");
    module.commit_to(FrameId(3), &mut headless).expect("frame 3");
    let light = root_bg(&module);
    assert_ne!(light, dark, "Auto mode follows the OS light scheme");
    assert!(headless.render_ascii_tree(module.surface().store()).contains("Count: 1"), "re-themed without losing state");

    assert!(module.dispatch_host_events(&os(ColorScheme::Dark, ContrastLevel::High)));
    module.commit_to(FrameId(4), &mut headless).expect("frame 4");
    let hc_dark = root_bg(&module);
    assert!(hc_dark != dark && hc_dark != light, "high contrast resolves its own polarity");

    // Auto -> Light -> Dark: a pinned scheme ignores the OS scheme.
    assert!(module.dispatch_host_events(&os(ColorScheme::Light, ContrastLevel::Standard)));
    press_test_id(module.surface_mut(), &mut headless, "ks-theme-toggle", 5);
    press_test_id(module.surface_mut(), &mut headless, "ks-theme-toggle", 6);
    assert!(headless.render_ascii_tree(module.surface().store()).contains("Theme: Dark"));
    assert_eq!(root_bg(&module), dark, "pinned Dark under a light OS");
}

/// Native backends do not evaluate `@media` rules yet, so this covers the base (row) layout; the
/// stacked 390px column is guarded in the browser by `run_docs_e2e.mjs` (collapsed flex items).
#[test]
fn two_col_cells_share_the_row_with_equal_widths() {
    let mut module = KitchenSinkModule::create(SurfaceId::ROOT);
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(SurfaceId::ROOT, Viewport { width: 1440.0, height: 900.0, scale_factor: 1.0 });
    module.commit_to(FrameId(1), &mut headless).expect("frame 1");
    let [counter, cursor, buttons] = ["ks-counter-card", "ks-cursor-card", "ks-buttons-card"].map(|tid| {
        headless.layout_of(find_node_by_test_id(module.surface().store(), tid)).unwrap_or_default()
    });
    assert_eq!(counter.y, cursor.y, "counter and cursor cards share one row");
    assert!((counter.width - cursor.width).abs() < 1.0, "equal-width cells {counter:?} {cursor:?}");
    assert!(counter.height > 60.0 && cursor.height > 60.0, "cells keep their content height");
    assert!(buttons.y >= counter.y + counter.height, "the next row starts below the first");
}
