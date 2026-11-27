#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native_bench {
    use std::hint::black_box;
    use std::sync::Arc;

    use bytes::Bytes;
    use criterion::{criterion_group, Criterion};
    use nve_query::{ManualClock, QueryClient, QueryClientConfig, QueryOptions};
    use nve_rpc::{
        procedures, BoxFuture, CborCodec, RawCall, RawReply, ReplyKind, Tag64, Transport,
        TransportError,
    };

    procedures! {
        query BenchItem = "bench.v1.Items/Get" (u64) -> u64, (),
            tags |id, sink| {
                sink.push(Tag64::of("bench.list"));
                sink.push(Tag64::keyed("bench.item", *id));
            };
    }

    struct NoopTransport;

    impl Transport for NoopTransport {
        fn call_raw<'a>(&'a self, _req: RawCall) -> BoxFuture<'a, Result<RawReply, TransportError>> {
            Box::pin(async {
                Ok(RawReply {
                    http_status: 200,
                    kind: ReplyKind::Ok,
                    invalidated_tags: Vec::new(),
                    invalidate_all: false,
                    body: Bytes::new(),
                })
            })
        }
    }

    fn bench_query_cache(c: &mut Criterion) {
        let clock = Arc::new(ManualClock::new(1_000));
        let client = QueryClient::with_clock(
            NoopTransport,
            CborCodec::default(),
            clock.clone(),
            QueryClientConfig {
                max_entries: 4096,
                ..QueryClientConfig::default()
            },
        );

        let mut handles = Vec::with_capacity(1000);
        for i in 0..1000u64 {
            let h = client
                .subscribe::<BenchItem>(&i, QueryOptions::default().stale_time_ms(60_000))
                .unwrap();
            client.set_query_data::<BenchItem>(&i, i * 2).unwrap();
            handles.push(h);
        }

        c.bench_function("nve_query_read_hit", |b| {
            let h = handles[500];
            b.iter(|| black_box(client.read::<BenchItem>(black_box(h))));
        });

        c.bench_function("nve_query_invalidate_1_tag_1_slot", |b| {
            let tags = [Tag64::keyed("bench.item", 500)];
            let mut scratch = Vec::with_capacity(16);
            b.iter(|| {
                client.invalidate_tags_into(black_box(&tags), &mut scratch);
                black_box(scratch.len())
            });
        });

        c.bench_function("nve_query_invalidate_all_1000_active", |b| {
            b.iter(|| black_box(client.invalidate_all().len()));
        });

        c.bench_function("nve_query_gc_sweep_1000_inactive", |b| {
            b.iter_batched(
                || {
                    let clk = Arc::new(ManualClock::new(1_000));
                    let c = QueryClient::with_clock(
                        NoopTransport,
                        CborCodec::default(),
                        clk.clone(),
                        QueryClientConfig {
                            max_entries: 2048,
                            ..QueryClientConfig::default()
                        },
                    );
                    for i in 0..1000u64 {
                        let h = c
                            .subscribe::<BenchItem>(&i, QueryOptions::default().gc_time_ms(100))
                            .unwrap();
                        c.unsubscribe(h);
                    }
                    clk.advance(500);
                    c
                },
                |c| black_box(c.gc_sweep(1000)),
                criterion::BatchSize::SmallInput,
            );
        });
    }

    criterion_group!(benches, bench_query_cache);
}

#[cfg(not(target_arch = "wasm32"))]
criterion::criterion_main!(native_bench::benches);
