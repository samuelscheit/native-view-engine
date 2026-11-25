//! Deep tests for `.nve` native overlay CSS reset, semantic HTML5 tag/attribute SSR/mirror/
//! hydration parity, `<dialog closedby>` backdrop fallback, `decode_form_urlencoded`, Stage-1
//! out-of-order DSD `<slot>` streaming, and Stage-2 `ZeroJsLiveSession` over real loopback TCP.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use nve_abi::{CommandBufferBuilder, ComponentKind, RawPropValue, ROOT_NODE_ID};
use nve_engine::RenderBackend;

use crate::{
    decode_form_urlencoded, decode_form_urlencoded_map, encode_http_chunk,
    scoped_slot_hiding_css, should_close_dialog_on_backdrop_click, slot_placeholder_html,
    streaming_http_headers, FormActionResponse, StreamingDocumentOptions, StreamingError,
    StreamingMode, StreamingSsrSession, WebBackend, ZeroJsLiveConfig, ZeroJsLiveSession,
    DEFAULT_MAX_LIVE_UPDATES, HTTP_CHUNK_TERMINATOR, NVE_BASE_RESET_CSS,
    NVE_LIVE_SINK_IFRAME_NAME,
};

fn build_semantic_frame() -> Vec<u8> {
    let mut b = CommandBufferBuilder::new(1, 1);

    // 2: <button id="open-btn" command="show-modal" commandfor="settings-dlg" type="button">
    b.create_node(2, ComponentKind::Pressable, None);
    b.set_raw_prop_str(2, "id", "open-btn");
    b.set_raw_prop_str(2, "type", "button");
    b.set_raw_prop_str(2, "command", "show-modal");
    b.set_raw_prop_str(2, "commandfor", "settings-dlg");
    b.set_text(2, "Open Settings");

    // 3: <dialog id="settings-dlg" open closedby="any" role="dialog" aria-modal="true">
    b.create_node(3, ComponentKind::View, None);
    b.set_raw_prop_str(3, "semanticTag", "dialog");
    b.set_raw_prop_str(3, "id", "settings-dlg");
    b.set_raw_prop(3, "open", &RawPropValue::Bool(true));
    b.set_raw_prop_str(3, "closedby", "any");
    b.set_raw_prop_str(3, "accessibilityRole", "dialog");
    b.set_raw_prop_str(3, "ariaModal", "true");

    // 4: <button popovertarget="help-pop" popovertargetaction="toggle">
    b.create_node(4, ComponentKind::Pressable, None);
    b.set_raw_prop_str(4, "popovertarget", "help-pop");
    b.set_raw_prop_str(4, "popovertargetaction", "toggle");
    b.set_text(4, "Help");

    // 5: <div id="help-pop" popover="auto" role="dialog"> (stays <div> because popover is set!)
    b.create_node(5, ComponentKind::View, None);
    b.set_raw_prop_str(5, "id", "help-pop");
    b.set_raw_prop_str(5, "popover", "auto");
    b.set_raw_prop_str(5, "accessibilityRole", "dialog");

    // 6: <details id="faq-1" open name="faq"> -> 7: <summary>
    b.create_node(6, ComponentKind::View, None);
    b.set_raw_prop_str(6, "semanticTag", "details");
    b.set_raw_prop_str(6, "id", "faq-1");
    b.set_raw_prop(6, "open", &RawPropValue::Bool(true));
    b.set_raw_prop_str(6, "name", "faq");
    b.create_node(7, ComponentKind::View, None);
    b.set_raw_prop_str(7, "semanticTag", "summary");
    b.set_text(7, "Question 1");
    b.set_children(6, &[7]);

    // 8: <form method="post" action="/save" target="nve-sink" novalidate>
    b.create_node(8, ComponentKind::View, None);
    b.set_raw_prop_str(8, "semanticTag", "form");
    b.set_raw_prop_str(8, "method", "post");
    b.set_raw_prop_str(8, "action", "/save");
    b.set_raw_prop_str(8, "target", "nve-sink");
    b.set_raw_prop(8, "novalidate", &RawPropValue::Bool(true));
    // 9: <input name="email" type="email" required>
    b.create_node(9, ComponentKind::TextInput, None);
    b.set_raw_prop_str(9, "name", "email");
    b.set_raw_prop_str(9, "type", "email");
    b.set_raw_prop(9, "required", &RawPropValue::Bool(true));
    // 10: <button type="submit" name="intent" value="save">
    b.create_node(10, ComponentKind::Pressable, None);
    b.set_raw_prop_str(10, "type", "submit");
    b.set_raw_prop_str(10, "name", "intent");
    b.set_raw_prop_str(10, "value", "save");
    b.set_text(10, "Save");
    b.set_children(8, &[9, 10]);

    b.set_children(ROOT_NODE_ID, &[2, 3, 4, 5, 6, 8]);
    b.commit(true);
    b.finish().to_vec()
}

#[test]
fn base_reset_css_hides_closed_dialog_and_popover_without_hiding_open_states() {
    assert!(
        NVE_BASE_RESET_CSS.contains(".nve:is(dialog:not([open]),[popover]:not(:popover-open)){display:none}"),
        "base reset must preserve UA display:none on closed <dialog> and [popover]: {NVE_BASE_RESET_CSS}"
    );
    assert!(
        NVE_BASE_RESET_CSS.contains("dialog.nve::backdrop{"),
        "base reset must style dialog::backdrop cleanly"
    );
    // Must never unconditionally hide open dialogs or open popovers.
    assert!(!NVE_BASE_RESET_CSS.contains("dialog[open]{display:none"));
    assert!(!NVE_BASE_RESET_CSS.contains(":popover-open{display:none"));
}

#[test]
fn native_html5_semantic_primitives_have_ssr_mirror_and_zero_mismatch_hydration_parity() {
    let frame = build_semantic_frame();
    let mut web = WebBackend::new();
    web.submit_frame(&frame).unwrap();

    let ssr = web.render_ssr();
    let mirror = web.render_html();

    for html in [&ssr.html, &mirror] {
        assert!(html.contains("<button data-nve-id=\"2\""), "{html}");
        assert!(html.contains("command=\"show-modal\""), "{html}");
        assert!(html.contains("commandfor=\"settings-dlg\""), "{html}");

        assert!(html.contains("<dialog data-nve-id=\"3\""), "{html}");
        assert!(html.contains("id=\"settings-dlg\""), "{html}");
        assert!(html.contains(" open"), "{html}");
        assert!(html.contains("closedby=\"any\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");

        assert!(html.contains("popovertarget=\"help-pop\""), "{html}");
        assert!(html.contains("popovertargetaction=\"toggle\""), "{html}");
        // Popover stays <div> (never coerced to <dialog> even with role="dialog").
        assert!(html.contains("<div data-nve-id=\"5\""), "{html}");
        assert!(html.contains("popover=\"auto\""), "{html}");

        assert!(html.contains("<details data-nve-id=\"6\""), "{html}");
        assert!(html.contains("name=\"faq\""), "{html}");
        assert!(html.contains("<summary data-nve-id=\"7\""), "{html}");

        assert!(html.contains("<form data-nve-id=\"8\""), "{html}");
        assert!(html.contains("method=\"post\""), "{html}");
        assert!(html.contains("action=\"/save\""), "{html}");
        assert!(html.contains("target=\"nve-sink\""), "{html}");
        assert!(html.contains(" novalidate"), "{html}");
        assert!(html.contains("<input data-nve-id=\"9\""), "{html}");
        assert!(html.contains("name=\"email\""), "{html}");
        assert!(html.contains("type=\"email\""), "{html}");
        assert!(html.contains(" required"), "{html}");
        assert!(html.contains("type=\"submit\""), "{html}");
        assert!(html.contains("name=\"intent\""), "{html}");
        assert!(html.contains("value=\"save\""), "{html}");

        // Tag-only metadata must never leak as HTML attributes.
        assert!(!html.contains("semanticTag"), "{html}");
        assert!(!html.contains("semantictag"), "{html}");
    }

    #[cfg(feature = "hydrate")]
    {
        let mut hydrated = WebBackend::hydrate_from_ssr(&ssr);
        hydrated.submit_frame(&frame).unwrap();
        let report = hydrated.hydration_report();
        assert!(report.mismatches.is_empty(), "expected 0 mismatches, got {:?}", report.mismatches);
        assert_eq!(report.replaced_nodes, 0);
        assert_eq!(report.adopted_nodes, 10);
    }
}

#[test]
fn dialog_backdrop_click_fallback_respects_closedby_and_native_support() {
    // Native closedBy present -> JS fallback stays passive.
    assert!(!should_close_dialog_on_backdrop_click(
        Some("any"), true, true, 5.0, 5.0, 100.0, 400.0, 100.0, 300.0
    ));
    // Click outside bounding rect on dialog target with closedby="any" -> close.
    assert!(should_close_dialog_on_backdrop_click(
        Some("any"), false, true, 12.0, 150.0, 100.0, 400.0, 100.0, 300.0
    ));
    // Click inside bounding rect -> do NOT close.
    assert!(!should_close_dialog_on_backdrop_click(
        Some("any"), false, true, 200.0, 200.0, 100.0, 400.0, 100.0, 300.0
    ));
    // Click on child element (target_is_dialog == false) -> do NOT close.
    assert!(!should_close_dialog_on_backdrop_click(
        Some("any"), false, false, 12.0, 150.0, 100.0, 400.0, 100.0, 300.0
    ));
    // closedby="closerequest" or "none" -> backdrop click does NOT close.
    assert!(!should_close_dialog_on_backdrop_click(
        Some("closerequest"), false, true, 12.0, 150.0, 100.0, 400.0, 100.0, 300.0
    ));
    assert!(!should_close_dialog_on_backdrop_click(
        Some("none"), false, true, 12.0, 150.0, 100.0, 400.0, 100.0, 300.0
    ));
}

#[test]
fn decode_form_urlencoded_handles_utf8_plus_repeated_keys_and_malformed_percent() {
    let pairs = decode_form_urlencoded(
        "name=Ada+Lovelace&note=Caf%C3%A9+%F0%9F%9A%80&tag=rust&tag=web&empty=&flag&bad=%ZZ%2",
    );
    assert_eq!(
        pairs,
        vec![
            ("name".into(), "Ada Lovelace".into()),
            ("note".into(), "Café 🚀".into()),
            ("tag".into(), "rust".into()),
            ("tag".into(), "web".into()),
            ("empty".into(), "".into()),
            ("flag".into(), "".into()),
            ("bad".into(), "%ZZ%2".into()),
        ]
    );
    let map = decode_form_urlencoded_map("a=1&b=hello+world&a=2");
    assert_eq!(map.get("a").map(String::as_str), Some("2"));
    assert_eq!(map.get("b").map(String::as_str), Some("hello world"));
}

#[test]
fn stage1_dsd_slot_out_of_order_streaming_resolves_slot2_before_slot1_with_zero_js() {
    let opts = StreamingDocumentOptions {
        title: "Stage 1 OOO Stream".into(),
        ..Default::default()
    };
    let mut session = StreamingSsrSession::new(opts, ["slot-1", "slot-2"]);
    let s1 = slot_placeholder_html("slot-1", "<p>Loading slot 1...</p>").unwrap();
    let s2 = slot_placeholder_html("slot-2", "<p>Loading slot 2...</p>").unwrap();

    let shell = session
        .begin_shell(&format!("<main>{s1}{s2}</main>"))
        .unwrap();
    assert!(shell.contains("<template shadowrootmode=\"open\">"));
    assert!(shell.contains("<slot name=\"nve-slot-slot-1\"><p>Loading slot 1...</p></slot>"));
    assert!(shell.contains("<slot name=\"nve-slot-slot-2\"><p>Loading slot 2...</p></slot>"));
    assert!(shell.contains("<style data-nve-base>"));
    assert!(!session.all_slots_resolved());

    // Resolve slot-2 BEFORE slot-1 (out-of-order async completion!).
    let mut frag2 = WebBackend::new();
    let mut b2 = CommandBufferBuilder::new(1, 1);
    b2.create_node(20, ComponentKind::Text, None);
    b2.set_text(20, "Resolved Slot 2 First");
    b2.set_children(ROOT_NODE_ID, &[20]);
    b2.commit(true);
    frag2.submit_frame(b2.finish()).unwrap();

    let chunk2 = session.resolve_slot_from_backend("slot-2", &frag2).unwrap();
    let chunk1 = session
        .resolve_slot("slot-1", "<div data-nve-id=\"10\">Resolved Slot 1 Second</div>", None)
        .unwrap();
    assert!(session.all_slots_resolved());
    assert_eq!(session.resolved_order(), &["slot-2", "slot-1"]);

    let trailer = session.finish().unwrap();
    assert!(session.is_closed());
    assert_eq!(session.finish(), Err(StreamingError::StreamClosed));

    let full_doc = format!("{shell}{chunk2}{chunk1}{trailer}");
    assert!(!full_doc.contains("<script"), "Stage-1 DSD streaming must emit zero <script tags");
    let pos2 = full_doc.find("slot=\"nve-slot-slot-2\"").unwrap();
    let pos1 = full_doc.find("slot=\"nve-slot-slot-1\"").unwrap();
    assert!(pos2 < pos1, "slot-2 must arrive before slot-1 in streamed wire output");
    assert!(full_doc.ends_with("</body>\n</html>\n"));
}

#[test]
fn stage2_http11_gracefully_negotiates_to_prg_303_and_rejects_input_slot_overwrite() {
    assert_eq!(StreamingMode::negotiate(false, true), StreamingMode::AsyncSsrWithPrgFallback);
    assert_eq!(StreamingMode::negotiate(true, false), StreamingMode::AsyncSsrWithPrgFallback);
    assert_eq!(StreamingMode::negotiate(true, true), StreamingMode::ZeroJsLiveStream);

    let h1_headers = streaming_http_headers(false);
    assert!(h1_headers.contains(&("Transfer-Encoding", "chunked")));
    assert!(h1_headers.contains(&("X-Accel-Buffering", "no")));
    assert!(h1_headers.contains(&("Cache-Control", "no-cache, no-transform")));
    let h2_headers = streaming_http_headers(true);
    assert!(!h2_headers.iter().any(|(k, _)| *k == "Transfer-Encoding"));

    // HTTP/1.1 request degrades cleanly to closed PRG shell.
    let cfg_h1 = ZeroJsLiveConfig::negotiated(false, true)
        .with_compaction_redirect("/items")
        .with_input_slot("composer")
        .with_output_slot("feed");
    let mut sess_h1 = ZeroJsLiveSession::new(StreamingDocumentOptions::default(), cfg_h1);
    assert_eq!(sess_h1.form_target_attr(), None);
    assert_eq!(
        sess_h1.form_action_response(),
        FormActionResponse::SeeOther303 { location: "/items".into() }
    );
    let shell_h1 = sess_h1.begin_shell("<main>Fallback</main>").unwrap();
    assert!(!shell_h1.contains("name=\"nve-sink\""));
    assert!(shell_h1.ends_with("</html>\n"));
    assert!(sess_h1.is_closed());

    // HTTP/2+ live mode enforces immutable input slot protection.
    let cfg_h2 = ZeroJsLiveConfig::negotiated(true, true)
        .with_input_slot("composer")
        .with_output_slot("feed");
    assert_eq!(cfg_h2.max_live_updates, DEFAULT_MAX_LIVE_UPDATES);
    let mut sess_h2 = ZeroJsLiveSession::new(StreamingDocumentOptions::default(), cfg_h2);
    let _ = sess_h2.begin_shell("<main></main>").unwrap();
    assert_eq!(
        sess_h2.push_slot_update("composer", "<input>", None),
        Err(StreamingError::ImmutableInputSlot("composer".into()))
    );

    // Scoped CSS rule strictly targets each slot independently.
    let css = scoped_slot_hiding_css(["counter", "status", "counter"]);
    assert_eq!(
        css,
        "[data-nve-slot=\"counter\"]:has(~ [data-nve-slot=\"counter\"]){display:none!important}[data-nve-slot=\"status\"]:has(~ [data-nve-slot=\"status\"]){display:none!important}"
    );
}

#[test]
fn stage2_zero_js_live_session_end_to_end_over_real_loopback_tcp_with_compaction_cap() {
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
    let addr = listener.local_addr().unwrap();

    let cfg = ZeroJsLiveConfig::negotiated(true, true)
        .with_max_live_updates(3)
        .with_compaction_redirect("/live")
        .with_input_slot("controls")
        .with_output_slot("counter");
    let session = Arc::new(Mutex::new(ZeroJsLiveSession::new(
        StreamingDocumentOptions {
            title: "Zero-JS Live Counter".into(),
            ..Default::default()
        },
        cfg,
    )));
    let stream_writer: Arc<Mutex<Option<TcpStream>>> = Arc::new(Mutex::new(None));
    let counter = Arc::new(Mutex::new(0i32));

    let server_session = Arc::clone(&session);
    let server_writer = Arc::clone(&stream_writer);
    let server_counter = Arc::clone(&counter);

    let server_handle = thread::spawn(move || {
        // Expect 1 GET /live + 4 POST /action requests (3 live 204s + 1 compacted 303).
        for _ in 0..5 {
            let (mut conn, _) = listener.accept().expect("accept");
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut request_line = String::new();
            reader.read_line(&mut request_line).unwrap();

            let mut content_len = 0usize;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some((k, v)) = line.split_once(':')
                    && k.eq_ignore_ascii_case("content-length")
                {
                    content_len = v.trim().parse().unwrap_or(0);
                }
            }

            if request_line.starts_with("GET /live") {
                let mut header_block = String::from("HTTP/1.1 200 OK\r\n");
                for (k, v) in streaming_http_headers(false) {
                    header_block.push_str(&format!("{k}: {v}\r\n"));
                }
                header_block.push_str("\r\n");
                conn.write_all(header_block.as_bytes()).unwrap();

                let mut sess = server_session.lock().unwrap();
                let target = sess.form_target_attr().unwrap();
                let slot_html = slot_placeholder_html("counter", "<span>0</span>").unwrap();
                let shell = sess
                    .begin_shell(&format!(
                        "<form method=\"post\" action=\"/action\" target=\"{target}\"><button type=\"submit\" name=\"delta\" value=\"1\">+1</button></form>{slot_html}"
                    ))
                    .unwrap();
                conn.write_all(encode_http_chunk(&shell).as_bytes()).unwrap();
                conn.write_all(encode_http_chunk(sess.keepalive_comment()).as_bytes())
                    .unwrap();
                conn.flush().unwrap();
                *server_writer.lock().unwrap() = Some(conn);
            } else if request_line.starts_with("POST /action") {
                let mut body = vec![0u8; content_len];
                reader.read_exact(&mut body).unwrap();
                let params = decode_form_urlencoded_map(&String::from_utf8_lossy(&body));
                let delta: i32 = params
                    .get("delta")
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1);

                let mut sess = server_session.lock().unwrap();
                match sess.form_action_response() {
                    FormActionResponse::NoContent204 => {
                        let mut val = server_counter.lock().unwrap();
                        *val += delta;
                        let update = sess
                            .push_slot_update("counter", &format!("<span>{}</span>", *val), None)
                            .unwrap();
                        let triggered = update.compaction_triggered;
                        let chunk = encode_http_chunk(&update.into_combined_html());
                        if let Some(ref mut live_conn) = *server_writer.lock().unwrap() {
                            live_conn.write_all(chunk.as_bytes()).unwrap();
                            if triggered {
                                live_conn.write_all(HTTP_CHUNK_TERMINATOR.as_bytes()).unwrap();
                            }
                            live_conn.flush().unwrap();
                        }
                        if triggered {
                            *server_writer.lock().unwrap() = None;
                        }
                        conn.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
                            .unwrap();
                    }
                    FormActionResponse::SeeOther303 { location } => {
                        let resp = format!(
                            "HTTP/1.1 303 See Other\r\nLocation: {location}\r\nContent-Length: 0\r\n\r\n"
                        );
                        conn.write_all(resp.as_bytes()).unwrap();
                    }
                }
                conn.flush().unwrap();
            }
        }
    });

    // 1. Open live DSD stream on GET /live.
    let mut live_client = TcpStream::connect(addr).unwrap();
    live_client
        .write_all(b"GET /live HTTP/1.1\r\nHost: localhost\r\n\r\n")
        .unwrap();

    // 2. Fire 3 zero-JS form POST clicks -> each returns 204 No Content and streams a slot update.
    let post_action = |body: &str| -> String {
        let mut c = TcpStream::connect(addr).unwrap();
        let req = format!(
            "POST /action HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        c.write_all(req.as_bytes()).unwrap();
        let mut resp = String::new();
        c.read_to_string(&mut resp).unwrap();
        resp
    };

    for i in 1..=3 {
        let resp = post_action("delta=1&label=hello+world");
        assert!(
            resp.starts_with("HTTP/1.1 204 No Content"),
            "click #{i} should get 204 No Content, got {resp}"
        );
    }

    // 3. After hitting max_live_updates == 3, stream closed & next click gets 303 See Other compaction!
    let compacted_resp = post_action("delta=1");
    assert!(
        compacted_resp.starts_with("HTTP/1.1 303 See Other\r\nLocation: /live"),
        "post-cap click must trigger 303 See Other compaction, got {compacted_resp}"
    );

    // 4. Read the full streamed chunked document from live_client to EOF (proving clean close!).
    let mut raw_stream = String::new();
    live_client.read_to_string(&mut raw_stream).unwrap();
    server_handle.join().unwrap();

    assert!(raw_stream.contains("X-Accel-Buffering: no"));
    assert!(raw_stream.contains("Cache-Control: no-cache, no-transform"));
    assert!(raw_stream.contains("Transfer-Encoding: chunked"));
    assert!(raw_stream.contains("<template shadowrootmode=\"open\">"));
    assert!(raw_stream.contains(&format!("name=\"{NVE_LIVE_SINK_IFRAME_NAME}\"")));
    assert!(raw_stream.contains("[data-nve-slot=\"counter\"]:has(~ [data-nve-slot=\"counter\"]){display:none!important}"));
    assert!(raw_stream.contains("<!-- nve-keepalive -->"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\"><span>1</span></div>"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\"><span>2</span></div>"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\"><span>3</span></div>"));
    assert!(raw_stream.contains("</body>\n</html>\n"));
    assert!(raw_stream.ends_with(HTTP_CHUNK_TERMINATOR));
    assert!(!raw_stream.contains("<script"), "Stage-2 zero-JS live stream must contain zero <script tags");
}
