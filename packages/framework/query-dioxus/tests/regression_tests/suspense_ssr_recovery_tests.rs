//! Additional Slice 1c regression tests: `VirtualDom::wait_for_suspense()` direct SSR loop,
//! waterfall nested `<QuerySuspense>` atomic resolution, and `<QuerySuspense>` `ErrorBoundary`
//! Retry invalidation & recovery.

use core::future::Future;
use core::pin::Pin;
use core::task::{Context, Poll, Waker};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use dioxus_core::{ScopeId, VirtualDom};
use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryMode, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_rpc::{
    BoxFuture, CborCodec, Procedure, RawCall, RawReply, ReplyKind, RpcCodec, Transport,
    TransportError,
};

use crate::{find_by_test_id, pump_settled, render_once, GetItem, Item, ItemErr, NoopFlagWaker};

#[derive(Clone, Default)]
struct FlakyTransport {
    fail_next: Arc<AtomicBool>,
    calls: Arc<Mutex<usize>>,
}

impl Transport for FlakyTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        let fail_next = Arc::clone(&self.fail_next);
        let calls = Arc::clone(&self.calls);
        Box::pin(async move {
            assert_eq!(req.name, GetItem::NAME);
            let id: u64 = CborCodec::default().decode_bounded(&req.body).expect("decode RPC args");
            *calls.lock().expect("lock not poisoned") += 1;
            if fail_next.swap(false, Ordering::SeqCst) {
                let mut body = Vec::new();
                CborCodec::default()
                    .encode_into(&ItemErr::Denied, &mut body)
                    .expect("encode CBOR");
                return Ok(RawReply {
                    http_status: 403,
                    kind: ReplyKind::Domain,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::from(body),
                });
            }
            let mut body = Vec::new();
            CborCodec::default()
                .encode_into(
                    &Item {
                        id,
                        name: format!("Recovered#{id}"),
                    },
                    &mut body,
                )
                .expect("encode CBOR");
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

#[test]
fn dioxus_virtual_dom_wait_for_suspense_resolves_waterfall_in_ssr_mode() {
    let transport = FlakyTransport::default();
    let client = erased_ssr_query_client(transport.clone());
    let registry = SsrBoundaryRegistry::new();

    #[component]
    fn WaterfallChild(id: u64) -> Element {
        let q = use_query::<GetItem>(id);
        let item = q.suspend()?;
        rsx! { text { "STEP2:{item.name}" } }
    }

    #[component]
    fn WaterfallParent() -> Element {
        let q = use_query::<GetItem>(1);
        let first = q.suspend()?;
        let next_id = first.id + 1;
        rsx! {
            view {
                text { "STEP1:{first.name}" }
                QuerySuspense {
                    id: Some("nested-wf".into()),
                    fallback: rsx! { text { "INNER_LOADING" } },
                    WaterfallChild { id: next_id }
                }
            }
        }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                QuerySuspense {
                    id: Some("wf".into()),
                    fallback: rsx! { text { "OUTER_LOADING" } },
                    WaterfallParent {}
                }
            }
        }
    }

    let mut vdom = VirtualDom::new(Root);
    vdom.in_scope(ScopeId::ROOT, || {
        dioxus_core::provide_context(client.clone());
        dioxus_core::provide_context(registry.clone());
    });
    vdom.rebuild_in_place();

    assert!(registry.entry("wf").unwrap().is_pending());
    assert_eq!(registry.boundaries().len(), 1);

    // Drive `VirtualDom::wait_for_suspense()` directly (exact path `nve-ssr` uses without
    // `ClientOnly` tasks running!).
    let waker = Waker::noop();
    let mut cx = Context::from_waker(waker);
    let mut wait_fut = Box::pin(vdom.wait_for_suspense());
    for _ in 0..16 {
        if matches!(Pin::as_mut(&mut wait_fut).poll(&mut cx), Poll::Ready(())) {
            break;
        }
    }
    drop(wait_fut);

    let wf_entry = registry.entry("wf").unwrap();
    assert!(
        wf_entry.is_ready_to_flush(),
        "outer waterfall boundary must be ready to flush once BOTH step 1 and step 2 settle"
    );
    assert_eq!(
        wf_entry.query_handles().len(),
        2,
        "outer boundary must own BOTH waterfall query handles"
    );
    assert_eq!(*transport.calls.lock().unwrap(), 2);

    let mut cursor = DehydrateCursor::new();
    let delta = client
        .dehydrate_handles_delta(&wf_entry.query_handles(), &mut cursor)
        .unwrap();
    let verify_cli = erased_query_client(FlakyTransport::default());
    assert_eq!(verify_cli.hydrate_bytes(&delta).unwrap(), 2);
    assert!(registry.mark_flushed("wf"));
    assert!(!registry.has_pending());
}

#[test]
fn query_suspense_error_boundary_retry_invalidates_and_recovers() {
    let transport = FlakyTransport {
        fail_next: Arc::new(AtomicBool::new(true)),
        calls: Arc::new(Mutex::new(0)),
    };
    let clock = Shared::new(ManualClock::new(1_000));
    let client = erased_query_client_with_clock(
        transport.clone(),
        clock,
        QueryClientConfig {
            mode: QueryMode::Client,
            ..QueryClientConfig::default()
        },
    );

    #[component]
    fn FallibleItem() -> Element {
        let q = use_query_with::<GetItem>(77, QueryOptions::default().max_retries(0));
        let item = q.suspend()?;
        rsx! { text { "OK:{item.name}" } }
    }

    #[component]
    fn RetryApp() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                QuerySuspense {
                    id: Some("retry-box".into()),
                    fallback: rsx! { text { "RETRY_LOADING" } },
                    error_fallback: move |report: UiErrorReport, reset: ErrorBoundaryReset| rsx! {
                        view {
                            text { "ERR:{report.message}" }
                            pressable {
                                test_id: "retry-btn",
                                onpress: move |_| reset.reset(),
                                text { "Try Again" }
                            }
                        }
                    },
                    FallibleItem {}
                }
            }
        }
    }

    let mut frame = 0u64;
    let mut web = WebBackend::new();
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let mut surface = DioxusSurface::new(1, RetryApp).with_context(client);

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    let err_html = web.render_html(surface.store());
    assert!(
        err_html.contains("ERR:") && err_html.contains("Denied"),
        "initial domain error must render error_fallback: {err_html}"
    );
    assert_eq!(*transport.calls.lock().unwrap(), 1);

    // Press Retry -> QuerySuspense's ErrorBoundary on_reset invalidates boundary handles and
    // triggers a clean retry that recovers!
    let retry_node = find_by_test_id(surface.store(), "retry-btn");
    let press_ev = nve_abi::NativeEvent {
        surface_id: 1,
        node_id: retry_node,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    };
    assert!(surface.dispatch_events(&[press_ev]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    let recovered_html = web.render_html(surface.store());
    assert!(
        recovered_html.contains("OK:Recovered#77"),
        "pressing Retry must invalidate failed query and recover cleanly: {recovered_html}"
    );
    assert_eq!(*transport.calls.lock().unwrap(), 2);
}
