//! Slice 1c `nve-ssr` integration scenarios:
//! 3. Real loopback HTTP/1.1 TCP disconnect via `nve_http_hyper::serve_service_with_shutdown`
//!    proving in-flight RPC future drop guard fires within `< 250ms`.
//! 4. Concurrent User A vs User B principal isolation over a shared `SsrLocalPool` and
//!    `nve_server::DirectTransport`.

mod support;

use core::convert::Infallible;
use core::future::Future;
use core::pin::Pin;
use core::time::Duration;
use std::time::Instant;

use http::{Request, Response};
use hyper::body::Incoming;
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_http_hyper::{HttpBody, serve_service_with_shutdown};
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_rpc::BoxFuture;
use nve_server::{
    AuthError, AuthResolver, CsrfPolicy, Cx, DirectTransport, RpcRegistry, Server, ServerConfig,
};
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::oneshot;

use support::*;

fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    haystack
        .as_bytes()
        .windows(needle.len())
        .any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

// ---------------------------------------------------------------------------
// Test 3: Real loopback HTTP/1.1 TCP disconnect cancels in-flight RPC < 250ms
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LoopbackSsrService {
    pool: SsrLocalPool,
    transport: ControlledTransport,
}

impl hyper::service::Service<Request<Incoming>> for LoopbackSsrService {
    type Response = Response<HttpBody>;
    type Error = Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn call(&self, _req: Request<Incoming>) -> Self::Future {
        let pool = self.pool.clone();
        let transport = self.transport.clone();
        Box::pin(async move {
            let client = erased_ssr_query_client(transport);
            let plan = StreamPlan::default()
                .with_total_deadline(Duration::from_secs(10))
                .with_keepalive_interval(Duration::from_millis(20));
            let resp = pool
                .render_streaming(SsrPageOptions::new("Disconnect Test", web_backend), plan, client, |c| {
                    DioxusSurface::new(1, SlowPanelApp).with_context(c)
                })
                .await
                .expect("streaming SSR response");
            Ok(resp)
        })
    }
}

#[tokio::test]
async fn scenario_3_real_tcp_disconnect_cancels_in_flight_rpc_under_250ms() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let transport = ControlledTransport::default();
    let (started_tx, started_rx) = oneshot::channel();
    let (dropped_tx, dropped_rx) = oneshot::channel();
    transport.hang_with_drop_guard("slow", started_tx, dropped_tx);

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let svc = LoopbackSsrService {
        pool: pool.clone(),
        transport: transport.clone(),
    };
    let server_task = tokio::spawn(async move {
        serve_service_with_shutdown(listener, svc, async {
            let _ = shutdown_rx.await;
        })
        .await
        .unwrap();
    });

    let mut tcp = TcpStream::connect(addr).await.unwrap();
    tcp.write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await
        .unwrap();

    // Read until initial shell + seq=0 ack arrives over real HTTP/1.1 chunked TCP stream.
    let mut acc = Vec::new();
    let needle = b"data-nve-q-ack=\"0\"";
    tokio::time::timeout(Duration::from_secs(2), async {
        let mut buf = [0u8; 1024];
        while !acc.windows(needle.len()).any(|w| w == needle) {
            let n = tcp.read(&mut buf).await.expect("tcp read");
            assert!(n > 0, "unexpected EOF before shell arrived");
            acc.extend_from_slice(&buf[..n]);
        }
    })
    .await
    .expect("timed out reading shell over loopback TCP");

    let shell_wire = String::from_utf8_lossy(&acc);
    assert!(
        contains_ignore_ascii_case(&shell_wire, "transfer-encoding: chunked"),
        "Hyper must chunk-frame streaming SSR over HTTP/1.1: {shell_wire}"
    );

    // Wait until the slow RPC future is definitely in flight before disconnecting.
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .expect("slow RPC must start")
        .expect("started sender");

    // Abruptly close the client TCP socket and assert in-flight RPC drop guard fires in < 250ms!
    let drop_start = Instant::now();
    drop(tcp);

    let dropped_at = tokio::time::timeout(Duration::from_millis(250), dropped_rx)
        .await
        .expect("in-flight RPC future MUST be cancelled within 250ms of TCP disconnect")
        .expect("drop guard must send timestamp");
    let elapsed = dropped_at.saturating_duration_since(drop_start);
    assert!(
        elapsed < Duration::from_millis(250),
        "cancellation took {elapsed:?} (>= 250ms)"
    );

    let _ = shutdown_tx.send(());
    server_task.await.unwrap();
}

// ---------------------------------------------------------------------------
// Test 4: Concurrent User A vs User B principal isolation on shared SsrLocalPool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_4_concurrent_user_a_vs_user_b_isolation_shared_pool() {
    #[derive(Clone, Debug)]
    struct Principal {
        name: String,
        secret: String,
    }

    struct PrincipalAuth;
    impl AuthResolver<(), Principal> for PrincipalAuth {
        fn resolve<'a>(
            &'a self,
            _state: &'a (),
            _headers: &'a http::HeaderMap,
        ) -> BoxFuture<'a, Result<Principal, AuthError>> {
            Box::pin(async { Err(AuthError::unauthenticated("principal required")) })
        }
    }

    let mut reg = RpcRegistry::<(), Principal>::builder();
    reg.register::<GetProfile, _, _>(|cx: Cx<(), Principal>, _req_id: u32| async move {
        // Yield briefly to guarantee User A and User B interleave on the SAME LocalSet workers!
        tokio::time::sleep(Duration::from_millis(5)).await;
        Ok(ProfileData {
            user: cx.principal.name.clone(),
            secret: cx.principal.secret.clone(),
        })
    });

    let server = Server::new(
        (),
        reg.build(),
        PrincipalAuth,
        CsrfPolicy::allow_origins(["http://127.0.0.1"]),
        ServerConfig::default(),
    );

    #[component]
    fn ProfileApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    QuerySuspense {
                        id: Some("profile".into()),
                        fallback: rsx! { text { "Loading profile..." } },
                        ProfileCard {}
                    }
                }
            }
        }
    }

    #[component]
    fn ProfileCard() -> Element {
        let q = use_query_with::<GetProfile>(1, QueryOptions::default().max_retries(0));
        let profile = q.suspend()?;
        rsx! { text { "USER={profile.user}|SECRET={profile.secret}" } }
    }

    // 2 workers handling 20 concurrent interleaved User A / User B streaming renders!
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(2))
        .expect("pool init");

    let mut handles = Vec::with_capacity(20);
    for i in 0..20usize {
        let pool = pool.clone();
        let server = server.clone();
        let is_alice = i % 2 == 0;
        let principal = if is_alice {
            Principal {
                name: "alice".into(),
                secret: "ALICE_TOP_SECRET_777".into(),
            }
        } else {
            Principal {
                name: "bob".into(),
                secret: "BOB_TOP_SECRET_888".into(),
            }
        };
        handles.push(tokio::spawn(async move {
            let transport = DirectTransport::with_principal(server, principal.clone());
            let client = erased_ssr_query_client(transport);
            let resp = pool
                .render_streaming(
                    SsrPageOptions::new(format!("Profile-{}", principal.name), web_backend),
                    StreamPlan::default(),
                    client,
                    |c| DioxusSurface::new(1, ProfileApp).with_context(c),
                )
                .await
                .expect("profile SSR");
            let html = collect_all_html(resp.into_body()).await;
            (is_alice, html)
        }));
    }

    for h in handles {
        let (is_alice, html) = h.await.unwrap();
        let verify_client = erased_query_client(ControlledTransport::default());
        hydrate_client_from_html(&verify_client, &html);
        let h_prof = verify_client
            .subscribe::<GetProfile>(&1, QueryOptions::default())
            .unwrap();
        let hydrated_profile = verify_client
            .read::<GetProfile>(h_prof)
            .data
            .expect("hydrated profile in NVEQ");

        if is_alice {
            assert!(
                html.contains("USER=alice|SECRET=ALICE_TOP_SECRET_777"),
                "{html}"
            );
            assert!(
                !html.contains("BOB_TOP_SECRET_888"),
                "Bob leaked into Alice HTML: {html}"
            );
            assert_eq!(hydrated_profile.user, "alice");
            assert_eq!(hydrated_profile.secret, "ALICE_TOP_SECRET_777");
        } else {
            assert!(
                html.contains("USER=bob|SECRET=BOB_TOP_SECRET_888"),
                "{html}"
            );
            assert!(
                !html.contains("ALICE_TOP_SECRET_777"),
                "Alice leaked into Bob HTML: {html}"
            );
            assert_eq!(hydrated_profile.user, "bob");
            assert_eq!(hydrated_profile.secret, "BOB_TOP_SECRET_888");
        }
    }
}
