//! Slice 1c `nve-ssr` integration scenarios:
//! 5. Post-headers `DomainErr` inside `<QuerySuspense>` streaming `<ErrorBoundary>` fallback +
//!    `DomainErr` `NVEQ` v2 entry with zero mismatches and zero duplicate client RPCs.
//! 6. `total_deadline` expiry emitting `<template data-nve-stream-end>` cleanly and client
//!    recovery via `QueryClient::finish_ssr_stream()`.
//! 7. Adversarial XSS payloads across query data and `SsrPageOptions` attributes safely escaped
//!    and round-tripping byte-for-byte in `NVEQ` v2.

mod support;

use core::time::Duration;
use std::time::Instant;

use http::StatusCode;
use nve_backend_web::{NVE_STREAM_KEEPALIVE_COMMENT, WebBackend};
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_http_hyper::HttpBody;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};
use tokio::sync::oneshot;

use support::*;

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
    let q1 = err_chunk
        .find("data-nve-seq=\"1\"")
        .expect("seq=1 DomainErr NVEQ");
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
    cli_surface.commit_to(frame, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(
        rep.mismatches.is_empty(),
        "DomainErr mismatches: {:?}",
        rep.mismatches
    );
    assert_eq!(rep.replaced_nodes, 0);
    pump_client_settled(&mut cli_surface, &mut cli_web, &mut frame);
    let cli_html = cli_web.render_html(cli_surface.store());
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
    assert!(
        !full_html.contains("<div slot=\"nve-slot-slow\""),
        "{full_html}"
    );
    assert!(
        full_html.contains(NVE_STREAM_KEEPALIVE_COMMENT),
        "keepalive should tick before deadline"
    );
    assert!(
        full_html
            .contains("<template data-nve-stream-end></template></div></div>\n</body>\n</html>\n"),
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
    cli_surface.commit_to(frame, &mut cli_web).unwrap();
    let rep = cli_web.hydration_report();
    assert!(
        rep.mismatches.is_empty(),
        "deadline hydration mismatches: {:?}",
        rep.mismatches
    );
    assert_eq!(cli_transport.call_count(), 0);

    // Client detects `<template data-nve-stream-end>` and calls `finish_ssr_stream()` ->
    // recovers ONLY `slow` via client Transport (1 RPC, NOT re-fetching `fast`!)
    assert_eq!(cli_client.finish_ssr_stream(), 1);
    pump_client_settled(&mut cli_surface, &mut cli_web, &mut frame);
    let recovered = cli_web.render_html(cli_surface.store());
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

        assert!(
            !html.contains("<script>alert(1)"),
            "script XSS leaked ({streaming}): {html}"
        );
        assert!(
            !html.contains("<script>alert(2)"),
            "title XSS leaked ({streaming}): {html}"
        );
        assert!(
            !html.contains("<img onerror=1>"),
            "style/img XSS leaked ({streaming}): {html}"
        );
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
