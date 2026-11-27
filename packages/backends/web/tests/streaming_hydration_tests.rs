#![cfg(all(feature = "ssr", feature = "hydrate"))]
//! Slice 1c top-down positional hydration over `NodeStore` (`hydrate_from_ssr` &
//! `hydrate_from_streaming_html`):
//! - `is_stream_slot_resolved` + `render_ssr_subtree_delta` stateful CSS deduplication.
//! - Post-stream out-of-order `[slot2, slot1]` and mid-stream partial `[slot2 only]` with
//!   COMPLETELY SHIFTED `NodeId`s.
//! - Mutation test: swapping two same-tag siblings with different child counts fails loudly;
//!   a tag mismatch replaces only the diverging subtree.
//! - 3-way dirty `<input>` preservation + SSR->client remapped pre-hydration click replay.
//! - Duplicate slot chunks (last wins) and TCP-split truncated slot chunks.

mod support;

use nve_abi::{EventMask, EventPayload, NativeEventKind};
use nve_backend_web::{
    CapturedPreHydrationEvent, PendingStreamSlot, StreamingDocumentOptions, StreamingSsrSession,
    WebBackend,
};
use nve_components::tables::{Accessibility, AriaRole, Interaction, TextContent, TextInput};
use nve_engine::StoreBackend;
use nve_store::{NodeId, NodeStore, Transaction};

use support::*;

#[test]
fn streaming_shell_delta_css_dedup_and_shifted_id_post_and_mid_stream_hydration() {
    // Server Frame 0 with two `<QuerySuspense>` boundaries (slot1, slot2) both pending.
    let mut server = web();
    let mut store = NodeStore::new();
    commit(&mut server, &mut store, |tx| {
        let h = heading(tx, 2);
        let s1 = pending_slot(tx, 3, 4, 5, "slot1");
        let s2 = pending_slot(tx, 6, 7, 8, "slot2");
        tx.set_children(NodeId::ROOT, &[h, s1, s2]);
    });

    let mut shell_out = server.render_ssr_streaming_shell(&store).unwrap();
    assert_eq!(
        shell_out.pending_slots,
        vec![
            PendingStreamSlot {
                slot_id: "slot1".into(),
                parent_node_id: NodeId(3)
            },
            PendingStreamSlot {
                slot_id: "slot2".into(),
                parent_node_id: NodeId(6)
            },
        ]
    );
    assert!(!server.is_stream_slot_resolved(&store, NodeId(3), "slot1"));
    assert!(!server.is_stream_slot_resolved(&store, NodeId(6), "slot2"));
    assert!(!server.is_stream_slot_resolved(&store, NodeId(999), "slot1"));
    // Shell keeps `<div data-nve-id="4" ... data-nve-stream-slot="slot1">` INSIDE `<slot>`.
    assert!(
        shell_out
            .ssr
            .html
            .contains("<slot name=\"nve-slot-slot1\"><div data-nve-id=\"4\"")
    );
    assert!(
        shell_out
            .ssr
            .html
            .contains("<slot name=\"nve-slot-slot2\"><div data-nve-id=\"7\"")
    );
    assert!(
        shell_out
            .ssr
            .html
            .contains("data-nve-stream-slot=\"slot1\"")
    );
    assert!(shell_out.ssr.html.contains("data-nve-suspense=\"slot2\""));
    assert_eq!(shell_out.ssr.html.matches("</slot>").count(), 2);
    // Plain SSR never emits shadow `<slot>` wrappers.
    assert!(!server.render_ssr(&store).html.contains("<slot "));

    let mut sess = StreamingSsrSession::new(
        StreamingDocumentOptions::from_streaming_shell("Stream App", &shell_out),
        ["slot1", "slot2"],
    );
    let shell_html = sess.begin_shell(&shell_out.ssr.html).unwrap();

    // Resolve slot2 FIRST (out-of-order!) using BOTH the already-sent shared style and a new one.
    commit(&mut server, &mut store, |tx| {
        tx.remove(NodeId(7));
        let boxed = node(tx, 20);
        shared_style(tx, boxed); // already sent in shell -> must NOT re-emit!
        let t = text(tx, 21, "Slot 2 Resolved");
        delta_style(tx, t); // brand new -> MUST emit once!
        tx.set_children(boxed, &[t]);
        tx.set_children(NodeId(6), &[boxed]);
    });
    assert!(server.is_stream_slot_resolved(&store, NodeId(6), "slot2"));
    assert!(!server.is_stream_slot_resolved(&store, NodeId(3), "slot1"));
    let delta2 = server.render_ssr_subtree_delta(&store, NodeId(6), &mut shell_out.style_tracker);
    assert_eq!(
        delta2.rule_hashes.len(),
        1,
        "only delta style should be emitted: {:?}",
        delta2.rule_hashes
    );
    assert!(delta2.style_css.contains("#16a34a"));
    assert!(
        !delta2.style_css.contains("#0f172a"),
        "already-sent shell rule must NOT re-emit"
    );
    assert!(
        delta2.html.starts_with("<div data-nve-id=\"20\""),
        "{}",
        delta2.html
    );
    let chunk2 = sess.resolve_slot_delta("slot2", &delta2).unwrap();

    // --- TEST A: MID-STREAM PARTIAL HYDRATION (slot2 arrived, slot1 STILL PENDING, no stream-end!) ---
    let mid_stream_html = format!("{shell_html}{chunk2}");
    let mut mid_client = WebBackend::hydrate_from_streaming_html(&mid_stream_html);
    let ssr_ids = [1, 2, 4, 5, 20, 21];
    let mid_instances: Vec<u64> = ssr_ids
        .iter()
        .map(|&id| mid_client.native_dom_instance_id(id).unwrap())
        .collect();

    // Client Frame 0 with COMPLETELY SHIFTED NodeIds (+1000)!
    let mut mid_store = NodeStore::new();
    commit(&mut mid_client, &mut mid_store, |tx| {
        let h = heading(tx, 1002);
        let s1 = pending_slot(tx, 1003, 1004, 1005, "slot1");
        let s2 = suspense(tx, 1006, "slot2");
        let boxed = node(tx, 1020);
        shared_style(tx, boxed);
        let t = text(tx, 1021, "Slot 2 Resolved");
        delta_style(tx, t);
        tx.set_children(boxed, &[t]);
        tx.set_children(s2, &[boxed]);
        tx.set_children(NodeId::ROOT, &[h, s1, s2]);
    });

    let mid_rep = mid_client.hydration_report();
    assert_eq!(mid_rep.mismatches, Vec::new(), "mid-stream mismatches");
    assert_eq!(mid_rep.replaced_nodes, 0);
    assert_eq!(mid_rep.adopted_nodes, 8);
    assert!(no_new_css_rules(&mid_client), "0 duplicate CSS rules");
    for (client_id, instance) in [1, 1002, 1004, 1005, 1020, 1021]
        .into_iter()
        .zip(&mid_instances)
    {
        assert_eq!(
            mid_client.native_dom_instance_id(client_id),
            Some(*instance),
            "node {client_id}"
        );
    }

    // --- TEST B: POST-STREAM OUT-OF-ORDER [slot2, slot1] HYDRATION WITH SHIFTED NodeIds ---
    commit(&mut server, &mut store, |tx| {
        tx.remove(NodeId(4));
        let t = text(tx, 30, "Slot 1 Resolved Second");
        delta_style(tx, t);
        tx.set_children(NodeId(3), &[t]);
    });
    assert!(server.is_stream_slot_resolved(&store, NodeId(3), "slot1"));
    let delta1 = server.render_ssr_subtree_delta(&store, NodeId(3), &mut shell_out.style_tracker);
    assert!(
        delta1.rule_hashes.is_empty(),
        "style already sent in slot2 must dedup"
    );
    assert!(delta1.style_css.is_empty());
    let chunk1 = sess.resolve_slot_delta("slot1", &delta1).unwrap();
    assert!(!chunk1.contains("data-nve-ssr-chunk"));
    let trailer = sess.finish().unwrap();

    let full_html = format!("{shell_html}{chunk2}{chunk1}{trailer}");
    let mut full_client = WebBackend::hydrate_from_streaming_html(&full_html);
    let full_slot1_text = full_client.native_dom_instance_id(30).unwrap();
    let full_slot2_text = full_client.native_dom_instance_id(21).unwrap();

    let mut full_store = NodeStore::new();
    commit(&mut full_client, &mut full_store, |tx| {
        let h = heading(tx, 2002);
        let s1 = suspense(tx, 2003, "slot1");
        let t1 = text(tx, 2030, "Slot 1 Resolved Second");
        delta_style(tx, t1);
        tx.set_children(s1, &[t1]);
        let s2 = suspense(tx, 2006, "slot2");
        let boxed = node(tx, 2020);
        shared_style(tx, boxed);
        let t2 = text(tx, 2021, "Slot 2 Resolved");
        delta_style(tx, t2);
        tx.set_children(boxed, &[t2]);
        tx.set_children(s2, &[boxed]);
        tx.set_children(NodeId::ROOT, &[h, s1, s2]);
    });

    let full_rep = full_client.hydration_report();
    assert!(
        full_rep.mismatches.is_empty(),
        "post-stream mismatches: {:?}",
        full_rep.mismatches
    );
    assert_eq!(full_rep.replaced_nodes, 0);
    assert_eq!(full_rep.adopted_nodes, 7);
    assert!(no_new_css_rules(&full_client));
    assert_eq!(
        full_client.native_dom_instance_id(2030),
        Some(full_slot1_text)
    );
    assert_eq!(
        full_client.native_dom_instance_id(2021),
        Some(full_slot2_text)
    );
}

#[test]
fn mutation_test_swapping_same_tag_siblings_with_different_child_counts_produces_mismatch() {
    let mut server = web();
    let mut store = NodeStore::new();
    commit(&mut server, &mut store, |tx| {
        // Sibling A (`div` with 2 `span` children), Sibling B (`div` with 0 children).
        let a = node(tx, 2);
        let one = text(tx, 3, "One");
        let two = text(tx, 4, "Two");
        tx.set_children(a, &[one, two]);
        let b = node(tx, 5);
        tx.set_children(NodeId::ROOT, &[a, b]);
    });
    let ssr = server.render_ssr(&store);

    // Client swaps sibling order: first `div` has 0 children, second `div` has 2 `span` children!
    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    let mut client_store = NodeStore::new();
    commit(&mut client, &mut client_store, |tx| {
        let a = node(tx, 10);
        let b = node(tx, 11);
        let one = text(tx, 12, "One");
        let two = text(tx, 13, "Two");
        tx.set_children(b, &[one, two]);
        tx.set_children(NodeId::ROOT, &[a, b]);
    });

    let rep = client.hydration_report();
    assert_eq!(
        rep.mismatches.len(),
        4,
        "2 excess spans under first div + 2 missing spans under second div: {:?}",
        rep.mismatches
    );
    assert_eq!(rep.replaced_nodes, 2);
    assert_eq!(rep.expected_root_hash, Some(ssr.root_hash));
    assert!(
        rep.mismatches
            .iter()
            .any(|m| m.node_id == 12 && m.actual_tag == "<missing>" && m.expected_tag == "span")
    );
    assert!(
        rep.mismatches
            .iter()
            .any(|m| m.node_id == 10 && m.expected_tag.is_empty() && m.actual_tag == "span")
    );
    // Missing nodes get fresh simulated DOM identities.
    assert!(client.native_dom_instance_id(12).is_some());
}

#[test]
fn tag_mismatch_replaces_only_the_diverging_subtree() {
    let mut server = web();
    let mut store = NodeStore::new();
    commit(&mut server, &mut store, |tx| {
        let keep = text(tx, 2, "keep");
        let swap = tagged(tx, 3, "section");
        let inner = text(tx, 4, "inner");
        tx.set_children(swap, &[inner]);
        tx.set_children(NodeId::ROOT, &[keep, swap]);
    });
    let ssr = server.render_ssr(&store);
    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    client.register_table_system(TagOverrideSystem);
    let keep_instance = client.native_dom_instance_id(2).unwrap();
    let inner_instance = client.native_dom_instance_id(4).unwrap();
    let mut client_store = NodeStore::new();
    commit(&mut client, &mut client_store, |tx| {
        let keep = text(tx, 2, "keep");
        let swap = tagged(tx, 3, "article");
        let inner = text(tx, 4, "inner");
        tx.set_children(swap, &[inner]);
        tx.set_children(NodeId::ROOT, &[keep, swap]);
    });
    let rep = client.hydration_report();
    assert_eq!(rep.replaced_nodes, 1);
    assert_eq!(rep.adopted_nodes, 2, "root + keep");
    assert_eq!(
        rep.mismatches,
        vec![nve_backend_web::HydrationMismatch {
            node_id: 3,
            expected_tag: "article".into(),
            actual_tag: "section".into(),
        }]
    );
    assert_eq!(client.native_dom_instance_id(2), Some(keep_instance));
    assert_ne!(
        client.native_dom_instance_id(4),
        Some(inner_instance),
        "subtree rebuilt"
    );
}

#[test]
fn shifted_node_ids_preserve_dirty_input_and_remap_pre_hydration_click_replay() {
    let build_tree = |tx: &mut Transaction<'_>, offset: u32, input_val: &str| {
        let inp = node(tx, offset + 2);
        tx.edit::<TextInput>(inp).value(input_val);
        tx.edit::<Interaction>(inp)
            .event_mask(EventMask::TEXT_CHANGE as u32);
        let btn = node(tx, offset + 3);
        tx.edit::<Accessibility>(btn).role(AriaRole::Button);
        tx.edit::<Interaction>(btn)
            .event_mask(EventMask::PRESS as u32);
        let lbl = text(tx, offset + 4, "Submit");
        tx.set_children(btn, &[lbl]);
        tx.set_children(NodeId::ROOT, &[inp, btn]);
    };

    let mut server = web();
    let mut store = NodeStore::new();
    commit(&mut server, &mut store, |tx| build_tree(tx, 0, "draft"));
    let ssr = server.render_ssr(&store);
    assert!(ssr.html.contains("value=\"draft\""), "{}", ssr.html);

    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    let before: Vec<u64> = [2, 3, 4]
        .iter()
        .map(|&id| client.native_dom_instance_id(id).unwrap())
        .collect();

    // User types into SSR input #2 and clicks SSR text label #4 before WASM boots!
    client.set_hydrated_dom_control_value(2, "user-edited-query");
    assert_eq!(
        client.native_dom_control_value(2),
        Some("user-edited-query")
    );
    client.queue_pre_hydration_events(vec![
        CapturedPreHydrationEvent::Press {
            node_id: 4,
            x: 15.0,
            y: 25.0,
            age_ms: 40,
        },
        // Event targeting a non-existent SSR ID must be dropped (never misrouted).
        CapturedPreHydrationEvent::Press {
            node_id: 503,
            x: 1.0,
            y: 1.0,
            age_ms: 10,
        },
    ]);

    // Client Frame 0 shifts ALL NodeIds by +500 (inp=502, btn=503, lbl=504)!
    let mut client_store = NodeStore::new();
    commit(&mut client, &mut client_store, |tx| {
        build_tree(tx, 500, "server-incoming")
    });

    let rep = client.hydration_report();
    assert!(rep.mismatches.is_empty(), "{:?}", rep.mismatches);
    assert_eq!(rep.replaced_nodes, 0);
    assert_eq!(rep.adopted_nodes, 4);
    assert_eq!(rep.reconciled_dirty_controls, 1);
    assert_eq!(rep.replayed_events, 1);

    // Exact DOM instances adopted under shifted IDs!
    for (client_id, instance) in [502, 503, 504].into_iter().zip(&before) {
        assert_eq!(client.native_dom_instance_id(client_id), Some(*instance));
    }
    // Dirty local input preserved via PreferLocal under shifted ID 502!
    assert_eq!(
        client.native_dom_control_value(502),
        Some("user-edited-query")
    );

    // Reconciled TextChange (on 502) followed by remapped+bubbled Press (4 -> 504 -> 503)!
    let events = client.drain_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].node_id, 502);
    assert_eq!(events[0].kind, NativeEventKind::TextChange);
    assert_eq!(
        events[0].payload,
        EventPayload::Text("user-edited-query".into())
    );
    assert_eq!(events[1].node_id, 503);
    assert_eq!(events[1].kind, NativeEventKind::Press);
    assert_eq!(events[1].payload, EventPayload::Point { x: 15.0, y: 25.0 });

    // Clean (untouched) control adopts the incoming client value without emitting events.
    let mut clean = WebBackend::hydrate_from_ssr(&ssr);
    let mut clean_store = NodeStore::new();
    commit(&mut clean, &mut clean_store, |tx| {
        build_tree(tx, 500, "server-incoming")
    });
    assert_eq!(clean.hydration_report().reconciled_dirty_controls, 0);
    assert_eq!(clean.native_dom_control_value(502), Some("server-incoming"));
    assert!(clean.drain_events().is_empty());
}

#[test]
fn duplicate_slot_chunks_in_streaming_html_last_wins_on_promotion() {
    let mut sess = StreamingSsrSession::new(StreamingDocumentOptions::default(), ["feed"]);
    let shell = sess
        .begin_shell("<div data-nve-id=\"1\" class=\"nve\"><div data-nve-id=\"2\" class=\"nve\"><slot name=\"nve-slot-feed\"><div data-nve-id=\"3\" data-nve-stream-slot=\"feed\"><span>pending</span></div></slot></div></div>")
        .unwrap();
    let stale = sess
        .resolve_slot("feed", "<span data-nve-id=\"10\">stale</span>", None)
        .unwrap();
    let latest = sess
        .resolve_slot("feed", "<button data-nve-id=\"11\">latest</button>", None)
        .unwrap();
    let end = sess.finish().unwrap();

    let html = format!("{shell}{stale}{latest}{end}");
    let mut client = WebBackend::hydrate_from_streaming_html(&html);
    let latest_btn_instance = client.native_dom_instance_id(11).unwrap();
    let mut store = NodeStore::new();
    commit(&mut client, &mut store, |tx| {
        let wrap = node(tx, 2);
        let btn = text(tx, 99, "latest");
        tx.edit::<Accessibility>(btn).role(AriaRole::Button);
        tx.set_children(wrap, &[btn]);
        tx.set_children(NodeId::ROOT, &[wrap]);
    });
    assert!(
        client.hydration_report().mismatches.is_empty(),
        "{:?}",
        client.hydration_report()
    );
    assert_eq!(client.native_dom_instance_id(99), Some(latest_btn_instance));
}

#[test]
fn truncated_tcp_split_slot_chunk_does_not_prematurely_replace_pending_fallback() {
    let mut sess = StreamingSsrSession::new(StreamingDocumentOptions::default(), ["feed"]);
    let shell = sess
        .begin_shell("<div data-nve-id=\"1\" class=\"nve\"><div data-nve-id=\"2\" data-nve-suspense=\"feed\"><slot name=\"nve-slot-feed\"><div data-nve-id=\"3\" data-nve-stream-slot=\"feed\"><span data-nve-id=\"4\">Loading...</span></div></slot></div></div>")
        .unwrap();
    // Simulate a TCP split midway through a Light-DOM slot chunk (unclosed `<div>`!).
    let truncated = format!(
        "{shell}<div slot=\"nve-slot-feed\" data-nve-slot=\"feed\" class=\"nve-slot\"><button data-nve-id=\"10\">Half"
    );
    let mut client = WebBackend::hydrate_from_streaming_html(&truncated);
    let fallback_slot = client.native_dom_instance_id(3).unwrap();
    let fallback_span = client.native_dom_instance_id(4).unwrap();
    let mut store = NodeStore::new();
    commit(&mut client, &mut store, |tx| {
        let s = pending_slot(tx, 200, 201, 202, "feed");
        tx.set_children(NodeId::ROOT, &[s]);
    });
    assert!(client.hydration_report().mismatches.is_empty());
    assert_eq!(client.hydration_report().replaced_nodes, 0);
    assert_eq!(client.native_dom_instance_id(201), Some(fallback_slot));
    assert_eq!(client.native_dom_instance_id(202), Some(fallback_span));
}

/// `[press button, disabled?] -> label` tree with ids `offset + {2, 3}`.
fn button_with_label(tx: &mut Transaction<'_>, offset: u32, disabled: bool) {
    let btn = node(tx, offset + 2);
    tx.edit::<Accessibility>(btn).role(AriaRole::Button);
    tx.edit::<Interaction>(btn)
        .event_mask(EventMask::PRESS as u32)
        .disabled(disabled);
    let lbl = text(tx, offset + 3, "Go");
    tx.set_children(btn, &[lbl]);
    tx.set_children(NodeId::ROOT, &[btn]);
}

fn press(node_id: u32) -> CapturedPreHydrationEvent {
    CapturedPreHydrationEvent::Press {
        node_id,
        x: 1.0,
        y: 1.0,
        age_ms: 10,
    }
}

#[test]
fn pre_hydration_press_inside_disabled_button_is_dropped_not_replayed() {
    for (disabled, expected) in [(true, 0), (false, 1)] {
        let mut server = web();
        let mut store = NodeStore::new();
        commit(&mut server, &mut store, |tx| button_with_label(tx, 0, disabled));
        let mut client = WebBackend::hydrate_from_ssr(&server.render_ssr(&store));
        client.queue_pre_hydration_events(vec![press(3), press(2)]);
        let mut client_store = NodeStore::new();
        commit(&mut client, &mut client_store, |tx| button_with_label(tx, 100, disabled));
        let events = client.drain_events();
        assert_eq!(events.len(), expected * 2, "disabled={disabled}: {events:?}");
        assert!(events.iter().all(|e| e.node_id == 102 && e.kind == NativeEventKind::Press));
        assert_eq!(client.hydration_report().replayed_events, (expected * 2) as u32);
    }
}

#[test]
fn queued_pre_hydration_events_without_ssr_pairing_are_discarded() {
    // No SSR root to pair against: captured SSR ids cannot be remapped, so replaying them raw
    // would hit whatever client node happens to own that id.
    for html in ["", "<!doctype html><html><body><div id=\"app\"></div></body></html>"] {
        let mut client = WebBackend::hydrate_from_streaming_html(html);
        client.queue_pre_hydration_events(vec![press(2), press(3)]);
        let mut store = NodeStore::new();
        commit(&mut client, &mut store, |tx| button_with_label(tx, 0, false));
        assert!(client.drain_events().is_empty(), "html={html:?}");
        assert_eq!(client.hydration_report().replayed_events, 0);
    }

    // Events queued after the pairing commit also carry stale SSR ids: discarded too.
    let mut server = web();
    let mut store = NodeStore::new();
    commit(&mut server, &mut store, |tx| button_with_label(tx, 0, false));
    let mut client = WebBackend::hydrate_from_ssr(&server.render_ssr(&store));
    let mut client_store = NodeStore::new();
    commit(&mut client, &mut client_store, |tx| button_with_label(tx, 0, false));
    client.queue_pre_hydration_events(vec![press(2)]);
    commit(&mut client, &mut client_store, |tx| {
        tx.edit::<TextContent>(NodeId(3)).text("Again");
    });
    assert!(client.drain_events().is_empty());
}
