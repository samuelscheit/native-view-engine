//! Criterion micro-benchmarks for canonical `NveHostEventBatch` encoding, validation,
//! `FfiHostServices::ingest_batch`, and zero-alloc synchronous `HostCallbackBridge` round-trips.

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::hint::black_box;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{
        encode_single_push_into, NveHostEventBatchBuilder, NveHostEventBatchRef, NveHostStatus,
        SERVICE_CLIPBOARD, SERVICE_SYSTEM_ENV,
    };
    use nve_host_services::{
        CompletionSink, FfiHostServices, HandlerReply, HostCallbackBridge, HostServiceHandler,
        HostServiceProvider, HostServiceRegistry, ResponseBuf, ServiceError,
    };

    struct InlineEchoHandler;

    impl HostServiceHandler for InlineEchoHandler {
        fn service_id(&self) -> u32 {
            SERVICE_CLIPBOARD
        }

        fn handle(
            &self,
            _method_id: u32,
            _request_id: u64,
            request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            Ok(HandlerReply::Ready(ResponseBuf::from_slice_or_vec(request)))
        }
    }

    fn bench_host_event_batch(c: &mut Criterion) {
        let theme_bytes = [1u8, 0, 0, 0, 0xFF, 0x7A, 0x00, 0x00, 0xE8, 0x03, 0xC8, 0x00];

        c.bench_function("host_batch_encode_decode_single_push_stack", |b| {
            let mut stack = [0u8; 56];
            b.iter(|| {
                let slice = encode_single_push_into(
                    &mut stack,
                    SERVICE_SYSTEM_ENV,
                    1,
                    black_box(&theme_bytes),
                )
                .expect("encode stack push");
                let parsed = NveHostEventBatchRef::parse(black_box(slice)).expect("parse");
                let rec = parsed.iter().next().expect("one record");
                black_box((rec.service_id, rec.payload.len()))
            });
        });

        let mut builder = NveHostEventBatchBuilder::with_capacity(512);
        for i in 0..8u64 {
            builder
                .push_response(SERVICE_CLIPBOARD, 1, i + 1, NveHostStatus::Ok, b"ok-payload")
                .expect("push response");
        }
        builder
            .push_push(SERVICE_SYSTEM_ENV, 1, &theme_bytes)
            .expect("push theme");
        builder
            .push_channel(7, true, b"channel-msg")
            .expect("push channel");
        let sample_batch = builder.into_bytes();

        c.bench_function("host_batch_parse_10_records", |b| {
            b.iter(|| {
                let parsed =
                    NveHostEventBatchRef::parse(black_box(&sample_batch)).expect("parse batch");
                let mut total_payload = 0usize;
                for rec in parsed.iter() {
                    total_payload += rec.payload.len();
                }
                black_box(total_payload)
            });
        });

        let reg = HostServiceRegistry::new().with_handler(InlineEchoHandler);
        let bridge = HostCallbackBridge::new(reg);
        let ffi = FfiHostServices::with_callbacks(bridge.callbacks());
        let req = b"sync-bridge-payload";

        c.bench_function("host_sync_bridge_roundtrip_zero_alloc", |b| {
            b.iter(|| {
                let resp = ffi
                    .invoke_sync(SERVICE_CLIPBOARD, 1, black_box(req))
                    .expect("sync bridge invoke");
                black_box(resp.len())
            });
        });

        // `FfiHostServices::ingest_batch` across 10 mixed records (8 responses + 1 push + 1 channel)
        // draining the typed bus each iteration so queue length stays constant.
        let ingest_ffi = FfiHostServices::new();
        c.bench_function("host_batch_ingest_10_records", |b| {
            b.iter(|| {
                let stats = ingest_ffi
                    .ingest_batch(black_box(&sample_batch))
                    .expect("ingest batch");
                let drained = ingest_ffi.typed_events().poll_all();
                black_box((stats.total(), drained.len()))
            });
        });

        // `CompletionSink::drain_batch_into` reusing a pre-allocated `NveHostEventBatchBuilder`.
        let sink = CompletionSink::new();
        let mut reuse_builder = NveHostEventBatchBuilder::with_capacity(512);
        c.bench_function("host_sink_drain_batch_into_10_records", |b| {
            b.iter(|| {
                for i in 0..8u64 {
                    let _ = sink.register_in_flight(i + 1, SERVICE_CLIPBOARD, 1);
                    let _ = sink.complete_ok(i + 1, b"ok-payload".to_vec());
                }
                sink.push_event(SERVICE_SYSTEM_ENV, 1, theme_bytes.to_vec());
                sink.push_channel(7, true, b"channel-msg".to_vec());
                reuse_builder.clear();
                let written = sink
                    .drain_batch_into(&mut reuse_builder)
                    .expect("drain batch into");
                black_box((written, reuse_builder.as_bytes().len()))
            });
        });
    }

    criterion_group!(benches, bench_host_event_batch);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
