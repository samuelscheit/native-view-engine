#![cfg(all(feature = "ssr", feature = "hydrate"))]
//! Comprehensive tests for Slice 1c Track B:
//! 1. CSP `style_nonce` stamping + attribute escaping + `</style` / `</script` breakout defense.
//! 2. `.nve-slot,[data-nve-dsd-host]:not([hidden]){display:contents}` reset & `streaming_http_headers()`.
//! 3. Ephemeral `<div data-nve-dsd-host>` + Option B mid-stream wire ordering helpers.
//! 4. `render_ssr_streaming_shell` DSD placement hygiene (`<form>`, foster-parenting traps, nested slots).
//! 5. `stream_slot_state` + `render_ssr_subtree_delta` stateful CSS deduplication.
//! 6. `PRE_HYDRATION_REPLAY_SCRIPT_TAG` `composedPath` shadow-piercing capture.
//! 7. Top-down structural positional hydration (`hydrate_from_ssr` & `hydrate_from_streaming_html`):
//!    - Post-stream out-of-order `[slot2, slot1]` with COMPLETELY SHIFTED `NodeId`s.
//!    - Mid-stream partial `[slot2 only, slot1 pending]` with shifted `NodeId`s.
//!    - Mutation test: swapping two same-tag siblings with different child counts fails loudly!
//!    - 3-way dirty `<input>` preservation + `ssr_id_remap` pre-hydration click replay with shifted IDs.

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, EventMask, EventPayload, NativeEventKind,
    NodeId, RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_engine::RenderBackend;

use nve_backend_web::{
    streaming_http_headers, CapturedPreHydrationEvent, PendingStreamSlot, SsrDocumentOptions, StreamSlotState,
    StreamingDocumentOptions, StreamingError, StreamingSsrSession, WebBackend, ZeroJsLiveConfig,
    ZeroJsLiveSession, NVE_BASE_RESET_CSS, PRE_HYDRATION_REPLAY_SCRIPT_TAG,
};

fn apply(web: &mut WebBackend, build: impl FnOnce(&mut CommandBufferBuilder)) {
    let mut b = CommandBufferBuilder::new(1, 1);
    build(&mut b);
    b.commit(true);
    web.submit_frame(b.finish()).unwrap();
}

#[test]
fn csp_style_nonce_breakout_escaping_reset_css_and_composed_path_replay() {
    assert!(
        NVE_BASE_RESET_CSS.contains(
            ".nve-slot,[data-nve-dsd-host]:not([hidden]){display:contents}[data-nve-dsd-host][hidden]{display:none}"
        ),
        "base reset must use display:contents for slot wrappers and a visible DSD host, while a retained `hidden` host stays hidden: {NVE_BASE_RESET_CSS}"
    );
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("composedPath"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.contains("getRootNode"));
    assert!(PRE_HYDRATION_REPLAY_SCRIPT_TAG.len() < 1024);

    let headers = streaming_http_headers();
    assert_eq!(headers.len(), 5);
    assert!(headers.contains(&("Content-Type", "text/html; charset=utf-8")));
    assert!(headers.contains(&("Cache-Control", "private, no-store, no-transform")));
    assert!(headers.contains(&("Vary", "Cookie, Authorization")));
    assert!(headers.contains(&("X-Content-Type-Options", "nosniff")));
    assert!(headers.contains(&("X-Accel-Buffering", "no")));
    assert!(!headers.iter().any(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding")));

    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        b.create_node(2, ComponentKind::Text, None);
        b.set_text(2, "Nonce test");
        b.set_style(
            2,
            &StyleProps {
                text_color: ColorRgba::rgb(20, 40, 60),
                ..Default::default()
            },
        );
        b.set_children(ROOT_NODE_ID, &[2]);
    });

    // 1. Default `SsrDocumentOptions` omits `nonce=`.
    let no_nonce = web.render_ssr_document(&SsrDocumentOptions::default());
    assert!(!no_nonce.contains("nonce="));

    // 2. Opt-in `style_nonce` stamps attribute-escaped `nonce="..."` on base & ssr `<style>`.
    let opts = SsrDocumentOptions::default().with_style_nonce("abc\"&<>123");
    let with_nonce = web.render_ssr_document(&opts);
    assert!(with_nonce.contains("<style data-nve-base nonce=\"abc&quot;&amp;&lt;&gt;123\">"));
    assert!(with_nonce.contains("<style data-nve-ssr nonce=\"abc&quot;&amp;&lt;&gt;123\" data-nve-hashes=\""));

    // 3. Streaming `<style>` tags stamp `nonce="..."` AND escape case-insensitive `</style` breakout!
    let stream_opts = StreamingDocumentOptions {
        title: "Streaming Nonce & Escape".into(),
        style_css: ".evil{content:'</StYlE><script>alert(1)</script>'}</STYLE>".into(),
        rule_hashes: vec!["s123".into()],
        ..Default::default()
    }
    .with_style_nonce("req-nonce-42");
    let mut sess = StreamingSsrSession::new(stream_opts, ["s1"]);
    let shell = sess
        .begin_shell_with_head_scripts(
            "<div data-nve-id=\"1\"><slot name=\"nve-slot-s1\"></slot></div>",
            "<link rel=\"modulepreload\" href=\"/app.js\">",
        )
        .unwrap();
    assert!(shell.contains("<link rel=\"modulepreload\" href=\"/app.js\">"));
    assert!(shell.contains("<div id=\"app\" data-nve-stream-host=\"ssr\">\n<div data-nve-dsd-host>\n<template shadowrootmode=\"open\">\n"));
    assert!(!shell.to_ascii_lowercase().contains("</style><script>"));
    assert!(shell.contains("<\\/StYlE>"));
    assert!(shell.contains("<\\/STYLE>"));
    assert!(shell.contains("<style data-nve-base nonce=\"req-nonce-42\">"));
    assert!(shell.contains("<style data-nve-ssr nonce=\"req-nonce-42\" data-nve-hashes=\"s123\">"));

    let chunk = sess
        .resolve_slot_with_hashes(
            "s1",
            "<span>done</span>",
            Some(".c1{color:red}</STYLE><img src=x>"),
            &["sc1".into()],
        )
        .unwrap();
    assert!(chunk.contains(
        "<style data-nve-ssr-chunk=\"s1\" nonce=\"req-nonce-42\" data-nve-hashes=\"sc1\">.c1{color:red}<\\/STYLE><img src=x></style>"
    ));
    assert!(chunk.contains("<div slot=\"nve-slot-s1\" data-nve-slot=\"s1\" class=\"nve-slot\"><span>done</span></div><template data-nve-slot-ack=\"s1\"></template>\n"));
    assert!(!chunk.contains("role="));
    assert!(!chunk.contains("tabindex="));
    assert!(!chunk.contains(" id=\""));

    let q_chunk = sess
        .emit_query_payload(7, "AQID</ScRiPt>BA==")
        .unwrap();
    assert_eq!(
        q_chunk,
        "<script type=\"application/x-nve-query\" data-nve-seq=\"7\">AQID<\\/ScRiPt>BA==</script><template data-nve-q-ack=\"7\"></template>\n"
    );
    let boot = sess.emit_async_boot_script("/boot.js?v=1&a=\"2\"").unwrap();
    assert_eq!(
        boot,
        "<script type=\"module\" async src=\"/boot.js?v=1&amp;a=&quot;2&quot;\"></script>\n"
    );
    let end = sess.finish().unwrap();
    assert_eq!(
        end,
        "<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n"
    );

    // 4. Live mode `<style data-nve-live-slots>` and `<style data-nve-live-slot>` also receive `nonce`.
    let live_opts = StreamingDocumentOptions::default().with_style_nonce("live-nonce");
    let cfg = ZeroJsLiveConfig::negotiated(true, true).with_output_slot("feed");
    let mut live = ZeroJsLiveSession::new(live_opts, cfg);
    let live_shell = live.begin_shell("<div data-nve-id=\"1\"></div>").unwrap();
    assert!(live_shell.contains("<style data-nve-live-slots nonce=\"live-nonce\">"));
    let upd = live.push_slot_update("dynamic", "<span>hi</span>", None).unwrap();
    assert!(upd.slot_html.contains("<style data-nve-live-slot=\"dynamic\" nonce=\"live-nonce\">"));
}

#[test]
fn dsd_placement_hygiene_rejects_form_ancestor_foster_traps_and_nested_slots() {
    // 1. Direct or deeply nested inside `<form>` -> rejected.
    let mut form_web = WebBackend::new();
    apply(&mut form_web, |b| {
        b.create_node(2, ComponentKind::View, None);
        b.set_raw_prop_str(2, "semanticTag", "form");
        b.create_node(3, ComponentKind::View, None); // intermediate div inside <form>
        b.create_node(4, ComponentKind::View, None);
        b.set_raw_prop_str(4, "data-nve-stream-slot", "inside-form");
        b.set_children(3, &[4]);
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
    });
    assert!(matches!(
        form_web.render_ssr_streaming_shell(),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("<form>")
    ));

    // 2. HTML5 foster-parenting traps as direct parent: table, tbody, thead, tfoot, tr, select.
    for trap in ["table", "tbody", "thead", "tfoot", "tr", "select"] {
        let mut sem_web = WebBackend::new();
        apply(&mut sem_web, |b| {
            b.create_node(2, ComponentKind::View, None);
            b.set_raw_prop_str(2, "semanticTag", trap);
            b.create_node(3, ComponentKind::View, None);
            b.set_raw_prop_str(3, "data-nve-stream-slot", "bad-trap");
            b.set_children(2, &[3]);
            b.set_children(ROOT_NODE_ID, &[2]);
        });
        assert!(
            matches!(
                sem_web.render_ssr_streaming_shell(),
                Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains(trap)
            ),
            "expected foster-parent trap rejection for semanticTag={trap}"
        );
    }
    // Also verify `ComponentKind::Custom` `"web.select"` direct parent is rejected.
    let mut custom_select = WebBackend::new();
    apply(&mut custom_select, |b| {
        b.create_node(2, ComponentKind::Custom, Some("web.select"));
        b.create_node(3, ComponentKind::View, None);
        b.set_raw_prop_str(3, "data-nve-stream-slot", "bad-select");
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
    });
    assert!(matches!(
        custom_select.render_ssr_streaming_shell(),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("select")
    ));

    // 3. Nested `data-nve-stream-slot` inside another `data-nve-stream-slot` -> rejected.
    let mut nested = WebBackend::new();
    apply(&mut nested, |b| {
        b.create_node(2, ComponentKind::View, None);
        b.set_raw_prop_str(2, "data-nve-suspense", "outer");
        b.create_node(3, ComponentKind::View, None);
        b.set_raw_prop_str(3, "data-nve-stream-slot", "outer");
        b.create_node(4, ComponentKind::View, None);
        b.set_raw_prop_str(4, "data-nve-stream-slot", "inner");
        b.set_children(3, &[4]);
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
    });
    assert!(matches!(
        nested.render_ssr_streaming_shell(),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("nested")
    ));
}

/// Builds `ROOT > <tags...> > div[data-nve-suspense] > div[data-nve-stream-slot]` (the exact
/// `<QuerySuspense>` + `NveStreamSlot` shape) and renders the streaming shell.
fn shell_with_suspense_under(tags: &[&str]) -> Result<String, StreamingError> {
    let mut web = WebBackend::new();
    apply(&mut web, |b| {
        let mut parent = ROOT_NODE_ID;
        let mut id = 2;
        for tag in tags {
            if tag.starts_with("web.") {
                b.create_node(id, ComponentKind::Custom, Some(tag));
            } else {
                b.create_node(id, ComponentKind::View, None);
                b.set_raw_prop_str(id, "semanticTag", tag);
            }
            b.set_children(parent, &[id]);
            parent = id;
            id += 1;
        }
        b.create_node(id, ComponentKind::View, None);
        b.set_raw_prop_str(id, "data-nve-suspense", "qs");
        b.create_node(id + 1, ComponentKind::View, None);
        b.set_raw_prop_str(id + 1, "data-nve-stream-slot", "qs");
        b.set_children(id, &[id + 1]);
        b.set_children(parent, &[id]);
    });
    web.render_ssr_streaming_shell().map(|out| out.ssr.html)
}

#[test]
fn query_suspense_wrapper_never_hides_foster_parent_or_select_traps() {
    for (tags, needle) in [
        (&["table"][..], "<table>"),
        (&["table", "tbody"][..], "<tbody>"),
        (&["table", "thead"][..], "<thead>"),
        (&["table", "tfoot"][..], "<tfoot>"),
        (&["table", "tbody", "tr"][..], "<tr>"),
        (&["select"][..], "<select>"),
        (&["select", "web.optgroup"][..], "<select>"),
        (&["web.select", "web.optgroup"][..], "<select>"),
        (&["form", "div"][..], "<form>"),
    ] {
        let res = shell_with_suspense_under(tags);
        assert!(
            matches!(&res, Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains(needle)),
            "<QuerySuspense> under {tags:?} must be rejected mentioning {needle}: {res:?}"
        );
    }
    // Stacked wrappers stay transparent: the nearest real ancestor is still `<tr>`.
    let mut stacked = WebBackend::new();
    apply(&mut stacked, |b| {
        b.create_node(2, ComponentKind::View, None);
        b.set_raw_prop_str(2, "semanticTag", "tr");
        b.create_node(3, ComponentKind::View, None);
        b.set_raw_prop_str(3, "data-nve-suspense", "outer");
        b.create_node(4, ComponentKind::View, None);
        b.set_raw_prop_str(4, "data-nve-suspense", "inner");
        b.create_node(5, ComponentKind::View, None);
        b.set_raw_prop_str(5, "data-nve-stream-slot", "inner");
        b.set_children(4, &[5]);
        b.set_children(3, &[4]);
        b.set_children(2, &[3]);
        b.set_children(ROOT_NODE_ID, &[2]);
    });
    assert!(matches!(
        stacked.render_ssr_streaming_shell(),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("<tr>")
    ));

    // Valid placements: a real element between the trap and the wrapper, or no trap at all.
    for tags in [&["table", "tbody", "tr", "td"][..], &["div", "p"][..], &[][..]] {
        let html = shell_with_suspense_under(tags)
            .unwrap_or_else(|err| panic!("{tags:?} must be a valid placement: {err}"));
        assert!(html.contains("<slot name=\"nve-slot-qs\">"), "{html}");
    }
}

#[test]
fn streaming_shell_delta_css_dedup_and_shifted_id_post_and_mid_stream_hydration() {
    let shared_style = StyleProps {
        background: ColorRgba::rgb(15, 23, 42),
        text_color: ColorRgba::WHITE,
        ..Default::default()
    };
    let delta_only_style = StyleProps {
        background: ColorRgba::rgb(22, 163, 74),
        border_radius: 10.0,
        ..Default::default()
    };

    // Build Server Frame 0 with two `<QuerySuspense>` boundaries (slot1, slot2) both pending.
    let mut server = WebBackend::new();
    apply(&mut server, |b| {
        // 2: static header
        b.create_node(2, ComponentKind::Text, None);
        b.set_raw_prop_str(2, "accessibilityRole", "heading");
        b.set_raw_prop(2, "ariaLevel", &RawPropValue::I64(1));
        b.set_style(2, &shared_style);
        b.set_text(2, "Dashboard");

        // 3: <div data-nve-suspense="slot1"> -> 4: <div data-nve-stream-slot="slot1"> -> 5: fallback
        b.create_node(3, ComponentKind::View, None);
        b.set_raw_prop_str(3, "data-nve-suspense", "slot1");
        b.create_node(4, ComponentKind::View, None);
        b.set_raw_prop_str(4, "data-nve-stream-slot", "slot1");
        b.create_node(5, ComponentKind::Text, None);
        b.set_text(5, "Loading slot 1...");
        b.set_children(4, &[5]);
        b.set_children(3, &[4]);

        // 6: <div data-nve-suspense="slot2"> -> 7: <div data-nve-stream-slot="slot2"> -> 8: fallback
        b.create_node(6, ComponentKind::View, None);
        b.set_raw_prop_str(6, "data-nve-suspense", "slot2");
        b.create_node(7, ComponentKind::View, None);
        b.set_raw_prop_str(7, "data-nve-stream-slot", "slot2");
        b.create_node(8, ComponentKind::Text, None);
        b.set_text(8, "Loading slot 2...");
        b.set_children(7, &[8]);
        b.set_children(6, &[7]);

        b.set_children(ROOT_NODE_ID, &[2, 3, 6]);
    });

    let mut shell_out = server.render_ssr_streaming_shell().unwrap();
    assert_eq!(
        shell_out.pending_slots,
        vec![
            PendingStreamSlot { slot_id: "slot1".into(), parent_node_id: 3 },
            PendingStreamSlot { slot_id: "slot2".into(), parent_node_id: 6 },
        ]
    );
    assert_eq!(server.stream_slot_state(3, "slot1"), StreamSlotState::Pending);
    assert_eq!(server.stream_slot_state(6, "slot2"), StreamSlotState::Pending);
    assert_eq!(server.stream_slot_state(999, "slot1"), StreamSlotState::Unmounted);
    // Shell keeps `<div data-nve-id="4" data-nve-stream-slot="slot1">` INSIDE `<slot>`.
    assert!(shell_out.ssr.html.contains("<slot name=\"nve-slot-slot1\"><div data-nve-id=\"4\""));
    assert!(shell_out.ssr.html.contains("<slot name=\"nve-slot-slot2\"><div data-nve-id=\"7\""));

    let mut sess = StreamingSsrSession::new(
        StreamingDocumentOptions::from_streaming_shell("Stream App", &shell_out),
        ["slot1", "slot2"],
    );
    let shell_html = sess.begin_shell(&shell_out.ssr.html).unwrap();

    // Resolve slot2 FIRST (out-of-order!) using BOTH `shared_style` (already sent!) and `delta_only_style` (new!).
    apply(&mut server, |b| {
        b.create_node(20, ComponentKind::View, None);
        b.set_style(20, &shared_style); // already sent in shell -> must NOT re-emit!
        b.create_node(21, ComponentKind::Text, None);
        b.set_style(21, &delta_only_style); // brand new -> MUST emit once!
        b.set_text(21, "Slot 2 Resolved");
        b.set_children(20, &[21]);
        b.set_children(6, &[20]);
    });
    assert_eq!(server.stream_slot_state(6, "slot2"), StreamSlotState::Resolved);
    assert_eq!(server.stream_slot_state(3, "slot1"), StreamSlotState::Pending);
    let delta2 = server.render_ssr_subtree_delta(6, &mut shell_out.style_tracker);
    assert_eq!(delta2.rule_hashes.len(), 1, "only delta_only_style should be emitted: {:?}", delta2.rule_hashes);
    assert!(delta2.style_css.contains("#16a34a"));
    assert!(!delta2.style_css.contains("#0f172a"), "already-sent shell rule must NOT re-emit");
    let chunk2 = sess.resolve_slot_delta("slot2", &delta2, None).unwrap();

    // --- TEST A: MID-STREAM PARTIAL HYDRATION (slot2 arrived, slot1 STILL PENDING, no stream-end!) ---
    let mid_stream_html = format!("{shell_html}{chunk2}");
    let mut mid_client = WebBackend::hydrate_from_streaming_html(&mid_stream_html);
    let mid_root_id = mid_client.native_dom_instance_id(ROOT_NODE_ID).unwrap();
    let mid_h1_id = mid_client.native_dom_instance_id(2).unwrap();
    let mid_slot1_pending_id = mid_client.native_dom_instance_id(4).unwrap();
    let mid_slot1_text_id = mid_client.native_dom_instance_id(5).unwrap();
    let mid_slot2_box_id = mid_client.native_dom_instance_id(20).unwrap();
    let mid_slot2_text_id = mid_client.native_dom_instance_id(21).unwrap();

    // Client Frame 0 with COMPLETELY SHIFTED NodeIds (+1000)!
    apply(&mut mid_client, |b| {
        b.create_node(1002, ComponentKind::Text, None);
        b.set_raw_prop_str(1002, "accessibilityRole", "heading");
        b.set_raw_prop(1002, "ariaLevel", &RawPropValue::I64(1));
        b.set_style(1002, &shared_style);
        b.set_text(1002, "Dashboard");

        // slot1 still pending -> `<div data-nve-stream-slot="slot1"><span>Loading...</span></div>`
        b.create_node(1003, ComponentKind::View, None);
        b.set_raw_prop_str(1003, "data-nve-suspense", "slot1");
        b.create_node(1004, ComponentKind::View, None);
        b.set_raw_prop_str(1004, "data-nve-stream-slot", "slot1");
        b.create_node(1005, ComponentKind::Text, None);
        b.set_text(1005, "Loading slot 1...");
        b.set_children(1004, &[1005]);
        b.set_children(1003, &[1004]);

        // slot2 resolved -> direct children `[1020 -> [1021]]`
        b.create_node(1006, ComponentKind::View, None);
        b.set_raw_prop_str(1006, "data-nve-suspense", "slot2");
        b.create_node(1020, ComponentKind::View, None);
        b.set_style(1020, &shared_style);
        b.create_node(1021, ComponentKind::Text, None);
        b.set_style(1021, &delta_only_style);
        b.set_text(1021, "Slot 2 Resolved");
        b.set_children(1020, &[1021]);
        b.set_children(1006, &[1020]);

        b.set_children(ROOT_NODE_ID, &[1002, 1003, 1006]);
    });

    let mid_rep = mid_client.hydration_report();
    assert_eq!(mid_rep.mismatches, Vec::new(), "mid-stream mismatches: {:?}", mid_rep.mismatches);
    assert_eq!(mid_rep.replaced_nodes, 0);
    assert_eq!(mid_rep.adopted_nodes, 8);
    assert_eq!(mid_client.stylesheet().flushed_css_rules().len(), 0, "0 duplicate CSS rules");
    assert_eq!(mid_client.native_dom_instance_id(ROOT_NODE_ID), Some(mid_root_id));
    assert_eq!(mid_client.native_dom_instance_id(1002), Some(mid_h1_id));
    assert_eq!(mid_client.native_dom_instance_id(1004), Some(mid_slot1_pending_id));
    assert_eq!(mid_client.native_dom_instance_id(1005), Some(mid_slot1_text_id));
    assert_eq!(mid_client.native_dom_instance_id(1020), Some(mid_slot2_box_id));
    assert_eq!(mid_client.native_dom_instance_id(1021), Some(mid_slot2_text_id));

    // --- TEST A2: TCP SPLIT right after `</div>` (slot-ack not yet parsed) ---
    // A closed `<div slot>` without its trailing ack is NOT complete: the shadow fallback stays.
    let ack2 = "<template data-nve-slot-ack=\"slot2\"></template>";
    let split_at = chunk2.find(ack2).expect("chunk2 carries its slot ack");
    let split_client = WebBackend::hydrate_from_streaming_html(&format!(
        "{shell_html}{}",
        &chunk2[..split_at]
    ));
    assert!(split_client.native_dom_instance_id(8).is_some(), "fallback must stay adopted");
    assert_eq!(split_client.native_dom_instance_id(21), None, "unacked chunk adopted early");

    // --- TEST B: POST-STREAM OUT-OF-ORDER [slot2, slot1] HYDRATION WITH SHIFTED NodeIds ---
    // Resolve slot1 using `delta_only_style` (already sent in slot2 -> 0 new CSS rules!).
    apply(&mut server, |b| {
        b.create_node(30, ComponentKind::Text, None);
        b.set_style(30, &delta_only_style);
        b.set_text(30, "Slot 1 Resolved Second");
        b.set_children(3, &[30]);
    });
    assert_eq!(server.stream_slot_state(3, "slot1"), StreamSlotState::Resolved);
    let delta1 = server.render_ssr_subtree_delta(3, &mut shell_out.style_tracker);
    assert!(delta1.rule_hashes.is_empty(), "style already sent in slot2 must dedup");
    assert!(delta1.style_css.is_empty());
    let chunk1 = sess.resolve_slot_delta("slot1", &delta1, None).unwrap();
    let trailer = sess.finish().unwrap();

    let full_html = format!("{shell_html}{chunk2}{chunk1}{trailer}");
    let mut full_client = WebBackend::hydrate_from_streaming_html(&full_html);
    let full_slot1_text_id = full_client.native_dom_instance_id(30).unwrap();
    let full_slot2_text_id = full_client.native_dom_instance_id(21).unwrap();

    apply(&mut full_client, |b| {
        b.create_node(2002, ComponentKind::Text, None);
        b.set_raw_prop_str(2002, "accessibilityRole", "heading");
        b.set_raw_prop(2002, "ariaLevel", &RawPropValue::I64(1));
        b.set_style(2002, &shared_style);
        b.set_text(2002, "Dashboard");

        b.create_node(2003, ComponentKind::View, None);
        b.set_raw_prop_str(2003, "data-nve-suspense", "slot1");
        b.create_node(2030, ComponentKind::Text, None);
        b.set_style(2030, &delta_only_style);
        b.set_text(2030, "Slot 1 Resolved Second");
        b.set_children(2003, &[2030]);

        b.create_node(2006, ComponentKind::View, None);
        b.set_raw_prop_str(2006, "data-nve-suspense", "slot2");
        b.create_node(2020, ComponentKind::View, None);
        b.set_style(2020, &shared_style);
        b.create_node(2021, ComponentKind::Text, None);
        b.set_style(2021, &delta_only_style);
        b.set_text(2021, "Slot 2 Resolved");
        b.set_children(2020, &[2021]);
        b.set_children(2006, &[2020]);

        b.set_children(ROOT_NODE_ID, &[2002, 2003, 2006]);
    });

    let full_rep = full_client.hydration_report();
    assert!(full_rep.mismatches.is_empty(), "post-stream mismatches: {:?}", full_rep.mismatches);
    assert_eq!(full_rep.replaced_nodes, 0);
    assert_eq!(full_rep.adopted_nodes, 7);
    assert_eq!(full_client.stylesheet().flushed_css_rules().len(), 0);
    assert_eq!(full_client.native_dom_instance_id(2030), Some(full_slot1_text_id));
    assert_eq!(full_client.native_dom_instance_id(2021), Some(full_slot2_text_id));
}

#[test]
fn mutation_test_swapping_same_tag_siblings_with_different_child_counts_produces_mismatch() {
    let mut server = WebBackend::new();
    apply(&mut server, |b| {
        // Sibling A (`div` with 2 `span` children), Sibling B (`div` with 0 children).
        b.create_node(2, ComponentKind::View, None);
        b.create_node(3, ComponentKind::Text, None);
        b.set_text(3, "One");
        b.create_node(4, ComponentKind::Text, None);
        b.set_text(4, "Two");
        b.set_children(2, &[3, 4]);

        b.create_node(5, ComponentKind::View, None);
        b.set_children(ROOT_NODE_ID, &[2, 5]);
    });
    let ssr = server.render_ssr();

    // Client swaps sibling order: first `div` has 0 children, second `div` has 2 `span` children!
    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    apply(&mut client, |b| {
        b.create_node(10, ComponentKind::View, None);

        b.create_node(11, ComponentKind::View, None);
        b.create_node(12, ComponentKind::Text, None);
        b.set_text(12, "One");
        b.create_node(13, ComponentKind::Text, None);
        b.set_text(13, "Two");
        b.set_children(11, &[12, 13]);

        b.set_children(ROOT_NODE_ID, &[10, 11]);
    });

    let rep = client.hydration_report();
    assert_eq!(
        rep.mismatches.len(),
        4,
        "2 excess trailing spans under first div + 2 missing spans under second div: {:?}",
        rep.mismatches
    );
    assert_eq!(rep.replaced_nodes, 2);
}

#[test]
fn shifted_node_ids_preserve_dirty_input_and_remap_pre_hydration_click_replay() {
    let build_tree = |offset: NodeId, input_val: &str| -> Vec<u8> {
        let mut b = CommandBufferBuilder::new(1, 1);
        let inp = offset + 2;
        let btn = offset + 3;
        let lbl = offset + 4;

        b.create_node(inp, ComponentKind::TextInput, None);
        b.set_raw_prop_str(inp, "value", input_val);
        b.set_event_mask(inp, EventMask::TEXT_CHANGE);

        b.create_node(btn, ComponentKind::Pressable, None);
        b.set_event_mask(btn, EventMask::PRESS);
        b.create_node(lbl, ComponentKind::Text, None);
        b.set_text(lbl, "Submit");
        b.set_children(btn, &[lbl]);

        b.set_children(ROOT_NODE_ID, &[inp, btn]);
        b.commit(true);
        b.finish().to_vec()
    };

    let mut server = WebBackend::new();
    server.submit_frame(&build_tree(0, "draft")).unwrap();
    let ssr = server.render_ssr();

    let mut client = WebBackend::hydrate_from_ssr(&ssr);
    let inp_instance_before = client.native_dom_instance_id(2).unwrap();
    let btn_instance_before = client.native_dom_instance_id(3).unwrap();
    let lbl_instance_before = client.native_dom_instance_id(4).unwrap();

    // User types into SSR input #2 and clicks SSR text label #4 before WASM boots!
    client.set_hydrated_dom_control_value(2, "user-edited-query");
    client.queue_pre_hydration_events(vec![
        CapturedPreHydrationEvent::Press {
            node_id: 4, // old SSR ID of child <span> inside <button> #3
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
    client.submit_frame(&build_tree(500, "server-incoming")).unwrap();

    let rep = client.hydration_report();
    assert!(rep.mismatches.is_empty(), "{:?}", rep.mismatches);
    assert_eq!(rep.replaced_nodes, 0);
    assert_eq!(rep.adopted_nodes, 4);
    assert_eq!(rep.reconciled_dirty_controls, 1);
    assert_eq!(rep.replayed_events, 1);

    // Exact DOM instances adopted under shifted IDs!
    assert_eq!(client.native_dom_instance_id(502), Some(inp_instance_before));
    assert_eq!(client.native_dom_instance_id(503), Some(btn_instance_before));
    assert_eq!(client.native_dom_instance_id(504), Some(lbl_instance_before));

    // Dirty local input preserved via PreferLocal under shifted ID 502!
    assert_eq!(client.native_dom_control_value(502), Some("user-edited-query"));
    assert_eq!(client.node(502).unwrap().text.as_deref(), Some("user-edited-query"));

    // Reconciled TextChange (on 502) followed by remapped+bubbled Press (4 -> 504 -> 503)!
    let events = client.drain_events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].node_id, 502);
    assert_eq!(events[0].kind, NativeEventKind::TextChange);
    assert_eq!(events[0].payload, EventPayload::Text("user-edited-query".into()));

    assert_eq!(events[1].node_id, 503);
    assert_eq!(events[1].kind, NativeEventKind::Press);
    assert_eq!(events[1].payload, EventPayload::Point { x: 15.0, y: 25.0 });
}

#[test]
fn duplicate_slot_chunks_in_streaming_html_last_wins_on_promotion() {
    let mut sess = StreamingSsrSession::new(
        StreamingDocumentOptions::default(),
        ["feed"],
    );
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
    apply(&mut client, |b| {
        b.create_node(2, ComponentKind::View, None);
        b.create_node(99, ComponentKind::Pressable, None);
        b.set_text(99, "latest");
        b.set_children(2, &[99]);
        b.set_children(ROOT_NODE_ID, &[2]);
    });
    assert!(client.hydration_report().mismatches.is_empty());
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
    let fallback_slot_id = client.native_dom_instance_id(3).unwrap();
    let fallback_span_id = client.native_dom_instance_id(4).unwrap();
    apply(&mut client, |b| {
        b.create_node(200, ComponentKind::View, None);
        b.set_raw_prop_str(200, "data-nve-suspense", "feed");
        b.create_node(201, ComponentKind::View, None);
        b.set_raw_prop_str(201, "data-nve-stream-slot", "feed");
        b.create_node(202, ComponentKind::Text, None);
        b.set_text(202, "Loading...");
        b.set_children(201, &[202]);
        b.set_children(200, &[201]);
        b.set_children(ROOT_NODE_ID, &[200]);
    });
    assert!(client.hydration_report().mismatches.is_empty());
    assert_eq!(client.hydration_report().replaced_nodes, 0);
    assert_eq!(client.native_dom_instance_id(201), Some(fallback_slot_id));
    assert_eq!(client.native_dom_instance_id(202), Some(fallback_span_id));
}
