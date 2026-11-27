//! Server RPC dispatch, status/protocol errors, ABA-proof `PreparedRequest`, `DirectTransport`
//! trusted-SSR vs strict-HTTP enforcement, and progressive 0-JS form action negotiation tests.

use bytes::Bytes;
use http::header::{
    ACCEPT, ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_ORIGIN,
    ACCESS_CONTROL_EXPOSE_HEADERS, ACCESS_CONTROL_REQUEST_METHOD, AUTHORIZATION, CONTENT_TYPE,
    COOKIE, LOCATION, ORIGIN, VARY,
};
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use nve_rpc::{
    call, CborCodec, CborLimits, Procedure, ReplyKind, RpcCodec, RpcError, RpcStatus, Tag64,
    CONTENT_TYPE_CBOR, CONTENT_TYPE_FORM_URLENCODED, ERROR_KIND_FORM_VALIDATION, HEADER_ERROR_KIND,
    HEADER_INVALIDATE, MAX_INVALIDATE_HEADER_TAGS,
};
use nve_server::{
    parse_rpc_http_reply, protocol_error_response, AnonymousAuth, CorsPolicy, CsrfPolicy, Cx,
    DirectTransport, FormOutcome, FormRoute, RpcRegistry, RpcRegistryBuilder, Server, ServerConfig,
};

use crate::{block_on, build_test_server, BulkTouch, DemoAuth, DemoErr, Ping, Rename, RenameReq};

#[test]
fn server_handles_404_405_413_415_domain_ok_clamping_and_wildcard_invalidation() {
    let server = build_test_server();

    // 1. GET on RPC -> 405 Method Not Allowed; GET on non-RPC path -> 404 Not Found (RFC 9110).
    let get_req = Request::builder()
        .method(Method::GET)
        .uri("/rpc/demo.v1.Health/Ping")
        .body(Bytes::new())
        .unwrap();
    assert_eq!(
        block_on(server.handle_rpc_request(get_req)).status(),
        StatusCode::METHOD_NOT_ALLOWED
    );

    let get_favicon = Request::builder()
        .method(Method::GET)
        .uri("/favicon.ico")
        .body(Bytes::new())
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(get_favicon)).status(),
        StatusCode::NOT_FOUND
    );

    // `protocol_error_response(RpcStatus::Ok, ...)` MUST clamp to 500 Internal.
    assert_eq!(
        protocol_error_response(RpcStatus::Ok, "buggy resolver").status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );

    // Malformed `Nve-Invalidate` on HTTP 200 collapses to `invalidate_all = true` instead of
    // returning `TransportError` (so committed actions are never retried).
    let mut bad_inv_headers = HeaderMap::new();
    bad_inv_headers.insert(HEADER_INVALIDATE, HeaderValue::from_static("deadbeef"));
    let safe_reply =
        parse_rpc_http_reply(StatusCode::OK, &bad_inv_headers, Bytes::from_static(&[0x01]))
            .unwrap();
    assert!(safe_reply.invalidate_all);
    assert!(safe_reply.invalidated_tags.is_empty());

    // 2. Unknown prefix & unknown procedure -> 404.
    let bad_route = Request::builder()
        .method(Method::POST)
        .uri("/other/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(AUTHORIZATION, "Bearer native-token")
        .body(Bytes::new())
        .unwrap();
    assert_eq!(
        block_on(server.handle_rpc_request(bad_route)).status(),
        StatusCode::NOT_FOUND
    );

    let unknown_proc = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Missing")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(AUTHORIZATION, "Bearer native-token")
        .body(Bytes::new())
        .unwrap();
    assert_eq!(
        block_on(server.handle_rpc_request(unknown_proc)).status(),
        StatusCode::NOT_FOUND
    );

    // 3. Oversized body -> 413 Payload Too Large.
    let huge_req = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(AUTHORIZATION, "Bearer native-token")
        .body(Bytes::from(vec![0u8; 600]))
        .unwrap();
    assert_eq!(
        block_on(server.handle_rpc_request(huge_req)).status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // 4. Codec MaxBytesExceeded also maps to 413 Payload Too Large (not 400).
    let tight_codec = CborCodec::new(CborLimits {
        max_bytes: 2,
        ..CborLimits::DEFAULT
    });
    let mut tight_reg = RpcRegistryBuilder::<(), (), CborCodec>::with_codec(tight_codec);
    tight_reg.register::<Ping, _, _>(|_cx: Cx<(), ()>, n: u64| async move { Ok(n) });
    let tight_server = Server::new(
        (),
        tight_reg.build(),
        AnonymousAuth,
        CsrfPolicy::bearer_only(),
        ServerConfig::default(),
    );
    let mut big_u64 = Vec::new();
    CborCodec::default()
        .encode_into(&u64::MAX, &mut big_u64)
        .unwrap();
    assert!(big_u64.len() > 2);
    let codec_413 = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .body(Bytes::from(big_u64))
        .unwrap();
    assert_eq!(
        block_on(tight_server.handle_rpc_request(codec_413)).status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // 5. Buggy DomainStatus returning RpcStatus::Ok is clamped to 422 on server.
    let transport = DirectTransport::new(server.clone())
        .with_bearer_token("native-token")
        .unwrap();
    let clamped = block_on(call::<Ping>(&transport, &999)).unwrap_err();
    assert_eq!(
        clamped,
        RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: DemoErr::BadStatusOk,
        }
    );

    // 6. Action invalidating > 128 tags (via `invalidate_many`) AND explicit `cx.invalidate_all()`
    //    both emit `Nve-Invalidate: *` -> `invalidate_all: true`.
    let bulk_count = (MAX_INVALIDATE_HEADER_TAGS as u64) + 20;
    let bulk_transport = DirectTransport::strict_http(server.clone())
        .with_bearer_token("native-token")
        .unwrap();
    let bulk_reply = block_on(call::<BulkTouch>(&bulk_transport, &bulk_count)).unwrap();
    assert!(bulk_reply.invalidate_all);
    assert!(bulk_reply.invalidated_tags.is_empty());

    let explicit_all = block_on(call::<BulkTouch>(&bulk_transport, &0)).unwrap();
    assert!(explicit_all.invalidate_all);
    assert!(explicit_all.invalidated_tags.is_empty());

    // 7. Cross-Server `PreparedRequest` MUST be rejected safely (500, no OOB panic/misroute)
    //    even across drop-and-recreate cycles that trigger allocator LIFO heap address ABA reuse!
    for _ in 0..64 {
        let ephemeral_big = build_test_server();
        let (mut parts, _) = Request::builder()
            .method(Method::POST)
            .uri("/rpc/demo.v1.Issues/BulkTouch")
            .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
            .header(AUTHORIZATION, "Bearer native-token")
            .body(Bytes::new())
            .unwrap()
            .into_parts();
        let prepared = block_on(ephemeral_big.preflight_request(&mut parts, false)).unwrap();
        drop(ephemeral_big);

        let mut reg_small = RpcRegistry::<(), String>::builder();
        reg_small.register::<Ping, _, _>(|_cx: Cx<(), String>, n: u64| async move { Ok(n) });
        let small_server = Server::new(
            (),
            reg_small.build(),
            DemoAuth,
            CsrfPolicy::allow_origins(["https://app.example.com"]),
            ServerConfig::default(),
        );
        let cross_resp =
            block_on(small_server.execute_prepared(prepared, Bytes::from_static(&[0x00])));
        assert_eq!(cross_resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
    }
}

#[test]
fn direct_transport_supports_trusted_ssr_and_strict_http_modes() {
    let server = build_test_server();

    // SSR navigation arrives with user's Cookie + `Sec-Fetch-Site: cross-site`: trusted
    // DirectTransport succeeds on read-only Query without header spoofing, and blocks Action
    // unless `allow_trusted_actions(true)` is explicitly set!
    let ssr_transport = DirectTransport::new(server.clone())
        .with_header(COOKIE, HeaderValue::from_static("nve_session=web-cookie"))
        .with_header(
            http::header::HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
    let reply = block_on(call::<Ping>(&ssr_transport, &41)).unwrap();
    assert_eq!(reply.value, 42);

    let blocked_action = block_on(call::<Rename>(
        &ssr_transport,
        &RenameReq {
            id: 7,
            title: "Blocked".into(),
        },
    ))
    .unwrap_err();
    assert!(matches!(blocked_action, RpcError::Transport(_)));

    // Forged `RawCall` claiming `Effect::Query` for a registered Action MUST still be blocked
    // via the registry's authoritative `entry.effect`!
    let mut forged_body = Vec::new();
    CborCodec::default()
        .encode_into(
            &RenameReq {
                id: 7,
                title: "Forged".into(),
            },
            &mut forged_body,
        )
        .unwrap();
    let forged_err = block_on(nve_rpc::Transport::call_raw(
        &ssr_transport,
        nve_rpc::RawCall {
            proc_id: Rename::ID,
            name: Rename::NAME,
            effect: nve_rpc::Effect::Query,
            content_type: CONTENT_TYPE_CBOR,
            body: Bytes::from(forged_body),
        },
    ))
    .unwrap_err();
    assert!(forged_err.message.contains("blocks Action"));

    // Mismatched `content_type` on trusted DirectTransport MUST return 415 Unsupported Media Type
    // (parity with StrictHttp and HyperTransport).
    let wrong_ct_reply = block_on(nve_rpc::Transport::call_raw(
        &ssr_transport,
        nve_rpc::RawCall {
            proc_id: Ping::ID,
            name: Ping::NAME,
            effect: nve_rpc::Effect::Query,
            content_type: "application/json",
            body: Bytes::from_static(b"1"),
        },
    ))
    .unwrap();
    assert_eq!(wrong_ct_reply.http_status, StatusCode::UNSUPPORTED_MEDIA_TYPE.as_u16());
    assert_eq!(wrong_ct_reply.kind, ReplyKind::Protocol);

    // Pre-authenticated trusted transport with explicit `allow_trusted_actions(true)`.
    let principal_transport =
        DirectTransport::with_principal(server.clone(), "ssr-preauthed".to_owned())
            .allow_trusted_actions(true);
    let renamed = block_on(call::<Rename>(
        &principal_transport,
        &RenameReq {
            id: 7,
            title: "SSR".into(),
        },
    ))
    .unwrap();
    assert_eq!(renamed.value.actor, "ssr-preauthed");
    assert_eq!(
        renamed.invalidated_tags,
        vec![Tag64::of("issues.list"), Tag64::keyed("issue", 7)]
    );

    // Strict HTTP mode enforces full CSRF rules and rejects the same cross-site cookie call with 403.
    let strict_transport = DirectTransport::strict_http(server.clone())
        .with_header(COOKIE, HeaderValue::from_static("nve_session=web-cookie"))
        .with_header(
            http::header::HeaderName::from_static("sec-fetch-site"),
            HeaderValue::from_static("cross-site"),
        );
    let err = block_on(call::<Ping>(&strict_transport, &41)).unwrap_err();
    assert!(matches!(
        err,
        RpcError::Protocol {
            status: RpcStatus::PermissionDenied,
            ..
        }
    ));
}

#[test]
fn form_action_negotiates_0js_browser_text_vs_cbor_and_enforces_limits() {
    let server = build_test_server();

    // 1. Unified `handle_request` dispatches built-in `FormRoute` (`/forms/rename` -> 303).
    let ok_req = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header("sec-fetch-site", "same-origin")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(b"id=7&title=Ship+Slice+1a"))
        .unwrap();
    let ok_resp = block_on(server.handle_request(ok_req));
    assert_eq!(ok_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(ok_resp.headers().get(LOCATION).unwrap(), "/issues/7");
    assert!(!ok_resp.headers().contains_key(HEADER_INVALIDATE));

    // 2. 0-JS browser validation failure returns `422 text/plain` + `Nve-Error-Kind: form-validation`
    //    regardless of `Accept` (hydrated WASM & Native use local `FormDecode` + CBOR RPC).
    let browser_bad = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(ACCEPT, "text/html,application/xhtml+xml")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(b"id=nope&title="))
        .unwrap();
    let browser_resp = block_on(server.handle_request(browser_bad));
    assert_eq!(browser_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        browser_resp.headers().get(CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );
    assert_eq!(
        browser_resp.headers().get(HEADER_ERROR_KIND).unwrap(),
        ERROR_KIND_FORM_VALIDATION
    );
    assert!(core::str::from_utf8(browser_resp.body())
        .unwrap()
        .contains("title"));

    // 4. `execute_form_action` takes a sealed `&FormRoute` and preserves submitted `FormPairs`
    //    for SSR HTML re-rendering.
    let rename_route = FormRoute::new::<Rename>("/forms/rename", "/issues/7");
    let ssr_bad = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(b"id=7&title=+++"))
        .unwrap();
    match block_on(server.execute_form_action(&rename_route, ssr_bad)) {
        FormOutcome::ValidationFailed { errors, form } => {
            assert_eq!(errors.len(), 1);
            assert_eq!(form.get("id"), Some("7"));
        }
        other => panic!("expected ValidationFailed with preserved form, got {other:?}"),
    }

    // 5. Form edge errors: GET -> 405, wrong Content-Type -> 415, non-UTF8 -> 400, > max_form_fields -> 413,
    //    action registered without `register_with_form` -> 415.
    let form_get = Request::builder()
        .method(Method::GET)
        .uri("/forms/rename")
        .body(Bytes::new())
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(form_get)).status(),
        StatusCode::METHOD_NOT_ALLOWED
    );

    let form_wrong_ct = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, "text/plain")
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(b"id=7&title=Smuggled"))
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(form_wrong_ct)).status(),
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );

    let form_non_utf8 = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(&[0xff, 0xfe]))
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(form_non_utf8)).status(),
        StatusCode::BAD_REQUEST
    );

    let too_many_fields = "a=1&".repeat(20);
    let form_flood = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from(too_many_fields))
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(form_flood)).status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );

    // Calling `execute_form_action` for an Action registered without `register_with_form` -> 415.
    let bulk_route = FormRoute::new::<BulkTouch>("/forms/bulk", "/ok");
    let non_form_req = Request::builder()
        .method(Method::POST)
        .uri("/forms/bulk")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "https://app.example.com")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::new())
        .unwrap();
    match block_on(server.execute_form_action(&bulk_route, non_form_req)) {
        FormOutcome::ProtocolError(resp) => {
            assert_eq!(resp.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
        }
        other => panic!("expected ProtocolError(415) for non-form action, got {other:?}"),
    }
}

#[test]
fn opt_in_cors_policy_handles_preflight_rpc_errors_and_blocks_cross_site_forms() {
    let mut reg = RpcRegistry::<(), String>::builder();
    reg.register::<Ping, _, _>(|_cx: Cx<(), String>, n: u64| async move { Ok(n + 1) });
    reg.register_with_form::<Rename, _, _>(|cx: Cx<(), String>, req: crate::RenameReq| async move {
        Ok(crate::RenameRes {
            id: req.id,
            title: req.title,
            actor: cx.principal,
        })
    });
    let server = Server::new(
        (),
        reg.build(),
        DemoAuth,
        CsrfPolicy::allow_origins(["https://api.example.com"]),
        ServerConfig::default()
            .with_form_route::<Rename>("/forms/rename", "/issues/7")
            .with_cors(CorsPolicy::allow_origins(["http://localhost:8080"])),
    );

    // 1. Allowed OPTIONS preflight -> 204 with exact Origin + Expose-Headers + Vary.
    let preflight_ok = Request::builder()
        .method(Method::OPTIONS)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(ORIGIN, "http://localhost:8080")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Bytes::new())
        .unwrap();
    let resp = block_on(server.handle_request(preflight_ok));
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:8080"
    );
    assert_eq!(
        resp.headers().get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap(),
        "nve-error-kind, nve-invalidate"
    );
    assert_eq!(
        resp.headers().get(ACCESS_CONTROL_ALLOW_CREDENTIALS).unwrap(),
        "true"
    );
    assert!(resp.headers().get(VARY).is_some());

    // 2. Disallowed OPTIONS preflight -> 403 with NO Access-Control-Allow-Origin.
    let preflight_bad = Request::builder()
        .method(Method::OPTIONS)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(ORIGIN, "https://evil.example.com")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Bytes::new())
        .unwrap();
    let bad_resp = block_on(server.handle_request(preflight_bad));
    assert_eq!(bad_resp.status(), StatusCode::FORBIDDEN);
    assert!(!bad_resp.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));

    // 3. Cross-origin preflighted CBOR RPC (`Sec-Fetch-Site: cross-site`) succeeds AND exposes headers.
    let mut body = Vec::new();
    CborCodec::default().encode_into(&41u64, &mut body).unwrap();
    let rpc_ok = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(ORIGIN, "http://localhost:8080")
        .header("sec-fetch-site", "cross-site")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from(body))
        .unwrap();
    let rpc_resp = block_on(server.handle_request(rpc_ok));
    assert_eq!(rpc_resp.status(), StatusCode::OK);
    assert_eq!(
        rpc_resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:8080"
    );
    assert_eq!(
        rpc_resp.headers().get(ACCESS_CONTROL_EXPOSE_HEADERS).unwrap(),
        "nve-error-kind, nve-invalidate"
    );

    // 4. Auth failure (401) from allowed CORS origin STILL carries ACAO + Expose-Headers so
    //    browsers don't turn 401 into an opaque TypeError.
    let rpc_unauth = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(ORIGIN, "http://localhost:8080")
        .header("sec-fetch-site", "cross-site")
        .body(Bytes::from_static(&[0x01]))
        .unwrap();
    let unauth_resp = block_on(server.handle_request(rpc_unauth));
    assert_eq!(unauth_resp.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        unauth_resp.headers().get(ACCESS_CONTROL_ALLOW_ORIGIN).unwrap(),
        "http://localhost:8080"
    );

    // 5. Cross-site HTML `<form>` POST (CORS simple request, no preflight) MUST STILL BE BLOCKED
    //    even for origins allowed in `CorsPolicy`!
    let cross_form = Request::builder()
        .method(Method::POST)
        .uri("/forms/rename")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(ORIGIN, "http://localhost:8080")
        .header("sec-fetch-site", "cross-site")
        .header(COOKIE, "nve_session=web-cookie")
        .body(Bytes::from_static(b"id=7&title=CSRF"))
        .unwrap();
    assert_eq!(
        block_on(server.handle_request(cross_form)).status(),
        StatusCode::FORBIDDEN
    );
}
