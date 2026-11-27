//! CSRF policy matrix, credential extraction fail-closed rules, Bearer Cookie stripping,
//! redirect/cookie validators, duplicate-registration panic, and zero-alloc `CountingAlloc` tests.

use bytes::Bytes;
use http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE, ORIGIN};
use http::{HeaderMap, HeaderValue, Method, Request, StatusCode};
use nve_rpc::{CborCodec, RpcCodec, RpcStatus, CONTENT_TYPE_CBOR};
use nve_server::{
    build_session_cookie, extract_cookie_value, extract_credential, validate_same_origin_redirect,
    AnonymousAuth, AuthCredential, AuthResolver, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig,
};

use crate::{block_on, count_allocs, CookieFirstBuggyAuth, Ping, Rename, RenameReq, RenameRes};

#[test]
fn csrf_policy_matrix_fails_closed_including_duplicate_headers_and_normalization() {
    // Rejects malformed configured origins at startup (including non-ASCII byte-boundary strings).
    assert!(CsrfPolicy::try_allow_origins(["app.example.com"]).is_err());
    assert!(CsrfPolicy::try_allow_origins(["https://app.example.com/path"]).is_err());
    assert!(CsrfPolicy::try_allow_origins(["https://user@app.example.com"]).is_err());
    assert!(CsrfPolicy::try_allow_origins(["aaaaaaaé"]).is_err());
    assert!(CsrfPolicy::try_allow_origins(Vec::<String>::new()).is_err());

    let policy = CsrfPolicy::allow_origins(["https://APP.example.com:443/"]);
    assert_eq!(policy.allowed_origins(), &["https://app.example.com"]);

    let check = |headers: HeaderMap, ct: Option<&str>| {
        let mut h = headers;
        if let Some(c) = ct {
            h.append(CONTENT_TYPE, HeaderValue::from_str(c).unwrap());
        }
        policy.check_content_type_and_origin(&Method::POST, &h, CONTENT_TYPE_CBOR)
    };

    // 1. Same-origin + allowed Origin (with default :443 + mixed case) + Content-Type parameter -> OK.
    let mut h = HeaderMap::new();
    h.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    h.insert(ORIGIN, HeaderValue::from_static("https://App.Example.com:443"));
    h.insert(COOKIE, HeaderValue::from_static("nve_session=web-cookie"));
    assert!(check(h, Some("application/cbor; charset=utf-8")).is_ok());

    // 2. Sec-Fetch-Site: none -> OK; cross-site / same-site / unknown -> 403.
    let mut h = HeaderMap::new();
    h.insert("sec-fetch-site", HeaderValue::from_static("none"));
    assert!(check(h, Some(CONTENT_TYPE_CBOR)).is_ok());

    for bad_site in ["cross-site", "same-site", "unknown-value"] {
        let mut h = HeaderMap::new();
        h.insert("sec-fetch-site", HeaderValue::from_static(bad_site));
        h.insert(ORIGIN, HeaderValue::from_static("https://app.example.com"));
        assert_eq!(
            check(h, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
            RpcStatus::PermissionDenied
        );
    }

    // 3. Duplicate Origin / Sec-Fetch-Site / Content-Type / Authorization -> fail closed!
    let mut dup_origin = HeaderMap::new();
    dup_origin.append(ORIGIN, HeaderValue::from_static("https://app.example.com"));
    dup_origin.append(ORIGIN, HeaderValue::from_static("https://evil.example.com"));
    assert_eq!(
        check(dup_origin, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
        RpcStatus::PermissionDenied
    );

    let mut dup_sfs = HeaderMap::new();
    dup_sfs.append("sec-fetch-site", HeaderValue::from_static("same-origin"));
    dup_sfs.append("sec-fetch-site", HeaderValue::from_static("cross-site"));
    assert_eq!(
        check(dup_sfs, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
        RpcStatus::PermissionDenied
    );

    let mut dup_ct = HeaderMap::new();
    dup_ct.insert(ORIGIN, HeaderValue::from_static("https://app.example.com"));
    dup_ct.append(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_CBOR));
    assert_eq!(
        check(dup_ct, Some("text/plain")).unwrap_err().status,
        RpcStatus::UnsupportedMediaType
    );

    let mut dup_auth = HeaderMap::new();
    dup_auth.append(AUTHORIZATION, HeaderValue::from_static("Bearer tok1"));
    dup_auth.append(AUTHORIZATION, HeaderValue::from_static("Bearer tok2"));
    assert_eq!(
        check(dup_auth, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
        RpcStatus::PermissionDenied
    );

    // 4. Empty Bearer / Basic auth do NOT bypass Origin checks, AND `extract_credential` MUST
    //    NOT fall back to ambient `Cookie` whenever any `Authorization` header is present.
    for non_bearer in ["Bearer ", "Basic dXNlcjpwYXNz"] {
        let mut h = HeaderMap::new();
        h.insert(AUTHORIZATION, HeaderValue::from_static(non_bearer));
        h.insert(COOKIE, HeaderValue::from_static("nve_session=web-cookie"));
        assert_eq!(
            extract_credential(&h, "nve_session"),
            AuthCredential::None,
            "extract_credential must fail closed on `{non_bearer}` instead of falling back to Cookie"
        );
        assert_eq!(
            check(h, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
            RpcStatus::PermissionDenied
        );
    }

    // 5. Origin: null, mismatched Origin, Cookie without Origin/Sec-Fetch-Site, text/plain -> reject.
    let mut h = HeaderMap::new();
    h.insert(ORIGIN, HeaderValue::from_static("null"));
    assert_eq!(
        check(h, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
        RpcStatus::PermissionDenied
    );

    let mut h = HeaderMap::new();
    h.insert(COOKIE, HeaderValue::from_static("nve_session=web-cookie"));
    assert_eq!(
        check(h, Some(CONTENT_TYPE_CBOR)).unwrap_err().status,
        RpcStatus::PermissionDenied
    );

    let mut h = HeaderMap::new();
    h.insert(ORIGIN, HeaderValue::from_static("https://app.example.com"));
    assert_eq!(
        check(h, Some("text/plain")).unwrap_err().status,
        RpcStatus::UnsupportedMediaType
    );

    // 6. GET skips origin check in CsrfPolicy; bearer_only rejects browser Origin and Cookie.
    let mut get_h = HeaderMap::new();
    get_h.insert(ORIGIN, HeaderValue::from_static("https://evil.example.com"));
    assert!(policy.check_origin(&Method::GET, &get_h).is_ok());

    let bearer_only = CsrfPolicy::bearer_only();
    let mut bo_h = HeaderMap::new();
    bo_h.insert(ORIGIN, HeaderValue::from_static("https://app.example.com"));
    assert_eq!(
        bearer_only.check_origin(&Method::POST, &bo_h).unwrap_err().status,
        RpcStatus::PermissionDenied
    );
    let mut bo_cookie = HeaderMap::new();
    bo_cookie.insert(COOKIE, HeaderValue::from_static("nve_session=web-cookie"));
    bo_cookie.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));
    assert_eq!(
        bearer_only
            .check_origin(&Method::POST, &bo_cookie)
            .unwrap_err()
            .status,
        RpcStatus::PermissionDenied
    );

    // 7. Duplicate cookie name fails closed (cookie tossing defense), while unrelated UTF-8
    //    sibling cookies on the same domain MUST NOT break session cookie extraction.
    let mut tossed = HeaderMap::new();
    tossed.insert(
        COOKIE,
        HeaderValue::from_static("nve_session=evil; nve_session=real"),
    );
    assert_eq!(extract_cookie_value(&tossed, "nve_session"), None);

    let mut utf8_sibling = HeaderMap::new();
    utf8_sibling.insert(
        COOKIE,
        HeaderValue::from_bytes(b"theme=caf\xc3\xa9; nve_session=web-cookie").unwrap(),
    );
    assert_eq!(
        extract_cookie_value(&utf8_sibling, "nve_session"),
        Some("web-cookie")
    );
    assert_eq!(
        extract_credential(&utf8_sibling, "nve_session"),
        AuthCredential::Cookie("web-cookie")
    );

    // Split HTTP/2-style Cookie headers with a UTF-8 sibling in the second header.
    let mut split_utf8 = HeaderMap::new();
    split_utf8.append(COOKIE, HeaderValue::from_static("nve_session=web-cookie"));
    split_utf8.append(COOKIE, HeaderValue::from_bytes(b"pref=\xe2\x9c\x93").unwrap());
    assert_eq!(
        extract_cookie_value(&split_utf8, "nve_session"),
        Some("web-cookie")
    );

    // Invalid non-UTF-8 bytes in the TARGET cookie itself still fail closed.
    let mut bad_target = HeaderMap::new();
    bad_target.insert(COOKIE, HeaderValue::from_bytes(b"nve_session=\xff\xfe").unwrap());
    assert_eq!(extract_cookie_value(&bad_target, "nve_session"), None);
}

#[test]
fn bearer_csrf_bypass_physically_strips_cookie_before_auth_and_handler() {
    let mut reg = RpcRegistry::<(), String>::builder();
    reg.register::<Ping, _, _>(|cx: Cx<(), String>, n: u64| async move {
        assert!(!cx.headers.contains_key(COOKIE));
        Ok(n)
    });
    let server = Server::new(
        (),
        reg.build(),
        CookieFirstBuggyAuth,
        CsrfPolicy::allow_origins(["https://app.example.com"]),
        ServerConfig::default(),
    );

    let mut body = Vec::new();
    CborCodec::default().encode_into(&1u64, &mut body).unwrap();
    let req = Request::builder()
        .method(Method::POST)
        .uri("/rpc/demo.v1.Health/Ping")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(AUTHORIZATION, "Bearer dummy-bypass")
        .header(COOKIE, "nve_session=stolen")
        .body(Bytes::from(body))
        .unwrap();
    let resp = block_on(server.handle_rpc_request(req));
    // Even though CookieFirstBuggyAuth only checks Cookie, Server stripped Cookie on Bearer bypass!
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

#[test]
fn redirect_and_cookie_helpers_reject_non_ascii_and_spaces() {
    assert_eq!(validate_same_origin_redirect("/issues/42"), Ok("/issues/42"));
    assert!(validate_same_origin_redirect("//evil.com").is_err());
    assert!(validate_same_origin_redirect("/\\evil.com").is_err());
    assert!(validate_same_origin_redirect("https://evil.com").is_err());
    assert!(validate_same_origin_redirect("/ok\r\nSet-Cookie: bad=1").is_err());
    assert!(validate_same_origin_redirect("/issues/café").is_err());
    assert!(validate_same_origin_redirect("/issues/with space").is_err());

    let cookie = build_session_cookie("__Host-nve_session", "tok_123", true, Some(3600)).unwrap();
    assert_eq!(
        cookie,
        "__Host-nve_session=tok_123; Path=/; HttpOnly; SameSite=Lax; Secure; Max-Age=3600"
    );
    assert!(build_session_cookie("bad;name", "tok", true, None).is_err());
    assert!(build_session_cookie("ok", "bad;val", true, None).is_err());
    assert!(build_session_cookie("ok", "toké", true, None).is_err());
    assert!(build_session_cookie("ok", "tok space", true, None).is_err());
    assert!(build_session_cookie("ok", "", true, None).is_err());
}

#[test]
#[should_panic(expected = "duplicate ProcId")]
fn registry_panics_on_duplicate_procedure_registration() {
    let mut reg = RpcRegistry::<(), ()>::builder();
    reg.register::<Ping, _, _>(|_cx: Cx<(), ()>, n: u64| async move { Ok(n) });
    reg.register::<Ping, _, _>(|_cx: Cx<(), ()>, n: u64| async move { Ok(n) });
    let _ = reg.build();
}

#[test]
fn registry_lookup_csrf_and_anonymous_auth_perform_zero_allocations() {
    let mut reg = RpcRegistry::<(), ()>::builder();
    reg.register::<Ping, _, _>(|_cx: Cx<(), ()>, n: u64| async move { Ok(n) });
    reg.register::<Rename, _, _>(|_cx: Cx<(), ()>, r: RenameReq| async move {
        Ok(RenameRes {
            id: r.id,
            title: r.title,
            actor: String::new(),
        })
    });
    let registry = reg.build();

    let (found, lookup_allocs) = count_allocs(|| registry.lookup("demo.v1.Health/Ping").is_some());
    assert!(found);
    assert_eq!(lookup_allocs, 0, "RpcRegistry::lookup must allocate 0 times");

    let policy = CsrfPolicy::allow_origins(["https://app.example.com"]);
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE_CBOR));
    headers.insert(ORIGIN, HeaderValue::from_static("https://app.example.com"));
    headers.insert("sec-fetch-site", HeaderValue::from_static("same-origin"));

    let (csrf_res, csrf_allocs) = count_allocs(|| {
        policy.check_content_type_and_origin(&Method::POST, &headers, CONTENT_TYPE_CBOR)
    });
    csrf_res.unwrap();
    assert_eq!(
        csrf_allocs, 0,
        "CsrfPolicy::check_content_type_and_origin must allocate 0 times"
    );

    let (auth_res, auth_allocs) = count_allocs(|| AnonymousAuth.try_resolve_sync(&(), &headers));
    assert!(matches!(auth_res, Some(Ok(()))));
    assert_eq!(
        auth_allocs, 0,
        "AnonymousAuth::try_resolve_sync must allocate 0 times"
    );
}
