//! Slice 1c `nve-ssr` integration scenarios:
//! 3. Real loopback HTTP/1.1 TCP disconnect via `nve_http_hyper::serve_service_with_shutdown`
//!    proving in-flight RPC future drop guard fires within `< 250ms`.
//! 4. Concurrent User A vs User B principal isolation over a shared `SsrLocalPool` and
//!    `nve_server::DirectTransport`.

mod support;

use core::time::Duration;

use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_rpc::BoxFuture;
use nve_server::{
    AuthError, AuthResolver, CsrfPolicy, Cx, DirectTransport, RpcRegistry, Server, ServerConfig,
};
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};

use support::*;

// ---------------------------------------------------------------------------
// Test 3: Real loopback HTTP/1.1 TCP disconnect cancels in-flight RPC < 250ms
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_3_real_tcp_disconnect_cancels_in_flight_rpc_under_250ms() {
    assert_tcp_disconnect_cancels_in_flight_rpc(Duration::from_millis(20)).await;
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
