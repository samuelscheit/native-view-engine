use std::path::PathBuf;

use nve_abi::{encode_events, EventPayload, NativeEvent, NativeEventKind};
use nve_backend_headless::HeadlessBackend;
use nve_backend_skia::SkiaBackend;
use nve_backend_web::WebBackend;
use nve_engine::loader::HotAppInstance;
use nve_engine::{HotPatchMsg, RenderBackend, Viewport};
use nve_example_counter::create_surface;

#[test]
fn single_command_buffer_drives_headless_skia_and_web_with_events() {
    let mut app = create_surface(1);
    let mut headless = HeadlessBackend::new();
    let mut skia = SkiaBackend::new();
    let mut web = WebBackend::new();

    let vp = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
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

    // Find the "Increment +" pressable via its layout rect and simulate two native presses.
    let inc_node = headless
        .nodes()
        .values()
        .find(|n| n.event_mask != 0 && (n.computed_rect.width - 160.0).abs() < 0.5)
        .expect("increment button node")
        .id;

    for frame_id in 2..=3 {
        let rect = headless.layout_of(inc_node).unwrap();
        let hit = headless
            .hit_test(rect.x + 10.0, rect.y + 10.0, true)
            .expect("hit test should find increment button");
        assert_eq!(hit, inc_node);

        let handled = app.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: hit,
            timestamp_us: frame_id * 16_000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point {
                x: rect.x + 10.0,
                y: rect.y + 10.0,
            },
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
        template_id: "counter".into(),
        target_test_id: None,
        node_path: vec![],
        attr_updates: vec![("opacity".into(), "0.8".into())],
        text_updates: vec![],
        text_replacements: vec![],
    });
    assert!(patched);
}

#[test]
fn tier2_cdylib_hotswap_preserves_state_across_generations() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
    let dylib_path = workspace_root.join(format!(
        "target/debug/{}nve_example_counter.{}",
        std::env::consts::DLL_PREFIX,
        std::env::consts::DLL_EXTENSION
    ));
    if !dylib_path.exists() {
        // Ensure the cdylib artifact is built when running this test standalone.
        let status = std::process::Command::new("cargo")
            .args(["build", "-p", "nve-example-counter"])
            .current_dir(workspace_root)
            .status()
            .expect("build cdylib");
        assert!(status.success());
    }

    let mut host = HeadlessBackend::new();
    host.set_viewport(1, Viewport::default());

    let mut inst = HotAppInstance::load(&dylib_path, 1, 1, None).expect("load gen 1");
    let frame1 = inst.render(1);
    host.submit_frame(&frame1).unwrap();
    assert!(host.render_ascii_tree().contains("Count: 0"));

    // Tap Increment twice through the C ABI EventRing!
    let inc_node = host
        .nodes()
        .values()
        .find(|n| n.event_mask != 0 && (n.computed_rect.width - 160.0).abs() < 0.5)
        .unwrap()
        .id;
    let press_bytes = encode_events(&[NativeEvent {
        surface_id: 1,
        node_id: inc_node,
        timestamp_us: 1000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 40.0, y: 150.0 },
    }]);

    assert!(inst.dispatch_events(&press_bytes));
    let f2 = inst.render(2);
    host.submit_frame(&f2).unwrap();
    assert!(inst.dispatch_events(&press_bytes));
    let f3 = inst.render(3);
    host.submit_frame(&f3).unwrap();
    assert!(host.render_ascii_tree().contains("Count: 2"));

    // Live Tier-2 dylib hot-swap to generation 2: count == 2 MUST survive!
    let outcome = inst.reload(&dylib_path).expect("reload gen 2");
    assert_eq!(outcome.previous_generation, 1);
    assert_eq!(outcome.generation, 2);
    assert_eq!(inst.generation(), 2);

    let mut host2 = HeadlessBackend::new();
    host2.set_viewport(1, Viewport::default());
    let f4 = inst.render(4);
    host2.submit_frame(&f4).unwrap();
    assert!(
        host2.render_ascii_tree().contains("Count: 2"),
        "restored tree was:\n{}",
        host2.render_ascii_tree()
    );
}
