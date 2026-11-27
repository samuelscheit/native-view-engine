//! Procedure contract validation, exhaustive JSON protocol-error codec roundtrip, tag
//! domain-separation/overflow, and transport error-clamping tests for `nve-rpc`.

use core::future::Future as _;

use bytes::Bytes;
use nve_rpc::{
    call_with_codec, decode_protocol_error_json, encode_protocol_error_json,
    format_invalidate_header, format_invalidate_sink, format_rpc_path, is_valid_procedure_name,
    parse_invalidate_header, Action, BoxFuture, CborCodec, EffectKind, InvalidateParse, ProcId,
    Procedure, Query, RawCall, RawReply, RpcCodec, RpcError, RpcStatus, Tag64, TagSink, Transport,
    TransportError, INVALIDATE_ALL_WILDCARD, MAX_INVALIDATE_HEADER_SEGMENTS,
    MAX_INVALIDATE_HEADER_TAGS, STATUS_TABLE,
};

use crate::{DemoError, GetIssue, IssueV1, RenameIssue};

#[test]
fn procedure_ids_name_validation_and_static_tags_work() {
    assert_eq!(GetIssue::NAME, "issues.v1.Issues/Get");
    assert_eq!(GetIssue::ID, ProcId::of("issues.v1.Issues/Get"));
    assert_ne!(GetIssue::ID, RenameIssue::ID);
    const {
        assert!(<GetIssue as Procedure>::Effect::IS_QUERY);
        assert!(!<RenameIssue as Procedure>::Effect::IS_QUERY);
        assert!(is_valid_procedure_name("issues.v1.Issues/Get"));
        assert!(!is_valid_procedure_name(""));
        assert!(!is_valid_procedure_name("/leading/slash"));
        assert!(!is_valid_procedure_name("trailing/"));
        assert!(!is_valid_procedure_name("bad//double"));
        assert!(!is_valid_procedure_name("bad name"));
        assert!(!is_valid_procedure_name("bad?query=1"));
        assert!(!is_valid_procedure_name("bad#frag"));
        assert!(!is_valid_procedure_name("bad%20pct"));
        assert!(!is_valid_procedure_name("."));
        assert!(!is_valid_procedure_name(".."));
        assert!(!is_valid_procedure_name("./x"));
        assert!(!is_valid_procedure_name("a/../b"));
        assert!(!is_valid_procedure_name("a/."));
    }
    assert_eq!(
        format_rpc_path("/rpc", GetIssue::NAME),
        "/rpc/issues.v1.Issues/Get"
    );
    assert_eq!(
        format_rpc_path("rpc/", "/issues.v1.Issues/Get"),
        "/rpc/issues.v1.Issues/Get"
    );

    let mut q_sink = TagSink::new();
    GetIssue::tags(&42, &mut q_sink);
    assert_eq!(q_sink.as_slice(), &[Tag64::keyed("issue", 42)]);

    let mut a_sink = TagSink::new();
    RenameIssue::invalidates(
        &IssueV1 {
            id: 7,
            title: "t".into(),
            open: true,
        },
        &mut a_sink,
    );
    assert_eq!(
        a_sink.as_slice(),
        &[Tag64::of("issues.list"), Tag64::keyed("issue", 7)]
    );

    fn require_query<P: Procedure<Effect = Query>>() -> &'static str {
        P::NAME
    }
    fn require_action<P: Procedure<Effect = Action>>() -> &'static str {
        P::NAME
    }
    assert_eq!(require_query::<GetIssue>(), "issues.v1.Issues/Get");
    assert_eq!(require_action::<RenameIssue>(), "issues.v1.Issues/Rename");
}

#[test]
fn protocol_error_json_roundtrips_all_statuses_and_resists_adversarial_json() {
    // Exhaustive roundtrip across EVERY error RpcStatus (explicitly catching 400 vs 415 and 413 vs 429).
    for &(status, http_code, _) in STATUS_TABLE {
        if status == RpcStatus::Ok {
            continue;
        }
        let msg = format!("msg for {http_code}");
        let encoded = encode_protocol_error_json(status, &msg);
        let (decoded_status, decoded_msg) = decode_protocol_error_json(http_code, &encoded);
        assert_eq!(
            decoded_status, status,
            "HTTP {http_code} must roundtrip losslessly"
        );
        assert_eq!(decoded_msg, msg);
    }

    // `"code":"ok"` can NEVER decode as `RpcStatus::Ok` in error channels.
    assert_eq!(
        decode_protocol_error_json(500, br#"{"code":"ok","message":"oops"}"#),
        (RpcStatus::Internal, "oops".to_owned())
    );
    assert_eq!(
        decode_protocol_error_json(200, br#"{"code":"ok","message":"buggy"}"#),
        (RpcStatus::Internal, "buggy".to_owned())
    );

    // Escaped characters + control chars.
    let msg = "bad \"input\"\nline\\2\t\x07end";
    let encoded = encode_protocol_error_json(RpcStatus::PermissionDenied, msg);
    let (status, decoded_msg) = decode_protocol_error_json(403, &encoded);
    assert_eq!(status, RpcStatus::PermissionDenied);
    assert_eq!(decoded_msg, msg);

    // Adversarial: reversed key order with escaped `"code":"internal"` inside `"message"`,
    // nested `"details":[{"message":"inner"}]`, and UTF-16 surrogate pair `\uD83D\uDE00` (😀).
    let tricky = br#"{"details":[{"code":"internal","message":"nested"}],"message":"bad \"code\": \"internal\" \uD83D\uDE00","code":"invalid_argument"}"#;
    let (tricky_status, tricky_msg) = decode_protocol_error_json(415, tricky);
    assert_eq!(tricky_status, RpcStatus::UnsupportedMediaType);
    assert_eq!(tricky_msg, "bad \"code\": \"internal\" 😀");

    // Lone high surrogate followed by normal `\u0041` (`A`) or a real pair `\uD83D\uDE00` (`😀`)
    // MUST NOT swallow the subsequent escape.
    let (_, lone_then_a) = decode_protocol_error_json(400, br#"{"message":"\uD800\u0041B"}"#);
    assert_eq!(lone_then_a, "\u{FFFD}AB");
    let (_, lone_then_emoji) =
        decode_protocol_error_json(400, br#"{"message":"\uD800\uD83D\uDE00"}"#);
    assert_eq!(lone_then_emoji, "\u{FFFD}😀");

    // Malformed escape inside a string value MUST abort parsing immediately rather than
    // resuming at an embedded comma inside the literal.
    let (_, spoofed) = decode_protocol_error_json(500, br#"{"code":"\x,"message":"spoofed"}"#);
    assert_ne!(spoofed, "spoofed");

    // Fallback when body is empty or non-JSON.
    let (fallback_status, _) = decode_protocol_error_json(413, b"");
    assert_eq!(fallback_status, RpcStatus::PayloadTooLarge);
}

#[test]
fn tag_domain_separation_sink_overflow_and_header_wildcard_work() {
    // Strict domain separation: NUL-crafted strings cannot collide across `of` / `keyed` / `keyed_str`.
    let nul_one = "\x01\0\0\0\0\0\0\0";
    let t_of = Tag64::of("doc\0\x01\0\0\0\0\0\0\0");
    let t_keyed = Tag64::keyed("doc", 1);
    let t_keyed_str = Tag64::keyed_str("doc", nul_one);
    assert_ne!(t_of, t_keyed);
    assert_ne!(t_keyed, t_keyed_str);
    assert_ne!(t_of, t_keyed_str);

    // TagSink overflow past 8 inline slots + deduplication, plus O(1) latch past 128 tags.
    let mut sink = TagSink::new();
    for i in 0..20u64 {
        sink.push(Tag64::keyed("item", i));
        sink.push(Tag64::keyed("item", i)); // duplicate ignored
    }
    assert_eq!(sink.len(), 20);
    assert!(!sink.is_overflow_all());

    let mut huge_sink = TagSink::new();
    for i in 0..10_000u64 {
        huge_sink.push(Tag64::keyed("bulk", i));
    }
    assert!(huge_sink.is_overflow_all());
    assert_eq!(
        format_invalidate_sink(&huge_sink).as_deref(),
        Some(INVALIDATE_ALL_WILDCARD)
    );

    // Standard formatting & uppercase/whitespace parsing.
    let t1 = Tag64::of("issues.list");
    let t2 = Tag64::keyed("issue", 42);
    let t3 = Tag64::keyed_str("doc", "getting-started");
    let header = format_invalidate_header(&[t1, t2, t3]).unwrap();
    let mut parsed = Vec::new();
    assert_eq!(
        parse_invalidate_header(&header.to_ascii_uppercase(), &mut parsed),
        Ok(InvalidateParse::Tags)
    );
    assert_eq!(parsed, vec![t1, t2, t3]);

    // > MAX_INVALIDATE_HEADER_TAGS collapses cleanly to `"*"` on server and `InvalidateParse::All` on client.
    let many: Vec<Tag64> = (0..(MAX_INVALIDATE_HEADER_TAGS as u64 + 10))
        .map(|i| Tag64::keyed("bulk", i))
        .collect();
    let bulk_hdr = format_invalidate_header(&many).unwrap();
    assert_eq!(bulk_hdr, INVALIDATE_ALL_WILDCARD);
    parsed.clear();
    assert_eq!(
        parse_invalidate_header(&bulk_hdr, &mut parsed),
        Ok(InvalidateParse::All)
    );
    assert!(parsed.is_empty());

    // Exact 128 unique tags + 1 duplicate still parses as Tags (does not falsely overflow).
    let mut exact_128 = String::new();
    for i in 0..MAX_INVALIDATE_HEADER_TAGS as u64 {
        if i > 0 {
            exact_128.push(',');
        }
        exact_128.push_str(&format!("{:016x}", Tag64::keyed("k", i).raw()));
    }
    exact_128.push(',');
    exact_128.push_str(&format!("{:016x}", Tag64::keyed("k", 0).raw()));
    parsed.clear();
    assert_eq!(
        parse_invalidate_header(&exact_128, &mut parsed),
        Ok(InvalidateParse::Tags)
    );
    assert_eq!(parsed.len(), MAX_INVALIDATE_HEADER_TAGS);

    // 129 unique raw tags collapse safely to InvalidateParse::All instead of silently truncating.
    exact_128.push(',');
    exact_128.push_str(&format!(
        "{:016x}",
        Tag64::keyed("k", MAX_INVALIDATE_HEADER_TAGS as u64).raw()
    ));
    parsed.clear();
    assert_eq!(
        parse_invalidate_header(&exact_128, &mut parsed),
        Ok(InvalidateParse::All)
    );
    assert!(parsed.is_empty());

    // Comma flood exceeding MAX_INVALIDATE_HEADER_SEGMENTS rejected.
    let commas = ",".repeat(MAX_INVALIDATE_HEADER_SEGMENTS + 5);
    parsed.clear();
    assert!(parse_invalidate_header(&commas, &mut parsed).is_err());

    // Invalid hex rejected.
    parsed.clear();
    assert!(parse_invalidate_header("not-hex", &mut parsed).is_err());
}

#[test]
fn call_with_codec_rejects_buggy_200_domain_error_as_domain_err() {
    struct Buggy200DomainTransport;

    impl Transport for Buggy200DomainTransport {
        fn call_raw<'a>(
            &'a self,
            _req: RawCall<'a>,
        ) -> BoxFuture<'a, Result<RawReply, TransportError>> {
            Box::pin(async {
                let mut err_bytes = Vec::new();
                CborCodec::default()
                    .encode_into(&DemoError::NotFound, &mut err_bytes)
                    .unwrap();
                Ok(RawReply {
                    http_status: 200,
                    is_domain_error: true,
                    is_protocol_error: false,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(err_bytes),
                })
            })
        }
    }

    let codec = CborCodec::default();
    let arg = 1u64;
    let mut fut = std::pin::pin!(call_with_codec::<GetIssue, CborCodec>(
        &Buggy200DomainTransport,
        &codec,
        &arg
    ));
    let waker = core::task::Waker::noop();
    let mut cx = core::task::Context::from_waker(waker);
    let res = match fut.as_mut().poll(&mut cx) {
        core::task::Poll::Ready(r) => r,
        core::task::Poll::Pending => panic!("in-memory future must complete immediately"),
    };
    assert_eq!(
        res,
        Err(RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: DemoError::NotFound,
        })
    );
}
