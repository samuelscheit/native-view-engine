//! Unit tests for Progressive & Island DOM Hydration (`HydrateTrigger`,
//! `IslandHydrationScheduler`, event capture + replay, and Studio Workspace SSR byte reduction).

use nve_abi::{CommandBufferBuilder, ComponentKind, RawPropValue, ROOT_NODE_ID};
use nve_backend_web::WebBackend;
use nve_engine::{RenderBackend, Viewport};

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
