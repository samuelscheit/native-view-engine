//! Comprehensive Slice 1c Step 4 integration tests for `nve-ssr`:
//! 1. Adaptive 0-suspense Light-DOM fast path (`HttpBody::Full`, CSP nonce, headers, hydration).
//! 2. Out-of-order streaming (`stats` fast, `comments` slow) with incremental chunks, strict
//!    `NVEQ + ack` before `<div slot>`, zero duplicate CSS/NVEQ entries, and both mid-stream &
//!    post-stream `WebBackend::hydrate_from_streaming_html` (`mismatches == 0`).
//! 3. Real loopback HTTP/1.1 TCP disconnect via `nve_http_hyper::serve_service_with_shutdown`
//!    proving in-flight RPC future drop guard fires within `< 250ms`.
//! 4. Concurrent User A vs User B principal isolation over a shared `SsrLocalPool` and
//!    `nve_server::DirectTransport`.
//! 5. Post-headers `DomainErr` inside `<QuerySuspense>` streaming `<ErrorBoundary>` fallback +
//!    `DomainErr` `NVEQ` v2 entry with zero mismatches and zero duplicate client RPCs.
//! 6. `total_deadline` expiry emitting `<template data-nve-stream-end>` cleanly and client
//!    recovery via `QueryClient::finish_ssr_stream()`.
//! 7. Adversarial XSS payloads across query data and `SsrPageOptions` attributes safely escaped
//!    and round-tripping byte-for-byte in `NVEQ` v2.
//! 8. DSD placement hygiene failure (`<form>` ancestor and `<table>` foster-parent trap) returning
//!    `Err(SsrError::Streaming(StreamingError::InvalidSlotPlacement(_)))` BEFORE headers.
//! 9. CSP nonce entropy and non-blocking `Drop`.

use core::time::Duration;
use std::time::Instant;

use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use base64::Engine as _;
use http::StatusCode;
use hyper::body::Body as _;
use nve_abi::{ColorRgba, StyleProps};
use nve_backend_web::{
    streaming_http_headers, StreamingError, WebBackend, NVE_STREAM_KEEPALIVE_COMMENT,
};
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_http_hyper::HttpBody;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::BoxFuture;
use nve_server::{
    AuthError, AuthResolver, CsrfPolicy, Cx, DirectTransport, RpcRegistry, Server, ServerConfig,
};
use nve_ssr::{
    generate_csp_nonce, SsrError, SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan,
};
use tokio::sync::oneshot;

mod support;

use support::*;

// ---------------------------------------------------------------------------
// Test 1: Adaptive 0-suspense Light-DOM fast path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_1_adaptive_zero_suspense_fast_path() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(2))
        .expect("pool init");

    let transport = ControlledTransport::default();
    let client = erased_ssr_query_client(transport);
    client
        .set_query_data::<GetPanel>(
            &"hero".to_owned(),
            PanelData {
                key: "hero".into(),
                body: "Prefetched Hero".into(),
            },
        )
        .unwrap();

    #[component]
    fn FastApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    text { "Fast Header" }
                    QuerySuspense {
                        id: Some("hero".into()),
                        fallback: rsx! { text { "Should not appear" } },
                        FastHeroPanel {}
                    }
                }
            }
        }
    }

    #[component]
    fn FastHeroPanel() -> Element {
        let q = use_query_with::<GetPanel>(
            "hero".to_owned(),
            QueryOptions::default().stale_time_ms(60_000),
        );
        let data = q.suspend()?;
        rsx! { text { "HERO:{data.body}" } }
    }

    let nonce = generate_csp_nonce().unwrap();
    let page = SsrPageOptions::new("Fast Path Page")
        .with_lang("en")
        .with_description("Zero suspense Light DOM")
        .with_stylesheet_link("/assets/base.css")
        .with_style_nonce(nonce.clone())
        .with_modulepreload_src("/assets/app.js")
        .with_async_boot_src("/assets/app.js")
        .with_status(StatusCode::OK);

    let resp = pool
        .render_streaming(page, StreamPlan::default(), client, |c| {
            DioxusSurface::new(1, FastApp).with_context(c)
        })
        .await
        .expect("fast path render");

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        matches!(resp.body(), HttpBody::Full(_)),
        "0-suspense must take adaptive HttpBody::Full fast path"
    );
    assert!(resp.body().size_hint().exact().is_some());

    let headers = resp.headers();
    assert_eq!(
        headers.get("content-type").unwrap().to_str().unwrap(),
        "text/html; charset=utf-8"
    );
    assert_eq!(
        headers.get("cache-control").unwrap().to_str().unwrap(),
        "private, no-store, no-transform"
    );
    assert_eq!(
        headers.get("vary").unwrap().to_str().unwrap(),
        "Cookie, Authorization"
    );
    assert_eq!(
        headers.get("x-content-type-options").unwrap().to_str().unwrap(),
        "nosniff"
    );

    let html = collect_all_html(resp.into_body()).await;
    assert!(!html.contains("shadowrootmode"), "fast path must emit Light DOM: {html}");
    assert!(!html.contains("<div data-nve-dsd-host"), "fast path must omit DSD host element: {html}");
    assert!(!html.contains("<slot "), "fast path must omit shadow slots: {html}");
    assert!(!html.contains("data-nve-replay"), "default CSP disables inline replay: {html}");
    assert!(html.contains("HERO:Prefetched Hero"), "{html}");
    assert!(html.contains(&format!("<style data-nve-base nonce=\"{nonce}\">")), "{html}");
    assert!(html.contains(&format!("<style data-nve-ssr nonce=\"{nonce}\"")), "{html}");
    assert!(html.contains("<link rel=\"stylesheet\" href=\"/assets/base.css\">"), "{html}");
    assert!(html.contains("<link rel=\"modulepreload\" href=\"/assets/app.js\">"), "{html}");

    let seq0_pos = html
        .find("<script type=\"application/x-nve-query\" data-nve-seq=\"0\">")
        .expect("seq=0 query payload present");
    let ack0_pos = html
        .find("<template data-nve-q-ack=\"0\"></template>")
        .expect("ack=0 present");
    let end_pos = html
        .find("<template data-nve-stream-end></template>")
        .expect("stream-end present");
    let boot_pos = html
        .find("<script type=\"module\" async src=\"/assets/app.js\"></script>")
        .expect("async boot script present");
    assert!(seq0_pos < ack0_pos && ack0_pos < end_pos && end_pos < boot_pos);

    // Client hydration of the Light-DOM fast path: 0 mismatches, 0 RPCs!
    let cli_transport = ControlledTransport::default();
    let cli_client = erased_query_client_with_clock(
        cli_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let hydrated = hydrate_client_from_html(&cli_client, &html);
    assert_eq!(hydrated, vec![(0, 1, 1)]);

    let mut cli_web = WebBackend::hydrate_from_streaming_html(&html);
    let mut cli_surface = DioxusSurface::new(10, FastApp).with_context(cli_client);
    cli_surface.flush_into(1, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(rep.mismatches.is_empty(), "fast path mismatches: {:?}", rep.mismatches);
    assert_eq!(rep.replaced_nodes, 0);
    assert_eq!(cli_transport.call_count(), 0);

    // Also verify 0-query static page omits `<script type="application/x-nve-query">` yet keeps `<template data-nve-stream-end>`.
    #[component]
    fn StaticApp() -> Element {
        rsx! { view { text { "Static only" } } }
    }
    let static_client = erased_ssr_query_client(ControlledTransport::default());
    let static_resp = pool
        .render_streaming(
            SsrPageOptions::new("Static").with_status(StatusCode::NOT_FOUND),
            StreamPlan::default(),
            static_client,
            |_| DioxusSurface::new(2, StaticApp),
        )
        .await
        .unwrap();
    assert_eq!(static_resp.status(), StatusCode::NOT_FOUND);
    assert!(matches!(static_resp.body(), HttpBody::Full(_)));
    let static_html = collect_all_html(static_resp.into_body()).await;
    assert!(!static_html.contains("application/x-nve-query"));
    assert!(static_html.contains("<template data-nve-stream-end></template>"));
}

// ---------------------------------------------------------------------------
// Test 2: Out-of-order streaming + mid-stream & post-stream hydration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_2_out_of_order_streaming_css_nveq_dedup_and_mid_and_post_hydration() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(2))
        .expect("pool init");

    let transport = ControlledTransport::default();
    transport.set_ok("stats", "42 open issues");
    transport.set_ok("comments", "99 comments");
    transport.gate("stats");
    transport.gate("comments");

    let ssr_client = erased_ssr_query_client(transport.clone());

    fn shell_style() -> TypedStyle {
        let mut ts = TypedStyle::new();
        ts.style = StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::WHITE,
            ..Default::default()
        };
        ts.has_style = true;
        ts
    }

    fn delta_style() -> TypedStyle {
        let mut ts = TypedStyle::new();
        ts.style = StyleProps {
            background: ColorRgba::rgb(22, 163, 74),
            border_radius: 12.0,
            ..Default::default()
        };
        ts.has_style = true;
        ts
    }

    #[component]
    fn StyledPanel(panel_key: &'static str, use_delta_style: bool) -> Element {
        let q = use_query_with::<GetPanel>(
            panel_key.to_owned(),
            QueryOptions::default().stale_time_ms(60_000).max_retries(0),
        );
        let data = q.suspend()?;
        let style_attr = if use_delta_style {
            delta_style()
        } else {
            shell_style()
        };
        rsx! {
            view {
                nve_style: style_attr,
                text { "{data.key}:{data.body}" }
            }
        }
    }

    #[component]
    fn DashboardApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let hdr_style = shell_style();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    text { nve_style: hdr_style, "Dashboard Shell" }
                    QuerySuspense {
                        id: Some("comments".into()),
                        fallback: rsx! { text { "Loading comments..." } },
                        // Reuses `delta_style` (already introduced by `stats`!) -> 0 duplicate CSS!
                        StyledPanel { panel_key: "comments", use_delta_style: true }
                    }
                    QuerySuspense {
                        id: Some("stats".into()),
                        fallback: rsx! { text { "Loading stats..." } },
                        // Introduces `delta_style` first!
                        StyledPanel { panel_key: "stats", use_delta_style: true }
                    }
                }
            }
        }
    }

    let nonce = generate_csp_nonce().unwrap();
    let page = SsrPageOptions::new("Streaming Dashboard")
        .with_style_nonce(nonce.clone())
        .with_modulepreload_src("/app.js")
        .with_async_boot_src("/app.js");

    let resp = pool
        .render_streaming(page, StreamPlan::default(), ssr_client, |c| {
            DioxusSurface::new(1, DashboardApp).with_context(c)
        })
        .await
        .expect("streaming response headers");

    assert_eq!(resp.status(), StatusCode::OK);
    assert!(matches!(resp.body(), HttpBody::Stream(_)));
    for (k, v) in streaming_http_headers() {
        assert_eq!(
            resp.headers()
                .get(k)
                .unwrap_or_else(|| panic!("missing streaming header {k}"))
                .to_str()
                .unwrap(),
            v
        );
    }

    let mut body = resp.into_body();

    // Chunk 0: Initial DSD shell + seq=0 (2 StreamPending) + async boot script arrives immediately!
    let shell_chunk = tokio::time::timeout(Duration::from_secs(2), read_next_chunk(&mut body))
        .await
        .expect("shell chunk must arrive immediately")
        .expect("shell chunk present");

    assert!(shell_chunk.contains("<div data-nve-dsd-host>"), "{shell_chunk}");
    assert!(shell_chunk.contains("<template shadowrootmode=\"open\">"), "{shell_chunk}");
    assert!(shell_chunk.contains("<slot name=\"nve-slot-comments\">"), "{shell_chunk}");
    assert!(shell_chunk.contains("<slot name=\"nve-slot-stats\">"), "{shell_chunk}");
    assert!(shell_chunk.contains(&format!("nonce=\"{nonce}\"")), "{shell_chunk}");
    let ack0_pos = shell_chunk
        .find("<template data-nve-q-ack=\"0\"></template>")
        .expect("seq=0 ack in shell");
    let boot_pos = shell_chunk
        .find("<script type=\"module\" async src=\"/app.js\"></script>")
        .expect("async boot script in shell");
    assert!(ack0_pos < boot_pos, "async boot script must strictly follow ack=0");
    assert!(!shell_chunk.contains("42 open issues"));
    assert!(!shell_chunk.contains("99 comments"));

    // Release ONLY `stats` (declared SECOND in DOM!) -> arrives out-of-order BEFORE `comments`!
    transport.release("stats");
    let stats_chunk = tokio::time::timeout(Duration::from_secs(2), read_next_chunk(&mut body))
        .await
        .expect("stats chunk must stream before comments resolves")
        .expect("stats chunk present");

    assert!(stats_chunk.contains("stats:42 open issues"), "{stats_chunk}");
    assert!(!stats_chunk.contains("99 comments"), "comments must still be pending");
    let q1_pos = stats_chunk
        .find("<script type=\"application/x-nve-query\" data-nve-seq=\"1\" data-nve-slot-for=\"stats\">")
        .expect("seq=1 NVEQ in stats chunk, bound to the stats slot");
    let ack1_pos = stats_chunk
        .find("<template data-nve-q-ack=\"1\"></template>")
        .expect("ack=1 in stats chunk");
    let slot1_pos = stats_chunk
        .find("<div slot=\"nve-slot-stats\" data-nve-slot=\"stats\" class=\"nve-slot\">")
        .expect("stats slot div in stats chunk");
    assert!(
        q1_pos < ack1_pos && ack1_pos < slot1_pos,
        "NVEQ + ack must strictly precede <div slot> in same atomic chunk"
    );
    // New delta CSS rule emitted once for `delta_style` (#16a34a), NOT re-emitting `shell_style` (#0f172a)!
    assert!(stats_chunk.contains("data-nve-ssr-chunk=\"stats\""));
    assert!(stats_chunk.contains("#16a34a"), "{stats_chunk}");
    assert!(!stats_chunk.contains("#0f172a"), "shell CSS must not duplicate: {stats_chunk}");

    // --- MID-STREAM HYDRATION (shell + stats_chunk, comments STILL pending!) ---
    let mid_html = format!("{shell_chunk}{stats_chunk}");
    let mid_transport = ControlledTransport::default();
    let mid_client = erased_query_client_with_clock(
        mid_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let mid_hydrated = hydrate_client_from_html(&mid_client, &mid_html);
    assert_eq!(
        mid_hydrated,
        vec![(0, 2, 2), (1, 1, 1)],
        "seq=0 has 2 StreamPending, seq=1 has 1 resolved stats entry"
    );

    let mut mid_web = WebBackend::hydrate_from_streaming_html(&mid_html);
    let mut mid_surface = DioxusSurface::new(20, DashboardApp).with_context(mid_client);
    mid_surface.flush_into(1, &mut mid_web).unwrap();
    let mid_rep = mid_web.hydration_report();
    assert!(
        mid_rep.mismatches.is_empty(),
        "mid-stream hydration must have 0 mismatches: {:?}",
        mid_rep.mismatches
    );
    assert_eq!(mid_rep.replaced_nodes, 0);
    assert_eq!(
        mid_web.stylesheet().flushed_css_rules().len(),
        0,
        "mid-stream hydration must emit 0 duplicate CSS rules"
    );
    assert_eq!(
        mid_transport.call_count(),
        0,
        "mid-stream boot must fire 0 client RPCs while comments is StreamPending"
    );

    // Release `comments` -> arrives second, reuses `delta_style` so emits ZERO `<style data-nve-ssr-chunk>`!
    transport.release("comments");
    let comments_chunk = tokio::time::timeout(Duration::from_secs(2), read_next_chunk(&mut body))
        .await
        .expect("comments chunk must arrive after release")
        .expect("comments chunk present");

    assert!(comments_chunk.contains("comments:99 comments"), "{comments_chunk}");
    let q2_pos = comments_chunk
        .find("<script type=\"application/x-nve-query\" data-nve-seq=\"2\" data-nve-slot-for=\"comments\">")
        .expect("seq=2 NVEQ in comments chunk");
    let ack2_pos = comments_chunk
        .find("<template data-nve-q-ack=\"2\"></template>")
        .expect("ack=2 in comments chunk");
    let slot2_pos = comments_chunk
        .find("<div slot=\"nve-slot-comments\" data-nve-slot=\"comments\" class=\"nve-slot\">")
        .expect("comments slot div");
    assert!(q2_pos < ack2_pos && ack2_pos < slot2_pos);
    assert!(
        !comments_chunk.contains("data-nve-ssr-chunk"),
        "already-emitted delta_style must NOT duplicate in comments chunk: {comments_chunk}"
    );

    // Final trailer chunk closes stream cleanly.
    let trailer_chunk = tokio::time::timeout(Duration::from_secs(2), read_next_chunk(&mut body))
        .await
        .expect("trailer chunk")
        .expect("trailer chunk present");
    assert_eq!(
        trailer_chunk,
        "<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n"
    );
    assert!(read_next_chunk(&mut body).await.is_none(), "stream must hit EOF");

    // --- POST-STREAM HYDRATION ---
    let full_html = format!("{shell_chunk}{stats_chunk}{comments_chunk}{trailer_chunk}");
    let post_transport = ControlledTransport::default();
    let post_client = erased_query_client_with_clock(
        post_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    let post_hydrated = hydrate_client_from_html(&post_client, &full_html);
    assert_eq!(post_hydrated, vec![(0, 2, 2), (1, 1, 1), (2, 1, 1)]);

    let mut post_web = WebBackend::hydrate_from_streaming_html(&full_html);
    let mut post_surface = DioxusSurface::new(30, DashboardApp).with_context(post_client);
    post_surface.flush_into(1, &mut post_web).unwrap();
    let post_rep = post_web.hydration_report();
    assert!(
        post_rep.mismatches.is_empty(),
        "post-stream hydration must have 0 mismatches: {:?}",
        post_rep.mismatches
    );
    assert_eq!(post_rep.replaced_nodes, 0);
    assert_eq!(post_web.stylesheet().flushed_css_rules().len(), 0);
    assert_eq!(post_transport.call_count(), 0);
    assert_eq!(transport.call_count(), 2, "SSR issued exactly 2 RPCs total");
}

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
                    SsrPageOptions::new(format!("Profile-{}", principal.name)),
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
            assert!(html.contains("USER=alice|SECRET=ALICE_TOP_SECRET_777"), "{html}");
            assert!(!html.contains("BOB_TOP_SECRET_888"), "Bob leaked into Alice HTML: {html}");
            assert_eq!(hydrated_profile.user, "alice");
            assert_eq!(hydrated_profile.secret, "ALICE_TOP_SECRET_777");
        } else {
            assert!(html.contains("USER=bob|SECRET=BOB_TOP_SECRET_888"), "{html}");
            assert!(!html.contains("ALICE_TOP_SECRET_777"), "Alice leaked into Bob HTML: {html}");
            assert_eq!(hydrated_profile.user, "bob");
            assert_eq!(hydrated_profile.secret, "BOB_TOP_SECRET_888");
        }
    }
}

// ---------------------------------------------------------------------------
// Test 5: Post-headers DomainErr streams ErrorBoundary fallback + NVEQ DomainErr
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_5_post_headers_domain_err_streams_error_boundary_and_hydrates_cleanly() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let transport = ControlledTransport::default();
    transport.set_domain_err("secret-panel", "insufficient role");
    transport.gate("secret-panel");

    #[component]
    fn DomainErrApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    text { "Account" }
                    QuerySuspense {
                        id: Some("secret-panel".into()),
                        fallback: rsx! { text { "Loading panel..." } },
                        error_fallback: move |report: UiErrorReport, _reset: ErrorBoundaryReset| rsx! {
                            view {
                                text { "DOMAIN_ERR_UI:{report.message}" }
                            }
                        },
                        DomainErrPanel {}
                    }
                }
            }
        }
    }

    #[component]
    fn DomainErrPanel() -> Element {
        let q = use_query_with::<GetPanel>(
            "secret-panel".to_owned(),
            QueryOptions::default().stale_time_ms(60_000).max_retries(0),
        );
        let data = q.suspend()?;
        rsx! { text { "OK:{data.body}" } }
    }

    let ssr_client = erased_ssr_query_client(transport.clone());
    let resp = pool
        .render_streaming(
            SsrPageOptions::new("DomainErr Stream"),
            StreamPlan::default(),
            ssr_client,
            |c| DioxusSurface::new(1, DomainErrApp).with_context(c),
        )
        .await
        .expect("headers sent before DomainErr resolves");

    assert_eq!(resp.status(), StatusCode::OK);
    let mut body = resp.into_body();

    let shell = read_next_chunk(&mut body).await.unwrap();
    assert!(shell.contains("<slot name=\"nve-slot-secret-panel\">"));

    // Resolve with DomainErr post-headers!
    transport.release("secret-panel");
    let err_chunk = read_next_chunk(&mut body).await.unwrap();
    let trailer = read_next_chunk(&mut body).await.unwrap();
    assert!(read_next_chunk(&mut body).await.is_none());

    assert!(err_chunk.contains("DOMAIN_ERR_UI:"), "{err_chunk}");
    assert!(err_chunk.contains("insufficient role"), "{err_chunk}");
    let q1 = err_chunk.find("data-nve-seq=\"1\"").expect("seq=1 DomainErr NVEQ");
    let slot = err_chunk
        .find("<div slot=\"nve-slot-secret-panel\"")
        .expect("resolved error slot");
    assert!(q1 < slot);

    // Hydrate into a fresh Client -> 0 mismatches, 0 duplicate RPCs, same ErrorBoundary fallback!
    let full_html = format!("{shell}{err_chunk}{trailer}");
    let cli_transport = ControlledTransport::default();
    let cli_client = erased_query_client_with_clock(
        cli_transport.clone(),
        Shared::new(ManualClock::new(2_000)),
        QueryClientConfig {
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );
    let hydrated = hydrate_client_from_html(&cli_client, &full_html);
    assert_eq!(hydrated, vec![(0, 1, 1), (1, 1, 1)]);

    let mut cli_web = WebBackend::hydrate_from_streaming_html(&full_html);
    let mut cli_surface = DioxusSurface::new(50, DomainErrApp).with_context(cli_client.clone());
    let mut frame = 1u64;
    cli_surface.flush_into(frame, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(rep.mismatches.is_empty(), "DomainErr mismatches: {:?}", rep.mismatches);
    assert_eq!(rep.replaced_nodes, 0);
    pump_client_settled(&mut cli_surface, &mut cli_web, &mut frame);
    let cli_html = cli_web.render_html();
    assert!(cli_html.contains("DOMAIN_ERR_UI:") && cli_html.contains("insufficient role"));
    assert_eq!(
        cli_transport.call_count(),
        0,
        "hydrated DomainErr must fire 0 duplicate client RPCs"
    );
}

// ---------------------------------------------------------------------------
// Test 6: total_deadline expiry + client finish_ssr_stream() recovery
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_6_total_deadline_expiry_emits_stream_end_and_client_recovers() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");
    let ssr_transport = ControlledTransport::default();
    ssr_transport.set_ok("fast", "Fast Ready");
    let (started_tx, _started_rx) = oneshot::channel();
    let (dropped_tx, _dropped_rx) = oneshot::channel();
    ssr_transport.hang_with_drop_guard("slow", started_tx, dropped_tx);

    #[component]
    fn DeadlinePanel(panel_key: &'static str) -> Element {
        let q = use_query_with::<GetPanel>(
            panel_key.to_owned(),
            QueryOptions::default().stale_time_ms(60_000).max_retries(0),
        );
        let data = q.suspend()?;
        rsx! { text { "READY:{data.body}" } }
    }

    #[component]
    fn DeadlineApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    QuerySuspense {
                        id: Some("fast".into()),
                        fallback: rsx! { text { "WAIT_FAST" } },
                        DeadlinePanel { panel_key: "fast" }
                    }
                    QuerySuspense {
                        id: Some("slow".into()),
                        fallback: rsx! { text { "WAIT_SLOW" } },
                        DeadlinePanel { panel_key: "slow" }
                    }
                }
            }
        }
    }

    let plan = StreamPlan::default()
        .with_total_deadline(Duration::from_millis(90))
        .with_keepalive_interval(Duration::from_millis(30));

    let t0 = Instant::now();
    let resp = pool
        .render_streaming(
            SsrPageOptions::new("Deadline Recovery"),
            plan,
            erased_ssr_query_client(ssr_transport),
            |c| DioxusSurface::new(1, DeadlineApp).with_context(c),
        )
        .await
        .expect("headers ok");

    let full_html = collect_all_html(resp.into_body()).await;
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "stream must close promptly at total_deadline"
    );
    assert!(full_html.contains("READY:Fast Ready"), "{full_html}");
    assert!(full_html.contains("WAIT_SLOW"), "{full_html}");
    assert!(!full_html.contains("<div slot=\"nve-slot-slow\""), "{full_html}");
    assert!(full_html.contains(NVE_STREAM_KEEPALIVE_COMMENT), "keepalive should tick before deadline");
    assert!(
        full_html.contains("<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n"),
        "deadline expiry must emit clean stream-end trailer: {full_html}"
    );

    // Client hydrates the deadline-truncated stream: 0 mismatches while `slow` stays StreamPending.
    let cli_transport = ControlledTransport::default();
    cli_transport.set_ok("slow", "Recovered On Client");
    let cli_client = erased_query_client_with_clock(
        cli_transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    hydrate_client_from_html(&cli_client, &full_html);

    let mut cli_web = WebBackend::hydrate_from_streaming_html(&full_html);
    let mut cli_surface = DioxusSurface::new(60, DeadlineApp).with_context(cli_client.clone());
    let mut frame = 1u64;
    cli_surface.flush_into(frame, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(rep.mismatches.is_empty(), "deadline hydration mismatches: {:?}", rep.mismatches);
    assert_eq!(cli_transport.call_count(), 0);

    // Client detects `<template data-nve-stream-end>` and calls `finish_ssr_stream()` ->
    // recovers ONLY `slow` via client Transport (1 RPC, NOT re-fetching `fast`!)
    assert_eq!(cli_client.finish_ssr_stream(), 1);
    pump_client_settled(&mut cli_surface, &mut cli_web, &mut frame);
    let recovered = cli_web.render_html();
    assert!(
        recovered.contains("READY:Fast Ready") && recovered.contains("READY:Recovered On Client"),
        "finish_ssr_stream must recover timed-out slow slot: {recovered}"
    );
    assert_eq!(
        cli_transport.call_count(),
        1,
        "only the timed-out slow slot should fetch on client"
    );
}

// ---------------------------------------------------------------------------
// Test 7: Adversarial XSS escaping & lossless NVEQ v2 round-trip
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_7_adversarial_xss_payloads_safely_escaped_and_round_tripped() {
    const XSS_SCRIPT: &str = "</script><script>alert(1)</script>";
    const XSS_STYLE: &str = "</style><img onerror=1>";
    const XSS_DSD: &str = "<template shadowrootmode=\"open\">pwn</template>";
    const XSS_NUL: &str = "before\0after";

    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");

    #[component]
    fn XssApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    QuerySuspense {
                        id: Some("xss".into()),
                        fallback: rsx! { text { "Loading..." } },
                        XssPanel {}
                    }
                }
            }
        }
    }

    #[component]
    fn XssPanel() -> Element {
        let q1 = use_query_with::<GetPanel>("s".into(), QueryOptions::default().max_retries(0));
        let q2 = use_query_with::<GetPanel>("st".into(), QueryOptions::default().max_retries(0));
        let q3 = use_query_with::<GetPanel>("t".into(), QueryOptions::default().max_retries(0));
        let q4 = use_query_with::<GetPanel>("n".into(), QueryOptions::default().max_retries(0));
        let d1 = q1.suspend()?;
        let d2 = q2.suspend()?;
        let d3 = q3.suspend()?;
        let d4 = q4.suspend()?;
        rsx! {
            view {
                text { "{d1.body}" }
                text { "{d2.body}" }
                text { "{d3.body}" }
                text { "{d4.body}" }
            }
        }
    }

    let adversarial_page = || {
        SsrPageOptions::new("</title><script>alert(2)</script>")
            .with_style_nonce("n\" onload=\"alert(1)<>&'")
            .with_modulepreload_src("/m.js?a=\"'<>")
            .with_async_boot_src("/b.js?x=\"'<>")
            .with_stylesheet_link("/s.css?q=\"'<>")
    };

    // Check BOTH Fast Path (pre-seeded) and Streaming Path (hybrid sync+async waterfall where
    // `s` resolves immediately, `st` suspends asynchronously, `t` resolves immediately, and `n`
    // suspends asynchronously — exercising both XSS escaping AND Dioxus Suspense re-suspension!).
    for streaming in [false, true] {
        let transport = ControlledTransport::default();
        transport.set_ok("s", XSS_SCRIPT);
        transport.set_ok("st", XSS_STYLE);
        transport.set_ok("t", XSS_DSD);
        transport.set_ok("n", XSS_NUL);
        if streaming {
            // Hybrid waterfall: "s" resolves synchronously during Frame 1, then "st" suspends
            // asynchronously (forcing SuspenseBoundary to restore `<NveStreamSlot>` before headers!),
            // then "t" resolves synchronously and "n" suspends asynchronously mid-stream.
            transport.gate("st");
            transport.gate("n");
        }

        let client = erased_ssr_query_client(transport.clone());
        if !streaming {
            for (k, v) in [
                ("s", XSS_SCRIPT),
                ("st", XSS_STYLE),
                ("t", XSS_DSD),
                ("n", XSS_NUL),
            ] {
                client
                    .set_query_data::<GetPanel>(
                        &k.to_owned(),
                        PanelData {
                            key: k.into(),
                            body: v.into(),
                        },
                    )
                    .unwrap();
            }
        }

        let resp = pool
            .render_streaming(adversarial_page(), StreamPlan::default(), client, |c| {
                DioxusSurface::new(1, XssApp).with_context(c)
            })
            .await
            .expect("adversarial render");
        assert_eq!(matches!(resp.body(), HttpBody::Stream(_)), streaming);
        if streaming {
            transport.release("st");
            transport.release("n");
        }
        let html = collect_all_html(resp.into_body()).await;

        assert!(!html.contains("<script>alert(1)"), "script XSS leaked ({streaming}): {html}");
        assert!(!html.contains("<script>alert(2)"), "title XSS leaked ({streaming}): {html}");
        assert!(!html.contains("<img onerror=1>"), "style/img XSS leaked ({streaming}): {html}");
        assert!(
            !html.contains("<template shadowrootmode=\"open\">pwn"),
            "DSD template injection leaked ({streaming}): {html}"
        );
        assert!(
            !html.contains("nonce=\"n\" onload="),
            "style_nonce broke out of attribute ({streaming}): {html}"
        );
        assert!(
            html.contains("href=\"/m.js?a=&quot;&#39;&lt;&gt;\""),
            "modulepreload_src must be attribute-escaped ({streaming}): {html}"
        );
        assert!(
            html.contains("src=\"/b.js?x=&quot;") && !html.contains("src=\"/b.js?x=\""),
            "async_boot_src must be attribute-escaped ({streaming}): {html}"
        );

        // Verify lossless byte-for-byte NVEQ v2 round-trip of ALL 4 adversarial payloads!
        let verify_client = erased_query_client(ControlledTransport::default());
        let hydrated_stats = hydrate_client_from_html(&verify_client, &html);
        for (k, expected) in [
            ("s", XSS_SCRIPT),
            ("st", XSS_STYLE),
            ("t", XSS_DSD),
            ("n", XSS_NUL),
        ] {
            let h = verify_client
                .subscribe::<GetPanel>(&k.to_owned(), QueryOptions::default())
                .unwrap();
            let snap = verify_client.read::<GetPanel>(h);
            let got = snap.data.unwrap_or_else(|| {
                panic!(
                    "missing round-tripped XSS payload for streaming={streaming}, key={k}, hydrated={hydrated_stats:?}, html={html}"
                )
            });
            assert_eq!(got.body, expected, "lossless NVEQ round-trip for key={k}");
        }
    }
}

// ---------------------------------------------------------------------------
// Test 8: DSD placement hygiene fails closed BEFORE headers (<form> & <table>)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_8_dsd_placement_hygiene_fails_before_headers() {
    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .expect("pool init");

    #[component]
    fn InsideFormApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    "semanticTag": "form",
                    QuerySuspense {
                        id: Some("bad-form-slot".into()),
                        fallback: rsx! { text { "Loading..." } },
                        SlowPanelInner {}
                    }
                }
            }
        }
    }

    let transport1 = ControlledTransport::default();
    transport1.gate("slow");
    let err_form = pool
        .render_streaming(
            SsrPageOptions::new("Bad Form"),
            StreamPlan::default(),
            erased_ssr_query_client(transport1),
            |c| DioxusSurface::new(1, InsideFormApp).with_context(c),
        )
        .await
        .expect_err("slot inside <form> must fail BEFORE headers");
    assert!(
        matches!(
            err_form,
            SsrError::Streaming(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("<form>")
        ),
        "expected InvalidSlotPlacement(<form>), got {err_form:?}"
    );

    #[component]
    fn InsideTableApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                // Place `NveStreamSlot` directly inside `<table>` (HTML5 foster-parenting trap!).
                view {
                    "semanticTag": "table",
                    NveStreamSlot {
                        slot_id: "bad-table-slot".to_owned(),
                        text { "Loading row..." }
                    }
                }
            }
        }
    }

    let err_table = pool
        .render_streaming(
            SsrPageOptions::new("Bad Table"),
            StreamPlan::default(),
            erased_ssr_query_client(ControlledTransport::default()),
            |c| DioxusSurface::new(1, InsideTableApp).with_context(c),
        )
        .await
        .expect_err("slot directly inside <table> must fail BEFORE headers");
    assert!(
        matches!(
            err_table,
            SsrError::Streaming(StreamingError::InvalidSlotPlacement(ref msg)) if msg.contains("table")
        ),
        "expected InvalidSlotPlacement(table), got {err_table:?}"
    );
}

// ---------------------------------------------------------------------------
// Test 9: Nonce CSPRNG and non-blocking Drop drain (backpressure: `ssr_hardening_tests.rs`)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_9_csp_nonce_and_clean_drop() {
    // 1. CSPRNG nonce: 16 bytes -> 24 Standard Base64 chars, unique across calls.
    let n1 = generate_csp_nonce().unwrap();
    let n2 = generate_csp_nonce().unwrap();
    assert_eq!(n1.len(), 24);
    assert_eq!(BASE64_STANDARD.decode(&n1).unwrap().len(), 16);
    assert_ne!(n1, n2);

    // 2. Clean non-blocking Drop: drop the pool INSIDE an async runtime while an active streaming
    //    response is still in flight -> Drop does not block, and the in-flight stream finishes!
    let ephem_pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1))
        .unwrap();
    let transport = ControlledTransport::default();
    transport.set_ok("slow", "Finished After Pool Drop");
    transport.gate("slow");

    let resp = ephem_pool
        .render_streaming(
            SsrPageOptions::new("Drop Drain"),
            StreamPlan::default(),
            erased_ssr_query_client(transport.clone()),
            |c| DioxusSurface::new(1, SlowPanelApp).with_context(c),
        )
        .await
        .unwrap();

    // Drop the last pool handle right here inside async context!
    let drop_t0 = Instant::now();
    drop(ephem_pool);
    assert!(
        drop_t0.elapsed() < Duration::from_millis(50),
        "SsrLocalPool::drop must never block inside async context"
    );

    transport.release("slow");
    let html = collect_all_html(resp.into_body()).await;
    assert!(
        html.contains("Finished After Pool Drop") && html.contains("data-nve-stream-end"),
        "already-dispatched stream must drain cleanly after pool drop: {html}"
    );
}
