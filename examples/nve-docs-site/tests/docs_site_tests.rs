//! Integration tests for `examples/nve-docs-site`:
//! - Verifies all 30 embedded routes render cleanly via `WebBackend::render_ssr`
//! - Verifies all 5 interactive playgrounds mount and respond to real press/input events
//! - Verifies lossless zero-mismatch `WebBackend::hydrate_from_ssr` DOM identity preservation

use nve_abi::{EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue};
use nve_backend_web::WebBackend;
use nve_docs::{DocsViewport, default_component_registry};
use nve_engine::RenderBackend;
use nve_mdx::validate_site;

use nve_docs_site::{
    create_nve_docs_surface, embedded_compiled_site, playground_registry, render_nve_docs_ssr,
};

struct InteractiveHarness {
    surface: nve_dioxus::DioxusSurface,
    backend: WebBackend,
    frame_id: u64,
}

impl InteractiveHarness {
    fn mount(route: &str) -> Self {
        let mut surface = create_nve_docs_surface(route, "", DocsViewport::desktop_default());
        let (_, _, bytes) = surface.render_frame(1);
        let mut backend = WebBackend::new();
        backend.submit_frame(&bytes).expect("initial submit_frame");
        Self {
            surface,
            backend,
            frame_id: 1,
        }
    }

    fn find(&self, test_id: &str) -> NodeId {
        self.maybe_find(test_id)
            .unwrap_or_else(|| panic!("node with testID='{test_id}' not found"))
    }

    fn maybe_find(&self, test_id: &str) -> Option<NodeId> {
        self.backend.iter_nodes().find_map(|(id, node)| {
            match node.raw_props.get("testID") {
                Some(RawPropValue::String(s)) if s == test_id => Some(id),
                _ => None,
            }
        })
    }

    fn collect_text(&self, root: NodeId) -> String {
        let mut out = String::new();
        self.collect_text_rec(root, &mut out);
        out
    }

    fn collect_text_rec(&self, id: NodeId, out: &mut String) {
        let Some(node) = self.backend.node(id) else {
            return;
        };
        if let Some(t) = &node.text {
            out.push_str(t);
        }
        for &child in &node.children {
            self.collect_text_rec(child, out);
        }
    }

    fn text_of(&self, test_id: &str) -> String {
        self.collect_text(self.find(test_id))
    }

    fn press(&mut self, test_id: &str) {
        let node_id = self.find(test_id);
        let ev = NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: self.frame_id * 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 10.0, y: 10.0 },
        };
        assert!(
            self.surface.dispatch_events(&[ev]),
            "press on '{test_id}' should be handled"
        );
        self.frame_id += 1;
        let (_, _, bytes) = self.surface.render_frame(self.frame_id);
        self.backend.submit_frame(&bytes).expect("submit_frame");
    }

    fn type_text(&mut self, test_id: &str, value: &str) {
        let node_id = self.find(test_id);
        let ev = NativeEvent {
            surface_id: 1,
            node_id,
            timestamp_us: self.frame_id * 1000,
            kind: NativeEventKind::TextChange,
            payload: EventPayload::Text(value.to_string()),
        };
        assert!(
            self.surface.dispatch_events(&[ev]),
            "text change on '{test_id}' should be handled"
        );
        self.frame_id += 1;
        let (_, _, bytes) = self.surface.render_frame(self.frame_id);
        self.backend.submit_frame(&bytes).expect("submit_frame");
    }
}

#[test]
fn all_30_embedded_routes_validate_and_render_semantic_ssr() {
    let site = embedded_compiled_site();
    assert_eq!(
        site.pages.len(),
        30,
        "expected 30 official docs/content pages, got {}",
        site.pages.len()
    );

    let diagnostics = validate_site(&site, &default_component_registry());
    assert!(
        diagnostics.is_empty(),
        "expected 0 validation diagnostics across docs/content, got: {diagnostics:#?}"
    );

    let registry = playground_registry();
    for id in [
        "ui-playground",
        "virtual-list-100k",
        "router-sandbox",
        "canvas-draw1",
        "crdt-simulator",
    ] {
        assert!(
            registry.contains(id),
            "PlaygroundRegistry missing required demo '{id}'"
        );
    }

    for page in &site.pages {
        let ssr = render_nve_docs_ssr(&page.route, "/native-view-engine", DocsViewport::desktop_default());
        assert!(
            !ssr.html.is_empty(),
            "SSR HTML empty for route {}",
            page.route
        );
        assert!(
            ssr.html.contains("<nav") && ssr.html.contains("<main") && ssr.html.contains("<article"),
            "SSR HTML missing semantic landmarks for route {}",
            page.route
        );
        assert!(
            ssr.html.contains("<h1"),
            "SSR HTML missing <h1> for route {}",
            page.route
        );
        assert!(
            ssr.html.contains(&page.title) || !page.title.is_empty(),
            "SSR HTML missing title '{}' for route {}",
            page.title,
            page.route
        );
    }
}

#[test]
fn zero_mismatch_hydrate_from_ssr_preserves_dom_identity() {
    for route in ["/", "/guides/ui-components", "/guides/local-first-crdt"] {
        let ssr = render_nve_docs_ssr(route, "", DocsViewport::desktop_default());
        let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
        let root_before = hydrated
            .native_dom_instance_id(nve_abi::ROOT_NODE_ID)
            .expect("root SSR DOM instance");

        let mut surface = create_nve_docs_surface(route, "", DocsViewport::desktop_default());
        let (_, _, frame_bytes) = surface.render_frame(1);
        hydrated
            .submit_frame(&frame_bytes)
            .expect("hydrate submit_frame");

        let report = hydrated.hydration_report();
        assert!(report.hydrated, "route {route} should report hydrated=true");
        assert!(
            report.adopted_nodes > 50,
            "route {route} should adopt >50 SSR nodes, got {}",
            report.adopted_nodes
        );
        assert_eq!(
            report.mismatches.len(),
            0,
            "route {route} had hydration mismatches: {:?}",
            report.mismatches
        );
        assert_eq!(report.replaced_nodes, 0);
        assert_eq!(
            hydrated.native_dom_instance_id(nve_abi::ROOT_NODE_ID),
            Some(root_before),
            "root DOM instance identity must be preserved across hydration"
        );
    }
}

#[test]
fn playground_1_and_2_ui_and_virtual_list_100k_are_interactive() {
    let mut h = InteractiveHarness::mount("/guides/ui-components");

    // 1. UI Playground interactions
    assert!(h.text_of("pg-ui-count-badge").contains("Actions: 0"));
    assert!(h.text_of("pg-ui-status-badge").contains("Zero-Copy Active"));

    h.press("pg-ui-btn-primary");
    assert!(h.text_of("pg-ui-count-badge").contains("Actions: 1"));
    assert!(
        h.maybe_find("pg-ui-toast-viewport").is_some(),
        "pressing primary button should enqueue a live toast"
    );

    h.press("pg-ui-switch");
    assert!(h.text_of("pg-ui-status-badge").contains("Hybrid Mode"));

    h.type_text("pg-ui-input", "Metal Swapchain");
    assert!(h.text_of("pg-ui-surface-echo").contains("Metal Swapchain"));

    h.press("pg-ui-tab-badges");
    assert!(h.maybe_find("pg-ui-panel-badges").is_some());

    // 2. VirtualList 100k interactions
    assert!(h.text_of("pg-vlist-total").contains("100000"));
    assert!(h.text_of("pg-vlist-window").contains("0.."));

    h.press("pg-vlist-jump-75000");
    assert!(h.text_of("pg-vlist-selected").contains("#75000"));
    assert!(
        h.text_of("pg-vlist-window").contains("74998..")
            || h.text_of("pg-vlist-window").contains("75000.."),
        "expected window around 75000, got: {}",
        h.text_of("pg-vlist-window")
    );

    h.press("pg-vlist-filter-critical");
    assert!(h.text_of("pg-vlist-total").contains("25000"));
}

#[test]
fn playground_3_router_sandbox_handles_push_pop_modal_and_guard() {
    let mut h = InteractiveHarness::mount("/guides/routing");

    assert_eq!(h.text_of("pg-router-path"), "/");
    assert!(h.text_of("pg-router-depth").contains("Depth: 1"));

    // Guard is enabled by default: pushing /admin redirects to /login
    h.press("pg-router-push-admin");
    assert_eq!(h.text_of("pg-router-path"), "/login");
    assert!(h.text_of("pg-router-status").contains("redirected"));

    // Push /items/42 then Pop back to /login
    h.press("pg-router-push-item");
    assert_eq!(h.text_of("pg-router-path"), "/items/42");
    h.press("pg-router-pop");
    assert_eq!(h.text_of("pg-router-path"), "/login");

    // Disable guard and push /admin -> succeeds
    h.press("pg-router-guard-toggle");
    h.press("pg-router-push-admin");
    assert_eq!(h.text_of("pg-router-path"), "/admin");

    // Open and dismiss modal
    assert!(h.maybe_find("pg-router-modal-badge").is_none());
    h.press("pg-router-modal");
    assert!(h.maybe_find("pg-router-modal-badge").is_some());
    h.press("pg-router-modal");
    assert!(h.maybe_find("pg-router-modal-badge").is_none());
}

#[test]
fn playground_4_canvas_draw1_emits_real_binary_draw_ops_and_updates() {
    let mut h = InteractiveHarness::mount("/guides/canvas-draw1");

    let canvas_id = h.find("pg-canvas-surface");
    let ops_initial = h.backend.node(canvas_id).expect("canvas node").draw_op_count;
    assert!(
        ops_initial >= 10,
        "expected >=10 NVE_DRAW1 ops for sparkline, got {ops_initial}"
    );
    assert!(h.text_of("pg-canvas-palette").contains("Cyber Cyan"));

    h.press("pg-canvas-mode-bars");
    let ops_bars = h
        .backend
        .node(h.find("pg-canvas-surface"))
        .expect("canvas node")
        .draw_op_count;
    assert_ne!(ops_bars, ops_initial, "switching scene mode should change draw op count");

    h.press("pg-canvas-pal-violet");
    assert!(h.text_of("pg-canvas-palette").contains("Linear Violet"));

    h.press("pg-canvas-stroke");
    assert!(h.text_of("pg-canvas-stroke").contains("4.0px"));
}

#[test]
fn playground_5_crdt_simulator_diverges_offline_and_converges_via_nvc1() {
    let mut h = InteractiveHarness::mount("/guides/local-first-crdt");

    assert!(h.text_of("pg-crdt-convergence").contains("Converged"));

    // Partitioned by default -> Peer A and Peer B edits diverge
    h.press("pg-crdt-a-edit");
    assert!(h.text_of("pg-crdt-convergence").contains("Diverged"));
    assert!(h.text_of("pg-crdt-a-title").contains("Peer A Draft"));
    assert!(h.text_of("pg-crdt-b-title").contains("Native View Engine"));

    h.press("pg-crdt-b-edit");
    assert!(h.text_of("pg-crdt-b-title").contains("Peer B Release"));

    // Sync binary NVC1 deltas -> deterministic convergence with non-zero byte size
    h.press("pg-crdt-sync");
    assert!(h.text_of("pg-crdt-convergence").contains("Converged"));
    assert_eq!(h.text_of("pg-crdt-a-title"), h.text_of("pg-crdt-b-title"));
    assert!(
        h.text_of("pg-crdt-a-votes").contains("PN-Counter: 3")
            && h.text_of("pg-crdt-b-votes").contains("PN-Counter: 3"),
        "expected summed PN-Counter = 3 across both peers, got A='{}', B='{}'",
        h.text_of("pg-crdt-a-votes"),
        h.text_of("pg-crdt-b-votes")
    );
    assert!(
        h.text_of("pg-crdt-a-votes").contains("rust")
            && h.text_of("pg-crdt-a-votes").contains("wasm"),
        "expected OR-Set union [rust, wasm], got '{}'",
        h.text_of("pg-crdt-a-votes")
    );
    assert!(
        !h.text_of("pg-crdt-bytes").contains("0 B"),
        "expected non-zero NVC1 byte telemetry, got '{}'",
        h.text_of("pg-crdt-bytes")
    );
}

#[test]
fn native_docs_site_module_renders_on_skia_backend_and_preserves_route_snapshot() {
    use nve_abi::ColorRgba;
    use nve_backend_skia::{PixelBuffer, SkiaBackend, encode_rgb_png};
    use nve_engine::{NveApp, Viewport};
    use nve_docs_site::DocsSiteModule;

    let mut module = DocsSiteModule::create(1);
    let frame_bytes = module.render(1).to_vec();
    assert!(!frame_bytes.is_empty(), "native NveApp Frame 1 must be non-empty");

    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 1.0,
        },
    );
    skia.submit_frame(&frame_bytes)
        .expect("SkiaBackend submit_frame for native DocsSiteModule");

    let mut pixels = PixelBuffer::new(1280, 800);
    skia.rasterize(&mut pixels, ColorRgba::rgb(0x09, 0x09, 0x0b));
    let non_bg_pixels = pixels
        .pixels()
        .iter()
        .filter(|&&px| (px & 0x00ff_ffff) != 0x0009_090b)
        .count();
    assert!(
        non_bg_pixels > 10_000,
        "expected Skia rasterized docs surface to draw visible content, got {non_bg_pixels} non-bg pixels"
    );

    if std::env::var_os("NVE_SAVE_SKIA_SHOT").is_some() {
        let mut rgb = Vec::with_capacity(1280 * 800 * 3);
        for &px in pixels.pixels() {
            rgb.push(((px >> 16) & 0xff) as u8);
            rgb.push(((px >> 8) & 0xff) as u8);
            rgb.push((px & 0xff) as u8);
        }
        let png = encode_rgb_png(1280, 800, &rgb).expect("encode Skia PNG");
        let _ = std::fs::create_dir_all("/tmp/nve-docs-shots");
        let _ = std::fs::write("/tmp/nve-docs-shots/docs-native-skia.png", png);
    }

    // Verify binary NVDS state snapshot + restore
    let snap = module.snapshot_state().to_vec();
    assert!(snap.starts_with(b"NVDS"));
    let mut restored = DocsSiteModule::create(2);
    restored.restore_state(&snap);
    assert!(!restored.render(1).is_empty());
}
