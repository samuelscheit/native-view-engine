//! Round-1 review hardening guards for `nve-ssr`:
//! 1. Streaming `enable_event_replay` emits `<script data-nve-replay>` only after `<div id="app">`.
//! 2. `<QuerySuspense>` (not just raw `NveStreamSlot`) under `<table>` / `<tbody>` / `<tr>` /
//!    `<select>` fails with `InvalidSlotPlacement` before headers.
//! 3. Real active-render backpressure: N held streaming renders saturate a worker, the N+1th gets
//!    `PoolOverloaded`, and a render slot frees as soon as one stream disconnects or finishes.
//! 4. Client disconnect cancels the `LocalSet` render with keepalives disabled.
//! 5. A suspense parent unmounting mid-stream ends the stream immediately instead of at
//!    `total_deadline`.
//! 6. A panicking render task maps to `SsrError::Render` (HTTP 500), never `PoolClosed` (503),
//!    and leaves the worker serving later renders.

use core::time::Duration;
use std::cell::RefCell;
use std::rc::Rc;
use std::time::Instant;

use http::Response;
use nve_backend_web::StreamingError;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_http_hyper::HttpBody;
use nve_query_dioxus::prelude::*;
use nve_ssr::{SsrError, SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};
use tokio::sync::oneshot;

mod support;

use support::*;

fn single_worker_pool(capacity: usize) -> SsrLocalPool {
    SsrLocalPool::with_config(
        SsrLocalPoolConfig::default()
            .with_worker_count(1)
            .with_queue_capacity_per_worker(capacity),
    )
    .expect("pool init")
}

/// Starts a streaming `SlowPanelApp` render whose `"slow"` query stays gated on `transport`.
async fn render_slow(
    pool: &SsrLocalPool,
    transport: &ControlledTransport,
    page: SsrPageOptions,
) -> Result<Response<HttpBody>, SsrError> {
    pool.render_streaming(
        page,
        StreamPlan::default().with_total_deadline(Duration::from_secs(10)),
        erased_ssr_query_client(transport.clone()),
        |c| DioxusSurface::new(1, SlowPanelApp).with_context(c),
    )
    .await
}

/// Retries a dispatch until the worker's freed render slot is observable (the permit drops on the
/// worker thread right after the response future returns).
async fn render_once_slot_frees(
    pool: &SsrLocalPool,
    transport: &ControlledTransport,
) -> Response<HttpBody> {
    tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match render_slow(pool, transport, SsrPageOptions::new("retry")).await {
                Err(SsrError::PoolOverloaded) => tokio::time::sleep(Duration::from_millis(2)).await,
                other => return other.expect("render after a slot freed"),
            }
        }
    })
    .await
    .expect("a render slot must free promptly")
}

#[tokio::test]
async fn streaming_event_replay_script_runs_only_after_app_container_opens() {
    let pool = single_worker_pool(4);
    let transport = ControlledTransport::default();
    transport.gate("slow");
    let mut resp = render_slow(
        &pool,
        &transport,
        SsrPageOptions::new("Replay").with_enable_event_replay(true),
    )
    .await
    .expect("streaming response");
    let initial = read_next_chunk(resp.body_mut()).await.expect("initial chunk");

    let head_end = initial.find("</head>").expect("</head>");
    let app = initial.find("<div id=\"app\"").expect("#app");
    let replay = initial.find("<script data-nve-replay>").expect("replay script");
    assert_eq!(initial.matches("<script data-nve-replay>").count(), 1, "{initial}");
    assert!(
        head_end < app && app < replay,
        "replay must follow the opened <div id=\"app\"> (getElementById('app') is null in <head>): {initial}"
    );
    let seq0 = initial
        .find("data-nve-seq=\"0\"")
        .expect("seq=0 payload");
    assert!(replay < seq0, "replay installs before the boot payload: {initial}");

    transport.release("slow");
    let html = collect_all_html(resp.into_body()).await;
    assert!(html.contains("data-nve-stream-end"), "{html}");
}

#[tokio::test]
async fn query_suspense_under_table_rows_or_select_fails_before_headers() {
    #[derive(Clone, Copy)]
    struct Trap(&'static [&'static str]);

    #[component]
    fn Nest(tags: &'static [&'static str]) -> Element {
        match tags.split_first() {
            Some((tag, rest)) => rsx! {
                view {
                    "semanticTag": *tag,
                    Nest { tags: rest }
                }
            },
            None => rsx! {
                QuerySuspense {
                    id: Some("trap".into()),
                    fallback: rsx! { text { "Loading..." } },
                    SlowPanelInner {}
                }
            },
        }
    }

    #[component]
    fn TrapApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let trap = dioxus_core::consume_context::<Trap>();
        rsx! {
            QueryClientProvider {
                client: client,
                Nest { tags: trap.0 }
            }
        }
    }

    let pool = single_worker_pool(4);
    for (tags, needle) in [
        (&["table"][..], "<table>"),
        (&["table", "tbody"][..], "<tbody>"),
        (&["table", "tbody", "tr"][..], "<tr>"),
        (&["select"][..], "<select>"),
        (&["select", "p"][..], "<select>"),
    ] {
        let transport = ControlledTransport::default();
        transport.gate("slow");
        let trap = Trap(tags);
        let err = pool
            .render_streaming(
                SsrPageOptions::new("Trap"),
                StreamPlan::default(),
                erased_ssr_query_client(transport),
                move |c| DioxusSurface::new(1, TrapApp).with_context(c).with_context(trap),
            )
            .await
            .expect_err("trap placement must fail before headers");
        assert!(
            matches!(
                &err,
                SsrError::Streaming(StreamingError::InvalidSlotPlacement(msg)) if msg.contains(needle)
            ),
            "<QuerySuspense> under {tags:?}: expected InvalidSlotPlacement({needle}), got {err:?}"
        );
    }
}

#[tokio::test]
async fn active_streaming_renders_hold_worker_capacity_until_disconnect_or_finish() {
    const N: usize = 2;
    let pool = single_worker_pool(N);
    assert_eq!(pool.queue_capacity_per_worker(), N);
    let transport = ControlledTransport::default();
    transport.gate("slow");

    // N concurrent streaming renders: headers arrive, bodies stay open on the gated RPC.
    let mut held = Vec::new();
    for i in 0..N {
        let mut resp = render_slow(&pool, &transport, SsrPageOptions::new(format!("held-{i}")))
            .await
            .expect("render within capacity");
        let shell = read_next_chunk(resp.body_mut()).await.expect("shell chunk");
        assert!(shell.contains("shadowrootmode"), "streaming path: {shell}");
        held.push(resp);
    }

    // N+1th: every render slot is held by an active stream (queue itself is empty).
    let err = render_slow(&pool, &transport, SsrPageOptions::new("over"))
        .await
        .expect_err("saturated worker");
    assert_eq!(err, SsrError::PoolOverloaded);

    // Client disconnect of one stream frees its slot.
    drop(held.pop());
    let replacement = render_once_slot_frees(&pool, &transport).await;
    held.push(replacement);
    assert_eq!(
        render_slow(&pool, &transport, SsrPageOptions::new("over-again"))
            .await
            .expect_err("saturated again"),
        SsrError::PoolOverloaded
    );

    // Finishing streams frees slots as well.
    transport.release("slow");
    for resp in held {
        let html = collect_all_html(resp.into_body()).await;
        assert!(html.contains("data-nve-stream-end"), "{html}");
    }
    let after = render_once_slot_frees(&pool, &transport).await;
    let html = collect_all_html(after.into_body()).await;
    assert!(html.contains("default:slow"), "{html}");
}

#[tokio::test]
async fn tcp_disconnect_cancels_render_with_keepalive_disabled() {
    assert_tcp_disconnect_cancels_in_flight_rpc(Duration::ZERO).await;
}

#[tokio::test]
async fn suspense_parent_unmounting_mid_stream_ends_stream_without_waiting_for_deadline() {
    #[derive(Clone)]
    struct UnmountSignal(Rc<RefCell<Option<oneshot::Receiver<()>>>>);

    #[component]
    fn UnmountApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let signal = dioxus_core::consume_context::<UnmountSignal>();
        let mut show = use_signal(|| true);
        use_hook(move || {
            let rx = signal.0.borrow_mut().take();
            dioxus_core::spawn(async move {
                if let Some(rx) = rx {
                    let _ = rx.await;
                }
                show.set(false);
            })
        });
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    text { "Shell" }
                    if show() {
                        QuerySuspense {
                            id: Some("vanishing".into()),
                            fallback: rsx! { text { "Loading..." } },
                            SlowPanelInner {}
                        }
                    }
                }
            }
        }
    }

    let pool = single_worker_pool(4);
    let transport = ControlledTransport::default();
    transport.gate("slow");
    let (unmount_tx, unmount_rx) = oneshot::channel::<()>();
    let mut resp = pool
        .render_streaming(
            SsrPageOptions::new("Unmount"),
            StreamPlan::default().with_total_deadline(Duration::from_secs(10)),
            erased_ssr_query_client(transport.clone()),
            move |c| {
                DioxusSurface::new(1, UnmountApp)
                    .with_context(c)
                    .with_context(UnmountSignal(Rc::new(RefCell::new(Some(unmount_rx)))))
            },
        )
        .await
        .expect("streaming response");
    let shell = read_next_chunk(resp.body_mut()).await.expect("shell");
    assert!(shell.contains("<slot name=\"nve-slot-vanishing\">"), "{shell}");

    let t0 = Instant::now();
    unmount_tx.send(()).expect("app awaits the unmount signal");
    let rest = tokio::time::timeout(Duration::from_secs(2), collect_all_html(resp.into_body()))
        .await
        .expect("stream must end right after its only pending slot unmounted");
    assert!(
        t0.elapsed() < Duration::from_secs(2),
        "stream waited {:?} on an unmounted slot",
        t0.elapsed()
    );
    assert!(rest.contains("<template data-nve-stream-end></template>"), "{rest}");
    assert!(!rest.contains("data-nve-slot=\"vanishing\""), "{rest}");
}

#[tokio::test]
async fn panicking_render_task_is_a_render_error_not_pool_closed() {
    let pool = single_worker_pool(2);
    let transport = ControlledTransport::default();
    let res = pool
        .render_streaming(
            SsrPageOptions::new("Panic"),
            StreamPlan::default(),
            erased_ssr_query_client(transport.clone()),
            |_| panic!("build_surface exploded"),
        )
        .await;
    match res {
        Err(SsrError::Render(msg)) => assert!(msg.contains("panicked"), "{msg}"),
        other => panic!("expected SsrError::Render, got {:?}", other.map(|_| ())),
    }

    // The worker survives the contained panic and serves the next render.
    let resp = render_slow(&pool, &transport, SsrPageOptions::new("After")).await;
    transport.release("slow");
    let html = collect_all_html(resp.expect("render after a panic").into_body()).await;
    assert!(html.contains("</html>"), "{html}");
}

#[tokio::test]
async fn duplicate_explicit_id_with_first_settled_pre_headers_still_streams_second_nveq_delta() {
    #[component]
    fn PanelFor(panel_key: String) -> Element {
        let q = use_query_with::<GetPanel>(
            panel_key,
            nve_query::QueryOptions::default().max_retries(0),
        );
        let data = q.suspend()?;
        rsx! { text { "{data.body}" } }
    }

    #[component]
    fn DupPreHeadersApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    // First boundary suspends on Frame 1, then settles before headers via
                    // `flush_ssr_until_quiescent`.
                    QuerySuspense {
                        id: Some("dup".into()),
                        fallback: rsx! { text { "Loading fast..." } },
                        PanelFor { panel_key: "fast".to_owned() }
                    }
                    // Second boundary shares the explicit ID and stays pending across headers.
                    QuerySuspense {
                        id: Some("dup".into()),
                        fallback: rsx! { text { "Loading slow..." } },
                        PanelFor { panel_key: "slow".to_owned() }
                    }
                }
            }
        }
    }

    let pool = single_worker_pool(4);
    let transport = ControlledTransport::default();
    transport.set_ok("fast", "panel:fast");
    transport.set_ok("slow", "panel:slow");
    transport.gate("slow");

    let mut resp = pool
        .render_streaming(
            SsrPageOptions::new("DupPreHeaders"),
            StreamPlan::default().with_total_deadline(Duration::from_secs(5)),
            erased_ssr_query_client(transport.clone()),
            |c| DioxusSurface::new(1, DupPreHeadersApp).with_context(c),
        )
        .await
        .expect("streaming response");
    let shell = read_next_chunk(resp.body_mut()).await.expect("shell");
    assert!(shell.contains("<slot name=\"nve-slot-dup\">"), "{shell}");
    assert!(shell.contains("panel:fast"), "first boundary settled pre-headers: {shell}");

    transport.release("slow");
    let rest = collect_all_html(resp.into_body()).await;
    let full = format!("{shell}{rest}");
    assert!(rest.contains("data-nve-slot-for=\"dup\""), "second boundary must emit NVEQ delta: {rest}");
    assert!(rest.contains("panel:slow"), "second boundary markup must stream: {rest}");

    // Hydrate all NVEQ chunks into a fresh client and prove `slow` settled with zero RPCs.
    let client_transport = ControlledTransport::default();
    let client = erased_query_client(client_transport.clone());
    let stats = hydrate_client_from_html(&client, &full);
    assert!(stats.len() >= 2, "expected seq=0 and seq=1 NVEQ chunks, got {stats:?}");
    let slow_h = client
        .subscribe::<GetPanel>(&"slow".to_owned(), nve_query::QueryOptions::default())
        .unwrap();
    let snap = client.read::<GetPanel>(slow_h);
    assert_eq!(
        snap.data.as_ref().map(|d| d.body.as_str()),
        Some("panel:slow"),
        "second duplicate boundary's query must be hydrated from seq=1"
    );
    assert_eq!(client_transport.call_count(), 0);
}
