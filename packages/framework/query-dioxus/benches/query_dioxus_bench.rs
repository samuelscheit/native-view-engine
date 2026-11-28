//! Criterion micro-benchmarks for `nve-query-dioxus`: `BoundaryQuerySet` handle tracking,
//! `ErasedQueryClient` typed transport dispatch, and reactive `QueryClientProvider` + `use_query`
//! mount/refetch ticks inside `DioxusSurface`.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use core::task::Waker;
    use std::hint::black_box;

    use bytes::Bytes;
    use criterion::{criterion_group, BatchSize, Criterion};
    use dioxus_core::{consume_context, use_hook};
    use nve_abi::{FrameId, SurfaceId};
    use nve_backend_web::WebBackend;
    use nve_dioxus::prelude::*;
    use nve_dioxus::DioxusSurface;
    use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
    use nve_query_dioxus::prelude::*;
    use nve_rpc::{
        procedures, BoxFuture, CborCodec, DomainStatus, RawCall, RawReply, ReplyKind, RpcCodec,
        RpcStatus, Tag64, Transport, TransportError,
    };

    #[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
    #[cbor(map)]
    struct Item {
        #[n(0)]
        id: u64,
        #[n(1)]
        label: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
    enum ItemErr {
        #[n(0)]
        NotFound,
    }

    impl DomainStatus for ItemErr {
        fn rpc_status(&self) -> RpcStatus {
            RpcStatus::NotFound
        }
    }

    procedures! {
        query GetItem = "bench.v1.Items/Get" (u64) -> Item, ItemErr,
            tags |id, sink| {
                sink.push(Tag64::keyed("item", *id));
            };
    }

    #[derive(Clone)]
    struct InstantTransport {
        encoded: Bytes,
    }

    impl InstantTransport {
        fn new() -> Self {
            let mut body = Vec::new();
            CborCodec::default()
                .encode_into(
                    &Item {
                        id: 1,
                        label: "Cached item".to_owned(),
                    },
                    &mut body,
                )
                .expect("encode bench item");
            Self {
                encoded: Bytes::from(body),
            }
        }
    }

    impl Transport for InstantTransport {
        fn call_raw<'a>(
            &'a self,
            _req: RawCall,
        ) -> BoxFuture<'a, Result<RawReply, TransportError>> {
            let body = self.encoded.clone();
            Box::pin(async move {
                Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body,
                })
            })
        }
    }

    #[component]
    fn BenchItemView() -> Element {
        let q = use_query_with::<GetItem>(1u64, QueryOptions::default().stale_time_ms(60_000));
        let label = match q.data() {
            Some(item) => item.label.clone(),
            None => "loading".to_owned(),
        };
        rsx! {
            text { "{label}" }
        }
    }

    #[component]
    fn BenchApp() -> Element {
        let client = use_hook(consume_context::<ErasedQueryClient>);
        rsx! {
            QueryClientProvider {
                client: client,
                BenchItemView {}
            }
        }
    }

    fn bench_query_dioxus_hot_paths(c: &mut Criterion) {
        let transport = InstantTransport::new();
        let clock = Shared::new(ManualClock::new(1_000));
        let client = erased_query_client_with_clock(
            transport.clone(),
            clock.clone(),
            QueryClientConfig::default(),
        );
        let handles: Vec<_> = (0..32u64)
            .map(|id| {
                client
                    .subscribe::<GetItem>(&id, QueryOptions::default())
                    .expect("subscribe bench query")
            })
            .collect();

        c.bench_function("query_dioxus_boundary_set_32_handles", |b| {
            let set = BoundaryQuerySet::new();
            b.iter(|| {
                for &h in &handles {
                    set.record(black_box(h));
                }
                let snap = set.handles();
                for &h in &handles {
                    set.forget(black_box(h));
                }
                black_box(snap.len())
            });
        });

        let noop_waker = Waker::noop();

        c.bench_function("query_dioxus_surface_mount_and_settle", |b| {
            b.iter_batched(
                || {
                    let client = erased_query_client_with_clock(
                        transport.clone(),
                        Shared::new(ManualClock::new(1_000)),
                        QueryClientConfig::default(),
                    );
                    let mut web = WebBackend::new();
                    nve_web_html_web::register_web_html_systems(web.table_registry_mut());
                    (client, web)
                },
                |(client, mut web)| {
                    let mut surface =
                        DioxusSurface::new(SurfaceId::ROOT, BenchApp).with_context(client);
                    surface
                        .commit_to(FrameId(1), &mut web)
                        .expect("initial commit");
                    let mut frame = 1u64;
                    while surface.poll_work(noop_waker) {
                        frame += 1;
                        surface
                            .commit_to(FrameId(frame), &mut web)
                            .expect("settled commit");
                    }
                    let live = surface.store().live_nodes().count();
                    black_box((frame, live))
                },
                BatchSize::SmallInput,
            );
        });
    }

    criterion_group!(benches, bench_query_dioxus_hot_paths);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
