#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use crate::session::*;
#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use nve_abi::{
    EventMask, EventPayload, NativeEvent, NativeEventKind, NveRenderNode, NveSceneRef,
    NVE_SCENE_SCHEMA_HASH, ROOT_NODE_ID,
};
use nve_abi::NodeId;
#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use nve_canvas::Canvas;
use nve_components::tables::DebugId;
#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use nve_components::tables::{Image, ImageFit, Interaction, Overflow, TextContent, Typography, Visual};
#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use nve_backend_web::WebBackend;
#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
use nve_engine::{FrameTiming, HotPatchMsg, Viewport};
use nve_store::{NodeId as StoreNodeId, NodeStore};

pub(crate) fn find_node_by_test_id(store: &NodeStore, target: &str) -> Option<NodeId> {
    fn walk(store: &NodeStore, id: StoreNodeId, target: &str) -> Option<NodeId> {
        if !store.is_live(id) {
            return None;
        }
        if store
            .try_row::<DebugId>(id)
            .is_some_and(|d| d.test_id() == target)
        {
            return Some(id.raw());
        }
        for &child in store.children(id) {
            if let Some(found) = walk(store, child, target) {
                return Some(found);
            }
        }
        None
    }
    walk(store, StoreNodeId::ROOT, target)
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
fn press_test_id(session: &mut WebAppSession, test_id: &str, timestamp_us: u64) -> FrameTiming {
    let node_id = find_node_by_test_id(session.store(), test_id)
        .expect("expected mounted node with requested test_id");
    let mask = session
        .store()
        .try_row::<Interaction>(StoreNodeId(node_id))
        .map_or(0, |i| u64::from(i.event_mask()));
    assert_ne!(
        mask & EventMask::PRESS,
        0,
        "node '{test_id}' (id={node_id}) should have EventMask::PRESS"
    );
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id,
        timestamp_us,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 16.0, y: 12.0 },
    });
    session
        .pump_events()
        .expect("pump_events should succeed")
        .expect("pressing test_id should trigger an incremental frame")
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
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

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
fn assert_scene_view_matches_store(session: &WebAppSession) {
    let view = session.scene().view();
    // SAFETY: `session.scene().view_ptr()` stays valid while `session` is immutably borrowed.
    let ptr_view = unsafe { *session.scene().view_ptr() };
    assert_eq!(view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(ptr_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(ptr_view.frame_id, view.frame_id);
    assert_eq!(ptr_view.node_count, view.node_count);
    assert_eq!(ptr_view.nodes, view.nodes);

    // SAFETY: `ptr_view` borrows `session.scene()` which remains alive and unmutated for `scene`.
    let scene = unsafe { NveSceneRef::from_raw(ptr_view) }
        .expect("scene_view_ptr must yield a valid NveSceneRef");
    let store = session.store();
    assert_eq!(scene.nodes().len(), session.node_count());

    for rnode in scene.nodes() {
        let sid = StoreNodeId(rnode.node_id);
        assert!(store.is_live(sid), "scene node {} missing in NodeStore", rnode.node_id);
        assert_eq!(rnode.primary_slot, store.primary_slot(sid).0);
        let children = store.children(sid);
        assert_eq!(
            rnode.is_leaf_text(),
            store.has::<TextContent>(sid) && children.is_empty()
        );
        assert_eq!(
            rnode.is_interactive(),
            store
                .try_row::<Interaction>(sid)
                .is_some_and(|i| i.event_mask() != 0)
        );
        assert_eq!(
            rnode.clips_bounds(),
            store.try_row::<Visual>(sid).is_some_and(|v| {
                matches!(v.overflow_x(), Overflow::Hidden | Overflow::Scroll)
                    || matches!(v.overflow_y(), Overflow::Hidden | Overflow::Scroll)
            })
        );
        let expected_opacity = store
            .try_row::<Visual>(sid)
            .map_or(255, |v| NveRenderNode::encode_opacity_u8(v.opacity()));
        assert_eq!(rnode.opacity_u8, expected_opacity);
        let expected_bg = store.try_row::<Visual>(sid).map_or(0, |v| v.background().0);
        assert_eq!(rnode.bg_rgba, expected_bg);
        let expected_text_color = store.try_row::<Typography>(sid).map_or(0, |t| t.color().0);
        assert_eq!(rnode.text_rgba, expected_text_color);
        let expected_border_color = store
            .try_row::<Visual>(sid)
            .map_or(0, |v| v.border_color().0);
        assert_eq!(rnode.border_rgba, expected_border_color);
        let expected_radius = store
            .try_row::<Visual>(sid)
            .map_or(0.0, |v| v.border_radius());
        assert_eq!(rnode.border_radius.to_bits(), expected_radius.to_bits());
        let expected_border_width = store
            .try_row::<Visual>(sid)
            .map_or(0.0, |v| v.border_width());
        assert_eq!(rnode.border_width.to_bits(), expected_border_width.to_bits());
        let expected_font_size = store
            .try_row::<Typography>(sid)
            .map_or(16.0, |t| t.font_size());
        assert_eq!(rnode.font_size.to_bits(), expected_font_size.to_bits());
        let expected_z = store.try_row::<Visual>(sid).map_or(0, |v| v.z_index());
        assert_eq!(rnode.z_index, expected_z);
        let expected_text = store
            .try_row::<TextContent>(sid)
            .map_or("", |t| t.text());
        assert_eq!(scene.node_text(rnode).expect("valid scene node UTF-8"), expected_text);
        let expected_children: Vec<NodeId> = children.iter().map(|c| c.raw()).collect();
        assert_eq!(
            scene
                .node_children(rnode)
                .expect("valid scene node children"),
            expected_children.as_slice()
        );
    }
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
#[test]
fn kitchen_sink_app_mounts_clicks_incrementally_and_resets() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    let init = session.render_initial().unwrap();
    assert!(init.op_count > 0);
    assert!(init.mutation_count > 0);

    let html0 = session.render_html();
    assert!(html0.contains("NVE Kitchen Sink"));
    assert!(html0.contains("Count: 0"));
    assert!(html0.contains("Increment +"));
    assert!(html0.contains("Reset"));
    assert_scene_view_matches_store(&session);

    for expected in 1..=2 {
        let timing = press_test_id(&mut session, "ks-counter-inc", expected as u64 * 1000);
        assert!(
            timing.mutation_count > 0 && timing.mutation_count < 24,
            "incremental click should only patch changed nodes, got {} mutations",
            timing.mutation_count
        );
        let html = session.render_html();
        assert!(html.contains(&format!("Count: {expected}")));
        assert_scene_view_matches_store(&session);
        let view = session.scene().view();
        assert!(view.dirty_count >= 1 && view.dirty_count < view.node_count);
        assert_eq!(view.removed_count, 0);
    }

    press_test_id(&mut session, "ks-counter-reset", 3000);
    assert!(session.render_html().contains("Count: 0"));
    assert_scene_view_matches_store(&session);
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
#[test]
fn handles_nvp1_hot_patch_frames_for_tier1_tier2_and_status() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    session.render_initial().unwrap();
    assert_scene_view_matches_store(&session);

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
    assert!(session.render_html().contains("NVE LIVE PATCHED"));
    assert_scene_view_matches_store(&session);
    let scene_ref = session.scene_ref();
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
    assert!(session
        .handle_hot_patch_frame(br#"{"type":"dev_status"}"#)
        .is_err());
    assert!(session.handle_hot_patch_frame(b"NVP1\xFF").is_err());
    let frame = status.to_bytes();
    assert!(session
        .handle_hot_patch_frame(&frame[..frame.len() - 1])
        .is_err());
    let mut trailing = frame.clone();
    trailing.push(0);
    assert!(session.handle_hot_patch_frame(&trailing).is_err());
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
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
    assert!(session.node_count() > 20);
    assert_scene_view_matches_store(&session);

    let html0 = session.render_html();
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
    let html_after_status = session.render_html();
    assert_valid_dom_html(&html_after_status);
    assert!(html_after_status.contains("Status -&gt; Done"));
    assert!(html_after_status.contains("Done (2)"));
    assert!(html_after_status.contains("Snapshot 2 / 2"));
    assert_scene_view_matches_store(&session);

    press_test_id(&mut session, "nav-engineering", 2000);
    assert!(
        session.scene().view().removed_count > 0,
        "switching views should report removed nodes in NveSceneView"
    );
    assert_scene_view_matches_store(&session);
    let html_eng = session.render_html();
    assert_valid_dom_html(&html_eng);
    assert!(html_eng.contains("# engineering"));
    assert!(html_eng.contains("Single ABI drives all 4 hosts!"));
    assert!(html_eng.contains("QUICK REPLY"));
    assert!(html_eng.contains("Snapshot 3 / 3"));

    press_test_id(&mut session, "nav-metrics", 3000);
    assert_scene_view_matches_store(&session);
    let html_metrics = session.render_html();
    assert_valid_dom_html(&html_metrics);
    assert!(html_metrics.contains("Live Metrics"));
    assert!(html_metrics.contains("Node Batch #01"));
    assert!(html_metrics.contains("Snapshot 4 / 4"));

    press_test_id(&mut session, "accent-violet", 4000);
    assert_scene_view_matches_store(&session);
    let ssr_violet = session.render_ssr();
    assert_valid_dom_html(&ssr_violet.html);
    assert!(ssr_violet.html.contains("Accent -&gt; Violet"));
    let violet = nve_studio_workspace::theme::studio_theme(
        &nve_studio_workspace::StudioConfig::default(),
        nve_studio_workspace::AccentTheme::Violet,
        nve_studio_workspace::theme::Appearance::DARK,
    );
    let c = violet.status.accent.solid;
    let violet_solid = format!("#{:02x}{:02x}{:02x}", c.r, c.g, c.b);
    assert!(ssr_violet.style_css.contains(&violet_solid), "violet accent {violet_solid} must paint");
    assert!(ssr_violet.html.contains("Snapshot 5 / 5"));

    press_test_id(&mut session, "tt-undo", 5000);
    let html_undo = session.render_html();
    assert_valid_dom_html(&html_undo);
    assert!(html_undo.contains("Snapshot 4 / 5"));
    assert!(html_undo.contains("Live Metrics"));

    press_test_id(&mut session, "tt-first", 6000);
    let html_first = session.render_html();
    assert_valid_dom_html(&html_first);
    assert!(html_first.contains("Snapshot 1 / 5"));
    assert!(html_first.contains("Workspace Boot"));
    assert!(html_first.contains("Issue Board"));
    assert!(html_first.contains("Done (1)"));

    press_test_id(&mut session, "tt-live", 7000);
    assert_scene_view_matches_store(&session);
    let html_live = session.render_html();
    assert_valid_dom_html(&html_live);
    assert!(html_live.contains("Snapshot 5 / 5"));
    assert!(html_live.contains("Accent -&gt; Violet"));
    assert!(html_live.contains("Live Metrics"));
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
#[test]
fn scene_view_is_valid_before_initial_render_and_tracks_pointer_stability() {
    let mut session = WebAppSession::new_kitchen_sink(1, WebBackend::new(), Viewport::default());
    let ptr_before = session.scene().view_ptr();
    // SAFETY: `ptr_before` points to the session's stable `NveSceneView`.
    let empty_view = unsafe { *ptr_before };
    assert_eq!(empty_view.schema_hash, NVE_SCENE_SCHEMA_HASH);
    assert_eq!(empty_view.node_count, 0);
    // SAFETY: `empty_view` borrows the unmutated empty scene table.
    let empty_ref = unsafe { NveSceneRef::from_raw(empty_view) }.unwrap();
    assert!(empty_ref.nodes().is_empty());

    session.render_initial().unwrap();
    assert_eq!(session.scene().view_ptr(), ptr_before);
    // SAFETY: `ptr_before` remains valid across `render_initial()` (pointer stability invariant).
    let mounted_view = unsafe { *ptr_before };
    assert_eq!(mounted_view.frame_id, 1);
    assert_eq!(mounted_view.node_count as usize, session.node_count());
    assert_scene_view_matches_store(&session);
}

#[cfg(all(
    feature = "app-studio",
    feature = "app-kitchen-sink",
    feature = "dev",
    feature = "scene-view"
))]
#[test]
fn studio_workspace_full_parity_canvas_img_cmdk_stream_islands_and_mobile_resize() {
    let desktop_vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 2.0,
    };
    let mut session = WebAppSession::new_studio_workspace(1, WebBackend::new(), desktop_vp);
    session.render_initial().unwrap();
    assert_scene_view_matches_store(&session);

    // 1. Image avatar `<img>` carries `ImageFit::Cover`, `alt`, and PNG data-URI `src`.
    let avatar_id = find_node_by_test_id(session.store(), "task-avatar-1")
        .expect("expected task-avatar-1 image node");
    let avatar = session
        .store()
        .try_row::<Image>(StoreNodeId(avatar_id))
        .expect("task-avatar-1 should carry Image table");
    assert_eq!(avatar.fit(), ImageFit::Cover);
    let src = session
        .store()
        .resource::<nve_components::tables::ImageSrc>(avatar.src())
        .expect("image src resource");
    assert!(matches!(
        src,
        nve_components::tables::ImageSrc::Url(u) if u.starts_with("data:image/png;base64,")
    ));
    assert!(!avatar.alt().is_empty());

    // 2. Board TextInput search + custom task create via `submit` (and empty submit is a safe no-op).
    let search_id = find_node_by_test_id(session.store(), "board-search-input").unwrap();
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: search_id,
        timestamp_us: 1000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("AppKit".into()),
    });
    session.pump_events().unwrap().expect("search filter frame");
    let html_filtered = session.render_html();
    assert!(html_filtered.contains("AET-102"));
    assert!(!html_filtered.contains("AET-101"));
    press_test_id(&mut session, "board-search-clear", 1500);
    assert!(session.render_html().contains("AET-101"));

    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: ROOT_NODE_ID,
        timestamp_us: 1600,
        kind: NativeEventKind::Submit,
        payload: EventPayload::Text(String::new()),
    });
    assert!(session.pump_events().unwrap().is_none());

    let new_task_id = find_node_by_test_id(session.store(), "board-new-task-input").unwrap();
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: new_task_id,
        timestamp_us: 1700,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Web Canvas Parity".into()),
    });
    session.pump_events().unwrap().expect("draft update");
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: new_task_id,
        timestamp_us: 1800,
        kind: NativeEventKind::Submit,
        payload: EventPayload::Text("Web Canvas Parity".into()),
    });
    session.pump_events().unwrap().expect("submit custom task");
    assert!(session.render_html().contains("Web Canvas Parity"));
    assert_scene_view_matches_store(&session);

    // 3. Cmd+K palette open -> ARIA dialog/searchbox -> filter -> ArrowDown -> Enter -> Escape.
    assert!(session.dispatch_keydown("k", true, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.store(), "command-palette").is_some());
    let pal_html = session.render_html();
    assert!(pal_html.contains("aria-label=\"Command Palette\""));
    assert!(pal_html.contains("role=\"dialog\""));
    assert!(pal_html.contains("role=\"searchbox\""));

    let pal_in = find_node_by_test_id(session.store(), "palette-input").unwrap();
    session.backend_mut().push_event(NativeEvent {
        surface_id: 1,
        node_id: pal_in,
        timestamp_us: 2000,
        kind: NativeEventKind::TextChange,
        payload: EventPayload::Text("Accent:".into()),
    });
    session.pump_events().unwrap().expect("palette filter");
    assert!(session.dispatch_keydown("ArrowDown", false, false).unwrap().is_some());
    assert!(session.dispatch_keydown("Enter", false, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.store(), "command-palette").is_none());
    assert!(session.render_html().contains("Accent -&gt; Violet"));

    assert!(session.dispatch_keydown("k", false, true).unwrap().is_some());
    assert!(find_node_by_test_id(session.store(), "command-palette").is_some());
    assert!(session.dispatch_keydown("Escape", false, false).unwrap().is_some());
    assert!(find_node_by_test_id(session.store(), "command-palette").is_none());

    // 4. Islands view bus broadcast & gesture arbitration.
    press_test_id(&mut session, "nav-islands", 3000);
    press_test_id(&mut session, "island-bus-theme", 3100);
    press_test_id(&mut session, "island-gesture-claim", 3200);
    let html_islands = session.render_html();
    assert!(html_islands.contains("Bus: 1 tx"));
    assert!(html_islands.contains("Claim | winner:"));

    // 5. 100k Stream view jump #50k, #99,999, and NativeEventKind::Scroll.
    press_test_id(&mut session, "nav-stream", 4000);
    press_test_id(&mut session, "stream-jump-50k", 4100);
    assert!(session.render_html().contains("STR-50000"));
    press_test_id(&mut session, "stream-jump-end", 4200);
    assert!(session.render_html().contains("STR-99999"));
    let stream_vp = find_node_by_test_id(session.store(), "stream-viewport").unwrap();
    session.backend_mut().push_event(NativeEvent {
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
    let html_top = session.render_html();
    assert!(html_top.contains("Rows #0..") || html_top.contains("STR-00001"));

    // 6. Metrics view `<canvas>` carries direct typed `Canvas` draw list table.
    press_test_id(&mut session, "nav-metrics", 5000);
    let canvas_id = find_node_by_test_id(session.store(), "metrics-sparkline-canvas")
        .expect("expected metrics-sparkline-canvas");
    let canvas_row = session
        .store()
        .try_row::<Canvas>(StoreNodeId(canvas_id))
        .expect("metrics-sparkline-canvas should carry Canvas table");
    let dl = session
        .store()
        .resource::<nve_canvas::DrawList>(canvas_row.draw_list())
        .expect("metrics-sparkline-canvas DrawList resource");
    assert!(
        dl.op_count() > 3,
        "expected direct typed DrawList ops on metrics canvas, got {}",
        dl.op_count()
    );

    // 7. Dynamic viewport resize across 620pt mobile breakpoint and back to desktop.
    assert!(find_node_by_test_id(session.store(), "desktop-sidebar").is_some());
    assert!(find_node_by_test_id(session.store(), "mobile-top-bar").is_none());

    let mobile_vp = Viewport {
        width: 393.0,
        height: 852.0,
        scale_factor: 3.0,
    };
    assert!(session.resize_viewport(mobile_vp).unwrap().is_some());
    assert_eq!(session.backend().viewport(), mobile_vp);
    assert!(find_node_by_test_id(session.store(), "mobile-top-bar").is_some());
    assert!(find_node_by_test_id(session.store(), "desktop-sidebar").is_none());
    assert!(find_node_by_test_id(session.store(), "center-workspace").is_some());
    assert_scene_view_matches_store(&session);

    assert!(session.resize_viewport(desktop_vp).unwrap().is_some());
    assert!(find_node_by_test_id(session.store(), "desktop-sidebar").is_some());
    assert!(find_node_by_test_id(session.store(), "mobile-top-bar").is_none());
    assert_scene_view_matches_store(&session);
}

#[cfg(any(not(target_arch = "wasm32"), feature = "app-issues"))]
#[test]
fn pump_async_work_reports_hit_cap_and_finishes_via_continuation() {
    use crate::session::WebAppSession;
    use nve_backend_web::WebBackend;
    use nve_engine::Viewport;
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
                // MUST hit the pump cap and return `hit_cap = true` instead of stalling!
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
    let mut session = WebAppSession::new(
        1,
        surface,
        nve_web_html_web::register_web_html_systems,
        WebBackend::new(),
        Viewport::default(),
    );
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
        !session.render_html().contains("STEP:24"),
        "first tick must bound work at MAX_PUMPS instead of running all 24 steps: {}",
        session.render_html()
    );

    let mut final_hit_cap = true;
    for _ in 0..8 {
        let next = session.pump_async_work_with(waker, || {}).unwrap();
        final_hit_cap = next.hit_cap;
        if session.render_html().contains("STEP:24") {
            break;
        }
    }
    assert!(!final_hit_cap, "settled burst must clear hit_cap");
    assert!(
        session.render_html().contains("STEP:24"),
        "continuation pumps must finish all 24 steps: {}",
        session.render_html()
    );
}

#[cfg(all(feature = "app-studio", feature = "app-kitchen-sink"))]
#[test]
fn host_appearance_events_reach_both_apps_and_rerender_only_on_change() {
    use nve_abi::{ColorScheme, ContrastLevel};
    use nve_system_env::ThemeSnapshot;

    let light = ThemeSnapshot { color_scheme: Some(ColorScheme::Light), contrast: ContrastLevel::Standard, ..ThemeSnapshot::default() }
        .to_repr_c_bytes();
    let viewport = Viewport { width: 1280.0, height: 800.0, scale_factor: 1.0 };
    let mut sessions = [
        ("studio", WebAppSession::new_studio_workspace(1, WebBackend::new(), viewport)),
        ("kitchen-sink", WebAppSession::new_kitchen_sink(1, WebBackend::new(), viewport)),
    ];
    for (app, session) in &mut sessions {
        session.render_initial().expect("frame 0");
        let dark_html = session.render_html();
        let frame = session.dispatch_host_events(&light).expect("dispatch light appearance");
        assert!(frame.is_some(), "{app}: a new OS appearance must commit a frame");
        assert_ne!(session.render_html(), dark_html, "{app}: light appearance must re-theme the DOM");
    }
    // Kitchen Sink skips unchanged environments entirely.
    let (_, ks) = &mut sessions[1];
    assert!(ks.dispatch_host_events(&light).expect("re-dispatch").is_none());
}
