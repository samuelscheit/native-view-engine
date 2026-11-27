//! Criterion micro/macro benchmarks for `nve-ssr`:
//! 1. Fast-path Light-DOM SSR (`HttpBody::Full`, 0 suspense, pre-hydrated query + Base64 `NVEQ`).
//! 2. Out-of-order DSD streaming SSR (`HttpBody::Stream`, 2 `<QuerySuspense>` boundaries + Base64
//!    `NVEQ` v2 `seq=0` + per-slot deltas + CSS dedup).

use std::hint::black_box;

use bytes::Bytes;
use criterion::{criterion_group, criterion_main, Criterion};
use http_body_util::BodyExt as _;
use minicbor::{Decode, Encode};
use nve_abi::ColorRgba;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_http_hyper::HttpBody;
use nve_query::QueryOptions;
use nve_query_dioxus::prelude::*;
use nve_rpc::{
    procedures, BoxFuture, CborCodec, RawCall, RawReply, ReplyKind, RpcCodec, Transport,
    TransportError,
};
use nve_ssr::{SsrLocalPool, SsrLocalPoolConfig, SsrPageOptions, StreamPlan};

#[derive(Debug, Clone, Encode, Decode)]
struct BenchItem {
    #[n(0)]
    id: u32,
    #[n(1)]
    label: String,
}

procedures! {
    query BenchGetItem = "bench.ssr.v1/GetItem" (u32) -> BenchItem, ();
}

#[derive(Clone)]
struct YieldingBenchTransport;

impl Transport for YieldingBenchTransport {
    fn call_raw<'a>(&'a self, req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
        Box::pin(async move {
            // Yield once so Frame 1 genuinely suspends and exercises the full DSD streaming path.
            tokio::task::yield_now().await;
            let id: u32 = CborCodec::default().decode_bounded(&req.body).unwrap();
            let mut buf = Vec::with_capacity(32);
            CborCodec::default()
                .encode_into(
                    &BenchItem {
                        id,
                        label: format!("Bench item #{id}"),
                    },
                    &mut buf,
                )
                .unwrap();
            Ok(RawReply {
                http_status: 200,
                kind: ReplyKind::Ok,
                invalidated_tags: Vec::new(),
                invalidate_all: false,
                body: Bytes::from(buf),
            })
        })
    }
}

#[component]
fn BenchPanel(id: u32) -> Element {
    let q = use_query_with::<BenchGetItem>(id, QueryOptions::default().stale_time_ms(60_000));
    let item = q.suspend()?;
    let style = TypedStyle::new()
        .bg(ColorRgba::rgb(24, 34, 52))
        .radius(8.0);
    rsx! {
        view {
            nve_style: style,
            text { "{item.label}" }
        }
    }
}

#[component]
fn BenchApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    rsx! {
        QueryClientProvider {
            client: client,
            view {
                text { "Bench Header" }
                QuerySuspense {
                    id: Some("stats".into()),
                    fallback: rsx! { text { "Loading stats..." } },
                    BenchPanel { id: 1 }
                }
                QuerySuspense {
                    id: Some("comments".into()),
                    fallback: rsx! { text { "Loading comments..." } },
                    BenchPanel { id: 2 }
                }
            }
        }
    }
}

fn bench_nve_ssr(c: &mut Criterion) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();

    let pool = SsrLocalPool::with_config(SsrLocalPoolConfig::default().with_worker_count(2))
        .expect("pool init");
    let page = SsrPageOptions::new("Bench SSR")
        .with_style_nonce("benchNonce1234567890==")
        .with_modulepreload_src("/app.js")
        .with_async_boot_src("/app.js");

    c.bench_function("nve_ssr_fast_path_light_dom", |b| {
        b.iter(|| {
            let client = erased_ssr_query_client(YieldingBenchTransport);
            client
                .set_query_data::<BenchGetItem>(
                    &1,
                    BenchItem {
                        id: 1,
                        label: "Prefetched stats".into(),
                    },
                )
                .unwrap();
            client
                .set_query_data::<BenchGetItem>(
                    &2,
                    BenchItem {
                        id: 2,
                        label: "Prefetched comments".into(),
                    },
                )
                .unwrap();
            let bytes = rt.block_on(async {
                let resp = pool
                    .render_streaming(
                        black_box(page.clone()),
                        StreamPlan::default(),
                        client,
                        |c| DioxusSurface::new(1, BenchApp).with_context(c),
                    )
                    .await
                    .unwrap();
                debug_assert!(matches!(resp.body(), HttpBody::Full(_)));
                resp.into_body().collect().await.unwrap().to_bytes()
            });
            black_box(bytes.len())
        });
    });

    c.bench_function("nve_ssr_streaming_shell_and_2_slot_deltas", |b| {
        b.iter(|| {
            let client = erased_ssr_query_client(YieldingBenchTransport);
            let bytes = rt.block_on(async {
                let resp = pool
                    .render_streaming(
                        black_box(page.clone()),
                        StreamPlan::default(),
                        client,
                        |c| DioxusSurface::new(1, BenchApp).with_context(c),
                    )
                    .await
                    .unwrap();
                debug_assert!(matches!(resp.body(), HttpBody::Stream(_)));
                resp.into_body().collect().await.unwrap().to_bytes()
            });
            black_box(bytes.len())
        });
    });
}

criterion_group!(benches, bench_nve_ssr);
criterion_main!(benches);
