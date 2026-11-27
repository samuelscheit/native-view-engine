#![cfg(all(feature = "ssr", feature = "hydrate"))]
//! `<QuerySuspense>` wrapper placement (`data-nve-suspense` > `data-nve-stream-slot`) must never
//! hide an HTML foster-parent / `<select>` / `<form>` trap, and a TCP-truncated chunk `<style>`
//! must never seed its rule hashes (a closed one does).

mod support;

use nve_backend_web::{
    StreamingDocumentOptions, StreamingError, StreamingSsrSession, WebBackend,
};
use nve_components::tables::{Accessibility, AriaRole};
use nve_store::{NodeId, NodeStore};

use support::*;

/// Builds `ROOT > <tags...> > suspense("qs") > stream_slot("qs")` (the exact `<QuerySuspense>` +
/// `NveStreamSlot` shape) and renders the streaming shell.
fn shell_with_suspense_under(tags: &[&str]) -> Result<String, StreamingError> {
    let mut web = web();
    let mut store = NodeStore::new();
    commit(&mut web, &mut store, |tx| {
        let mut parent = NodeId::ROOT;
        let mut id = 2;
        for tag in tags {
            let n = tagged(tx, id, tag);
            tx.set_children(parent, &[n]);
            parent = n;
            id += 1;
        }
        let sus = pending_slot(tx, id, id + 1, id + 2, "qs");
        tx.set_children(parent, &[sus]);
    });
    web.render_ssr_streaming_shell(&store).map(|out| out.ssr.html)
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
        (&["select", "optgroup"][..], "<select>"),
        (&["form", "div"][..], "<form>"),
    ] {
        let res = shell_with_suspense_under(tags);
        assert!(
            matches!(&res, Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains(needle)),
            "<QuerySuspense> under {tags:?} must be rejected mentioning {needle}: {res:?}"
        );
    }

    // Stacked wrappers stay transparent: the nearest real ancestor is still `<tr>`.
    let mut backend = web();
    let mut store = NodeStore::new();
    commit(&mut backend, &mut store, |tx| {
        let tr = tagged(tx, 2, "tr");
        let outer = suspense(tx, 3, "outer");
        let inner = pending_slot(tx, 4, 5, 6, "inner");
        tx.set_children(outer, &[inner]);
        tx.set_children(tr, &[outer]);
        tx.set_children(NodeId::ROOT, &[tr]);
    });
    assert!(matches!(
        backend.render_ssr_streaming_shell(&store),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("<tr>")
    ));

    // A `<form>` role ancestor is rejected the same way.
    let mut backend = web();
    let mut store = NodeStore::new();
    commit(&mut backend, &mut store, |tx| {
        let form = node(tx, 2);
        tx.edit::<Accessibility>(form).role(AriaRole::Form);
        let sus = pending_slot(tx, 3, 4, 5, "qs");
        tx.set_children(form, &[sus]);
        tx.set_children(NodeId::ROOT, &[form]);
    });
    assert!(matches!(
        backend.render_ssr_streaming_shell(&store),
        Err(StreamingError::InvalidSlotPlacement(msg)) if msg.contains("<form>")
    ));

    // Valid placements: a real element between the trap and the wrapper, or no trap at all.
    for tags in [&["table", "tbody", "tr", "td"][..], &["div", "p"][..], &[][..]] {
        let html = shell_with_suspense_under(tags)
            .unwrap_or_else(|err| panic!("{tags:?} must be a valid placement: {err}"));
        assert!(html.contains("<slot name=\"nve-slot-qs\">"), "{html}");
    }
}

#[test]
fn truncated_chunk_style_never_seeds_its_hashes_but_a_closed_one_does() {
    let mut sess = StreamingSsrSession::new(StreamingDocumentOptions::default(), ["feed"]);
    let shell = sess
        .begin_shell("<div data-nve-id=\"1\" class=\"nve\"><slot name=\"nve-slot-feed\"><div data-nve-id=\"3\" data-nve-stream-slot=\"feed\"></div></slot></div>")
        .expect("shell");
    let hashes = ["nve-feed-a".to_owned(), "nve-feed-b".to_owned()];
    let chunk = sess
        .resolve_slot_with_hashes(
            "feed",
            "<span data-nve-id=\"10\">done</span>",
            Some(".nve-feed-a{color:red}.nve-feed-b{padding:4px 8px}"),
            &hashes,
        )
        .expect("chunk");
    let style_end = chunk.find("</style>").expect("chunk CSS precedes the slot markup");
    assert!(chunk[style_end..].contains("<div slot=\"nve-slot-feed\""));

    let baseline = WebBackend::hydrate_from_streaming_html(&shell);
    let base_seeded = baseline.hydration_report().reused_css_rules;

    // Every cut strictly inside the chunk `<style>` (including `</style` missing `>`) leaves
    // hashes unseeded so the client re-inserts those rules.
    let open_end = chunk.find("\">").expect("style open tag end") + 2;
    for cut in [
        open_end,
        open_end + 10,
        style_end,
        style_end + 4,
        style_end + "</style".len(),
    ] {
        let client = WebBackend::hydrate_from_streaming_html(&format!("{shell}{}", &chunk[..cut]));
        for h in &hashes {
            assert!(
                !client.stylesheet().knows_structural(h),
                "truncated chunk style (cut at {cut}) must not seed {h}"
            );
        }
        assert_eq!(client.hydration_report().reused_css_rules, base_seeded);
    }

    // Trailing whitespace after `</style` without `>` and a literal `</stylesheet>` prefix
    // inside CSS must not close the `<style>` tag prematurely either.
    for unclosed in [
        format!("{shell}{}</style ", &chunk[..style_end]),
        format!("{shell}{}</stylesheet>.after{{}}", &chunk[..style_end]),
    ] {
        let client = WebBackend::hydrate_from_streaming_html(&unclosed);
        for h in &hashes {
            assert!(!client.stylesheet().knows_structural(h), "unclosed style must not seed {h}");
        }
        assert_eq!(client.hydration_report().reused_css_rules, base_seeded);
    }

    // Control: the closed `<style>` seeds both hashes.
    let closed = format!("{shell}{}", &chunk[..style_end + "</style>".len()]);
    let client = WebBackend::hydrate_from_streaming_html(&closed);
    for h in &hashes {
        assert!(client.stylesheet().knows_structural(h), "closed chunk style seeds {h}");
    }
    assert_eq!(client.hydration_report().reused_css_rules, base_seeded + 2);
}
