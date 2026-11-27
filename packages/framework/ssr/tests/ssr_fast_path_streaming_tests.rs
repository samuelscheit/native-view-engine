//! Slice 1c `nve-ssr` integration scenarios:
//! 1. Adaptive 0-suspense Light-DOM fast path (`HttpBody::Full`, CSP nonce, headers, hydration).
//! 2. Out-of-order streaming (`stats` fast, `comments` slow) with incremental chunks, strict
//!    `NVEQ + ack` before `<div slot>`, zero duplicate CSS/NVEQ entries, and both mid-stream &
//!    post-stream `WebBackend::hydrate_from_streaming_html` (`mismatches == 0`).

mod support;

use core::time::Duration;

use http::StatusCode;
use hyper::body::Body as _;
use nve_abi::ColorRgba;
use nve_backend_web::{WebBackend, streaming_http_headers};
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_http_hyper::HttpBody;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan, generate_csp_nonce};

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
    let page = SsrPageOptions::new("Fast Path Page", web_backend)
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
        headers
            .get("x-content-type-options")
            .unwrap()
            .to_str()
            .unwrap(),
        "nosniff"
    );

    let html = collect_all_html(resp.into_body()).await;
    assert!(
        !html.contains("shadowrootmode"),
        "fast path must emit Light DOM: {html}"
    );
    assert!(
        !html.contains("<div data-nve-dsd-host"),
        "fast path must omit DSD host element: {html}"
    );
    assert!(
        !html.contains("<slot "),
        "fast path must omit shadow slots: {html}"
    );
    assert!(
        !html.contains("data-nve-replay"),
        "default CSP disables inline replay: {html}"
    );
    assert!(html.contains("HERO:Prefetched Hero"), "{html}");
    assert!(
        html.contains(&format!("<style data-nve-base nonce=\"{nonce}\">")),
        "{html}"
    );
    assert!(
        html.contains(&format!("<style data-nve-ssr nonce=\"{nonce}\"")),
        "{html}"
    );
    assert!(
        html.contains("<link rel=\"stylesheet\" href=\"/assets/base.css\">"),
        "{html}"
    );
    assert!(
        html.contains("<link rel=\"modulepreload\" href=\"/assets/app.js\">"),
        "{html}"
    );

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
    cli_surface.commit_to(1, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(
        rep.mismatches.is_empty(),
        "fast path mismatches: {:?}",
        rep.mismatches
    );
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
            SsrPageOptions::new("Static", web_backend).with_status(StatusCode::NOT_FOUND),
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
        TypedStyle::new()
            .bg(ColorRgba::rgb(15, 23, 42))
            .color(ColorRgba::WHITE)
    }

    fn delta_style() -> TypedStyle {
        TypedStyle::new()
            .bg(ColorRgba::rgb(22, 163, 74))
            .radius(12.0)
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
    let page = SsrPageOptions::new("Streaming Dashboard", web_backend)
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

    assert!(
        shell_chunk.contains("<div data-nve-dsd-host>"),
        "{shell_chunk}"
    );
    assert!(
        shell_chunk.contains("<template shadowrootmode=\"open\">"),
        "{shell_chunk}"
    );
    assert!(
        shell_chunk.contains("<slot name=\"nve-slot-comments\">"),
        "{shell_chunk}"
    );
    assert!(
        shell_chunk.contains("<slot name=\"nve-slot-stats\">"),
        "{shell_chunk}"
    );
    assert!(
        shell_chunk.contains(&format!("nonce=\"{nonce}\"")),
        "{shell_chunk}"
    );
    let ack0_pos = shell_chunk
        .find("<template data-nve-q-ack=\"0\"></template>")
        .expect("seq=0 ack in shell");
    let boot_pos = shell_chunk
        .find("<script type=\"module\" async src=\"/app.js\"></script>")
        .expect("async boot script in shell");
    assert!(
        ack0_pos < boot_pos,
        "async boot script must strictly follow ack=0"
    );
    assert!(!shell_chunk.contains("42 open issues"));
    assert!(!shell_chunk.contains("99 comments"));

    // Release ONLY `stats` (declared SECOND in DOM!) -> arrives out-of-order BEFORE `comments`!
    transport.release("stats");
    let stats_chunk = tokio::time::timeout(Duration::from_secs(2), read_next_chunk(&mut body))
        .await
        .expect("stats chunk must stream before comments resolves")
        .expect("stats chunk present");

    assert!(
        stats_chunk.contains("stats:42 open issues"),
        "{stats_chunk}"
    );
    assert!(
        !stats_chunk.contains("99 comments"),
        "comments must still be pending"
    );
    let q1_pos = stats_chunk
        .find("<script type=\"application/x-nve-query\" data-nve-seq=\"1\" data-nve-slot-for=\"stats\">")
        .expect("seq=1 NVEQ bound to the stats slot");
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
    assert!(
        !stats_chunk.contains("#0f172a"),
        "shell CSS must not duplicate: {stats_chunk}"
    );

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
    mid_surface.commit_to(1, &mut mid_web).unwrap();
    let mid_rep = mid_web.hydration_report();
    assert!(
        mid_rep.mismatches.is_empty(),
        "mid-stream hydration must have 0 mismatches: {:?}",
        mid_rep.mismatches
    );
    assert_eq!(mid_rep.replaced_nodes, 0);
    assert_eq!(
        mid_web.stylesheet().serialize_css().1,
        Vec::<String>::new(),
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

    assert!(
        comments_chunk.contains("comments:99 comments"),
        "{comments_chunk}"
    );
    let q2_pos = comments_chunk
        .find("<script type=\"application/x-nve-query\" data-nve-seq=\"2\" data-nve-slot-for=\"comments\">")
        .expect("seq=2 NVEQ bound to the comments slot");
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
    assert!(
        read_next_chunk(&mut body).await.is_none(),
        "stream must hit EOF"
    );

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
    post_surface.commit_to(1, &mut post_web).unwrap();
    let post_rep = post_web.hydration_report();
    assert!(
        post_rep.mismatches.is_empty(),
        "post-stream hydration must have 0 mismatches: {:?}",
        post_rep.mismatches
    );
    assert_eq!(post_rep.replaced_nodes, 0);
    assert_eq!(
        post_web.stylesheet().serialize_css().1,
        Vec::<String>::new()
    );
    assert_eq!(post_transport.call_count(), 0);
    assert_eq!(transport.call_count(), 2, "SSR issued exactly 2 RPCs total");
}
