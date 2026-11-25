//! Unit tests for Progressive & Island DOM Hydration (`HydrateTrigger`,
//! `IslandHydrationScheduler`, event capture + replay, and Studio Workspace SSR byte reduction).

use nve_abi::{
    CommandBufferBuilder, ComponentKind, EventMask, EventPayload, NativeEvent, NativeEventKind,
    NodeId, RawPropValue, ROOT_NODE_ID,
};
use nve_backend_web::{
    CapturedPreHydrationEvent, MemorySnapshotStorage, WebBackend, WebSnapshotStore,
    PRE_HYDRATION_REPLAY_SCRIPT_TAG, SNAPSHOT_BOOTSTRAP_MAX_BYTES,
    WEB_SHELL_SNAPSHOT_BOOT_SCRIPT_TAG,
};
use nve_engine::{RenderBackend, Viewport};
use nve_example_counter::COUNTER_STATE_BIN_VERSION;
use nve_studio_workspace::STUDIO_SNAPSHOT_SCHEMA_VERSION;

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
    assert_eq!(HydrateTrigger::parse("idle"), Some(HydrateTrigger::Idle));
    assert_eq!(HydrateTrigger::parse("unknown"), None);

    for trigger in [
        HydrateTrigger::Load,
        HydrateTrigger::Visible,
        HydrateTrigger::Interaction,
        HydrateTrigger::Idle,
    ] {
        assert_eq!(HydrateTrigger::parse(trigger.as_str()), Some(trigger));
    }
}

#[test]
fn progressive_island_scheduler_discovers_triggers_and_replays_interactions() {
    let mut backend = WebBackend::new();
    let mut b = CommandBufferBuilder::new(1, 1);
    for (id, name, trigger) in [
        (2u32, "hero-island", Some("load")),
        (3u32, "chart-island", Some("visible")),
        (4u32, "search-island", Some("interaction")),
        (5u32, "analytics-island", Some("idle")),
        (6u32, "default-load-island", None),
    ] {
        b.create_node(id, ComponentKind::View, None);
        b.set_raw_prop(id, "data-nve-island", &RawPropValue::String(name.into()));
        if let Some(tr) = trigger {
            b.set_raw_prop(id, "data-nve-hydrate", &RawPropValue::String(tr.into()));
        }
    }
    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6]);
    b.commit(true);
    backend.submit_frame(b.finish()).unwrap();

    let mut sched = IslandHydrationScheduler::discover_from_backend(&backend);
    assert_eq!(sched.islands().len(), 5);
    assert_eq!(sched.pending_count(), 5);
    assert_eq!(sched.hydrated_count(), 0);

    // 1. Load trigger hydrates only Load + default-load islands.
    let loaded = sched.on_load();
    assert_eq!(loaded, vec!["hero-island", "default-load-island"]);
    assert!(sched.is_hydrated("hero-island"));
    assert!(sched.is_hydrated("default-load-island"));
    assert!(!sched.is_hydrated("chart-island"));
    assert!(!sched.is_hydrated("search-island"));
    assert!(!sched.is_hydrated("analytics-island"));
    assert!(sched.on_load().is_empty());

    // 2. Visible trigger hydrates only on intersection of the matching Visible island.
    assert!(!sched.on_visible("search-island")); // wrong trigger kind
    assert!(sched.on_visible("chart-island"));
    assert!(sched.is_hydrated("chart-island"));
    assert!(!sched.on_visible("chart-island")); // idempotent

    // 3. Interaction trigger captures first event + replays all queued interactions in order.
    let pre_focus = CapturedInteraction {
        island_id: "search-island".into(),
        event_type: "focusin".into(),
        target_node_id: Some(4),
        client_x: 0,
        client_y: 0,
        key: None,
    };
    assert!(sched.queue_interaction(pre_focus.clone()));
    let click = CapturedInteraction {
        island_id: "search-island".into(),
        event_type: "click".into(),
        target_node_id: Some(4),
        client_x: 120,
        client_y: 44,
        key: None,
    };
    let replays = sched
        .on_interaction(click.clone())
        .expect("first interaction must hydrate and return replay queue");
    assert_eq!(replays, vec![pre_focus, click.clone()]);
    assert!(sched.is_hydrated("search-island"));
    // Subsequent interactions after hydration do not re-trigger replay.
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
    let mut server = WebAppSession::new_studio_workspace(1, WebBackend::new(), vp);
    server.render_initial().unwrap();

    let legacy_html = server.backend().render_html();
    let ssr = server.backend().render_ssr();
    let total_ssr_bytes = ssr.base_css.len() + ssr.style_css.len() + ssr.html.len();

    assert!(
        total_ssr_bytes < legacy_html.len() / 2,
        "Option 5 Hybrid SSR ({total_ssr_bytes} B: base={}, style={}, html={}) must achieve >50% reduction vs legacy inline HTML ({} B)",
        ssr.base_css.len(),
        ssr.style_css.len(),
        ssr.html.len(),
        legacy_html.len()
    );
    assert!(
        ssr.rule_hashes.len() < server.backend().node_count(),
        "structural rule deduplication should yield fewer rules ({}) than nodes ({})",
        ssr.rule_hashes.len(),
        server.backend().node_count()
    );

    // Zero-flicker hydration over Studio Frame 0 must produce 0 mismatches & 0 duplicate rules.
    let mut client = WebAppSession::new_studio_workspace(1, WebBackend::hydrate_from_ssr(&ssr), vp);
    client.render_initial().unwrap();
    let report = client.backend().hydration_report();
    assert!(report.hydrated);
    assert!(
        report.mismatches.is_empty(),
        "expected 0 Studio hydration mismatches, got {:?}",
        report.mismatches
    );
    assert_eq!(report.replaced_nodes, 0);
    assert_eq!(report.adopted_nodes, client.backend().node_count());
    assert_eq!(report.seeded_rule_count, ssr.rule_hashes.len());
    assert_eq!(client.backend().stylesheet().pending_count(), 0);
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

fn find_node_by_testid(backend: &WebBackend, testid: &str) -> Option<NodeId> {
    backend.iter_nodes().find_map(|(id, el)| {
        el.raw_props
            .get("testID")
            .or_else(|| el.raw_props.get("data-testid"))
            .and_then(|v| match v {
                RawPropValue::String(s) if s == testid => Some(id),
                _ => None,
            })
    })
}

#[test]
fn mutated_studio_canonical_snapshot_roundtrips_with_zero_mismatch_hydration_and_replay() {
    let vp = Viewport {
        width: 1280.0,
        height: 800.0,
        scale_factor: 1.0,
    };
    let mut session1 = WebAppSession::new_studio_workspace(1, WebBackend::new(), vp);
    session1.render_initial().unwrap();

    // Mutate Studio state across multiple incremental frames (advancing Dioxus `next_node_id`):
    // 1. Toggle AET-101 status -> Done (2)
    let status_btn = find_node_by_testid(session1.backend(), "task-status-1")
        .expect("task-status-1 present on initial Board");
    session1.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: status_btn,
        timestamp_us: 1,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    });
    session1.pump_events().unwrap();
    assert!(session1.backend().render_html().contains("Done (2)"));

    // 2. Switch active view to Docs (`nav-docs`)
    let docs_btn = find_node_by_testid(session1.backend(), "nav-docs")
        .expect("nav-docs present in sidebar");
    session1.backend().push_event(NativeEvent {
        surface_id: 1,
        node_id: docs_btn,
        timestamp_us: 2,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    });
    session1.pump_events().unwrap();
    assert!(
        session1
            .backend()
            .render_html()
            .contains("RFC-042: Zero-Copy NveSceneView")
    );

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
    // Queue a pre-hydration click on `doc-tab-2` (Release Checklist) using its canonical Frame-0 NodeId!
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
        1,
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
    assert_eq!(report.adopted_nodes, session2.backend().node_count());
    assert_eq!(report.seeded_rule_count, cached_frame.rule_hashes.len());
    assert_eq!(report.replayed_events, 1);
    assert_eq!(session2.backend().stylesheet().pending_count(), 0);

    // Pumping post-hydration replayed click switches Docs tab to Release Checklist seamlessly!
    assert!(session2.pump_events().unwrap().is_some());
    assert!(
        session2
            .backend()
            .render_html()
            .contains("Pre-Flight Gates"),
        "pre-hydration click on doc-tab-2 must replay and activate Release Checklist doc"
    );

    // Corrupt/truncated binary state falls back cleanly to default Board with zero panic.
    let (mut corrupt_session, corrupt_restored) = WebAppSession::new_studio_workspace_with_state(
        1,
        WebBackend::new(),
        vp,
        Some(b"NWS1-corrupt-truncated"),
    );
    assert!(!corrupt_restored);
    corrupt_session.render_initial().unwrap();
    assert!(corrupt_session.backend().render_html().contains("Issue Board"));
}

#[test]
fn counter_canonical_snapshot_roundtrips_nwc1_state_with_zero_mismatch_hydration() {
    let vp = Viewport::default();
    let mut session1 = WebAppSession::new_counter(1, WebBackend::new(), vp);
    session1.render_initial().unwrap();

    let mut pressables: Vec<NodeId> = session1
        .backend()
        .iter_nodes()
        .filter_map(|(id, el)| ((el.event_mask & EventMask::PRESS) != 0).then_some(id))
        .collect();
    pressables.sort_unstable();
    let inc_btn = pressables[0];

    for i in 1..=3 {
        session1.backend().push_event(NativeEvent {
            surface_id: 1,
            node_id: inc_btn,
            timestamp_us: i,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 10.0, y: 10.0 },
        });
        session1.pump_events().unwrap();
    }
    assert!(session1.backend().render_html().contains("Count: 3"));

    let (state_bytes, payload) = session1
        .capture_canonical_snapshot(Vec::new())
        .expect("Counter must produce canonical snapshot + NWC1 state");
    assert!(state_bytes.starts_with(b"NWC1"));
    assert!(payload.html.contains("Count: 3"));

    let mut store = WebSnapshotStore::new(
        MemorySnapshotStorage::new(),
        "counter",
        COUNTER_STATE_BIN_VERSION,
    );
    store.save_all(&payload, &state_bytes).unwrap();

    let cached_frame = store.load_frame().unwrap();
    let cached_state = store.load_state().unwrap();

    let (mut session2, restored) = WebAppSession::new_counter_with_state(
        1,
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
    assert_eq!(report.adopted_nodes, session2.backend().node_count());
    assert!(session2.backend().render_html().contains("Count: 3"));
}

fn extract_testid_node_id(html: &str, testid: &str) -> Option<NodeId> {
    let needle = format!("testid=\"{testid}\"");
    let pos = html.find(&needle)?;
    let tag_start = html[..pos].rfind('<')?;
    let tag = &html[tag_start..pos];
    let id_attr = "data-nve-id=\"";
    let id_pos = tag.find(id_attr)? + id_attr.len();
    let end = tag[id_pos..].find('"')?;
    tag[id_pos..id_pos + end].parse().ok()
}
