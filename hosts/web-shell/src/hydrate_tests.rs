//! Unit tests for Progressive & Island DOM Hydration (`HydrateTrigger`,
//! `IslandHydrationScheduler`, event capture + replay, and Studio Workspace SSR byte reduction).

use nve_abi::{EventPayload, NativeEvent, NativeEventKind, NodeId, SurfaceId};
use nve_backend_web::{
    CapturedPreHydrationEvent, MemorySnapshotStorage, WebBackend, WebSnapshotStore,
    PRE_HYDRATION_REPLAY_SCRIPT_TAG, SNAPSHOT_BOOTSTRAP_MAX_BYTES,
    WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG,
};
use nve_components::tables::{register_core_tables, DebugId};
use nve_engine::Viewport;
use nve_example_kitchen_sink::{KITCHEN_SINK_STATE_MAGIC, KITCHEN_SINK_STATE_VERSION};
use nve_store::{ChangeSet, NodeId as StoreNodeId, NodeStore};
use nve_studio_workspace::STUDIO_SNAPSHOT_SCHEMA_VERSION;
use nve_web_html::{register_web_html_tables, HtmlDataAttrs};

use crate::hydrate::{CapturedInteraction, HydrateTrigger, IslandHydrationScheduler};
use crate::session::WebAppSession;

#[test]
fn hydrate_trigger_parses_all_modes_and_aliases() {
    assert_eq!(HydrateTrigger::parse("load"), Some(HydrateTrigger::Load));
    assert_eq!(HydrateTrigger::parse("EAGER"), Some(HydrateTrigger::Load));
    assert_eq!(
        HydrateTrigger::parse("visible"),
        Some(HydrateTrigger::Visible)
    );
    assert_eq!(
        HydrateTrigger::parse("viewport"),
        Some(HydrateTrigger::Visible)
    );
    assert_eq!(
        HydrateTrigger::parse("interaction"),
        Some(HydrateTrigger::Interaction)
    );
    assert_eq!(
        HydrateTrigger::parse("event"),
        Some(HydrateTrigger::Interaction)
    );
    assert_eq!(HydrateTrigger::parse("idle"), Some(HydrateTrigger::Idle));
    assert_eq!(HydrateTrigger::parse("unknown"), None);
}

#[test]
fn island_scheduler_discovers_all_four_triggers_and_replays_interactions() {
    register_core_tables();
    register_web_html_tables();
    let mut store = NodeStore::new();
    let mut changes = ChangeSet::default();
    {
        let mut tx = store.tx();
        let root = StoreNodeId::ROOT;
        let _ = tx.create_at(root);
        let mut children = Vec::new();
        for (id, name, trigger) in [
            (2u32, "hero-island", Some("load")),
            (3, "chart-island", Some("visible")),
            (4, "search-island", Some("interaction")),
            (5, "analytics-island", Some("idle")),
            (6, "default-load-island", None),
        ] {
            let nid = StoreNodeId(id);
            let _ = tx.create_at(nid);
            children.push(nid);
            let mut pairs = vec![("data-nve-island".to_owned(), name.to_owned())];
            if let Some(tr) = trigger {
                pairs.push(("data-nve-hydrate".to_owned(), tr.to_owned()));
            }
            tx.set_resource(
                nid,
                HtmlDataAttrs::ENTRIES,
                Some(nve_web_html::DataAttrList::from_pairs(pairs)),
            );
        }
        tx.set_children(root, &children);
        tx.commit_into(&mut changes);
    }

    let mut sched = IslandHydrationScheduler::discover_from_store(&store);
    assert_eq!(sched.islands().len(), 5);
    assert_eq!(sched.pending_count(), 5);
    assert_eq!(sched.hydrated_count(), 0);

    // 1. Load trigger hydrates only Load islands (including default trigger).
    let loaded = sched.on_load();
    assert_eq!(loaded, vec!["hero-island", "default-load-island"]);
    assert!(sched.is_hydrated("hero-island"));
    assert!(sched.is_hydrated("default-load-island"));
    assert!(!sched.is_hydrated("chart-island"));
    assert!(!sched.is_hydrated("search-island"));
    assert!(!sched.is_hydrated("analytics-island"));
    assert!(sched.on_load().is_empty());

    // 2. Visible trigger hydrates only on intersection of the matching Visible island.
    assert!(!sched.on_visible("search-island"));
    assert!(sched.on_visible("chart-island"));
    assert!(sched.is_hydrated("chart-island"));
    assert!(!sched.on_visible("chart-island"));

    // 3. Interaction trigger captures first event + replays all queued interactions in order.
    let pre_focus = CapturedInteraction {
        island_id: "search-island".into(),
        event_type: "focusin".into(),
        target_node_id: Some(NodeId(4)),
        client_x: 0,
        client_y: 0,
        key: None,
    };
    assert!(sched.queue_interaction(pre_focus.clone()));
    let click = CapturedInteraction {
        island_id: "search-island".into(),
        event_type: "click".into(),
        target_node_id: Some(NodeId(4)),
        client_x: 120,
        client_y: 44,
        key: None,
    };
    let replays = sched
        .on_interaction(click.clone())
        .expect("first interaction must hydrate and return replay queue");
    assert_eq!(replays, vec![pre_focus, click.clone()]);
    assert!(sched.is_hydrated("search-island"));
    assert!(sched.on_interaction(click).is_none());

    // 4. Idle trigger hydrates remaining Idle islands.
    let idle = sched.on_idle();
    assert_eq!(idle, vec!["analytics-island"]);
    assert_eq!(sched.pending_count(), 0);
    assert_eq!(sched.hydrated_count(), 5);
}

#[test]
fn studio_workspace_frame0_ssr_achieves_massive_css_reduction_and_zero_mismatch_hydration() {
    let vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let mut server = WebAppSession::new_studio_workspace(SurfaceId(1), WebBackend::new(), vp);
    server.render_initial().unwrap();

    let ssr = server.render_ssr();
    assert!(
        ssr.rule_hashes.len() < server.node_count(),
        "structural rule deduplication should yield fewer rules ({}) than nodes ({})",
        ssr.rule_hashes.len(),
        server.node_count()
    );

    // Zero-flicker hydration over Studio Frame 0 must produce 0 mismatches & 0 duplicate rules.
    let mut client =
        WebAppSession::new_studio_workspace(SurfaceId(1), WebBackend::hydrate_from_ssr(&ssr), vp);
    client.render_initial().unwrap();
    let report = client.backend().hydration_report();
    assert!(report.hydrated);
    assert!(
        report.mismatches.is_empty(),
        "expected 0 Studio hydration mismatches, got {:?}",
        report.mismatches
    );
    assert_eq!(report.replaced_nodes, 0);
    assert_eq!(report.adopted_nodes as usize, client.node_count());
    assert_eq!(report.reused_css_rules as usize, ssr.rule_hashes.len());
}

#[test]
fn web_shell_index_html_embeds_snapshot_boot_and_replay_scripts_within_budget_and_in_order() {
    let html = include_str!("../index.html");
    assert!(
        WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.len() < SNAPSHOT_BOOTSTRAP_MAX_BYTES,
        "WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG ({} B) must remain < {SNAPSHOT_BOOTSTRAP_MAX_BYTES} B",
        WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG.len()
    );
    let main_close = html
        .find("</main>")
        .expect("hosts/web-shell/index.html must contain </main>");
    let snap_pos = html
        .find(WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG)
        .expect("hosts/web-shell/index.html must embed WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG verbatim");
    let replay_pos = html
        .find(PRE_HYDRATION_REPLAY_SCRIPT_TAG)
        .expect("hosts/web-shell/index.html must embed PRE_HYDRATION_REPLAY_SCRIPT_TAG verbatim");
    let module_pos = html
        .find("<script type=\"module\">")
        .expect("hosts/web-shell/index.html must contain <script type=\"module\">");
    assert!(
        main_close < snap_pos && snap_pos < replay_pos && replay_pos < module_pos,
        "bootstrap scripts must execute right after </main> and strictly before <script type=\"module\">"
    );
    assert!(
        html.contains("data-testid=\"shell-reset-btn\""),
        "hosts/web-shell/index.html must expose a Reset workspace control"
    );
}

fn find_node_by_testid(store: &NodeStore, testid: &str) -> Option<NodeId> {
    store.live_nodes().find_map(|id| {
        store
            .try_row::<DebugId>(id)
            .filter(|d| d.test_id() == testid)
            .map(|_| id)
    })
}

#[test]
fn mutated_studio_canonical_snapshot_roundtrips_with_zero_mismatch_hydration_and_replay() {
    let vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let mut session1 = WebAppSession::new_studio_workspace(SurfaceId(1), WebBackend::new(), vp);
    session1.render_initial().unwrap();

    // 1. Toggle AET-101 status -> Done (2)
    let status_btn = find_node_by_testid(session1.store(), "task-status-1")
        .expect("task-status-1 present on initial Board");
    session1.backend_mut().push_event(NativeEvent {
        surface_id: SurfaceId(1),
        node_id: status_btn,
        timestamp_us: 1,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    });
    session1.pump_events().unwrap();
    assert!(session1.render_html().contains("Done (2)"));

    // 2. Switch active view to Docs (`nav-docs`)
    let docs_btn = find_node_by_testid(session1.store(), "nav-docs")
        .expect("nav-docs present in sidebar");
    session1.backend_mut().push_event(NativeEvent {
        surface_id: SurfaceId(1),
        node_id: docs_btn,
        timestamp_us: 2,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    });
    session1.pump_events().unwrap();
    assert!(session1.render_html().contains("RFC-042: Zero-Copy NveSceneView"));

    // 3. Persist canonical Frame-0 snapshot + binary NWS1 state into WebSnapshotStore.
    let (state_bytes, payload) = session1
        .capture_canonical_snapshot(vec![(0, 120)])
        .expect("Studio must produce canonical snapshot + NWS1 state");
    assert!(state_bytes.starts_with(b"NWS1"));
    assert!(payload.html.contains("RFC-042: Zero-Copy NveSceneView"));

    let mut store = WebSnapshotStore::new(
        MemorySnapshotStorage::new(),
        "studio",
        STUDIO_SNAPSHOT_SCHEMA_VERSION,
    );
    store
        .save_all(&payload, &state_bytes)
        .expect("save_all succeeds");

    // 4. Reload: Layer 1 loads cached snapshot; Layer 2 loads NWS1 state & hydrates with 0 mismatches!
    let cached_frame = store.load_frame().expect("frame snapshot loaded");
    let cached_state = store.load_state().expect("NWS1 state loaded");
    assert_eq!(cached_frame.scroll_offsets, vec![(0, 120)]);

    let mut hydrated_backend = WebBackend::hydrate_from_ssr(&cached_frame.to_ssr_output());
    let cached_ssr_html = cached_frame.html.clone();
    let doc_tab_2_id = extract_testid_node_id(&cached_ssr_html, "doc-tab-2")
        .expect("doc-tab-2 must exist in cached Docs snapshot HTML");
    hydrated_backend.queue_pre_hydration_events(vec![CapturedPreHydrationEvent::Press {
        node_id: doc_tab_2_id,
        x: 20.0,
        y: 12.0,
        age_ms: 42,
    }]);

    let (mut session2, restored) = WebAppSession::new_studio_workspace_with_state(
        SurfaceId(1),
        hydrated_backend,
        vp,
        Some(&cached_state),
    );
    assert!(restored);
    session2.render_initial().unwrap();

    let report = session2.backend().hydration_report().clone();
    assert!(report.hydrated);
    assert!(
        report.mismatches.is_empty(),
        "expected 0 hydration mismatches on mutated Studio snapshot, got {:?}",
        report.mismatches
    );
    assert_eq!(report.replaced_nodes, 0);
    assert_eq!(report.adopted_nodes as usize, session2.node_count());
    assert_eq!(report.reused_css_rules as usize, cached_frame.rule_hashes.len());
    assert_eq!(report.replayed_events, 1);

    // Pumping post-hydration replayed click switches Docs tab to Release Checklist seamlessly!
    assert!(session2.pump_events().unwrap().is_some());
    assert!(
        session2.render_html().contains("Pre-Flight Gates"),
        "pre-hydration click on doc-tab-2 must replay and activate Release Checklist doc"
    );

    // Corrupt/truncated binary state falls back cleanly to default Board with zero panic.
    let (mut corrupt_session, corrupt_restored) = WebAppSession::new_studio_workspace_with_state(
        SurfaceId(1),
        WebBackend::new(),
        vp,
        Some(b"NWS1-corrupt-truncated"),
    );
    assert!(!corrupt_restored);
    corrupt_session.render_initial().unwrap();
    assert!(corrupt_session.render_html().contains("Issue Board"));
}

#[test]
fn kitchen_sink_canonical_snapshot_roundtrips_binary_state_with_zero_mismatch_hydration() {
    let vp = Viewport::default();
    let mut session1 = WebAppSession::new_kitchen_sink(SurfaceId(1), WebBackend::new(), vp);
    session1.render_initial().unwrap();

    let inc_btn = find_node_by_testid(session1.store(), "ks-counter-inc")
        .expect("ks-counter-inc button must exist in Kitchen Sink Frame 0");

    for i in 1..=3 {
        session1.backend_mut().push_event(NativeEvent {
            surface_id: SurfaceId(1),
            node_id: inc_btn,
            timestamp_us: i,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 10.0, y: 10.0 },
        });
        session1.pump_events().unwrap();
    }
    assert!(session1.render_html().contains("Count: 3"));

    let (state_bytes, payload) = session1
        .capture_canonical_snapshot(Vec::new())
        .expect("Kitchen Sink must produce canonical snapshot + binary state");
    assert!(state_bytes.starts_with(&KITCHEN_SINK_STATE_MAGIC));
    assert!(payload.html.contains("Count: 3"));

    let mut store = WebSnapshotStore::new(
        MemorySnapshotStorage::new(),
        "kitchen-sink",
        KITCHEN_SINK_STATE_VERSION,
    );
    store.save_all(&payload, &state_bytes).unwrap();

    let cached_frame = store.load_frame().unwrap();
    let cached_state = store.load_state().unwrap();

    let (mut session2, restored) = WebAppSession::new_kitchen_sink_with_state(
        SurfaceId(1),
        WebBackend::hydrate_from_ssr(&cached_frame.to_ssr_output()),
        vp,
        Some(&cached_state),
    );
    assert!(restored);
    session2.render_initial().unwrap();

    let report = session2.backend().hydration_report();
    assert!(report.hydrated);
    assert!(report.mismatches.is_empty());
    assert_eq!(report.replaced_nodes, 0);
    assert_eq!(report.adopted_nodes as usize, session2.node_count());
    assert!(session2.render_html().contains("Count: 3"));
}

fn extract_testid_node_id(html: &str, testid: &str) -> Option<NodeId> {
    let pos = html
        .find(&format!("data-testid=\"{testid}\""))
        .or_else(|| html.find(&format!("testid=\"{testid}\"")))?;
    let tag_start = html[..pos].rfind('<')?;
    let tag = &html[tag_start..pos];
    let id_attr = "data-nve-id=\"";
    let id_pos = tag.find(id_attr)? + id_attr.len();
    let end = tag[id_pos..].find('"')?;
    tag[id_pos..id_pos + end].parse::<u32>().ok().map(NodeId)
}
