//! Deep DOM-mirror and `wasm32` integration tests for `nve-web-shell`.
//!
//! - On native targets (`cargo test --locked -p nve-web-shell`): exercises full multi-view
//!   Aether Workspace + NVE Studio workflows, Counter rapid-click & edge-case resilience,
//!   and Tier-1/Tier-2 hot-patch behavior via `WebBackend`'s DOM mirror.
//! - On `wasm32-unknown-unknown` (`cargo check --locked -p nve-web-shell --tests --target wasm32-unknown-unknown`):
//!   verifies the `wasm_bindgen` entrypoints and `WebAppController` surface compile cleanly
//!   with zero warnings.

use nve_abi::{
    EventMask, EventPayload, NativeEvent, NativeEventKind, NodeId, RawPropValue, ROOT_NODE_ID,
};
use nve_backend_web::WebBackend;
use nve_engine::{FrameTiming, HotPatchMsg, Viewport};
use nve_web_shell::{HotReloadAction, WebAppSession};

fn find_node_by_test_id(backend: &WebBackend, target: &str) -> Option<NodeId> {
    fn walk(backend: &WebBackend, id: NodeId, target: &str) -> Option<NodeId> {
        let el = backend.node(id)?;
        if matches!(
            el.raw_props.get("testID"),
            Some(RawPropValue::String(s)) if s == target
        ) {
            return Some(id);
        }
        for &child in &el.children {
            if let Some(found) = walk(backend, child, target) {
                return Some(found);
            }
        }
        None
    }
    walk(backend, ROOT_NODE_ID, target)
}

fn press_test_id(session: &mut WebAppSession, test_id: &str, timestamp_us: u64) -> FrameTiming {
    let node_id = find_node_by_test_id(session.backend(), test_id)
        .unwrap_or_else(|| panic!("expected mounted node with testID='{test_id}'"));
    assert_ne!(
        session.backend().event_mask_of(node_id) & EventMask::PRESS,
        0,
        "node '{test_id}' (id={node_id}) should have EventMask::PRESS"
    );
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id,
        timestamp_us,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 12.0, y: 10.0 },
    });
    session
        .pump_events()
        .expect("event pump should succeed")
        .unwrap_or_else(|| panic!("pressing '{test_id}' should trigger an incremental frame"))
}

fn assert_balanced_dom(html: &str) {
    assert!(
        html.starts_with("<div data-nve-id=\"1\""),
        "DOM root should be node 1 <div>"
    );
    assert!(html.ends_with("</div>"), "DOM HTML should close root </div>");
    for tag in ["div", "button", "span"] {
        let opens = html.matches(&format!("<{tag} ")).count();
        let closes = html.matches(&format!("</{tag}>")).count();
        assert_eq!(
            opens, closes,
            "unbalanced <{tag}> tags in rendered DOM HTML (opens={opens}, closes={closes})"
        );
    }
}

#[test]
fn studio_workspace_full_workflow_and_dom_invariants() {
    let viewport = Viewport {
        width: 1280.0,
        height: 840.0,
        scale_factor: 2.0,
    };
    let mut session = WebAppSession::new_studio_workspace(1, WebBackend::new(), viewport);
    session.render_initial().expect("initial mount");

    let html0 = session.backend().render_html();
    assert_balanced_dom(&html0);
    assert!(html0.contains("AETHER"));
    assert!(html0.contains("Issue Board"));
    assert!(html0.contains("NVE STUDIO"));
    assert!(html0.contains("LIVE SYNC"));
    assert!(html0.contains("LAN QUIC"));
    assert!(html0.contains("X-Ray HUD: OFF"));
    assert!(find_node_by_test_id(session.backend(), "studio-panel").is_some());
    assert!(find_node_by_test_id(session.backend(), "studio-sync-telemetry").is_some());

    // Cycle Live Sync transport mode via Studio panel button.
    press_test_id(&mut session, "sync-transport-cycle", 50);
    let after_sync = session.backend().render_html();
    assert_balanced_dom(&after_sync);
    assert!(after_sync.contains("USB Mux"));
    assert!(after_sync.contains("1 peer"));

    // Create a new task and verify board counts + filter pills respond cleanly.
    press_test_id(&mut session, "btn-new-task", 100);
    let after_new = session.backend().render_html();
    assert_balanced_dom(&after_new);
    assert!(after_new.contains("All (5)"));

    press_test_id(&mut session, "filter-done", 200);
    let after_done_filter = session.backend().render_html();
    assert_balanced_dom(&after_done_filter);
    assert!(after_done_filter.contains("Done (1)"));

    press_test_id(&mut session, "filter-all", 300);

    // Switch to # engineering, react to a message, and post a quick reply.
    press_test_id(&mut session, "nav-engineering", 400);
    let eng_html = session.backend().render_html();
    assert_balanced_dom(&eng_html);
    assert!(eng_html.contains("Single ABI drives all 4 hosts!"));
    assert!(eng_html.contains("QUICK REPLY"));

    press_test_id(&mut session, "reply-ship", 500);
    let after_reply = session.backend().render_html();
    assert_balanced_dom(&after_reply);
    assert!(after_reply.contains("Ship It - all checks green!"));

    // Switch to # design-sync and then Live Metrics with stress row mutations.
    press_test_id(&mut session, "nav-design", 600);
    assert_balanced_dom(&session.backend().render_html());

    press_test_id(&mut session, "nav-metrics", 700);
    press_test_id(&mut session, "stress-add-8", 800);
    let after_stress = session.backend().render_html();
    assert_balanced_dom(&after_stress);
    assert!(after_stress.contains("Node Batch #12"));

    press_test_id(&mut session, "stress-add-24", 850);
    let after_stress_more = session.backend().render_html();
    assert_balanced_dom(&after_stress_more);
    assert!(after_stress_more.contains("Node Batch #36"));

    press_test_id(&mut session, "stress-reset", 900);
    let after_reset = session.backend().render_html();
    assert_balanced_dom(&after_reset);
    assert!(!after_reset.contains("Node Batch #12"));

    // Toggle radius, density, and X-Ray HUD, then verify Undo + Redo.
    press_test_id(&mut session, "radius-sharp", 1000);
    press_test_id(&mut session, "density-compact", 1100);
    press_test_id(&mut session, "toggle-xray", 1200);
    let xray_on = session.backend().render_html();
    assert_balanced_dom(&xray_on);
    assert!(xray_on.contains("X-Ray HUD: ON"));

    press_test_id(&mut session, "tt-undo", 1300);
    let after_undo = session.backend().render_html();
    assert!(after_undo.contains("X-Ray HUD: OFF"));

    press_test_id(&mut session, "tt-redo", 1400);
    let after_redo = session.backend().render_html();
    assert!(after_redo.contains("X-Ray HUD: ON"));

    // Collapse and reopen the docked NVE Studio panel.
    press_test_id(&mut session, "btn-toggle-studio", 1500);
    assert!(find_node_by_test_id(session.backend(), "studio-panel").is_none());

    press_test_id(&mut session, "btn-toggle-studio", 1600);
    assert!(find_node_by_test_id(session.backend(), "studio-panel").is_some());
}

#[test]
fn counter_app_rapid_clicks_and_unhandled_event_resilience() {
    let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
    session.render_initial().expect("initial counter mount");

    let mut pressable: Vec<NodeId> = (1..=64)
        .filter(|&id| (session.backend().event_mask_of(id) & EventMask::PRESS) != 0)
        .collect();
    pressable.sort_unstable();
    assert_eq!(pressable.len(), 2);
    let (inc_id, reset_id) = (pressable[0], pressable[1]);

    // Unhandled press on a nonexistent node must be ignored cleanly (no frame, no panic).
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: 99_999,
        timestamp_us: 10,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 0.0, y: 0.0 },
    });
    assert!(session.pump_events().expect("pump should not fail").is_none());

    for step in 1..=10u64 {
        session.backend().push_event(NativeEvent {
            surface_id: 1,
            node_id: inc_id,
            timestamp_us: step * 100,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 40.0, y: 20.0 },
        });
        let timing = session
            .pump_events()
            .expect("increment pump")
            .expect("increment frame");
        assert!(timing.mutation_count <= 2);
        let html = session.backend().render_html();
        assert_balanced_dom(&html);
        assert!(html.contains(&format!("Count: {step}")));
    }

    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: reset_id,
        timestamp_us: 2000,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 40.0, y: 20.0 },
    });
    session.pump_events().expect("reset pump").expect("reset frame");
    assert!(session.backend().render_html().contains("Count: 0"));
}

#[test]
fn hot_patch_empty_and_out_of_bounds_edge_cases() {
    let mut session = WebAppSession::new_counter(1, WebBackend::new(), Viewport::default());
    session.render_initial().expect("initial mount");

    // Empty TemplatePatch succeeds with 0 mutations.
    let empty_patch = HotPatchMsg::TemplatePatch {
        generation: 1,
        template_id: "CounterApp".into(),
        attr_updates: vec![],
        text_updates: vec![],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    };
    let action = session
        .handle_hot_patch_frame(&empty_patch.to_bytes())
        .expect("empty template patch");
    assert_eq!(
        action,
        HotReloadAction::TemplatePatched {
            generation: 1,
            template_id: "CounterApp".into(),
            mutations: 0,
        }
    );

    // Out-of-bounds text index does not panic and leaves DOM valid.
    let oob_patch = HotPatchMsg::TemplatePatch {
        generation: 2,
        template_id: "CounterApp".into(),
        attr_updates: vec![("outline".into(), "1px solid #38bdf8".into())],
        text_updates: vec![(9999, "ignored-oob".into())],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    };
    let oob_action = session
        .handle_hot_patch_frame(&oob_patch.to_bytes())
        .expect("oob patch");
    match oob_action {
        HotReloadAction::TemplatePatched {
            generation,
            mutations,
            ..
        } => {
            assert_eq!(generation, 2);
            assert!(mutations > 0);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert_balanced_dom(&session.backend().render_html());

    // Malformed payloads return Err without corrupting the session.
    assert!(session.handle_hot_patch_frame(b"").is_err());
    assert!(session.handle_hot_patch_frame(b"null").is_err());
    assert!(session.handle_hot_patch_frame(b"{\"type\":\"unknown\"}").is_err());
    assert!(session.handle_hot_patch_frame(b"NVP1\x01").is_err());
}

#[cfg(target_arch = "wasm32")]
mod wasm_exports_compile_check {
    use nve_web_shell::{start_counter_app, start_studio_workspace_app, WebAppController};

    #[allow(dead_code)]
    fn assert_wasm_entrypoints_signature() {
        let _: fn(&str, Option<String>) -> Result<WebAppController, wasm_bindgen::JsValue> =
            start_studio_workspace_app;
        let _: fn(&str, Option<String>) -> Result<WebAppController, wasm_bindgen::JsValue> =
            start_counter_app;
        let _: fn(&WebAppController) -> String = WebAppController::app_name;
        let _: fn(&WebAppController) -> String = WebAppController::mount_id;
        let _: fn(&WebAppController) -> usize = WebAppController::node_count;
        let _: fn(&WebAppController) -> String = WebAppController::render_html;
        let _: fn(&WebAppController) = WebAppController::unmount;
    }
}
