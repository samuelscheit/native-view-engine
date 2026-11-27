//! Streaming SSR `GET /issues` integration tests over real loopback HTTP/1.1:
//! - DSD shell + out-of-order `stats` / `comments` slots (both delay orders) + strict per-request
//!   nonce CSP with zero inline executable `<script>`
//! - pre-headers `404` for unknown `?issue=`, post-headers `<ErrorBoundary>` + `DomainErr` `NVEQ`
//!   chunk for unknown `?comments_issue=`
//! - external `/pkg/issues/boot.js` and opt-in `/_test/*` stream-control hooks

use core::time::Duration;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use bytes::Bytes;
use http::header::CONTENT_SECURITY_POLICY;
use http::{Method, Request, Response, StatusCode};
use http_body_util::{BodyExt as _, Empty};
use hyper::body::Incoming;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use nve_example_issues_api::{ApiError, IssueId, ListComments};
use nve_example_issues_server::auth::{DEMO_WEB_COOKIE_TOKEN, SESSION_COOKIE_NAME};
use nve_example_issues_server::store::IssueStore;
use nve_example_issues_server::{build_issues_server, IssuesHttpService};
use nve_http_hyper::serve_service_with_shutdown;
use nve_query::QueryOptions;
use nve_query_dioxus::erased_query_client;
use nve_rpc::{RpcError, RpcStatus};
use nve_server::DirectTransport;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

struct TestServer {
    origin: String,
    store: IssueStore,
    client: Client<hyper_util::client::legacy::connect::HttpConnector, Empty<Bytes>>,
    shutdown: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<()>>,
}

impl TestServer {
    async fn start(configure: impl FnOnce(IssuesHttpService) -> IssuesHttpService) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test listener on an ephemeral port");
        let origin = format!(
            "http://{}",
            listener.local_addr().expect("listener has a local address")
        );
        let store = IssueStore::seeded();
        let svc = configure(IssuesHttpService::new(build_issues_server(
            store.clone(),
            [origin.clone()],
        )));
        let (tx, rx) = oneshot::channel::<()>();
        let task = tokio::spawn(async move {
            serve_service_with_shutdown(listener, svc, async {
                let _ = rx.await;
            })
            .await
            .expect("issues test server runs until shutdown");
        });
        Self {
            origin,
            store,
            client: Client::builder(TokioExecutor::new()).build_http(),
            shutdown: Some(tx),
            task: Some(task),
        }
    }

    async fn request(&self, method: Method, path: &str, cookie: bool) -> Response<Incoming> {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.origin));
        if cookie {
            req = req.header(
                http::header::COOKIE,
                format!("{SESSION_COOKIE_NAME}={DEMO_WEB_COOKIE_TOKEN}"),
            );
        }
        self.client
            .request(req.body(Empty::new()).expect("test request is well-formed"))
            .await
            .expect("test request completes")
    }

    async fn page(&self, path: &str) -> (Response<()>, String) {
        let resp = self.request(Method::GET, path, true).await;
        let (parts, body) = resp.into_parts();
        let bytes = body.collect().await.expect("page body streams to completion").to_bytes();
        (
            Response::from_parts(parts, ()),
            String::from_utf8(bytes.to_vec()).expect("page body is valid UTF-8"),
        )
    }

    async fn stream_status(&self) -> String {
        let resp = self.request(Method::GET, "/_test/stream-status", false).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp
            .into_body()
            .collect()
            .await
            .expect("stream-status body completes")
            .to_bytes();
        String::from_utf8(bytes.to_vec()).expect("stream-status body is valid UTF-8")
    }

    async fn stop(mut self) {
        let _ = self
            .shutdown
            .take()
            .expect("shutdown sender present until stop")
            .send(());
        self.task
            .take()
            .expect("server task present until stop")
            .await
            .expect("server task joins without panic");
    }
}

fn header<T>(resp: &Response<T>, name: impl http::header::AsHeaderName) -> &str {
    resp.headers()
        .get(name)
        .expect("expected response header is present")
        .to_str()
        .expect("response header is visible ASCII")
}

fn index_of(html: &str, needle: &str) -> usize {
    let pos = html.find(needle);
    assert!(pos.is_some(), "missing {needle:?} in streamed HTML:\n{html}");
    pos.expect("presence asserted above")
}

/// Asserts the strict zero-inline-JS CSP and returns its per-request nonce.
fn assert_strict_csp(resp: &Response<()>, html: &str) -> String {
    let csp = header(resp, CONTENT_SECURITY_POLICY);
    let script_src = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("script-src "))
        .expect("CSP has a script-src directive");
    assert_eq!(script_src, "script-src 'self' 'wasm-unsafe-eval'");
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .expect("CSP carries a nonce")
        .to_owned();
    assert_eq!(nonce.len(), 24, "128-bit base64 nonce: {csp}");
    // Every `<style>` carries the nonce; every `<script>` is either external or inert data.
    let style_count = html.matches("<style").count();
    assert!(style_count >= 2);
    assert_eq!(
        html.matches(&format!("nonce=\"{nonce}\"")).count(),
        style_count,
        "every <style> must carry the CSP nonce"
    );
    for (pos, _) in html.match_indices("<script") {
        let tag = &html[pos..pos + html[pos..].find('>').expect("script tag is closed")];
        assert!(
            tag.contains("type=\"application/x-nve-query\"")
                || (tag.contains(" src=\"") && tag.contains("type=\"module\"")),
            "inline executable script found: {tag}"
        );
    }
    nonce
}

/// Decodes every `<script type="application/x-nve-query" data-nve-seq>` payload in order.
fn nveq_chunks(html: &str) -> Vec<(u64, Vec<u8>)> {
    const MARKER: &str = "<script type=\"application/x-nve-query\" data-nve-seq=\"";
    html.match_indices(MARKER)
        .map(|(pos, _)| {
            let rest = &html[pos + MARKER.len()..];
            let (seq, rest) = rest.split_once('"').expect("nveq seq attribute is closed");
            // Skip the optional slot binding (`data-nve-slot-for="{id}"`).
            let rest = &rest[rest.find('>').expect("nveq script tag is closed") + 1..];
            let b64 = &rest[..rest.find("</script>").expect("nveq script is closed")];
            (
                seq.parse().expect("nveq seq is a u64"),
                BASE64_STANDARD.decode(b64).expect("nveq payload is valid base64"),
            )
        })
        .collect()
}

#[tokio::test]
async fn issues_page_streams_out_of_order_slots_with_strict_nonce_csp() {
    let srv = TestServer::start(|svc| svc.with_wasm_dir(std::env::temp_dir())).await;

    // Comments slower than stats -> stats slot streams first.
    srv.store.set_stats_delay_ms(5);
    srv.store.set_comments_delay_ms(60);
    let (resp, html) = srv.page("/issues").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(header(&resp, "content-type"), "text/html; charset=utf-8");
    let nonce_a = assert_strict_csp(&resp, &html);

    let host = index_of(&html, "<template shadowrootmode=\"open\">");
    let list = index_of(&html, "testid=\"issues-list\"");
    let stats_fallback = index_of(&html, "testid=\"stats-skeleton\"");
    let comments_fallback = index_of(&html, "testid=\"comments-skeleton\"");
    let stats_slot = index_of(&html, "<div slot=\"nve-slot-stats\"");
    let comments_slot = index_of(&html, "<div slot=\"nve-slot-comments\"");
    assert!(host < list && host < stats_fallback && stats_fallback < comments_fallback);
    assert!(comments_fallback < stats_slot && stats_slot < comments_slot);
    // seq=0 (issue list) precedes the async boot module; each slot's NVEQ precedes its delta.
    let seq0 = index_of(&html, "data-nve-seq=\"0\"");
    let boot = index_of(
        &html,
        "<script type=\"module\" async src=\"/pkg/issues/boot.js\"></script>",
    );
    assert!(seq0 < boot && boot < stats_slot);
    // Each slot's payload is bound to its own markup so clients ingest both together.
    assert!(index_of(&html, "data-nve-seq=\"1\" data-nve-slot-for=\"stats\"") < stats_slot);
    let seq2 = index_of(&html, "data-nve-seq=\"2\" data-nve-slot-for=\"comments\"");
    assert!(stats_slot < seq2 && seq2 < comments_slot && comments_slot > seq0);
    assert!(html.contains("<link rel=\"modulepreload\" href=\"/pkg/issues/boot.js\">"));
    assert_eq!(nveq_chunks(&html).len(), 3);

    // Reversed delays -> comments slot streams first (true out-of-order, not document order).
    srv.store.set_stats_delay_ms(60);
    srv.store.set_comments_delay_ms(5);
    let (resp_b, html_b) = srv.page("/issues").await;
    let nonce_b = assert_strict_csp(&resp_b, &html_b);
    assert!(
        index_of(&html_b, "<div slot=\"nve-slot-comments\"")
            < index_of(&html_b, "<div slot=\"nve-slot-stats\"")
    );
    assert_ne!(nonce_a, nonce_b, "CSP nonce must be fresh per request");

    srv.stop().await;
}

#[tokio::test]
async fn issues_page_without_wasm_dir_ships_zero_scripts_but_query_data() {
    let srv = TestServer::start(|svc| svc).await;
    let (resp, html) = srv.page("/issues").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_strict_csp(&resp, &html);
    assert!(!html.contains("boot.js"));
    assert!(!html.contains("type=\"module\""));
    assert!(html.contains("form") && html.contains("action=\"/issues/create\""));
    srv.stop().await;
}

#[tokio::test]
async fn unknown_focus_issue_is_404_before_headers_and_known_focus_is_200() {
    let srv = TestServer::start(|svc| svc).await;
    let (missing, body) = srv.page("/issues?issue=999").await;
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert!(missing.headers().get(CONTENT_SECURITY_POLICY).is_none());
    assert!(!body.contains("<html"));
    let (found, _) = srv.page("/issues?issue=2").await;
    assert_eq!(found.status(), StatusCode::OK);
    // Unauthenticated callers never reach SSR.
    let unauth = srv.request(Method::GET, "/issues?issue=999", false).await;
    assert_eq!(unauth.status(), StatusCode::UNAUTHORIZED);
    srv.stop().await;
}

#[tokio::test]
async fn unknown_comments_issue_streams_error_boundary_and_domain_err_nveq() {
    let srv = TestServer::start(|svc| svc).await;
    srv.store.set_comments_delay_ms(20);
    let (resp, html) = srv.page("/issues?comments_issue=999").await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_strict_csp(&resp, &html);
    let slot = index_of(&html, "<div slot=\"nve-slot-comments\"");
    assert!(index_of(&html, "testid=\"comments-error-boundary\"") > slot);

    // Hydrating every streamed chunk yields the typed `ApiError::NotFound(999)` without any RPC.
    let verify = erased_query_client(DirectTransport::new(build_issues_server(
        IssueStore::seeded(),
        ["http://unused"],
    )));
    for (seq, bytes) in nveq_chunks(&html) {
        if seq == 0 {
            verify.hydrate_bytes(&bytes).unwrap();
        } else {
            verify.hydrate_stream_chunk_bytes(&bytes).unwrap();
        }
    }
    let handle = verify
        .subscribe::<ListComments>(&IssueId(999), QueryOptions::default())
        .unwrap();
    let snap = verify.read::<ListComments>(handle);
    assert!(snap.data.is_none() && !snap.is_fetching && !snap.is_stream_pending);
    assert_eq!(
        *snap.error.expect("DomainErr hydrated from NVEQ"),
        RpcError::Domain {
            status: RpcStatus::NotFound,
            error: ApiError::NotFound(IssueId(999)),
        }
    );
    srv.stop().await;
}

#[tokio::test]
async fn boot_js_is_external_nosniff_module_only_with_wasm_dir() {
    let empty_dir = std::env::temp_dir().join(format!("nve-issues-boot-{}", std::process::id()));
    std::fs::create_dir_all(&empty_dir).unwrap();
    let srv = TestServer::start(|svc| svc.with_wasm_dir(&empty_dir)).await;
    let resp = srv.request(Method::GET, "/pkg/issues/boot.js", false).await;
    assert_eq!(resp.status(), StatusCode::OK);
    assert_eq!(
        header(&resp, "content-type"),
        "application/javascript; charset=utf-8"
    );
    assert_eq!(header(&resp, "x-content-type-options"), "nosniff");
    assert_eq!(header(&resp, "cache-control"), "no-store");
    let js = resp.into_body().collect().await.unwrap().to_bytes();
    let js = core::str::from_utf8(&js).unwrap();
    assert!(js.contains("mod.start_issues_app('app', '')"));
    assert!(js.contains("/pkg/issues/manifest.json"));
    srv.stop().await;
    let _ = std::fs::remove_dir_all(&empty_dir);

    let no_wasm = TestServer::start(|svc| svc).await;
    let resp = no_wasm.request(Method::GET, "/pkg/issues/boot.js", false).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    no_wasm.stop().await;
}

#[tokio::test]
async fn test_hooks_are_404_unless_enabled() {
    let srv = TestServer::start(|svc| svc).await;
    for (method, path) in [
        (Method::POST, "/_test/stream-control?hold_comments=1"),
        (Method::GET, "/_test/stream-status"),
    ] {
        let resp = srv.request(method, path, true).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }
    srv.stop().await;
}

#[tokio::test]
async fn test_hooks_hold_comments_slot_open_until_released() {
    let srv = TestServer::start(|svc| svc.with_test_hooks(true)).await;
    let unknown = srv.request(Method::GET, "/_test/nope", false).await;
    assert_eq!(unknown.status(), StatusCode::NOT_FOUND);
    let ctl = srv
        .request(
            Method::POST,
            "/_test/stream-control?stats_delay_ms=0&comments_delay_ms=0&hold_comments=1",
            false,
        )
        .await;
    assert_eq!(ctl.status(), StatusCode::OK);

    // Headers + shell arrive while the comments RPC is parked on the barrier (0ms stats resolve
    // synchronously during the shell pass, so they render inline without a slot delta).
    let resp = srv.request(Method::GET, "/issues", true).await;
    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();
    let mut html = String::new();
    while !html.contains("testid=\"stats-panel\"") || !html.contains("data-nve-q-ack=\"0\"") {
        let frame = tokio::time::timeout(Duration::from_secs(5), body.frame())
            .await
            .expect("stats slot streams while comments are held")
            .unwrap()
            .unwrap();
        if let Ok(data) = frame.into_data() {
            html.push_str(core::str::from_utf8(&data).unwrap());
        }
    }
    assert_eq!(srv.stream_status().await, "comments_waiting=1");
    assert!(html.contains("testid=\"comments-skeleton\""));
    assert!(!html.contains("<div slot=\"nve-slot-comments\""));
    assert!(
        tokio::time::timeout(Duration::from_millis(150), body.frame())
            .await
            .is_err(),
        "stream must stay open while held"
    );

    let release = srv
        .request(Method::POST, "/_test/stream-control?hold_comments=0", false)
        .await;
    assert_eq!(release.status(), StatusCode::OK);
    let rest = body.collect().await.unwrap().to_bytes();
    html.push_str(core::str::from_utf8(&rest).unwrap());
    assert!(html.contains("<div slot=\"nve-slot-comments\""));
    assert!(html.contains("testid=\"featured-comments-panel\""));
    assert_eq!(srv.stream_status().await, "comments_waiting=0");
    srv.stop().await;
}

async fn error_body(resp: Response<nve_http_hyper::HttpBody>) -> (StatusCode, String) {
    let (parts, body) = resp.into_parts();
    let bytes = body.collect().await.expect("error body").to_bytes();
    (parts.status, String::from_utf8(bytes.to_vec()).expect("utf-8 error body"))
}

#[tokio::test]
async fn server_error_pages_never_leak_upstream_or_internal_detail() {
    use nve_example_issues_server::ssr::{critical_error_response, ssr_error_response};
    use nve_ssr::SsrError;

    const SECRET: &str = "db=postgres://admin:hunter2@10.0.0.7 stack=panicked at store.rs:42";
    let cases: [(RpcError<ApiError>, StatusCode, &str); 4] = [
        (
            RpcError::Protocol { status: RpcStatus::Internal, message: SECRET.into() },
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
        (
            RpcError::Protocol { status: RpcStatus::Unavailable, message: SECRET.into() },
            StatusCode::SERVICE_UNAVAILABLE,
            "unavailable",
        ),
        (
            RpcError::Transport(nve_rpc::TransportError::network(SECRET)),
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
        ),
        // Client errors keep their actionable message.
        (
            RpcError::Protocol { status: RpcStatus::NotFound, message: "no such issue".into() },
            StatusCode::NOT_FOUND,
            "no such issue",
        ),
    ];
    for (err, status, expected) in cases {
        let (got_status, body) = error_body(critical_error_response(&err)).await;
        assert_eq!(got_status, status, "{body}");
        assert!(body.contains(expected), "{body}");
        assert!(!body.contains("hunter2") && !body.contains("store.rs"), "leaked: {body}");
    }
    for (err, status) in [
        (SsrError::PoolOverloaded, StatusCode::SERVICE_UNAVAILABLE),
        (SsrError::PoolClosed, StatusCode::SERVICE_UNAVAILABLE),
        (SsrError::Render(SECRET.into()), StatusCode::INTERNAL_SERVER_ERROR),
    ] {
        let (got_status, body) = error_body(ssr_error_response(&err)).await;
        assert_eq!(got_status, status, "{body}");
        assert!(!body.contains("hunter2") && !body.contains("store.rs"), "leaked: {body}");
    }
}

#[tokio::test]
async fn stalled_critical_prefetch_answers_503_before_headers() {
    use nve_example_issues_server::ssr::{bounded_critical_prefetch, CRITICAL_PREFETCH_TIMEOUT};

    assert_eq!(CRITICAL_PREFETCH_TIMEOUT, Duration::from_secs(5));
    let started = std::time::Instant::now();
    let core::ops::ControlFlow::Break(resp) =
        bounded_critical_prefetch(Duration::from_millis(30), std::future::pending()).await
    else {
        panic!("a stalled prefetch must short-circuit with a response");
    };
    assert!(started.elapsed() < Duration::from_secs(2));
    let (status, body) = error_body(resp).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("unavailable"), "{body}");

    // A prefetch that settles in time passes its own outcome through untouched.
    let settled = bounded_critical_prefetch(Duration::from_secs(1), async {
        core::ops::ControlFlow::Continue(())
    })
    .await;
    assert!(settled.is_continue());
}

#[tokio::test]
async fn root_demo_login_and_logout_flow_enforce_csp_csrf_and_fail_closed_issues_route() {
    let srv = TestServer::start(|svc| svc).await;

    // Unauthenticated `GET /` serves the zero-JS sign-in page with a strict nonce CSP.
    let root_resp = srv.request(Method::GET, "/", false).await;
    assert_eq!(root_resp.status(), StatusCode::OK);
    assert_eq!(header(&root_resp, "content-type"), "text/html; charset=utf-8");
    assert_eq!(header(&root_resp, "x-content-type-options"), "nosniff");
    assert_eq!(header(&root_resp, "cache-control"), "no-store");
    let csp = header(&root_resp, CONTENT_SECURITY_POLICY).to_owned();
    let nonce = csp
        .split("'nonce-")
        .nth(1)
        .and_then(|rest| rest.split('\'').next())
        .unwrap()
        .to_owned();
    assert_eq!(nonce.len(), 24, "128-bit base64 nonce: {csp}");
    assert!(csp.contains("script-src 'self' 'wasm-unsafe-eval'"));
    let root_html = String::from_utf8(
        root_resp
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes()
            .to_vec(),
    )
    .unwrap();
    assert!(root_html.contains(&format!("<style nonce=\"{nonce}\">")));
    assert!(!root_html.contains("<script"));
    assert!(root_html.contains("action=\"/demo-login\""));

    // Unauthenticated `GET /issues` still fails closed with 401.
    let unauth_issues = srv.request(Method::GET, "/issues", false).await;
    assert_eq!(unauth_issues.status(), StatusCode::UNAUTHORIZED);

    // Cross-site `POST /demo-login` is blocked by CSRF with 403 and sets no cookie.
    let cross_site_login = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/demo-login", srv.origin))
        .header(http::header::ORIGIN, "https://evil.example.com")
        .header("sec-fetch-site", "cross-site")
        .body(Empty::new())
        .unwrap();
    let cross_resp = srv.client.request(cross_site_login).await.unwrap();
    assert_eq!(cross_resp.status(), StatusCode::FORBIDDEN);
    assert!(cross_resp.headers().get(http::header::SET_COOKIE).is_none());

    // Same-origin `POST /demo-login` mints the hardened HttpOnly cookie and redirects 303 -> /issues.
    let same_origin_login = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/demo-login", srv.origin))
        .header(http::header::ORIGIN, &srv.origin)
        .header("sec-fetch-site", "same-origin")
        .body(Empty::new())
        .unwrap();
    let login_resp = srv.client.request(same_origin_login).await.unwrap();
    assert_eq!(login_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(header(&login_resp, http::header::LOCATION), "/issues");
    let set_cookie = header(&login_resp, http::header::SET_COOKIE).to_owned();
    assert!(set_cookie.contains(&format!("{SESSION_COOKIE_NAME}={DEMO_WEB_COOKIE_TOKEN}")));
    assert!(set_cookie.contains("HttpOnly"));
    assert!(set_cookie.contains("SameSite=Lax"));
    assert!(set_cookie.contains("Path=/"));

    // Authenticated `GET /` redirects straight to `/issues`.
    let authed_root = srv.request(Method::GET, "/", true).await;
    assert_eq!(authed_root.status(), StatusCode::SEE_OTHER);
    assert_eq!(header(&authed_root, http::header::LOCATION), "/issues");

    // Cross-site `POST /demo-logout` fails with 403; same-origin clears cookie and redirects -> `/`.
    let cross_logout = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/demo-logout", srv.origin))
        .header(
            http::header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={DEMO_WEB_COOKIE_TOKEN}"),
        )
        .header(http::header::ORIGIN, "https://evil.example.com")
        .header("sec-fetch-site", "cross-site")
        .body(Empty::new())
        .unwrap();
    assert_eq!(
        srv.client.request(cross_logout).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );

    let same_logout = Request::builder()
        .method(Method::POST)
        .uri(format!("{}/demo-logout", srv.origin))
        .header(
            http::header::COOKIE,
            format!("{SESSION_COOKIE_NAME}={DEMO_WEB_COOKIE_TOKEN}"),
        )
        .header(http::header::ORIGIN, &srv.origin)
        .header("sec-fetch-site", "same-origin")
        .body(Empty::new())
        .unwrap();
    let logout_resp = srv.client.request(same_logout).await.unwrap();
    assert_eq!(logout_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(header(&logout_resp, http::header::LOCATION), "/");
    assert!(header(&logout_resp, http::header::SET_COOKIE).contains("Max-Age=0"));

    srv.stop().await;
}
