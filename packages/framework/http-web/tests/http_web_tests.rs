//! Native contract tests for `FetchConfig`, URL formatting, bearer token validation,
//! `CredentialsMode`, and `BrowserClock` monotonicity.

use nve_http_web::{BrowserClock, CredentialsMode, FetchConfig, FetchTransport};
use nve_query::Clock;
use nve_rpc::{call, procedures, RpcError, TransportErrorKind};

procedures! {
    query Ping = "demo.v1.Health/Ping" (u64) -> u64, ();
}

#[test]
fn fetch_config_urls_credentials_and_bearer_validation_work() {
    let same = FetchConfig::same_origin();
    assert_eq!(same.credentials, CredentialsMode::SameOrigin);
    assert_eq!(same.credentials.as_js_str(), "same-origin");
    assert_eq!(same.build_url("issues.v1.Issues/List"), "/rpc/issues.v1.Issues/List");

    let cross = FetchConfig::with_base_url("http://localhost:3000/")
        .with_rpc_prefix("/api/rpc/")
        .with_credentials(CredentialsMode::Include)
        .with_timeout_ms(5_000)
        .with_max_response_bytes(1024);
    assert_eq!(
        cross.build_url("issues.v1.Issues/List"),
        "http://localhost:3000/api/rpc/issues.v1.Issues/List"
    );
    assert_eq!(cross.credentials.as_js_str(), "include");

    let bearer = FetchConfig::same_origin()
        .with_bearer_token("tok_abc_123")
        .unwrap();
    assert_eq!(bearer.auth_header(), Some("Bearer tok_abc_123"));
    assert_eq!(bearer.credentials, CredentialsMode::Omit);

    assert!(FetchConfig::same_origin().with_bearer_token("").is_err());
    assert!(FetchConfig::same_origin()
        .with_bearer_token("bad\r\nheader")
        .is_err());

    let clock = BrowserClock::new();
    let t0 = clock.now_ms();
    let t1 = clock.now_ms();
    assert!(t1 >= t0);
}

#[test]
fn native_fetch_transport_returns_config_error_cleanly() {
    let transport = FetchTransport::same_origin();
    let mut fut = std::pin::pin!(call::<Ping>(&transport, &1));
    let waker = core::task::Waker::noop();
    let mut cx = core::task::Context::from_waker(waker);
    let res = match core::future::Future::poll(fut.as_mut(), &mut cx) {
        core::task::Poll::Ready(r) => r,
        core::task::Poll::Pending => panic!("native FetchTransport must return immediately"),
    };
    match res {
        Err(RpcError::Transport(err)) => assert_eq!(err.kind, TransportErrorKind::Config),
        other => panic!("expected TransportErrorKind::Config on native, got {other:?}"),
    }
}
