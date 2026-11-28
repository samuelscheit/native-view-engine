//! Criterion benchmark comparing lock-free [`ServiceSlots`] + cached [`ServiceHandle`] against
//! legacy [`HostServiceRegistry`] `resolve_typed` (`RwLock<BTreeMap>` + double-`Arc`) over a
//! realistic 7-slot capability table (`slots 0..=6`).

#[cfg(target_arch = "wasm32")]
fn main() {}

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::any::{Any, TypeId};
    use std::hint::black_box;
    use std::sync::Arc;

    use criterion::{criterion_group, Criterion};
    use nve_abi::{
        SERVICE_CLIPBOARD, SERVICE_FILESYSTEM, SERVICE_HAPTICS, SERVICE_LINKING,
        SERVICE_NOTIFICATIONS, SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV,
    };
    use nve_host_services::{
        pack_typed, resolve_typed, CompletionSink, HandlerReply, HostServiceHandler,
        HostServiceRegistry, ServiceError, ServiceHandle, ServiceSlot, ServiceSlotId, ServiceSlots,
        SLOT_CLIPBOARD, SLOT_FILESYSTEM, SLOT_HAPTICS, SLOT_LINKING, SLOT_NOTIFICATIONS,
        SLOT_SECURE_STORE, SLOT_SYSTEM_ENV,
    };

    trait BenchBackend: Send + Sync {
        fn add(&self, x: u64) -> u64;
    }

    impl ServiceSlot for dyn BenchBackend {
        const SLOT: ServiceSlotId = SLOT_NOTIFICATIONS;
    }

    struct FastBackend {
        bias: u64,
    }

    impl BenchBackend for FastBackend {
        #[inline(never)]
        fn add(&self, x: u64) -> u64 {
            x.wrapping_add(self.bias)
        }
    }

    impl HostServiceHandler for FastBackend {
        fn service_id(&self) -> u32 {
            SERVICE_NOTIFICATIONS
        }

        fn handle(
            &self,
            _method_id: u32,
            _request_id: u64,
            _request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            Err(ServiceError::Unsupported)
        }

        fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
            Some(pack_typed::<dyn BenchBackend>(self))
        }
    }

    struct FillerHandler(u32);

    impl HostServiceHandler for FillerHandler {
        fn service_id(&self) -> u32 {
            self.0
        }

        fn handle(
            &self,
            _method_id: u32,
            _request_id: u64,
            _request: &[u8],
            _done: &CompletionSink,
        ) -> Result<HandlerReply, ServiceError> {
            Err(ServiceError::Unsupported)
        }
    }

    fn bench_service_slots(c: &mut Criterion) {
        let mut group = c.benchmark_group("service_dispatch");

        let impl_arc = Arc::new(FastBackend { bias: 42 });
        let trait_arc: Arc<dyn BenchBackend> = Arc::clone(&impl_arc) as Arc<dyn BenchBackend>;

        // Populate a realistic multi-slot table (slots 0..=6) for both ServiceSlots and legacy
        // HostServiceRegistry so BTreeMap traversal and dense array indexing reflect production.
        let slots = ServiceSlots::builder()
            .set_value_at(SLOT_CLIPBOARD, 0u32)
            .set_value_at(SLOT_FILESYSTEM, 1u32)
            .set_value_at(SLOT_SECURE_STORE, 2u32)
            .set_value_at(SLOT_LINKING, 3u32)
            .set_value_at(SLOT_SYSTEM_ENV, 4u32)
            .set(Arc::clone(&trait_arc))
            .set_value_at(SLOT_HAPTICS, 6u32)
            .build();
        let cached: ServiceHandle<dyn BenchBackend> = slots
            .handle::<dyn BenchBackend>()
            .expect("cached ServiceHandle");

        let legacy_registry = HostServiceRegistry::new()
            .with_handler(FillerHandler(SERVICE_CLIPBOARD))
            .with_handler(FillerHandler(SERVICE_FILESYSTEM))
            .with_handler(FillerHandler(SERVICE_SECURE_STORE))
            .with_handler(FillerHandler(SERVICE_LINKING))
            .with_handler(FillerHandler(SERVICE_SYSTEM_ENV))
            .with_handler_arc(impl_arc)
            .with_handler(FillerHandler(SERVICE_HAPTICS));

        group.bench_function("service_handle_cached_call", |b| {
            b.iter(|| {
                let h = black_box(&cached);
                black_box(h.add(black_box(123)))
            });
        });

        group.bench_function("service_slots_get_and_call", |b| {
            b.iter(|| {
                let svc = black_box(&slots)
                    .get::<dyn BenchBackend>()
                    .expect("lock-free slot lookup");
                black_box(svc.add(black_box(123)))
            });
        });

        group.bench_function("service_slots_get_arc_and_call", |b| {
            b.iter(|| {
                let svc = black_box(&slots)
                    .get_arc::<dyn BenchBackend>()
                    .expect("lock-free arc lookup");
                black_box(svc.add(black_box(123)))
            });
        });

        group.bench_function("legacy_registry_resolve_typed_and_call", |b| {
            b.iter(|| {
                let svc = resolve_typed::<dyn BenchBackend>(
                    black_box(&legacy_registry),
                    SERVICE_NOTIFICATIONS,
                    1,
                )
                .expect("legacy RwLock<BTreeMap> lookup");
                black_box(svc.add(black_box(123)))
            });
        });

        group.finish();
    }

    criterion_group!(benches, bench_service_slots);
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    native::benches();
    criterion::Criterion::default()
        .configure_from_args()
        .final_summary();
}
