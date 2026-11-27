//! Ported from main `6739ce8a` `src/streaming_tests.rs` (dropped by the NodeStore port):
//! `decode_form_urlencoded`, Stage-1 out-of-order DSD `<slot>` streaming, and Stage-2
//! `ZeroJsLiveSession` (HTTP/1.1 PRG negotiation, input-slot immutability, real loopback TCP
//! live session with compaction cap).

mod support;

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::{Arc, Mutex};
use std::thread;

use nve_backend_web::{
    DEFAULT_MAX_LIVE_UPDATES, FormActionResponse, NVE_LIVE_SINK_IFRAME_NAME,
    StreamingDocumentOptions, StreamingError, StreamingMode, StreamingSsrSession, ZeroJsLiveConfig,
    ZeroJsLiveSession, scoped_slot_hiding_css,
    slot_placeholder_html, streaming_http_headers,
};
use nve_store::{NodeId, NodeStore};
use support::*;

#[test]
fn zero_js_form_bodies_decode_through_the_single_nve_rpc_decoder() {
    let pairs = nve_rpc::decode_form_urlencoded_limited(
        "name=Ada+Lovelace&note=Caf%C3%A9+%F0%9F%9A%80&tag=rust&tag=web&empty=&flag&bad=%ZZ%2",
        16,
    )
    .unwrap();
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
    let form = nve_rpc::FormPairs::from_urlencoded_limited("a=1&b=hello+world&a=2", 16).unwrap();
    assert_eq!(form.get("a"), Some("2"));
    assert_eq!(form.get("b"), Some("hello world"));
    assert!(nve_rpc::decode_form_urlencoded_limited("a=1&b=2&c=3", 2).is_err());
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
    let mut frag2 = web();
    let mut store2 = NodeStore::new();
    commit(&mut frag2, &mut store2, |tx| {
        let t = text(tx, 20, "Resolved Slot 2 First");
        tx.set_children(NodeId::ROOT, &[t]);
    });

    let chunk2 = session
        .resolve_slot_from_backend("slot-2", &frag2, &store2)
        .unwrap();
    let chunk1 = session
        .resolve_slot(
            "slot-1",
            "<div data-nve-id=\"10\">Resolved Slot 1 Second</div>",
            None,
        )
        .unwrap();
    assert!(session.all_slots_resolved());
    assert_eq!(session.resolved_order(), &["slot-2", "slot-1"]);

    let trailer = session.finish().unwrap();
    assert!(session.is_closed());
    assert_eq!(session.finish(), Err(StreamingError::StreamClosed));

    let full_doc = format!("{shell}{chunk2}{chunk1}{trailer}");
    assert!(
        !full_doc.contains("<script"),
        "Stage-1 DSD streaming must emit zero <script tags"
    );
    let pos2 = full_doc.find("slot=\"nve-slot-slot-2\"").unwrap();
    let pos1 = full_doc.find("slot=\"nve-slot-slot-1\"").unwrap();
    assert!(
        pos2 < pos1,
        "slot-2 must arrive before slot-1 in streamed wire output"
    );
    assert!(full_doc.ends_with("</body>\n</html>\n"));
}

#[test]
fn stage2_http11_gracefully_negotiates_to_prg_303_and_rejects_input_slot_overwrite() {
    assert_eq!(
        StreamingMode::negotiate(false, true),
        StreamingMode::AsyncSsrWithPrgFallback
    );
    assert_eq!(
        StreamingMode::negotiate(true, false),
        StreamingMode::AsyncSsrWithPrgFallback
    );
    assert_eq!(
        StreamingMode::negotiate(true, true),
        StreamingMode::ZeroJsLiveStream
    );

    let headers = streaming_http_headers();
    assert!(headers.contains(&("Content-Type", "text/html; charset=utf-8")));
    assert!(headers.contains(&("Cache-Control", "private, no-store, no-transform")));
    assert!(headers.contains(&("Vary", "Cookie, Authorization")));
    assert!(headers.contains(&("X-Content-Type-Options", "nosniff")));
    assert!(headers.contains(&("X-Accel-Buffering", "no")));
    assert!(!headers.iter().any(|(k, _)| *k == "Transfer-Encoding"));

    // HTTP/1.1 request degrades cleanly to closed PRG shell.
    let cfg_h1 = ZeroJsLiveConfig::negotiated(false, true)
        .with_compaction_redirect("/items")
        .with_input_slot("composer")
        .with_output_slot("feed");
    let mut sess_h1 = ZeroJsLiveSession::new(StreamingDocumentOptions::default(), cfg_h1);
    assert_eq!(sess_h1.form_target_attr(), None);
    assert_eq!(
        sess_h1.form_action_response(),
        FormActionResponse::SeeOther303 {
            location: "/items".into()
        }
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

    const CHUNK_END: &str = "0\r\n\r\n";
    let frame_chunk = |payload: &str| -> String {
        if payload.is_empty() {
            String::new()
        } else {
            format!("{:x}\r\n{payload}\r\n", payload.len())
        }
    };

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
                for (k, v) in streaming_http_headers() {
                    header_block.push_str(&format!("{k}: {v}\r\n"));
                }
                header_block.push_str("Transfer-Encoding: chunked\r\n\r\n");
                conn.write_all(header_block.as_bytes()).unwrap();

                let mut sess = server_session.lock().unwrap();
                let target = sess.form_target_attr().unwrap();
                let slot_html = slot_placeholder_html("counter", "<span>0</span>").unwrap();
                let shell = sess
                    .begin_shell(&format!(
                        "<form method=\"post\" action=\"/action\" target=\"{target}\"><button type=\"submit\" name=\"delta\" value=\"1\">+1</button></form>{slot_html}"
                    ))
                    .unwrap();
                conn.write_all(frame_chunk(&shell).as_bytes()).unwrap();
                conn.write_all(frame_chunk(sess.keepalive_comment()).as_bytes())
                    .unwrap();
                conn.flush().unwrap();
                *server_writer.lock().unwrap() = Some(conn);
            } else if request_line.starts_with("POST /action") {
                let mut body = vec![0u8; content_len];
                reader.read_exact(&mut body).unwrap();
                let params = nve_rpc::FormPairs::from_urlencoded_limited(
                    &String::from_utf8_lossy(&body),
                    16,
                )
                .unwrap();
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
                        let chunk = frame_chunk(&update.into_combined_html());
                        if let Some(ref mut live_conn) = *server_writer.lock().unwrap() {
                            live_conn.write_all(chunk.as_bytes()).unwrap();
                            if triggered {
                                live_conn.write_all(CHUNK_END.as_bytes()).unwrap();
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
    assert!(raw_stream.contains("Cache-Control: private, no-store, no-transform"));
    assert!(raw_stream.contains("Vary: Cookie, Authorization"));
    assert!(raw_stream.contains("X-Content-Type-Options: nosniff"));
    assert!(raw_stream.contains("<div data-nve-dsd-host>"));
    assert!(raw_stream.contains("<template shadowrootmode=\"open\">"));
    assert!(raw_stream.contains(&format!("name=\"{NVE_LIVE_SINK_IFRAME_NAME}\"")));
    assert!(raw_stream.contains(
        "[data-nve-slot=\"counter\"]:has(~ [data-nve-slot=\"counter\"]){display:none!important}"
    ));
    assert!(raw_stream.contains("<!-- nve-keepalive -->"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\" class=\"nve-slot\"><span>1</span></div>"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\" class=\"nve-slot\"><span>2</span></div>"));
    assert!(raw_stream.contains("<div slot=\"nve-slot-counter\" data-nve-slot=\"counter\" class=\"nve-slot\"><span>3</span></div>"));
    assert!(
        raw_stream
            .contains("<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n")
    );
    assert!(raw_stream.ends_with(CHUNK_END));
    assert!(
        !raw_stream.contains("<script"),
        "Stage-2 zero-JS live stream must contain zero <script tags"
    );
}
