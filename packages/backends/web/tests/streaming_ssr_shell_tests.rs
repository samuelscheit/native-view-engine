#![cfg(all(feature = "ssr", feature = "hydrate"))]
//! Slice 1c streaming SSR shell over `NodeStore`:
//! 1. CSP `style_nonce` stamping + attribute escaping + `</style` / `</script` breakout defense.
//! 2. `.nve-slot,[data-nve-dsd-host]{display:contents}` reset & `streaming_http_headers()`.
//! 3. Ephemeral `<div data-nve-dsd-host>` + Option B mid-stream wire ordering helpers.
//! 4. `render_ssr_streaming_shell` DSD placement hygiene (`<form>`, foster-parenting traps, nested
//!    and duplicate slots) with trap tags supplied by a 3P `WebTableSystem`.
//! 5. `PRE_HYDRATION_REPLAY_SCRIPT_TAG` `composedPath` shadow-piercing capture.

mod support;

use nve_backend_web::{
    NVE_BASE_RESET_CSS, PRE_HYDRATION_REPLAY_SCRIPT_TAG, SsrDocumentOptions,
    StreamingDocumentOptions, StreamingError, StreamingSsrSession, ZeroJsLiveConfig,
    ZeroJsLiveSession, streaming_http_headers,
};
use nve_components::tables::{Accessibility, AriaRole, Typography};
use nve_store::{NodeId, NodeStore, Rgba, Transaction};

use support::*;

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

#[test]
fn csp_style_nonce_breakout_escaping_reset_css_and_composed_path_replay() {
    assert!(
        NVE_BASE_RESET_CSS
            .contains(".nve-slot,[data-nve-dsd-host]:not([hidden]){display:contents}")
            && NVE_BASE_RESET_CSS.contains("[data-nve-dsd-host][hidden]{display:none}"),
        "base reset must include display:contents for DSD host and slot wrapper: {NVE_BASE_RESET_CSS}"
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
    assert!(
        !headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("transfer-encoding"))
    );

    let mut web = web();
    let mut store = NodeStore::new();
    commit(&mut web, &mut store, |tx| {
        let n = text(tx, 2, "Nonce test");
        tx.edit::<Typography>(n).color(Rgba::rgb(20, 40, 60));
        tx.set_children(NodeId::ROOT, &[n]);
    });

    // 1. Default `SsrDocumentOptions` omits `nonce=`.
    let no_nonce = web.render_ssr_document(&store, &SsrDocumentOptions::default());
    assert!(!no_nonce.contains("nonce="));

    // 2. Opt-in `style_nonce` stamps attribute-escaped `nonce="..."` on base & ssr `<style>`.
    let opts = SsrDocumentOptions::default().with_style_nonce("abc\"&<>123");
    let with_nonce = web.render_ssr_document(&store, &opts);
    assert!(with_nonce.contains("<style data-nve-base nonce=\"abc&quot;&amp;&lt;&gt;123\">"));
    assert!(
        with_nonce
            .contains("<style data-nve-ssr nonce=\"abc&quot;&amp;&lt;&gt;123\" data-nve-hashes=\"")
    );
    assert_eq!(
        SsrDocumentOptions::default()
            .with_style_nonce("")
            .style_nonce,
        None,
        "empty nonce must stay disabled"
    );

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
    assert!(!contains_ignore_ascii_case(&shell, "</style><script>"));
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
    assert!(chunk.contains(
        "<div slot=\"nve-slot-s1\" data-nve-slot=\"s1\" class=\"nve-slot\"><span>done</span></div>"
    ));
    assert!(!chunk.contains("role="));
    assert!(!chunk.contains("tabindex="));
    assert!(!chunk.contains(" id=\""));

    let q_chunk = sess.emit_query_payload(7, "AQID</ScRiPt>BA==").unwrap();
    assert_eq!(
        q_chunk,
        "<script type=\"application/x-nve-query\" data-nve-seq=\"7\">AQID\\u003c/ScRiPt>BA==</script><template data-nve-q-ack=\"7\"></template>\n"
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
    assert_eq!(
        sess.emit_query_payload(8, "AA=="),
        Err(StreamingError::StreamClosed)
    );

    // 4. Live mode `<style data-nve-live-slots>` and `<style data-nve-live-slot>` also receive `nonce`.
    let live_opts = StreamingDocumentOptions::default().with_style_nonce("live-nonce");
    let cfg = ZeroJsLiveConfig::negotiated(true, true).with_output_slot("feed");
    let mut live = ZeroJsLiveSession::new(live_opts, cfg);
    let live_shell = live.begin_shell("<div data-nve-id=\"1\"></div>").unwrap();
    assert!(live_shell.contains("<style data-nve-live-slots nonce=\"live-nonce\">"));
    let upd = live
        .push_slot_update("dynamic", "<span>hi</span>", None)
        .unwrap();
    assert!(
        upd.slot_html
            .contains("<style data-nve-live-slot=\"dynamic\" nonce=\"live-nonce\">")
    );
}

#[test]
fn dsd_placement_hygiene_rejects_form_ancestor_foster_traps_and_nested_slots() {
    let shell_err = |build: &dyn Fn(&mut Transaction<'_>)| {
        let mut web = web();
        let mut store = NodeStore::new();
        commit(&mut web, &mut store, |tx| build(tx));
        web.render_ssr_streaming_shell(&store).err()
    };

    // 1. Deeply nested inside `<form>` -> rejected.
    let err = shell_err(&|tx| {
        let form = node(tx, 2);
        tx.edit::<Accessibility>(form).role(AriaRole::Form);
        let mid = node(tx, 3);
        let slot = stream_slot(tx, 4, "inside-form");
        tx.set_children(mid, &[slot]);
        tx.set_children(form, &[mid]);
        tx.set_children(NodeId::ROOT, &[form]);
    });
    assert!(
        matches!(err, Some(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("<form>")),
        "{err:?}"
    );

    // 2. HTML5 foster-parenting traps as direct parent: table, tbody, thead, tfoot, tr, select.
    for trap in ["table", "tbody", "thead", "tfoot", "tr", "select"] {
        let err = shell_err(&|tx| {
            let parent = tagged(tx, 2, trap);
            let slot = stream_slot(tx, 3, "bad-trap");
            tx.set_children(parent, &[slot]);
            tx.set_children(NodeId::ROOT, &[parent]);
        });
        assert!(
            matches!(err, Some(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains(trap)),
            "expected foster-parent trap rejection for <{trap}>: {err:?}"
        );
    }

    // 3. Nested `data-nve-stream-slot` inside another `data-nve-stream-slot` -> rejected.
    let err = shell_err(&|tx| {
        let sus = suspense(tx, 2, "outer");
        let outer = stream_slot(tx, 3, "outer");
        let inner = stream_slot(tx, 4, "inner");
        tx.set_children(outer, &[inner]);
        tx.set_children(sus, &[outer]);
        tx.set_children(NodeId::ROOT, &[sus]);
    });
    assert!(
        matches!(err, Some(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("nested")),
        "{err:?}"
    );

    // 4. Duplicate slot ids and unsanitizable ids -> rejected; a valid slot inside a non-trap
    //    `<table>` descendant chain (`table > div > slot`) is accepted.
    let err = shell_err(&|tx| {
        let a = pending_slot(tx, 2, 3, 4, "dup");
        let b = pending_slot(tx, 5, 6, 7, "dup");
        tx.set_children(NodeId::ROOT, &[a, b]);
    });
    assert!(
        matches!(err, Some(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("duplicate")),
        "{err:?}"
    );
    let err = shell_err(&|tx| {
        let slot = stream_slot(tx, 2, "!!!");
        tx.set_children(NodeId::ROOT, &[slot]);
    });
    assert!(
        matches!(err, Some(StreamingError::InvalidSlotId(_))),
        "{err:?}"
    );
    let err = shell_err(&|tx| {
        let table = tagged(tx, 2, "table");
        let cell = node(tx, 3);
        let slot = stream_slot(tx, 4, "ok");
        tx.set_children(cell, &[slot]);
        tx.set_children(table, &[cell]);
        tx.set_children(NodeId::ROOT, &[table]);
    });
    assert_eq!(err, None);
}
