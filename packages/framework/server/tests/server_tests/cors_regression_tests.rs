//! Targeted CORS/CSRF regression tests promoted from 4-lens dual review of `nve-server`.

use bytes::Bytes;
use http::header::{ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_REQUEST_METHOD, ORIGIN};
use http::{Method, Request, StatusCode};
use nve_server::{CorsPolicy, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig};

use crate::{block_on, DemoAuth, Ping};

#[test]
fn options_preflight_checks_cors_origin_before_procedure_lookup() {
    let mut reg = RpcRegistry::<(), String>::builder();
    reg.register::<Ping, _, _>(|_cx: Cx<(), String>, n: u64| async move { Ok(n + 1) });
    let server = Server::new(
        (),
        reg.build(),
        DemoAuth,
        CsrfPolicy::allow_origins(["https://trusted.example.com"]),
        ServerConfig::default()
            .with_cors(CorsPolicy::allow_origins(["https://trusted.example.com"])),
    );

    // Untrusted origin probing an unknown procedure MUST get 403 Forbidden (not 404), closing
    // the procedure-existence oracle for cross-origin callers.
    let untrusted_unknown = Request::builder()
        .method(Method::OPTIONS)
        .uri("/rpc/secret.v1.Admin/DoesNotExist")
        .header(ORIGIN, "https://evil.example.com")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Bytes::new())
        .unwrap();
    let resp = block_on(server.handle_request(untrusted_unknown));
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    assert!(!resp.headers().contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));

    // Trusted origin probing an unknown procedure still gets 404 Not Found.
    let trusted_unknown = Request::builder()
        .method(Method::OPTIONS)
        .uri("/rpc/secret.v1.Admin/DoesNotExist")
        .header(ORIGIN, "https://trusted.example.com")
        .header(ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(Bytes::new())
        .unwrap();
    let resp_trusted = block_on(server.handle_request(trusted_unknown));
    assert_eq!(resp_trusted.status(), StatusCode::NOT_FOUND);
    assert!(!resp_trusted
        .headers()
        .contains_key(ACCESS_CONTROL_ALLOW_ORIGIN));
}

#[test]
fn csrf_and_cors_reject_malformed_ports_in_origins() {
    assert!(CsrfPolicy::try_allow_origins(["https://example.com:99999"]).is_err());
    assert!(CsrfPolicy::try_allow_origins(["https://example.com:abc"]).is_err());
    assert!(CorsPolicy::try_allow_origins(["https://example.com:99999"]).is_err());
    assert!(CorsPolicy::try_allow_origins(["https://example.com:abc"]).is_err());
}
