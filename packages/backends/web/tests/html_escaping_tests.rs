#![cfg(feature = "ssr")]
//! Single-source HTML attribute / raw-script escaping and tag emitters shared by
//! `StreamingSsrSession` and `nve-ssr`.

use nve_backend_web::{
    push_async_boot_script, push_attr_escaped, push_modulepreload_link, push_query_payload_tag,
    push_script_data_escaped, push_stylesheet_links, StreamingDocumentOptions, StreamingSsrSession,
};

fn attr(v: &str) -> String {
    let mut out = String::new();
    push_attr_escaped(v, &mut out);
    out
}

fn script(v: &str) -> String {
    let mut out = String::new();
    push_script_data_escaped(v, &mut out);
    out
}

#[test]
fn attr_escaping_covers_quotes_angles_amp_and_nul() {
    assert_eq!(attr(""), "");
    assert_eq!(attr("plain-ascii"), "plain-ascii");
    assert_eq!(
        attr("a&b\"c'd<e>f\0g"),
        "a&amp;b&quot;c&#39;d&lt;e&gt;f\u{FFFD}g"
    );
    assert_eq!(attr("'\"'"), "&#39;&quot;&#39;");
    // Multi-byte runs are copied intact around escapes.
    assert_eq!(attr("été<日本>"), "été&lt;日本&gt;");
    assert_eq!(attr("<"), "&lt;");
}

#[test]
fn script_data_escapes_close_tag_and_comment_open_case_insensitively() {
    assert_eq!(script("AQID+/=="), "AQID+/==");
    assert_eq!(script("a</script>b"), "a\\u003c/script>b");
    assert_eq!(script("a</ScRiPt b"), "a\\u003c/ScRiPt b");
    assert_eq!(script("<!--x-->"), "\\u003c!--x-->");
    assert_eq!(script("<!--<script></script>"), "\\u003c!--<script>\\u003c/script>");
    assert_eq!(script("</script</script"), "\\u003c/script\\u003c/script");
    // Partial needles and bare `<` pass through untouched.
    assert_eq!(script("<!-"), "<!-");
    assert_eq!(script("</scrip"), "</scrip");
    assert_eq!(script("a<b</s"), "a<b</s");
    assert_eq!(script("<"), "<");
    assert_eq!(script("日<!--本"), "日\\u003c!--本");
}

/// The escaped form decodes back to the original inside JSON (and JS) string literals.
#[test]
fn script_data_escapes_stay_valid_json_string_escapes() {
    for raw in ["</script><!--x", "a</SCRIPT>日<!--", "<!--</script>"] {
        let escaped = script(raw);
        assert!(!escaped.to_ascii_lowercase().contains("</script") && !escaped.contains("<!--"));
        // `\u003c` is the only escape introduced: decoding it restores the input exactly.
        assert_eq!(escaped.replace("\\u003c", "<"), raw);
    }
}

#[test]
fn shared_tag_emitters_trim_skip_blank_and_escape() {
    let mut out = String::new();
    push_stylesheet_links(&["  ".into(), " /a.css?x=1&y=\"2\" ".into()], &mut out);
    assert_eq!(
        out,
        "<link rel=\"stylesheet\" href=\"/a.css?x=1&amp;y=&quot;2&quot;\">\n"
    );

    out.clear();
    push_modulepreload_link(Some("  "), &mut out);
    push_modulepreload_link(None, &mut out);
    assert!(out.is_empty());
    push_modulepreload_link(Some(" /boot.js "), &mut out);
    assert_eq!(out, "<link rel=\"modulepreload\" href=\"/boot.js\">\n");

    out.clear();
    push_async_boot_script(Some(""), &mut out);
    assert!(out.is_empty());
    push_async_boot_script(Some("/b.js?'"), &mut out);
    assert_eq!(
        out,
        "<script type=\"module\" async src=\"/b.js?&#39;\"></script>\n"
    );

    out.clear();
    push_query_payload_tag(3, None, "QQ==<!--</script>", &mut out);
    assert_eq!(
        out,
        "<script type=\"application/x-nve-query\" data-nve-seq=\"3\">QQ==\\u003c!--\\u003c/script></script><template data-nve-q-ack=\"3\"></template>\n"
    );

    out.clear();
    push_query_payload_tag(4, Some("st\"ats"), "QQ==", &mut out);
    assert_eq!(
        out,
        "<script type=\"application/x-nve-query\" data-nve-seq=\"4\" data-nve-slot-for=\"st&quot;ats\">QQ==</script><template data-nve-q-ack=\"4\"></template>\n"
    );
}

#[test]
fn slot_delta_binds_its_payload_and_orders_data_before_markup_before_ack() {
    let mut sess = StreamingSsrSession::new(StreamingDocumentOptions::default(), ["stats"]);
    sess.begin_shell("<slot name=\"nve-slot-stats\"></slot>").unwrap();
    let delta = nve_backend_web::SsrDeltaChunk {
        html: "<p>42</p>".into(),
        style_css: ".x{color:red}".into(),
        rule_hashes: vec!["h1".into()],
    };
    let chunk = sess.resolve_slot_delta("st$ats", &delta, Some((2, "QQ=="))).unwrap();
    let at = |needle: &str| {
        chunk
            .find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in {chunk}"))
    };
    // The slot id is sanitized once and used for both the payload binding and the chunk.
    let order = [
        at("data-nve-seq=\"2\" data-nve-slot-for=\"stats\">QQ==</script>"),
        at("<template data-nve-q-ack=\"2\">"),
        at("<style data-nve-ssr-chunk=\"stats\""),
        at("<div slot=\"nve-slot-stats\" data-nve-slot=\"stats\""),
        at("<template data-nve-slot-ack=\"stats\"></template>"),
    ];
    assert!(order.windows(2).all(|w| w[0] < w[1]), "{chunk}");
    assert!(chunk.ends_with("<template data-nve-slot-ack=\"stats\"></template>\n"));

    // Without a payload the chunk carries no query script at all.
    let mut sess = StreamingSsrSession::new(StreamingDocumentOptions::default(), ["a"]);
    sess.begin_shell("<slot name=\"nve-slot-a\"></slot>").unwrap();
    let bare = sess.resolve_slot_delta("a", &delta, None).unwrap();
    assert!(!bare.contains("x-nve-query"), "{bare}");
}

#[test]
fn streaming_session_emitters_match_shared_helpers_byte_for_byte() {
    let mut sess = StreamingSsrSession::new(
        StreamingDocumentOptions {
            stylesheet_links: vec!["/x.css?a&b".into()],
            ..Default::default()
        },
        ["a"],
    );
    let shell = sess.begin_shell("<slot name=\"nve-slot-a\"></slot>").unwrap();
    let mut links = String::new();
    push_stylesheet_links(&["/x.css?a&b".into()], &mut links);
    assert_eq!(shell.matches(links.as_str()).count(), 2, "head + shadow root");

    let payload = "AQ<!--</SCRIPT";
    let mut expected = String::new();
    push_query_payload_tag(9, None, payload, &mut expected);
    assert_eq!(sess.emit_query_payload(9, payload).unwrap(), expected);

    expected.clear();
    push_async_boot_script(Some(" /boot.js?v='1' "), &mut expected);
    assert_eq!(
        sess.emit_async_boot_script(" /boot.js?v='1' ").unwrap(),
        expected
    );
}
