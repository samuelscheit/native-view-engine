//! End-to-end integration tests for `examples/fullstack-issues`:
//! - Native `HyperTransport` Bearer client vs `DirectTransport` exact parity (+ trusted SSR mode)
//! - Web Cookie + Origin RPC and progressive 0-JS HTML `<form>` POST (`303 -> GET /issues`,
//!   `422 text/plain` validation errors, `403` CSRF)
//! - Mobile App Store schema skew (both new struct fields AND open-enum unknown variants)
//! - Axum `Router` mountability of `IssuesHttpService`

use axum::Router;
use bytes::Bytes;
use http::header::{ACCEPT, CONTENT_TYPE, COOKIE, LOCATION, ORIGIN};
use http::{Method, Request, StatusCode};
use http_body_util::{BodyExt as _, Full};
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use nve_example_issues_api::{
    AddComment, AddCommentInput, ApiError, CloseIssue, CloseIssueInput, CreateIssue,
    CreateIssueInput, GetIssue, Issue, IssueFilter, IssueId, IssueState, IssueTag, ListComments,
    ListIssues, RenameIssue, RenameIssueInput,
};
use nve_example_issues_server::auth::{
    DEMO_NATIVE_BEARER_TOKEN, DEMO_WEB_COOKIE_TOKEN, SESSION_COOKIE_NAME,
};
use nve_example_issues_server::store::IssueStore;
use nve_example_issues_server::{build_issues_server, IssuesHttpService};
use nve_http_hyper::{serve_service_with_shutdown, HyperTransport};
use nve_rpc::{
    call, CacheTag, CborCodec, RpcCodec, RpcError, RpcStatus, CONTENT_TYPE_FORM_URLENCODED,
};
use nve_server::DirectTransport;
use tokio::net::TcpListener;
use tokio::sync::oneshot;

/// Simulates an older shipped mobile binary that does not yet know about `priority` (`#[n(3)]`)
/// or `comment_count` (`#[n(4)]`).
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct OldMobileIssueV0 {
    #[n(0)]
    id: IssueId,
    #[n(1)]
    title: String,
    #[n(2)]
    state: IssueState,
}

#[tokio::test]
async fn fullstack_issues_end_to_end_native_web_0js_and_mobile_skew() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let origin = format!("http://{addr}");
    let (tx, rx) = oneshot::channel::<()>();

    let store = IssueStore::seeded();
    let server = build_issues_server(store.clone(), [origin.clone()]);
    let svc = IssuesHttpService::new(server.clone());
    // Compile-time proof that `IssuesHttpService` also mounts cleanly inside Axum.
    let _axum_proof: Router = Router::new().fallback_service(svc.clone());

    let task = tokio::spawn(async move {
        serve_service_with_shutdown(listener, svc, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    // 1. Native client (Bearer auth) + DirectTransport parity + SSR cross-site Cookie parity.
    let native_http = HyperTransport::new(&origin)
        .with_bearer_token(DEMO_NATIVE_BEARER_TOKEN)
        .unwrap();
    let native_direct = DirectTransport::new(server.clone())
        .with_bearer_token(DEMO_NATIVE_BEARER_TOKEN)
        .unwrap();

    let http_list = call::<ListIssues>(&native_http, &IssueFilter::default())
        .await
        .unwrap();
    let direct_list = call::<ListIssues>(&native_direct, &IssueFilter::default())
        .await
        .unwrap();
    assert_eq!(http_list.value, direct_list.value);
    assert_eq!(http_list.raw_body, direct_list.raw_body);
    assert_eq!(http_list.value.len(), 3);

    // Trusted SSR DirectTransport with forwarded Cookie + `Sec-Fetch-Site: cross-site`.
    let cookie_hdr = format!("{SESSION_COOKIE_NAME}={DEMO_WEB_COOKIE_TOKEN}");
    let ssr_direct = DirectTransport::new(server.clone())
        .with_header(COOKIE, http::HeaderValue::from_str(&cookie_hdr).unwrap())
        .with_header(
            http::header::HeaderName::from_static("sec-fetch-site"),
            http::HeaderValue::from_static("cross-site"),
        );
    let ssr_list = call::<ListIssues>(&ssr_direct, &IssueFilter::default())
        .await
        .unwrap();
    assert_eq!(ssr_list.value, http_list.value);

    // 2. Mobile App Store version-skew checks:
    //    (a) Old mobile struct decodes live server CBOR map (skips newer keys).
    let live_issue = call::<GetIssue>(&native_http, &IssueId(1)).await.unwrap();
    let old_client_view: OldMobileIssueV0 = CborCodec::default()
        .decode_bounded(&live_issue.raw_body)
        .unwrap();
    assert_eq!(old_client_view.id, IssueId(1));
    assert_eq!(old_client_view.state, IssueState::InProgress);

    //    (b) Open-enum evolution: future server `IssueState` index 99 & future `ApiError`
    //        variant 99 carrying a nested structured CBOR payload `[99, {0: "future", 1: [1, 2, 3]}]`
    //        decode cleanly as `Unknown(99)` without turning into `RpcError::Codec`.
    let future_issue = Issue {
        id: IssueId(77),
        title: "Future Archived Issue".into(),
        state: IssueState::Unknown(99),
        priority: None,
        comment_count: 0,
    };
    let mut future_buf = Vec::new();
    CborCodec::default()
        .encode_into(&future_issue, &mut future_buf)
        .unwrap();
    let decoded_future: Issue = CborCodec::default().decode_bounded(&future_buf).unwrap();
    assert_eq!(decoded_future.state, IssueState::Unknown(99));

    future_buf.clear();
    {
        let mut enc = minicbor::Encoder::new(&mut future_buf);
        enc.array(2)
            .unwrap()
            .u32(99)
            .unwrap()
            .map(2)
            .unwrap()
            .u8(0)
            .unwrap()
            .str("future-detail")
            .unwrap()
            .u8(1)
            .unwrap()
            .array(3)
            .unwrap()
            .u8(1)
            .unwrap()
            .u8(2)
            .unwrap()
            .u8(3)
            .unwrap();
    }
    let decoded_err: ApiError = CborCodec::default().decode_bounded(&future_buf).unwrap();
    assert_eq!(decoded_err, ApiError::Unknown(99));

    // Misusing `Unknown` with a known discriminant (0/1 for ApiError, 0..=2 for IssueState)
    // fails cleanly at encode time instead of emitting corrupt wire bytes.
    future_buf.clear();
    assert!(CborCodec::default()
        .encode_into(&ApiError::Unknown(0), &mut future_buf)
        .is_err());
    future_buf.clear();
    assert!(CborCodec::default()
        .encode_into(&IssueState::Unknown(1), &mut future_buf)
        .is_err());

    // 3. Native action triggers static `procedures!` invalidation automatically.
    let comment_reply = call::<AddComment>(
        &native_http,
        &AddCommentInput {
            issue_id: IssueId(1),
            body: "Verified from native HyperTransport".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(comment_reply.value.author, "native-alice");
    assert_eq!(
        comment_reply.invalidated_tags,
        vec![
            IssueTag::List.tag64(),
            IssueTag::Issue(IssueId(1)).tag64(),
            IssueTag::Comments(IssueId(1)).tag64(),
        ]
    );

    // 4. Web client (Cookie + Origin, NO forged Sec-Fetch-Site) reads same updated comments.
    let web_http = HyperTransport::new(&origin)
        .with_cookie_and_origin(&cookie_hdr, &origin)
        .unwrap();
    let comments = call::<ListComments>(&web_http, &IssueId(1)).await.unwrap();
    assert_eq!(comments.value.len(), 2);

    // 5. Typed domain error (`NotFound` -> HTTP 404).
    let missing = call::<GetIssue>(&web_http, &IssueId(999)).await.unwrap_err();
    assert_eq!(
        missing,
        RpcError::Domain {
            status: RpcStatus::NotFound,
            error: ApiError::NotFound(IssueId(999)),
        }
    );

    // 6. Progressive 0-JS HTML `<form>` POST via built-in `FormRoute`:
    let raw_client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();

    let form_ok = Request::builder()
        .method(Method::POST)
        .uri(format!("{origin}/issues/rename"))
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(COOKIE, &cookie_hdr)
        .header(ORIGIN, &origin)
        .header("sec-fetch-site", "same-origin")
        .body(Full::new(Bytes::from_static(
            b"id=2&title=Renamed+via+0-JS+form",
        )))
        .unwrap();
    let form_ok_resp = raw_client.request(form_ok).await.unwrap();
    assert_eq!(form_ok_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(form_ok_resp.headers().get(LOCATION).unwrap(), "/issues");

    // `GET /issues` requires authentication (unauthenticated callers receive 401).
    let unauth_page = raw_client
        .get(format!("{origin}/issues").parse().unwrap())
        .await
        .unwrap();
    assert_eq!(unauth_page.status(), StatusCode::UNAUTHORIZED);

    // Following the 303 redirect (`GET /issues`) with the session cookie renders live HTML.
    let auth_page_req = Request::builder()
        .method(Method::GET)
        .uri(format!("{origin}/issues"))
        .header(COOKIE, &cookie_hdr)
        .body(Full::new(Bytes::new()))
        .unwrap();
    let issues_page = raw_client.request(auth_page_req).await.unwrap();
    assert_eq!(issues_page.status(), StatusCode::OK);
    let page_bytes = issues_page.into_body().collect().await.unwrap().to_bytes();
    let page_html = core::str::from_utf8(&page_bytes).unwrap();
    assert!(page_html.contains("Renamed via 0-JS form"));

    // Blank title from a 0-JS browser -> 422 text/plain (not binary CBOR!).
    let form_bad_html = Request::builder()
        .method(Method::POST)
        .uri(format!("{origin}/issues/rename"))
        .header(ACCEPT, "text/html")
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(COOKIE, &cookie_hdr)
        .header(ORIGIN, &origin)
        .header("sec-fetch-site", "same-origin")
        .body(Full::new(Bytes::from_static(b"id=2&title=+++")))
        .unwrap();
    let form_bad_html_resp = raw_client.request(form_bad_html).await.unwrap();
    assert_eq!(form_bad_html_resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        form_bad_html_resp.headers().get(CONTENT_TYPE).unwrap(),
        "text/plain; charset=utf-8"
    );

    let form_csrf = Request::builder()
        .method(Method::POST)
        .uri(format!("{origin}/issues/rename"))
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(COOKIE, &cookie_hdr)
        .header(ORIGIN, "https://evil.example.com")
        .header("sec-fetch-site", "cross-site")
        .body(Full::new(Bytes::from_static(b"id=2&title=Hijacked")))
        .unwrap();
    let form_csrf_resp = raw_client.request(form_csrf).await.unwrap();
    assert_eq!(form_csrf_resp.status(), StatusCode::FORBIDDEN);

    // 7. Validation domain error via RPC returns 422 + ApiError::Validation.
    let rpc_invalid = call::<RenameIssue>(
        &web_http,
        &RenameIssueInput {
            id: IssueId(2),
            title: "   ".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        rpc_invalid,
        RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: ApiError::Validation(_),
        }
    ));

    // 8. CreateIssue + duplicate-title 422 + CloseIssue + 0-JS `/issues/create` form route.
    let created = call::<CreateIssue>(
        &native_http,
        &CreateIssueInput {
            title: "Native Bearer Issue".into(),
            priority: Some(1),
        },
    )
    .await
    .unwrap();
    assert_eq!(created.value.id, IssueId(4));
    assert_eq!(created.value.state, IssueState::Open);
    assert_eq!(created.invalidated_tags, vec![IssueTag::List.tag64()]);

    let dup_err = call::<CreateIssue>(
        &native_http,
        &CreateIssueInput {
            title: "native bearer issue".into(),
            priority: None,
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(
        dup_err,
        RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: ApiError::Validation(_),
        }
    ));

    let closed = call::<CloseIssue>(
        &native_http,
        &CloseIssueInput { id: IssueId(4) },
    )
    .await
    .unwrap();
    assert_eq!(closed.value.state, IssueState::Closed);
    assert_eq!(
        closed.invalidated_tags,
        vec![IssueTag::List.tag64(), IssueTag::Issue(IssueId(4)).tag64()]
    );

    let form_create = Request::builder()
        .method(Method::POST)
        .uri(format!("{origin}/issues/create"))
        .header(CONTENT_TYPE, CONTENT_TYPE_FORM_URLENCODED)
        .header(COOKIE, &cookie_hdr)
        .header(ORIGIN, &origin)
        .header("sec-fetch-site", "same-origin")
        .body(Full::new(Bytes::from_static(
            b"title=Created+via+0-JS+PRG&priority=2",
        )))
        .unwrap();
    let form_create_resp = raw_client.request(form_create).await.unwrap();
    assert_eq!(form_create_resp.status(), StatusCode::SEE_OTHER);
    assert_eq!(form_create_resp.headers().get(LOCATION).unwrap(), "/issues");

    // 9. Native Skia-style UI thread (no ambient Tokio runtime) via `with_background_runtime()`.
    let origin_bg = origin.clone();
    let (bg_tx, bg_rx) = oneshot::channel();
    let ui_thread = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        let client = HyperTransport::new(origin_bg)
            .with_bearer_token(DEMO_NATIVE_BEARER_TOKEN)
            .unwrap()
            .with_background_runtime()
            .unwrap();
        let mut fut = std::pin::pin!(call::<GetIssue>(&client, &IssueId(5)));
        let thread = std::thread::current();
        struct WakeThread(std::thread::Thread);
        impl std::task::Wake for WakeThread {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(std::sync::Arc::new(WakeThread(thread)));
        let mut cx = std::task::Context::from_waker(&waker);
        let value = loop {
            match fut.as_mut().poll(&mut cx) {
                std::task::Poll::Ready(res) => break res.unwrap().value,
                std::task::Poll::Pending => std::thread::park(),
            }
        };
        let _ = bg_tx.send(value);
    });
    let bg_issue = bg_rx.await.unwrap();
    ui_thread.join().unwrap();
    assert_eq!(bg_issue.title, "Created via 0-JS PRG");
    assert_eq!(bg_issue.priority, Some(2));

    let _ = tx.send(());
    task.await.unwrap();
}
