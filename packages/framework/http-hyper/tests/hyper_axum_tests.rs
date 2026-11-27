//! Loopback Hyper client↔server tests, Axum `nest_service` & `route_service` mountability,
//! real streaming chunked `413`, preflight-before-body-read proof, stalled body `408` timeout,
//! graceful keep-alive shutdown, `HyperTransport` response cap / `https://` rejection, and
//! `CountingAlloc` zero-alloc guards (both single-frame `Ok` and oversized rejection).

use core::convert::Infallible;
use core::pin::Pin;
use core::task::{Context, Poll};
use core::time::Duration;
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;

use axum::routing::get;
use axum::Router;
use bytes::Bytes;
use http::header::{CONTENT_TYPE, ORIGIN};
use http::{Method, Request, StatusCode};
use http_body_util::Full;
use hyper::body::{Body, Frame, SizeHint};
use hyper::service::Service as _;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use nve_http_hyper::{
    collect_body_limited, collect_body_limited_timeout, serve_rpc_with_shutdown,
    serve_service_with_config, BodyReadError, HyperTransport, NveHyperService, ServeConfig,
};
use nve_rpc::{
    call, procedures, DomainStatus, Procedure, RpcError, RpcStatus, Tag64, CONTENT_TYPE_CBOR,
    MAX_INVALIDATE_HEADER_TAGS,
};
use nve_server::{AnonymousAuth, CsrfPolicy, Cx, RpcRegistry, Server, ServerConfig};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

struct CountingAlloc;

thread_local! {
    static THREAD_ALLOCS: Cell<usize> = const { Cell::new(0) };
}

unsafe impl GlobalAlloc for CountingAlloc {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let _ = THREAD_ALLOCS.try_with(|c| c.set(c.get().saturating_add(1)));
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) }
    }
}

#[global_allocator]
static GLOBAL: CountingAlloc = CountingAlloc;

/// Unknown-size multi-frame body proving streaming per-frame byte enforcement.
struct ChunkedBody {
    chunks: Vec<Bytes>,
}

impl Body for ChunkedBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        if self.chunks.is_empty() {
            Poll::Ready(None)
        } else {
            let next = self.chunks.remove(0);
            Poll::Ready(Some(Ok(Frame::data(next))))
        }
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default() // unknown upper & lower == 0 -> bypasses size_hint shortcut!
    }
}

/// Body that panics if polled, proving preflight rejects invalid requests before touching the body.
struct PanicIfPolledBody;

impl Body for PanicIfPolledBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        panic!("body was polled despite failing preflight checks!");
    }
}

/// Body that stalls forever mid-stream, proving `body_read_timeout` returns `408 Request Timeout`.
struct StalledBody;

impl Body for StalledBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Pending
    }
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct EchoReq {
    #[n(0)]
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
struct EchoRes {
    #[n(0)]
    text: String,
}

#[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
enum EchoErr {
    #[n(0)]
    Empty,
}

impl DomainStatus for EchoErr {
    fn rpc_status(&self) -> RpcStatus {
        RpcStatus::Unprocessable
    }
}

procedures! {
    query Echo = "hyper.v1.Test/Echo" (EchoReq) -> EchoRes, EchoErr;
    action Bump = "hyper.v1.Test/Bump" (u64) -> u64, EchoErr;
    action BulkInvalidate = "hyper.v1.Test/BulkInvalidate" (u64) -> u64, EchoErr;
}

fn build_demo_server(max_body_bytes: usize) -> Server<(), (), AnonymousAuth> {
    let mut reg = RpcRegistry::<(), ()>::builder();
    reg.register::<Echo, _, _>(|_cx: Cx<(), ()>, req: EchoReq| async move {
        if req.text.is_empty() {
            return Err(EchoErr::Empty);
        }
        Ok(EchoRes { text: req.text })
    });
    reg.register::<Bump, _, _>(|cx: Cx<(), ()>, n: u64| async move {
        cx.invalidate(Tag64::keyed("counter", n));
        Ok(n + 1)
    });
    reg.register::<BulkInvalidate, _, _>(|cx: Cx<(), ()>, count: u64| async move {
        for i in 0..count {
            cx.invalidate(Tag64::keyed("bulk", i));
        }
        Ok(count)
    });
    Server::new(
        (),
        reg.build(),
        AnonymousAuth,
        CsrfPolicy::allow_origins(["http://127.0.0.1"]),
        ServerConfig {
            max_body_bytes,
            ..ServerConfig::default()
        },
    )
}

#[tokio::test]
async fn loopback_hyper_server_client_roundtrip_caps_and_wildcard_invalidation() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();

    let server = build_demo_server(256);
    let task = tokio::spawn(async move {
        serve_rpc_with_shutdown(listener, server, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    let transport = HyperTransport::new(format!("http://{addr}"));

    // 1. Query success.
    let res = call::<Echo>(
        &transport,
        &EchoReq {
            text: "hello hyper".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(res.value.text, "hello hyper");

    // 2. Typed domain error.
    let err = call::<Echo>(&transport, &EchoReq { text: String::new() })
        .await
        .unwrap_err();
    assert_eq!(
        err,
        RpcError::Domain {
            status: RpcStatus::Unprocessable,
            error: EchoErr::Empty
        }
    );

    // 3. Action returns explicit Nve-Invalidate tags; >128 returns wildcard `invalidate_all`.
    let bumped = call::<Bump>(&transport, &9).await.unwrap();
    assert_eq!(bumped.value, 10);
    assert_eq!(bumped.invalidated_tags, vec![Tag64::keyed("counter", 9)]);
    assert!(!bumped.invalidate_all);

    let bulk = call::<BulkInvalidate>(&transport, &(MAX_INVALIDATE_HEADER_TAGS as u64 + 15))
        .await
        .unwrap();
    assert!(bulk.invalidate_all);
    assert!(bulk.invalidated_tags.is_empty());

    // 4. Oversized request body -> 413 Payload Too Large.
    let raw_client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let huge_req = Request::builder()
        .method(Method::POST)
        .uri(format!("http://{addr}/rpc/{}", Echo::NAME))
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .body(Full::new(Bytes::from(vec![b'a'; 512])))
        .unwrap();
    let huge_resp = raw_client.request(huge_req).await.unwrap();
    assert_eq!(huge_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // 5. Client `max_response_bytes` cap & explicit `https://` rejection.
    let capped_transport = HyperTransport::new(format!("http://{addr}")).with_max_response_bytes(4);
    let cap_err = call::<Echo>(
        &capped_transport,
        &EchoReq {
            text: "longer than four bytes".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(matches!(cap_err, RpcError::Transport(_)));

    let https_transport = HyperTransport::new("https://example.com");
    let https_err = call::<Bump>(&https_transport, &1).await.unwrap_err();
    assert!(matches!(https_err, RpcError::Transport(_)));

    // Non-ASCII multi-byte host at byte index 8 MUST NOT panic on UTF-8 boundary slicing.
    let utf8_transport = HyperTransport::new("http://ü.example");
    let utf8_err = call::<Bump>(&utf8_transport, &1).await.unwrap_err();
    assert!(matches!(utf8_err, RpcError::Transport(_)));

    let _ = tx.send(());
    task.await.unwrap();
}

#[tokio::test]
async fn nve_hyper_service_mounts_cleanly_in_axum_nest_service_and_route_service() {
    let server = build_demo_server(1024);

    let mut raw_reg = RpcRegistry::<(), ()>::builder();
    raw_reg.register::<Echo, _, _>(|_cx: Cx<(), ()>, req: EchoReq| async move {
        Ok(EchoRes { text: req.text })
    });
    let raw_server = Server::new(
        (),
        raw_reg.build(),
        AnonymousAuth,
        CsrfPolicy::allow_origins(["http://127.0.0.1"]),
        ServerConfig {
            rpc_prefix: "/rpc-raw".to_owned(),
            ..ServerConfig::default()
        },
    );

    // Proves BOTH `Router::nest_service("/rpc", NveHyperService::nested(...))` AND
    // `Router::route_service("/rpc-raw/{*rest}", NveHyperService::new(...))` work live at runtime.
    let app = Router::new()
        .route("/healthz", get(|| async { "ok" }))
        .nest_service("/rpc", NveHyperService::nested(server))
        .route_service("/rpc-raw/{*rest}", NveHyperService::new(raw_server));

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();

    let task = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async {
                let _ = rx.await;
            })
            .await
            .unwrap();
    });

    let raw_client = Client::builder(TokioExecutor::new()).build_http::<Full<Bytes>>();
    let health = raw_client
        .get(format!("http://{addr}/healthz").parse().unwrap())
        .await
        .unwrap();
    assert_eq!(health.status(), StatusCode::OK);

    // 1. Live runtime call through `nest_service("/rpc", NveHyperService::nested(...))`.
    let nested_transport = HyperTransport::new(format!("http://{addr}"));
    let reply = call::<Echo>(
        &nested_transport,
        &EchoReq {
            text: "nested in axum".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(reply.value.text, "nested in axum");

    // 2. Live runtime call through `route_service("/rpc-raw/{*rest}", NveHyperService::new(...))`.
    let route_transport =
        HyperTransport::new(format!("http://{addr}")).with_rpc_prefix("/rpc-raw");
    let raw_reply = call::<Echo>(
        &route_transport,
        &EchoReq {
            text: "route_service in axum".into(),
        },
    )
    .await
    .unwrap();
    assert_eq!(raw_reply.value.text, "route_service in axum");

    let _ = tx.send(());
    task.await.unwrap();
}

#[tokio::test]
async fn preflight_rejects_before_polling_body_and_stalled_body_returns_408() {
    let server = build_demo_server(256);
    let svc = NveHyperService::new(server).with_body_read_timeout(Duration::from_millis(50));

    // Cross-site request MUST reject with 403 WITHOUT polling `PanicIfPolledBody`.
    let bad_csrf = Request::builder()
        .method(Method::POST)
        .uri(format!("/rpc/{}", Echo::NAME))
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .header(ORIGIN, "https://evil.example.com")
        .body(PanicIfPolledBody)
        .unwrap();
    let csrf_resp = svc.call(bad_csrf).await.unwrap();
    assert_eq!(csrf_resp.status(), StatusCode::FORBIDDEN);

    // Unknown route MUST reject with 404 WITHOUT polling `PanicIfPolledBody`.
    let bad_route = Request::builder()
        .method(Method::POST)
        .uri("/rpc/hyper.v1.Test/DoesNotExist")
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .body(PanicIfPolledBody)
        .unwrap();
    let route_resp = svc.call(bad_route).await.unwrap();
    assert_eq!(route_resp.status(), StatusCode::NOT_FOUND);

    // Real streaming unknown-length chunked body exceeding 256 B aborts with 413 mid-stream.
    let chunked_over = Request::builder()
        .method(Method::POST)
        .uri(format!("/rpc/{}", Echo::NAME))
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .body(ChunkedBody {
            chunks: vec![
                Bytes::from(vec![b'a'; 128]),
                Bytes::from(vec![b'b'; 128]),
                Bytes::from(vec![b'c'; 16]),
            ],
        })
        .unwrap();
    let over_resp = svc.call(chunked_over).await.unwrap();
    assert_eq!(over_resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Stalled body hits `body_read_timeout` (50ms) and returns 408 Request Timeout.
    let stalled = Request::builder()
        .method(Method::POST)
        .uri(format!("/rpc/{}", Echo::NAME))
        .header(CONTENT_TYPE, CONTENT_TYPE_CBOR)
        .body(StalledBody)
        .unwrap();
    let timeout_resp = svc.call(stalled).await.unwrap();
    assert_eq!(timeout_resp.status(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn graceful_shutdown_closes_idle_keep_alive_connection_cleanly() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();

    let server = build_demo_server(1024);
    let config = ServeConfig {
        graceful_shutdown_timeout: Duration::from_secs(2),
        ..ServeConfig::default()
    };
    let task = tokio::spawn(async move {
        serve_service_with_config(listener, NveHyperService::new(server), config, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    // Open a raw HTTP/1.1 keep-alive connection and complete one request.
    let mut tcp = TcpStream::connect(addr).await.unwrap();
    let req_bytes = b"POST /rpc/hyper.v1.Test/Bump HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/cbor\r\nContent-Length: 1\r\n\r\n\x01";
    tcp.write_all(req_bytes).await.unwrap();
    let mut head_buf = [0u8; 512];
    let n = tcp.read(&mut head_buf).await.unwrap();
    assert!(
        core::str::from_utf8(&head_buf[..n])
            .unwrap()
            .starts_with("HTTP/1.1 200 OK"),
        "expected 200 OK over keep-alive socket"
    );

    // Trigger graceful shutdown; server must close the idle keep-alive socket cleanly (EOF).
    let _ = tx.send(());
    let mut eof_buf = [0u8; 16];
    let eof_read = tokio::time::timeout(Duration::from_secs(2), tcp.read(&mut eof_buf))
        .await
        .expect("keep-alive connection must close on graceful shutdown")
        .unwrap();
    assert_eq!(eof_read, 0, "expected clean TCP EOF from graceful shutdown");
    task.await.unwrap();
}

#[tokio::test]
async fn collect_body_limited_zero_allocs_on_single_frame_and_size_hint_reject() {
    // 1. Oversized size_hint rejection: 0 heap allocations.
    let payload = Bytes::from_static(&[1u8; 2048]);
    let before = THREAD_ALLOCS.with(Cell::get);
    let res = collect_body_limited(Full::new(payload), 1024).await;
    let after = THREAD_ALLOCS.with(Cell::get);
    assert_eq!(res, Err(BodyReadError::TooLarge { limit: 1024 }));
    assert_eq!(
        after.saturating_sub(before),
        0,
        "oversized size_hint rejection must allocate 0 times"
    );

    // 2. Single-frame happy path: 0 heap allocations (no wasted BytesMut).
    let single = Bytes::from_static(b"hello single frame");
    let before_ok = THREAD_ALLOCS.with(Cell::get);
    let ok = collect_body_limited(Full::new(single.clone()), 1024)
        .await
        .unwrap();
    let after_ok = THREAD_ALLOCS.with(Cell::get);
    assert_eq!(ok, single);
    assert_eq!(
        after_ok.saturating_sub(before_ok),
        0,
        "single-frame collect_body_limited happy path must allocate 0 times"
    );

    // 3. Stalled body times out with BodyReadError::Timeout.
    let timed = collect_body_limited_timeout(StalledBody, 1024, Duration::from_millis(20)).await;
    assert!(matches!(timed, Err(BodyReadError::Timeout { .. })));
}

#[tokio::test]
async fn hyper_transport_background_runtime_works_on_non_tokio_thread_and_cors_options_skips_body() {
    let mut reg = RpcRegistry::<(), ()>::builder();
    reg.register::<Echo, _, _>(|_cx: Cx<(), ()>, req: EchoReq| async move {
        Ok(EchoRes { text: req.text })
    });
    let server = Server::new(
        (),
        reg.build(),
        AnonymousAuth,
        CsrfPolicy::allow_origins(["http://127.0.0.1"]),
        ServerConfig::default()
            .with_cors(nve_server::CorsPolicy::allow_origins(["http://localhost:8080"])),
    );

    // 1. CORS OPTIONS preflight MUST NOT poll the body (`PanicIfPolledBody`).
    let svc = NveHyperService::new(server.clone());
    let opt_req = Request::builder()
        .method(Method::OPTIONS)
        .uri("/rpc/hyper.v1.Echo/Call")
        .header(ORIGIN, "http://localhost:8080")
        .header(http::header::ACCESS_CONTROL_REQUEST_METHOD, "POST")
        .body(PanicIfPolledBody)
        .unwrap();
    let opt_resp = svc.call(opt_req).await.unwrap();
    assert_eq!(opt_resp.status(), StatusCode::NO_CONTENT);
    assert_eq!(
        opt_resp
            .headers()
            .get(http::header::ACCESS_CONTROL_ALLOW_ORIGIN)
            .unwrap(),
        "http://localhost:8080"
    );

    // 2. Start loopback Hyper server and call it from a plain `std::thread` with ZERO ambient
    //    Tokio runtime (simulating Skia's synchronous `winit` UI thread).
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = oneshot::channel::<()>();
    let srv_task = tokio::spawn(async move {
        serve_rpc_with_shutdown(listener, server, async {
            let _ = rx.await;
        })
        .await
        .unwrap();
    });

    let base_url = format!("http://{addr}");
    let (res_tx, res_rx) = oneshot::channel();
    let ui_thread = std::thread::spawn(move || {
        // Confirm zero ambient Tokio runtime on this thread!
        assert!(tokio::runtime::Handle::try_current().is_err());
        let client = HyperTransport::new(base_url)
            .with_bearer_token("native-skia-token")
            .unwrap()
            .with_background_runtime()
            .unwrap();
        let req = EchoReq {
            text: "from-winit-thread".into(),
        };
        let mut fut = std::pin::pin!(call::<Echo>(&client, &req));
        let thread = std::thread::current();
        struct ThreadWake(std::thread::Thread);
        impl std::task::Wake for ThreadWake {
            fn wake(self: std::sync::Arc<Self>) {
                self.0.unpark();
            }
        }
        let waker = std::task::Waker::from(std::sync::Arc::new(ThreadWake(thread)));
        let mut cx = Context::from_waker(&waker);
        let res = loop {
            match fut.as_mut().poll(&mut cx) {
                Poll::Ready(res) => break res,
                Poll::Pending => std::thread::park(),
            }
        };
        let _ = res_tx.send(res);
    });
    let reply = res_rx.await.expect("UI thread must report result").unwrap();
    ui_thread.join().expect("plain UI thread must not panic");
    assert_eq!(reply.value.text, "from-winit-thread");

    let _ = tx.send(());
    srv_task.await.unwrap();
}
