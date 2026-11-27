//! Slice 1c `<QuerySuspense>`, `UseQuery::suspend` / `suspend_result`, `SsrBoundaryRegistry`,
//! `BoundaryQuerySet`, `InDeferredSuspense`, sibling-boundary dedup, error hydration,
//! `keep_previous_data`, and mid-stream `StreamPending` / `finish_ssr_stream` regression tests.

use core::task::{Poll, Waker};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryMode, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::{
    BoxFuture, CborCodec, Procedure, RawCall, RawReply, ReplyKind, RpcCodec, Transport,
    TransportError,
};

use crate::{pump_settled, render_once, GetItem, Item, ItemErr, NoopFlagWaker};

#[derive(Clone, Default)]
struct SuspenseTransport {
    state: Arc<Mutex<SuspenseTransportState>>,
}

#[derive(Default)]
struct SuspenseTransportState {
    defer: bool,
    calls: usize,
    domain_err_ids: Vec<u64>,
    protocol_500_ids: Vec<u64>,
    wakers: Vec<Waker>,
}

impl SuspenseTransport {
    fn new(defer: bool) -> Self {
        let t = Self::default();
        t.state.lock().unwrap().defer = defer;
        t
    }

    fn release(&self) {
        let wakers = {
            let mut g = self.state.lock().unwrap();
            g.defer = false;
            core::mem::take(&mut g.wakers)
        };
        for w in wakers {
            w.wake();
        }
    }

    fn call_count(&self) -> usize {
        self.state.lock().unwrap().calls
    }
}

impl Transport for SuspenseTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let state = Arc::clone(&self.state);
        Box::pin(async move {
            assert_eq!(req.name, GetItem::NAME);
            let id: u64 = CborCodec::default().decode_bounded(&req.body).unwrap();
            core::future::poll_fn(|cx| {
                let mut g = state.lock().unwrap();
                if g.defer {
                    g.wakers.push(cx.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(())
                }
            })
            .await;

            let (is_dom, is_500) = {
                let mut g = state.lock().unwrap();
                g.calls += 1;
                (
                    g.domain_err_ids.contains(&id),
                    g.protocol_500_ids.contains(&id),
                )
            };

            if is_dom {
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(&ItemErr::Denied, &mut body)
                    .unwrap();
                return Ok(RawReply {
                    http_status: 403,
                    kind: ReplyKind::Domain,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            if is_500 {
                return Ok(RawReply {
                    http_status: 500,
                    kind: ReplyKind::Protocol,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from_static(
                        br#"{"code":"internal","message":"SECRET_DB_STACKTRACE"}"#,
                    ),
                });
            }

            let mut body = Vec::new();
            CborCodec::default()
                .encode_into(
                    &Item {
                        id,
                        name: format!("Resolved#{id}"),
                    },
                    &mut body,
                )
                .unwrap();
            Ok(RawReply {
                http_status: 200,
                kind: ReplyKind::Ok,
                invalidated_tags: Vec::new(),
                invalidate_all: false,
                body: Bytes::from(body),
            })
        })
    }
}

#[component]
fn SuspendItem(id: u64, label: &'static str) -> Element {
    let q = use_query_with::<GetItem>(
        id,
        QueryOptions::default()
            .stale_time_ms(60_000)
            .max_retries(0),
    );
    let item = q.suspend()?;
    rsx! {
        text { "{label}:{item.name}" }
    }
}

#[test]
fn two_sibling_query_suspense_boundaries_sharing_one_key_both_resolve_with_one_rpc() {
    for mode in [QueryMode::Client, QueryMode::Ssr] {
        let transport = SuspenseTransport::new(true);
        let clock = Shared::new(ManualClock::new(1_000));
        let client = erased_query_client_with_clock(
            transport.clone(),
            clock,
            QueryClientConfig {
                mode,
                ..QueryClientConfig::default()
            },
        );
        let registry = SsrBoundaryRegistry::new();

        #[component]
        fn SiblingApp() -> Element {
            let client = dioxus_core::consume_context::<ErasedQueryClient>();
            rsx! {
                QueryClientProvider {
                    client: client,
                    view {
                        QuerySuspense {
                            id: Some("left".into()),
                            fallback: rsx! { text { "LOADING_LEFT" } },
                            SuspendItem { id: 7, label: "LEFT" }
                        }
                        QuerySuspense {
                            id: Some("right".into()),
                            fallback: rsx! { text { "LOADING_RIGHT" } },
                            SuspendItem { id: 7, label: "RIGHT" }
                        }
                    }
                }
            }
        }

        let mut frame = 0u64;
        let mut web = WebBackend::new();
        let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
        let mut surface = DioxusSurface::new(1, SiblingApp).with_context(client.clone());
        if mode == QueryMode::Ssr {
            surface = surface.with_context(registry.clone());
        }

        let html1 = render_once(&mut surface, &mut web, &mut frame);
        let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
        let html_pending = web.render_html();
        assert!(
            html1.contains("LOADING_LEFT") && html1.contains("LOADING_RIGHT"),
            "both sibling boundaries must render fallback while deferred ({mode:?}): {html1}"
        );
        assert!(
            html_pending.contains("data-nve-suspense=\"left\"")
                && html_pending.contains("data-nve-stream-slot=\"left\"")
                && html_pending.contains("data-nve-suspense=\"right\"")
                && html_pending.contains("data-nve-stream-slot=\"right\""),
            "DOM contract markers missing ({mode:?}): {html_pending}"
        );
        if mode == QueryMode::Ssr {
            assert_eq!(registry.pending_handles().len(), 1);
            assert!(registry.entry("left").unwrap().is_pending());
            assert!(registry.entry("right").unwrap().is_pending());
        }

        // Release transport -> BOTH sibling boundaries resolve with EXACTLY ONE RPC call!
        transport.release();
        let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
        let html_done = web.render_html();
        assert!(
            html_done.contains("LEFT:Resolved#7") && html_done.contains("RIGHT:Resolved#7"),
            "BOTH sibling boundaries must resolve cleanly ({mode:?}): {html_done}"
        );
        assert_eq!(
            transport.call_count(),
            1,
            "sibling boundaries sharing one key must issue exactly 1 RPC ({mode:?})"
        );
        if mode == QueryMode::Ssr {
            let ready: Vec<_> = registry
                .boundaries()
                .into_iter()
                .filter(|e| e.is_ready_to_flush())
                .collect();
            assert_eq!(ready.len(), 2);
            let mut cursor = DehydrateCursor::new();
            let left_delta = client
                .dehydrate_handles_delta(&ready[0].query_handles(), &mut cursor)
                .unwrap();
            let right_delta = client
                .dehydrate_handles_delta(&ready[1].query_handles(), &mut cursor)
                .unwrap();
            assert!(left_delta.len() > 9, "first boundary emits shared handle");
            assert_eq!(
                right_delta.len(),
                9,
                "second boundary deduplicates shared handle via DehydrateCursor"
            );
        }
    }
}

#[test]
fn domain_and_protocol_errors_render_error_boundary_and_hydrate_with_zero_refetch() {
    let ssr_transport = SuspenseTransport::new(true);
    {
        let mut g = ssr_transport.state.lock().unwrap();
        g.domain_err_ids.push(10);
        g.protocol_500_ids.push(11);
    }
    let ssr_clock = Shared::new(ManualClock::new(1_000));
    let ssr_client = erased_query_client_with_clock(
        ssr_transport.clone(),
        ssr_clock,
        QueryClientConfig {
            mode: QueryMode::Ssr,
            ..QueryClientConfig::default()
        },
    );
    let registry = SsrBoundaryRegistry::new();

    #[component]
    fn InlineErrItem(id: u64) -> Element {
        let q = use_query_with::<GetItem>(id, QueryOptions::default().max_retries(0));
        match q.suspend_result()? {
            Ok(it) => rsx! { text { "INLINE_OK:{it.name}" } },
            Err(e) => rsx! { text { "INLINE_ERR:{e}" } },
        }
    }

    #[component]
    fn ErrApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    QuerySuspense {
                        id: Some("dom-err".into()),
                        fallback: rsx! { text { "LOADING_DOM" } },
                        error_fallback: move |report: UiErrorReport, _reset: ErrorBoundaryReset| rsx! {
                            text { "CAUGHT_DOM:{report.message}" }
                        },
                        SuspendItem { id: 10, label: "DOM" }
                    }
                    QuerySuspense {
                        id: Some("proto-err".into()),
                        fallback: rsx! { text { "LOADING_PROTO" } },
                        error_fallback: move |report: UiErrorReport, _reset: ErrorBoundaryReset| rsx! {
                            text { "CAUGHT_PROTO:{report.message}" }
                        },
                        SuspendItem { id: 11, label: "PROTO" }
                    }
                    QuerySuspense {
                        id: Some("inline-err".into()),
                        fallback: rsx! { text { "LOADING_INLINE" } },
                        InlineErrItem { id: 10 }
                    }
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut ssr_surface = DioxusSurface::new(1, ErrApp)
        .with_context(ssr_client.clone())
        .with_context(registry.clone());

    let _ = render_once(&mut ssr_surface, &mut web, &mut frame);
    let _ = pump_settled(&mut ssr_surface, &mut web, &waker, &mut frame);
    ssr_transport.release();
    let _ = pump_settled(&mut ssr_surface, &mut web, &waker, &mut frame);
    let ssr_html = web.render_html();
    assert!(
        ssr_html.contains("CAUGHT_DOM:") && ssr_html.contains("Denied"),
        "DomainErr must trigger ErrorBoundary fallback: {ssr_html}"
    );
    assert!(
        ssr_html.contains("CAUGHT_PROTO:") && ssr_html.contains("500"),
        "ProtocolErr must trigger ErrorBoundary fallback: {ssr_html}"
    );
    assert!(
        ssr_html.contains("INLINE_ERR:") && ssr_html.contains("Denied"),
        "suspend_result must surface error inline without tripping ErrorBoundary: {ssr_html}"
    );

    // Dehydrate all settled errors into NVEQ v2 and hydrate into a fresh Client!
    let mut cursor = DehydrateCursor::new();
    let blob = ssr_client
        .dehydrate_handles_delta(
            &registry
                .boundaries()
                .iter()
                .flat_map(|e| e.query_handles())
                .collect::<Vec<_>>(),
            &mut cursor,
        )
        .unwrap();

    let cli_transport = SuspenseTransport::new(false);
    let cli_clock = Shared::new(ManualClock::new(2_000));
    let cli_client = erased_query_client_with_clock(
        cli_transport.clone(),
        cli_clock,
        QueryClientConfig {
            hydration_grace_ms: 5_000,
            ..QueryClientConfig::default()
        },
    );
    assert_eq!(cli_client.hydrate_bytes(&blob).unwrap(), 2);

    let mut cli_frame = 0u64;
    let mut cli_web = WebBackend::new();
    let mut cli_surface = DioxusSurface::new(2, ErrApp).with_context(cli_client);
    let cli_html = render_once(&mut cli_surface, &mut cli_web, &mut cli_frame);
    let _ = pump_settled(&mut cli_surface, &mut cli_web, &waker, &mut cli_frame);
    let cli_settled = cli_web.render_html();

    assert!(cli_html.contains("CAUGHT_DOM:") && cli_settled.contains("Denied"));
    assert!(
        cli_settled.contains("CAUGHT_PROTO:") && !cli_settled.contains("SECRET_DB_STACKTRACE"),
        "hydrated 500 error must be clamped and never leak server internals: {cli_settled}"
    );
    assert!(cli_settled.contains("INLINE_ERR:"));
    assert_eq!(
        cli_transport.call_count(),
        0,
        "hydrated DomainErr and ProtocolErr MUST NOT fire duplicate fetches on Frame 0!"
    );
}

#[test]
fn keep_previous_data_does_not_re_suspend_on_key_switch() {
    let transport = SuspenseTransport::new(false);
    let clock = Shared::new(ManualClock::new(1_000));
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock,
        QueryClientConfig::default(),
    );

    type SharedIdSig = std::rc::Rc<std::cell::RefCell<Option<Signal<u64>>>>;
    let id_box: SharedIdSig = std::rc::Rc::new(std::cell::RefCell::new(None));

    #[component]
    fn KeepPrevChild(id: u64) -> Element {
        let q = use_query_with::<GetItem>(
            id,
            QueryOptions::default()
                .stale_time_ms(60_000)
                .keep_previous_data(true),
        );
        let item = q.suspend()?;
        rsx! { text { "ITEM:{item.name}" } }
    }

    #[component]
    fn KeepPrevApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let active_id = use_signal(|| 1u64);
        dioxus_core::use_hook(|| {
            *dioxus_core::consume_context::<SharedIdSig>().borrow_mut() = Some(active_id);
        });
        let id = *active_id.read();
        rsx! {
            QueryClientProvider {
                client: client,
                QuerySuspense {
                    id: Some("kpd".into()),
                    fallback: rsx! { text { "SUSPENDED_FALLBACK" } },
                    KeepPrevChild { id: id }
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut surface = DioxusSurface::new(1, KeepPrevApp)
        .with_context(client)
        .with_context(std::rc::Rc::clone(&id_box));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("ITEM:Resolved#1"));

    // Defer network and switch key 1 -> 2: MUST keep showing Resolved#1, NEVER re-suspend!
    transport.state.lock().unwrap().defer = true;
    let mut id_sig = id_box.borrow().unwrap();
    surface.in_root_scope(|| id_sig.set(2));
    let html_switching = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    let html_still_pending = web.render_html();
    assert!(
        html_switching.contains("ITEM:Resolved#1")
            && !html_switching.contains("SUSPENDED_FALLBACK")
            && html_still_pending.contains("ITEM:Resolved#1")
            && !html_still_pending.contains("SUSPENDED_FALLBACK"),
        "keep_previous_data(true) must NOT re-suspend on key switch: {html_still_pending}"
    );

    // Release network -> updates smoothly to Resolved#2!
    transport.release();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("ITEM:Resolved#2"));
}

#[test]
fn ssr_plain_use_query_never_fetches_while_suspend_attributes_handles_and_flattens_nested_slots() {
    let transport = SuspenseTransport::new(true);
    let clock = Shared::new(ManualClock::new(1_000));
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock,
        QueryClientConfig {
            mode: QueryMode::Ssr,
            ..QueryClientConfig::default()
        },
    );
    let registry = SsrBoundaryRegistry::new();

    #[component]
    fn PlainNoFetch() -> Element {
        let q = use_query::<GetItem>(999);
        let txt = q
            .data()
            .map(|i| i.name.clone())
            .unwrap_or_else(|| "PLAIN_EMPTY".to_owned());
        rsx! { text { "{txt}" } }
    }

    #[component]
    fn SsrApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    PlainNoFetch {}
                    QuerySuspense {
                        id: Some("outer".into()),
                        fallback: rsx! { text { "LOADING_OUTER" } },
                        SuspendItem { id: 1, label: "OUTER" }
                        QuerySuspense {
                            id: Some("inner".into()),
                            fallback: rsx! { text { "LOADING_INNER" } },
                            SuspendItem { id: 2, label: "INNER" }
                        }
                    }
                    QuerySuspense {
                        id: Some("sibling".into()),
                        fallback: rsx! { text { "LOADING_SIBLING" } },
                        SuspendItem { id: 3, label: "SIBLING" }
                    }
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut surface = DioxusSurface::new(1, SsrApp)
        .with_context(client)
        .with_context(registry.clone());

    let html1 = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    assert!(html1.contains("PLAIN_EMPTY"));
    assert!(html1.contains("data-nve-stream-slot=\"outer\""));
    assert!(html1.contains("data-nve-stream-slot=\"sibling\""));
    assert!(
        !html1.contains("data-nve-stream-slot=\"inner\""),
        "nested SSR <QuerySuspense> must NEVER emit a nested NveStreamSlot: {html1}"
    );
    assert_eq!(registry.boundaries().len(), 2);
    // Outer owns BOTH item 1 and nested item 2; sibling owns ONLY item 3 (zero leakage!).
    assert_eq!(registry.entry("outer").unwrap().query_handles().len(), 2);
    assert_eq!(registry.entry("sibling").unwrap().query_handles().len(), 1);

    transport.release();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    let html_done = web.render_html();
    assert!(
        html_done.contains("OUTER:Resolved#1")
            && html_done.contains("INNER:Resolved#2")
            && html_done.contains("SIBLING:Resolved#3")
            && html_done.contains("PLAIN_EMPTY"),
        "nested + sibling boundaries must resolve cleanly while plain use_query stayed empty: {html_done}"
    );
    assert_eq!(
        transport.call_count(),
        3,
        "only the 3 suspended queries should have fetched (NOT plain use_query(999))"
    );
}

#[test]
fn stream_pending_suspends_without_rpc_then_resolves_via_chunk_or_finish_ssr_stream_fallback() {
    // Prepare NVEQ v2 pending + settled chunks from an SSR client.
    let ssr_transport = SuspenseTransport::new(false);
    let ssr_client = erased_ssr_query_client(ssr_transport);
    let h_streamed = ssr_client
        .subscribe::<GetItem>(&50, QueryOptions::default())
        .unwrap();
    let h_truncated = ssr_client
        .subscribe::<GetItem>(&51, QueryOptions::default())
        .unwrap();
    let pending_blob = ssr_client
        .dehydrate_initial_stream_chunk(&[h_streamed, h_truncated], &mut DehydrateCursor::new())
        .unwrap();

    // Server settles item 50 into a stream chunk, while item 51 never arrives (truncated stream!).
    ssr_client
        .set_query_data::<GetItem>(
            &50,
            Item {
                id: 50,
                name: "StreamedChunk#50".into(),
            },
        )
        .unwrap();
    let mut cursor = DehydrateCursor::new();
    let chunk_50 = ssr_client
        .dehydrate_handles_delta(&[h_streamed], &mut cursor)
        .unwrap();

    let cli_transport = SuspenseTransport::new(false);
    let cli_clock = Shared::new(ManualClock::new(1_000));
    let cli_client = erased_query_client_with_clock(
        cli_transport.clone(),
        cli_clock,
        QueryClientConfig::default(),
    );
    assert_eq!(cli_client.hydrate_bytes(&pending_blob).unwrap(), 2);

    #[component]
    fn StreamBootApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                view {
                    QuerySuspense {
                        id: Some("chunked".into()),
                        fallback: rsx! { text { "WAIT_CHUNK_50" } },
                        SuspendItem { id: 50, label: "S50" }
                    }
                    QuerySuspense {
                        id: Some("trunc".into()),
                        fallback: rsx! { text { "WAIT_TRUNC_51" } },
                        SuspendItem { id: 51, label: "S51" }
                    }
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut surface = DioxusSurface::new(1, StreamBootApp).with_context(cli_client.clone());

    let html_boot = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        html_boot.contains("WAIT_CHUNK_50") && html_boot.contains("WAIT_TRUNC_51"),
        "StreamPending slots must suspend on boot: {html_boot}"
    );
    assert_eq!(
        cli_transport.call_count(),
        0,
        "StreamPending slots MUST NOT fire client RPCs while stream is open"
    );

    // 1. Streaming chunk for item 50 arrives -> resolves boundary 50 with ZERO client RPCs!
    assert_eq!(cli_client.hydrate_stream_chunk_bytes(&chunk_50).unwrap(), 1);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    let html_mid = web.render_html();
    assert!(
        html_mid.contains("S50:StreamedChunk#50") && html_mid.contains("WAIT_TRUNC_51"),
        "chunk 50 must resolve while slot 51 stays suspended: {html_mid}"
    );
    assert_eq!(cli_transport.call_count(), 0);

    // 2. Stream closes without sending item 51 -> `finish_ssr_stream()` triggers automatic client
    //    Transport fallback for ONLY item 51!
    assert_eq!(cli_client.finish_ssr_stream(), 1);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    let html_final = web.render_html();
    assert!(
        html_final.contains("S50:StreamedChunk#50") && html_final.contains("S51:Resolved#51"),
        "finish_ssr_stream must trigger automatic client fetch fallback: {html_final}"
    );
    assert_eq!(
        cli_transport.call_count(),
        1,
        "exactly 1 fallback RPC call should fire for truncated slot 51"
    );
}
