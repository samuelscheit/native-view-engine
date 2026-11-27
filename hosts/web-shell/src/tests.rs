use super::*;
use nve_abi::{
    component_kind_to_scene_u8, ComponentKind, EventMask, EventPayload, NativeEvent,
    NativeEventKind, NodeId, NveRenderNode, NveSceneRef, RawPropValue, NVE_SCENE_SCHEMA_HASH,
    ROOT_NODE_ID,
};

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
        .unwrap_or_else(|| panic!("expected mounted node with test_id='{test_id}'"));
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
        payload: EventPayload::Point { x: 16.0, y: 12.0 },
    });
    session
        .pump_events()
        .unwrap()
        .unwrap_or_else(|| panic!("pressing '{test_id}' should trigger an incremental frame"))
}

fn assert_valid_dom_html(html: &str) {
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

fn assert_scene_view_matches_backend(session: &WebAppSession) {
    let view = session.scene().view();
    let ptr_view = unsafe { *session.scene().view_ptr() };
    assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(ptr_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(ptr_view.frame_id, view.frame_id);
    assert_eq!(ptr_view.node_count, view.node_count);
    assert_eq!(ptr_view.nodes, view.nodes);

    let scene = unsafe { NveSceneRef::from_raw(ptr_view) }
        .expect("scene_view_ptr must yield a valid NveSceneRef");
    let backend = session.backend();
    assert_eq!(scene.nodes().len(), backend.node_count());

    for rnode in scene.nodes() {
        let el = backend
            .node(rnode.node_id)
            .unwrap_or_else(|| panic!("scene node {} missing in WebBackend", rnode.node_id));
        assert_eq!(rnode.kind, component_kind_to_scene_u8(el.kind as u16));
        assert_eq!(
            rnode.is_leaf_text(),
            el.kind == ComponentKind::Text && el.children.is_empty()
        );
        assert_eq!(
            rnode.is_interactive(),
            el.kind == ComponentKind::Pressable || el.event_mask != EventMask::NONE
        );
        assert_eq!(rnode.clips_bounds(), el.kind == ComponentKind::ScrollView);
        assert_eq!(
            rnode.opacity_u8,
            NveRenderNode::encode_opacity_u8(el.style.opacity)
        );
        assert_eq!(rnode.bg_rgba, el.style.background.to_u32());
        assert_eq!(rnode.text_rgba, el.style.text_color.to_u32());
        assert_eq!(rnode.border_rgba, el.style.border_color.to_u32());
        assert_eq!(rnode.border_radius.to_bits(), el.style.border_radius.to_bits());
        assert_eq!(rnode.border_width.to_bits(), el.style.border_width.to_bits());
        assert_eq!(rnode.font_size.to_bits(), el.style.font_size.to_bits());
        assert_eq!(rnode.z_index, el.style.z_index);
        assert_eq!(
            scene.node_text(rnode).unwrap(),
            el.text.as_deref().unwrap_or("")
        );
        assert_eq!(scene.node_children(rnode).unwrap(), el.children.as_slice());
    }
}

#[test]
fn kitchen_sink_app_mounts_clicks_incrementally_and_resets() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    let init = session.render_initial().unwrap();
    assert!(init.op_count > 0);
    assert!(init.mutation_count > 0);

    let html0 = session.backend().render_html();
    assert!(html0.contains("NVE Kitchen Sink"));
    assert!(html0.contains("Count: 0"));
    assert!(html0.contains("Increment +"));
    assert!(html0.contains("Reset"));
    assert_scene_view_matches_backend(&session);

    for expected in 1..=2 {
        let timing = press_test_id(&mut session, "ks-counter-inc", expected as u64 * 1000);
        assert!(
            timing.mutation_count > 0 && timing.mutation_count < 24,
            "incremental click should only patch changed nodes, got {} mutations",
            timing.mutation_count
        );
        let html = session.backend().render_html();
        assert!(html.contains(&format!("Count: {expected}")));
        assert_scene_view_matches_backend(&session);
        let view = session.scene().view();
        assert!(view.dirty_count >= 1 && view.dirty_count < view.node_count);
        assert_eq!(view.removed_count, 0);
    }

    press_test_id(&mut session, "ks-counter-reset", 3000);
    assert!(session.backend().render_html().contains("Count: 0"));
    assert_scene_view_matches_backend(&session);
}

#[test]
fn handles_nvp1_hot_patch_frames_for_tier1_tier2_and_status() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    session.render_initial().unwrap();
    assert_scene_view_matches_backend(&session);

    // Tier-1 TemplatePatch over binary NVP1 wire frame.
    let tpl = HotPatchMsg::TemplatePatch {
        generation: 3,
        template_id: "KitchenSinkApp".into(),
        attr_updates: vec![("border-color".into(), "#38bdf8".into())],
        text_updates: vec![(0, "NVE LIVE PATCHED".into())],
        target_test_id: None,
        node_path: vec![],
        text_replacements: vec![],
    };
    let act1 = session.handle_hot_patch_frame(&tpl.to_bytes()).unwrap();
    match act1 {
        HotReloadAction::TemplatePatched {
            generation,
            template_id,
            mutations,
        } => {
            assert_eq!(generation, 3);
            assert_eq!(template_id, "KitchenSinkApp");
            assert!(mutations > 0);
        }
        other => panic!("unexpected action: {other:?}"),
    }
    assert!(session.backend().render_html().contains("NVE LIVE PATCHED"));
    assert_scene_view_matches_backend(&session);
    let scene_ref = session.scene().as_ref();
    let patched_in_scene = scene_ref
        .nodes()
        .iter()
        .any(|node| scene_ref.node_text(node).ok() == Some("NVE LIVE PATCHED"));
    assert!(patched_in_scene);

    // Tier-2 DylibReload over a binary NVP1 frame.
    let reload = HotPatchMsg::DylibReload {
        generation: 4,
        dylib_path: "target/debug/libnve_example_kitchen_sink.dylib".into(),
        build_ms: 215,
    };
    let act2 = session.handle_hot_patch_frame(&reload.to_bytes()).unwrap();
    assert_eq!(
        act2,
        HotReloadAction::ReloadPage {
            generation: 4,
            dylib_path: "target/debug/libnve_example_kitchen_sink.dylib".into(),
            build_ms: 215,
        }
    );

    // DevStatus over binary NVP1 wire frame.
    let status = HotPatchMsg::DevStatus {
        message: "watching examples/kitchen-sink/src".into(),
        latency_ms: 12,
    };
    let act3 = session.handle_hot_patch_frame(&status.to_bytes()).unwrap();
    assert_eq!(
        act3,
        HotReloadAction::DevStatus {
            message: "watching examples/kitchen-sink/src".into(),
            latency_ms: 12,
        }
    );

    // Legacy JSON text, unknown variants, truncation and trailing bytes are all rejected.
    assert!(session.handle_hot_patch_frame(br#"{"type":"dev_status"}"#).is_err());
    assert!(session.handle_hot_patch_frame(b"NVP1\xFF").is_err());
    let frame = status.to_bytes();
    assert!(session.handle_hot_patch_frame(&frame[..frame.len() - 1]).is_err());
    let mut trailing = frame.clone();
    trailing.push(0);
    assert!(session.handle_hot_patch_frame(&trailing).is_err());
}

#[test]
fn studio_workspace_mounts_interacts_and_time_travels() {
    let viewport = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let mut session = WebAppSession::new_studio_workspace(1, WebBackend::new(), viewport);

    let init = session.render_initial().unwrap();
    assert!(init.op_count > 0);
    assert!(init.mutation_count > 0);
    assert!(session.backend().node_count() > 20);
    assert_scene_view_matches_backend(&session);

    let html0 = session.backend().render_html();
    assert_valid_dom_html(&html0);
    assert!(html0.contains("AETHER"));
    assert!(html0.contains("Issue Board"));
    assert!(html0.contains("NVE STUDIO"));
    assert!(html0.contains("LIVE SYNC"));
    assert!(html0.contains("LAN QUIC"));
    assert!(html0.contains("AET-101"));
    assert!(html0.contains("Zero-copy EventRing ABI"));
    assert!(html0.contains("In Progress"));
    assert!(html0.contains("Done (1)"));
    assert!(html0.contains("Snapshot 1 / 1"));
    assert!(html0.contains("Workspace Boot"));

    let t_status = press_test_id(&mut session, "task-status-1", 1000);
    assert!(t_status.mutation_count > 0);
    let html_after_status = session.backend().render_html();
    assert_valid_dom_html(&html_after_status);
    assert!(html_after_status.contains("Status -&gt; Done"));
    assert!(html_after_status.contains("Done (2)"));
    assert!(html_after_status.contains("Snapshot 2 / 2"));
    assert_scene_view_matches_backend(&session);

    press_test_id(&mut session, "nav-engineering", 2000);
    assert!(
        session.scene().view().removed_count > 0,
        "switching views should report removed nodes in NveSceneView"
    );
    assert_scene_view_matches_backend(&session);
    let html_eng = session.backend().render_html();
    assert_valid_dom_html(&html_eng);
    assert!(html_eng.contains("# engineering"));
    assert!(html_eng.contains("Single ABI drives all 4 hosts!"));
    assert!(html_eng.contains("QUICK REPLY"));
    assert!(html_eng.contains("Snapshot 3 / 3"));

    press_test_id(&mut session, "nav-metrics", 3000);
    assert_scene_view_matches_backend(&session);
    let html_metrics = session.backend().render_html();
    assert_valid_dom_html(&html_metrics);
    assert!(html_metrics.contains("Live Metrics"));
    assert!(html_metrics.contains("Node Batch #01"));
    assert!(html_metrics.contains("Snapshot 4 / 4"));

    press_test_id(&mut session, "accent-violet", 4000);
    assert_scene_view_matches_backend(&session);
    let html_violet = session.backend().render_html();
    assert_valid_dom_html(&html_violet);
    assert!(html_violet.contains("Accent -&gt; Violet"));
    assert!(html_violet.contains("rgba(167,139,250,1)"));
    assert!(html_violet.contains("Snapshot 5 / 5"));

    press_test_id(&mut session, "tt-undo", 5000);
    let html_undo = session.backend().render_html();
    assert_valid_dom_html(&html_undo);
    assert!(html_undo.contains("Snapshot 4 / 5"));
    assert!(html_undo.contains("Live Metrics"));

    press_test_id(&mut session, "tt-first", 6000);
    let html_first = session.backend().render_html();
    assert_valid_dom_html(&html_first);
    assert!(html_first.contains("Snapshot 1 / 5"));
    assert!(html_first.contains("Workspace Boot"));
    assert!(html_first.contains("Issue Board"));
    assert!(html_first.contains("Done (1)"));

    press_test_id(&mut session, "tt-live", 7000);
    assert_scene_view_matches_backend(&session);
    let html_live = session.backend().render_html();
    assert_valid_dom_html(&html_live);
    assert!(html_live.contains("Snapshot 5 / 5"));
    assert!(html_live.contains("Accent -&gt; Violet"));
    assert!(html_live.contains("Live Metrics"));
}

#[test]
fn scene_view_is_valid_before_initial_render_and_tracks_pointer_stability() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    let ptr_before = session.scene().view_ptr();
    let empty_view = unsafe { *ptr_before };
    assert_eq!(empty_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(empty_view.node_count, 0);
    let empty_ref = unsafe { NveSceneRef::from_raw(empty_view) }.unwrap();
    assert!(empty_ref.nodes().is_empty());

    session.render_initial().unwrap();
    assert_eq!(session.scene().view_ptr(), ptr_before);
    let mounted_view = unsafe { *ptr_before };
    assert_eq!(mounted_view.frame_id, 1);
    assert_eq!(
        mounted_view.node_count as usize,
        session.backend().node_count()
    );
    assert_scene_view_matches_backend(&session);
}

#[test]
fn studio_workspace_full_parity_canvas_img_cmdk_stream_islands_and_mobile_resize() {
    let desktop_vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 2.0,
    };
    let mut session = WebAppSession::new_studio_workspace(1, WebBackend::new(), desktop_vp);
    session.render_initial().unwrap();
    assert_scene_view_matches_backend(&session);

    // 1. Image avatar `<img>` carries `object-fit:cover;`, `alt`, and PNG data-URI `src`.
    let avatar_id = find_node_by_test_id(session.backend(), "task-avatar-1")
        .expect("expected task-avatar-1 image node");
    let avatar_el = session.backend().node(avatar_id).unwrap();
    assert_eq!(avatar_el.kind, ComponentKind::Image);
    assert_eq!(avatar_el.raw_style.object_fit, Some("cover"));
    assert!(avatar_el.combined_css().contains("object-fit:cover;"));
    assert!(matches!(
        avatar_el.raw_props.get("src"),
        Some(RawPropValue::String(s)) if s.starts_with("data:image/png;base64,")
    ));
    assert!(matches!(
        avatar_el.raw_props.get("alt"),
        Some(RawPropValue::String(s)) if !s.is_empty()
    ));

    // 2. Board TextInput search + custom task create via `submit` (and empty submit is a safe no-op).
    let search_id = find_node_by_test_id(session.backend(), "board-search-input").unwrap();
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: search_id,
        timestamp_us: 1000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("AppKit".into()),
    });
    session.pump_events().unwrap().expect("search filter frame");
    let html_filtered = session.backend().render_html();
    assert!(html_filtered.contains("AET-102"));
    assert!(!html_filtered.contains("AET-101"));
    press_test_id(&mut session, "board-search-clear", 1500);
    assert!(session.backend().render_html().contains("AET-101"));

    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: ROOT_NODE_ID,
        timestamp_us: 1600,
        kind: NativeEventKind::Submit,
        payload: EventPayload::Text(String::new()),
    });
    assert!(session.pump_events().unwrap().is_none());

    let new_task_id = find_node_by_test_id(session.backend(), "board-new-task-input").unwrap();
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: new_task_id,
        timestamp_us: 1700,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Web Canvas Parity".into()),
    });
    session.pump_events().unwrap().expect("draft update");
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: new_task_id,
        timestamp_us: 1800,
        kind: NativeEventKind::Submit,
        payload: EventPayload::Text("Web Canvas Parity".into()),
    });
    session.pump_events().unwrap().expect("submit custom task");
    assert!(session.backend().render_html().contains("Web Canvas Parity"));
    assert_scene_view_matches_backend(&session);

    // 3. Cmd+K palette open -> ARIA dialog/searchbox -> filter -> ArrowDown -> Enter -> Escape.
    assert!(session.dispatch_keydown("k", true, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.backend(), "command-palette").is_some());
    let pal_html = session.backend().render_html();
    assert!(pal_html.contains("aria-label=\"Command Palette\""));
    assert!(pal_html.contains("role=\"dialog\""));
    assert!(pal_html.contains("role=\"searchbox\""));

    let pal_in = find_node_by_test_id(session.backend(), "palette-input").unwrap();
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: pal_in,
        timestamp_us: 2000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Accent:".into()),
    });
    session.pump_events().unwrap().expect("palette filter");
    assert!(session.dispatch_keydown("ArrowDown", false, false).unwrap().is_some());
    assert!(session.dispatch_keydown("Enter", false, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.backend(), "command-palette").is_none());
    assert!(session.backend().render_html().contains("Accent -&gt; Violet"));

    assert!(session.dispatch_keydown("k", false, true).unwrap().is_some());
    assert!(find_node_by_test_id(session.backend(), "command-palette").is_some());
    assert!(session.dispatch_keydown("Escape", false, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.backend(), "command-palette").is_none());

    // 4. Islands view bus broadcast & gesture arbitration.
    press_test_id(&mut session, "nav-islands", 3000);
    press_test_id(&mut session, "island-bus-theme", 3100);
    press_test_id(&mut session, "island-gesture-claim", 3200);
    let html_islands = session.backend().render_html();
    assert!(html_islands.contains("Bus: 1 tx"));
    assert!(html_islands.contains("Claim | winner:"));

    // 5. 100k Stream view jump #50k, #99,999, and NativeEventKind::Scroll.
    press_test_id(&mut session, "nav-stream", 4000);
    press_test_id(&mut session, "stream-jump-50k", 4100);
    assert!(session.backend().render_html().contains("STR-50000"));
    press_test_id(&mut session, "stream-jump-end", 4200);
    assert!(session.backend().render_html().contains("STR-99999"));
    let stream_vp = find_node_by_test_id(session.backend(), "stream-viewport").unwrap();
    session.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: stream_vp,
        timestamp_us: 4300,
        kind: NativeEventKind::Scroll,
        payload: EventPayload::Scroll {
            x: 0.0,
            y: 0.0,
            content_width: 800.0,
            content_height: 4_000_000.0,
        },
    });
    session.pump_events().unwrap().expect("scroll frame");
    let html_top = session.backend().render_html();
    assert!(html_top.contains("Rows #0..") || html_top.contains("STR-00001"));

    // 6. Metrics view `<canvas>` receives direct typed `DrawList` ops without raw `drawList` bytes.
    press_test_id(&mut session, "nav-metrics", 5000);
    let canvas_id = find_node_by_test_id(session.backend(), "metrics-sparkline-canvas")
        .expect("expected metrics-sparkline-canvas");
    let canvas_el = session.backend().node(canvas_id).unwrap();
    assert_eq!(canvas_el.kind, ComponentKind::SkiaCanvas);
    assert!(
        canvas_el.draw_op_count > 3,
        "expected direct typed DrawList ops on metrics canvas, got {}",
        canvas_el.draw_op_count
    );
    assert!(
        !canvas_el.raw_props.contains_key("drawList"),
        "in-process RenderSink must not serialize drawList into raw_props"
    );

    // 7. Dynamic viewport resize across 620pt mobile breakpoint and back to desktop.
    assert!(find_node_by_test_id(session.backend(), "desktop-sidebar").is_some());
    assert!(find_node_by_test_id(session.backend(), "mobile-top-bar").is_none());

    let mobile_vp = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
    };
    assert!(session.resize_viewport(mobile_vp).unwrap().is_some());
    assert_eq!(session.backend().viewport(), mobile_vp);
    assert!(find_node_by_test_id(session.backend(), "mobile-top-bar").is_some());
    assert!(find_node_by_test_id(session.backend(), "desktop-sidebar").is_none());
    assert!(find_node_by_test_id(session.backend(), "center-workspace").is_some());
    assert_scene_view_matches_backend(&session);

    assert!(session.resize_viewport(desktop_vp).unwrap().is_some());
    assert!(find_node_by_test_id(session.backend(), "desktop-sidebar").is_some());
    assert!(find_node_by_test_id(session.backend(), "mobile-top-bar").is_none());
    assert_scene_view_matches_backend(&session);
}

#[test]
fn pump_async_work_reports_hit_cap_and_finishes_via_continuation() {
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::task::{Poll, Waker};
    use nve_dioxus::prelude::*;
    use nve_dioxus::DioxusSurface;

    #[derive(Clone, Default)]
    struct StepGate {
        ready: Arc<AtomicBool>,
        waker: Rc<RefCell<Option<Waker>>>,
    }

    impl StepGate {
        fn trigger(&self) {
            self.ready.store(true, Ordering::Release);
            if let Some(w) = self.waker.borrow_mut().take() {
                w.wake();
            }
        }
    }

    #[component]
    fn BurstAsyncApp() -> Element {
        let gate = dioxus_core::consume_context::<StepGate>();
        let mut count = use_signal(|| 0u32);
        let gate_for_task = gate.clone();
        use_hook(move || {
            spawn(async move {
                // 24 render-chained steps exceed MAX_PUMPS (8) so the first `pump_async_work_with`
                // MUST hit the pump cap and call `waker.wake_by_ref()` instead of stalling!
                for step in 1..=24u32 {
                    core::future::poll_fn(|cx| {
                        if gate_for_task.ready.swap(false, Ordering::AcqRel) {
                            Poll::Ready(())
                        } else {
                            *gate_for_task.waker.borrow_mut() = Some(cx.waker().clone());
                            Poll::Pending
                        }
                    })
                    .await;
                    count.set(step);
                }
            });
        });
        let cur = count();
        if cur > 0 && cur < 24 {
            gate.trigger();
        }
        rsx! { text { "STEP:{cur}" } }
    }

    let gate = StepGate::default();
    let surface = DioxusSurface::new(1, BurstAsyncApp).with_context(gate.clone());
    let mut session = WebAppSession::new(1, surface, WebBackend::new(), Viewport::default());
    session.render_initial().unwrap();

    let waker = Waker::noop();
    let idle = session.pump_async_work_with(waker, || {}).unwrap();
    assert!(idle.timing.is_none());
    assert!(!idle.hit_cap);

    // Kick off the 24-step render-chained async burst -> first tick MUST hit MAX_PUMPS (8) before
    // reaching STEP:24 and report `hit_cap = true` so the browser host yields to paint via macrotask.
    gate.trigger();
    let first = session.pump_async_work_with(waker, || {}).unwrap();
    assert!(first.timing.is_some());
    assert!(
        first.hit_cap,
        "24-step burst must hit MAX_PUMPS (8) and signal macrotask continuation"
    );
    assert!(
        !session.backend().render_html().contains("STEP:24"),
        "first tick must bound work at MAX_PUMPS instead of running all 24 steps: {}",
        session.backend().render_html()
    );

    let mut final_hit_cap = true;
    for _ in 0..8 {
        let next = session.pump_async_work_with(waker, || {}).unwrap();
        final_hit_cap = next.hit_cap;
        if session.backend().render_html().contains("STEP:24") {
            break;
        }
    }
    assert!(!final_hit_cap, "settled burst must clear hit_cap");
    assert!(
        session.backend().render_html().contains("STEP:24"),
        "continuation pumps must finish all 24 steps: {}",
        session.backend().render_html()
    );
}
