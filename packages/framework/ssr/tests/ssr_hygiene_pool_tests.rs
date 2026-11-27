//! Slice 1c `nve-ssr` integration scenarios:
//! 8. DSD placement hygiene failure (`<form>` ancestor and `<table>` foster-parent trap) returning
//!    `Err(SsrError::Streaming(StreamingError::InvalidSlotPlacement(_)))` BEFORE headers.
//! 9. CSP nonce entropy, pool bounded backpressure (`PoolOverloaded`), and non-blocking `Drop`.

mod support;

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use core::time::Duration;
use std::time::Instant;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64_STANDARD;
use nve_backend_web::StreamingError;
use nve_dioxus::DioxusSurface;
use nve_dioxus::prelude::*;
use nve_query_dioxus::prelude::*;
use nve_ssr::{
    SsrError, SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan, generate_csp_nonce,
};
use nve_web_html::{HtmlSemantics, SemanticElement};

use support::*;

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
                    accessibility_role: "form",
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
            SsrPageOptions::new("Bad Form", web_backend),
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
                    nve_table: table_attr_fn(|tx, node| {
                        tx.edit::<HtmlSemantics>(node).element(SemanticElement::Table);
                    }),
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
            SsrPageOptions::new("Bad Table", web_backend),
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
// Test 9: Nonce CSPRNG, pool backpressure, and non-blocking Drop drain
// ---------------------------------------------------------------------------

#[tokio::test]
async fn scenario_9_csp_nonce_pool_backpressure_and_clean_drop() {
    // 1. CSPRNG nonce: 16 bytes -> 24 Standard Base64 chars, unique across calls.
    let n1 = generate_csp_nonce().unwrap();
    let n2 = generate_csp_nonce().unwrap();
    assert_eq!(n1.len(), 24);
    assert_eq!(BASE64_STANDARD.decode(&n1).unwrap().len(), 16);
    assert_ne!(n1, n2);

    // 2. Non-panicking bounded backpressure (`PoolOverloaded`) when single worker is blocked in
    //    sync build_surface and queue_capacity_per_worker == 1.
    let small_pool = SsrLocalPool::with_config(
        SsrLocalPoolConfig::default()
            .with_worker_count(1)
            .with_queue_capacity_per_worker(1),
    )
    .unwrap();
    assert_eq!(small_pool.worker_count(), 1);
    assert_eq!(small_pool.queue_capacity_per_worker(), 1);

    let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel::<()>(1);
    let (unblock_tx, unblock_rx) = std::sync::mpsc::sync_channel::<()>(1);

    #[component]
    fn TinyApp() -> Element {
        rsx! { view { text { "ok" } } }
    }

    let p1 = small_pool.clone();
    let holder = tokio::spawn(async move {
        let c = erased_ssr_query_client(ControlledTransport::default());
        p1.render_streaming(
            SsrPageOptions::new("block", web_backend),
            StreamPlan::default(),
            c,
            move |_| {
                let _ = entered_tx.send(());
                let _ = unblock_rx.recv();
                DioxusSurface::new(1, TinyApp)
            },
        )
        .await
    });

    // Wait until worker 0 is actively executing `holder`'s build_surface (blocking its event loop).
    tokio::task::spawn_blocking(move || entered_rx.recv().unwrap())
        .await
        .unwrap();

    // Fill the single remaining queue slot deterministically by polling `queued_fut` once
    // (`render_streaming` calls `dispatch()` synchronously before awaiting `headers_rx`).
    let mut queued_fut = Box::pin(small_pool.render_streaming(
        SsrPageOptions::new("q", web_backend),
        StreamPlan::default(),
        erased_ssr_query_client(ControlledTransport::default()),
        |_| DioxusSurface::new(2, TinyApp),
    ));
    let noop_waker = Waker::noop();
    let mut cx = Context::from_waker(noop_waker);
    assert!(
        matches!(Pin::as_mut(&mut queued_fut).poll(&mut cx), Poll::Pending),
        "queued job must enter bounded worker channel and wait for blocked holder"
    );

    // Next dispatch MUST fail immediately with `SsrError::PoolOverloaded` (non-panicking!).
    let overload_err = small_pool
        .render_streaming(
            SsrPageOptions::new("over", web_backend),
            StreamPlan::default(),
            erased_ssr_query_client(ControlledTransport::default()),
            |_| DioxusSurface::new(3, TinyApp),
        )
        .await
        .expect_err("bounded worker queue must return SsrError::PoolOverloaded");
    assert_eq!(overload_err, SsrError::PoolOverloaded);

    let _ = unblock_tx.send(());
    assert!(holder.await.unwrap().is_ok());
    assert!(queued_fut.await.is_ok());

    // 3. Clean non-blocking Drop: drop the pool INSIDE an async runtime while an active streaming
    //    response is still in flight -> Drop does not block, and the in-flight stream finishes!
    let ephem_pool =
        SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(1)).unwrap();
    let transport = ControlledTransport::default();
    transport.set_ok("slow", "Finished After Pool Drop");
    transport.gate("slow");

    let resp = ephem_pool
        .render_streaming(
            SsrPageOptions::new("Drop Drain", web_backend),
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
