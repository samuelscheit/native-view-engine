//! End-to-end `NVE_IFACE_HOST = 5` tests for `hosts/web-shell`: `WebHostServices` via
//! `HostCallbackBridge`, `WebThemeWatcher` push batching reaching Studio, async `Pending`
//! cancellation on unbind, and `WebAppSession` drop-order safety.

#[cfg(all(feature = "app-studio", feature = "app-kitchen-sink"))]
mod suite {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use nve_abi::{
        encode_single_push_into, ColorScheme, NveHostCallbacks, NveHostEventBatchBuilder,
        NveHostStatus, SERVICE_CLIPBOARD, SERVICE_HAPTICS, SERVICE_LINKING, SERVICE_SYSTEM_ENV,
    };
    use nve_backend_web::WebBackend;
    use nve_clipboard_web::WebClipboard;
    use nve_engine::{NveApp, Viewport};
    use nve_example_kitchen_sink::KitchenSinkModule;
    use nve_host_services::{
        FfiHostServices, HostCallbackBridge, HostEvent, HostPushEvent, HostServiceProvider,
        InvokeResult, MemoryHostServices, ServiceError,
    };
    use nve_linking_web::LinkingClient;
    use nve_studio_workspace::StudioWorkspaceModule;
    use nve_system_env_web::{
        locale_from_browser_fields, theme_from_media_flags, LocaleSnapshot, ThemeSnapshot,
        WebSystemEnv, EVENT_THEME_CHANGED,
    };

    use crate::host_services::WebHostServices;
    use crate::session::{WebAppSession, DEFAULT_SURFACE_ID};

    #[test]
    fn web_host_services_binds_studio_delivers_theme_push_batch_and_drops_safely() {
        let clip = Arc::new(WebClipboard::headless());
        let initial_light = theme_from_media_flags(false, false, false, false, 2.0);
        let env = Arc::new(WebSystemEnv::with_initial(
            initial_light,
            LocaleSnapshot::default(),
        ));
        let web_host = WebHostServices::headless(clip, env);
        let watcher = web_host.install_theme_watcher();

        let vp = Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 2.0,
        };
        let mut session =
            WebAppSession::new_studio_workspace(DEFAULT_SURFACE_ID, WebBackend::new(), vp);
        session.bind_web_host_services(Arc::clone(&web_host));
        session.render_initial().expect("initial studio render");
        let html_light = session.render_html();
        assert!(
            html_light.contains("Light | en-US"),
            "binding WebHostServices must sync initial browser Light theme into Studio"
        );

        // Trigger a live browser matchMedia theme transition -> canonical NveHostEventBatch Push.
        let dark = theme_from_media_flags(true, true, true, false, 2.0);
        watcher.notify_theme_change(dark);
        let batch = web_host
            .drain_host_event_batch()
            .expect("watcher must emit canonical NveHostEventBatch");
        let timing = session
            .dispatch_host_events(&batch)
            .expect("dispatch batch ok");
        assert!(
            timing.is_some(),
            "Studio must re-render on canonical SERVICE_SYSTEM_ENV Push batch"
        );
        let html_dark = session.render_html();
        assert!(
            html_dark.contains("Dark | en-US"),
            "Studio must reflect Dark theme pushed via NveHostEventBatch"
        );

        // Live locale push (EVENT_LOCALE_CHANGED) re-renders Studio with updated locale tag.
        let de_locale = locale_from_browser_fields("de-DE", Some("Europe/Berlin"), true);
        watcher.notify_locale_change(de_locale);
        let locale_batch = web_host
            .drain_host_event_batch()
            .expect("watcher must emit locale push batch");
        assert!(
            session
                .dispatch_host_events(&locale_batch)
                .expect("dispatch locale batch")
                .is_some(),
            "Studio must re-render on canonical EVENT_LOCALE_CHANGED Push batch"
        );
        assert!(
            session.render_html().contains("Dark | de-DE"),
            "Studio must reflect de-DE locale pushed via NveHostEventBatch"
        );

        // Raw 12-byte ThemeSnapshot (non-batch) MUST be ignored (zero repr-sniffing).
        let raw_12 = initial_light.to_repr_c_bytes();
        let ignored = session
            .dispatch_host_events(&raw_12)
            .expect("malformed non-batch must not error");
        assert!(
            ignored.is_none(),
            "non-NVHB 12-byte slice must never be sniffed as ThemeSnapshot"
        );

        // Binding an identical WebHostServices returns dirty=false (no redundant hydration render),
        // and swapping to raw bind_host(Some(...)) releases the previous owned bridge cleanly.
        assert!(
            !session.bind_web_host_services(Arc::clone(&web_host)),
            "binding matching env must not dirty Studio"
        );
        let raw_noop = NveHostCallbacks {
            host_ctx: std::ptr::null_mut(),
            invoke: None,
            channel_publish: None,
        };
        let _ = session.bind_host(Some(raw_noop));
        assert_eq!(
            Arc::strong_count(&web_host),
            1,
            "bind_host(Some(...)) must drop previously owned WebHostServices bridge immediately"
        );

        // Dropping session unbinds Studio before releasing WebHostServices.
        drop(watcher);
        drop(session);
        assert_eq!(Arc::strong_count(&web_host), 1);
    }

    #[test]
    fn web_host_services_async_clipboard_linking_haptics_and_drop_order() {
        use nve_host_services::{CompletionSink, HandlerReply, HostServiceHandler, HostServiceRegistry};

        let clip = Arc::new(WebClipboard::headless());
        let env = Arc::new(WebSystemEnv::with_initial(
            ThemeSnapshot::default(),
            LocaleSnapshot::default(),
        ));
        let web_host = WebHostServices::headless(Arc::clone(&clip), env);

        let bridge = web_host.into_bridge();
        let ffi = FfiHostServices::with_callbacks(bridge.callbacks());

        // Linking allowlist + rejection over C ABI bridge.
        let linking = LinkingClient::new(&ffi);
        assert!(linking.can_open_url("https://nve.dev").unwrap());
        assert!(!linking.can_open_url("javascript:alert(1)").unwrap());
        assert!(matches!(
            linking.open_url("file:///etc/passwd"),
            Err(ServiceError::InvalidArg(_))
        ));

        // Haptics returns Unsupported cleanly on non-browser host.
        assert_eq!(
            ffi.invoke_sync(SERVICE_HAPTICS, 3, &[]),
            Err(ServiceError::Unsupported)
        );
        assert_eq!(
            ffi.invoke_sync(SERVICE_LINKING, 99, &[]),
            Err(ServiceError::Unsupported)
        );

        // Async Pending handler over C ABI bridge + cancellation on host cancel_pending.
        struct PendingClipHandler;
        impl HostServiceHandler for PendingClipHandler {
            fn service_id(&self) -> u32 {
                SERVICE_CLIPBOARD
            }
            fn handle(
                &self,
                _: u32,
                _: u64,
                _: &[u8],
                _: &CompletionSink,
            ) -> Result<HandlerReply, ServiceError> {
                Ok(HandlerReply::Pending)
            }
        }
        let async_reg = Arc::new(HostServiceRegistry::new().with_handler(PendingClipHandler));
        let async_bridge = HostCallbackBridge::new(Arc::clone(&async_reg));
        let async_ffi = FfiHostServices::with_callbacks(async_bridge.callbacks());

        let cb_results = Arc::new(Mutex::new(Vec::new()));
        let cb_clone = Arc::clone(&cb_results);
        let req_id = async_ffi
            .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
                cb_clone.lock().unwrap().push(res);
            })
            .unwrap()
            .expect("clipboard read must return Pending");
        assert_eq!(async_reg.completion_sink().in_flight_count(), 1);
        assert_eq!(async_reg.cancel_pending(), 1);
        let cancel_batch = async_reg
            .drain_host_event_batch()
            .expect("cancelled batch");
        let stats = async_ffi.ingest_batch(&cancel_batch).unwrap();
        assert_eq!(stats.responses, 1);
        assert_eq!(
            cb_results.lock().unwrap().as_slice(),
            &[Err(ServiceError::Cancelled)]
        );
        assert_eq!(async_ffi.take_async_response(req_id), None);

        // Verify WebAppSession drops KitchenSink unbinding BEFORE bridge drops by observing
        // `!bridge_dropped` inside a real `Pending` call's `Cancelled` callback triggered on drop!
        struct DropOrderProvider {
            bridge_dropped: Arc<AtomicBool>,
        }
        impl Drop for DropOrderProvider {
            fn drop(&mut self) {
                self.bridge_dropped.store(true, Ordering::SeqCst);
            }
        }
        impl HostServiceProvider for DropOrderProvider {
            fn invoke_raw_with_id(
                &self,
                _service_id: u32,
                _method_id: u32,
                request_id: u64,
                _request: &[u8],
            ) -> Result<InvokeResult, ServiceError> {
                assert!(
                    !self.bridge_dropped.load(Ordering::SeqCst),
                    "provider invoked after drop!"
                );
                Ok(InvokeResult::Pending { request_id })
            }
            fn channel_publish(&self, _: u32, _: bool, _: &[u8]) -> Result<(), ServiceError> {
                Ok(())
            }
            fn poll_host_events(&self) -> Vec<HostEvent> {
                Vec::new()
            }
        }

        let dropped = Arc::new(AtomicBool::new(false));
        let callback_fired = Arc::new(AtomicUsize::new(0));
        let vp = Viewport {
            width: 800.0,
            height: 600.0,
            scale_factor: 1.0,
        };
        let mut ks = WebAppSession::new_kitchen_sink(DEFAULT_SURFACE_ID, WebBackend::new(), vp);
        ks.bind_host_bridge(HostCallbackBridge::new(DropOrderProvider {
            bridge_dropped: Arc::clone(&dropped),
        }));
        ks.render_initial().expect("kitchen-sink render");

        let dropped_in_cb = Arc::clone(&dropped);
        let fired_in_cb = Arc::clone(&callback_fired);
        let pending_id = ks
            .kitchen_sink_host_services()
            .expect("kitchen sink host services")
            .invoke_with_callback(SERVICE_CLIPBOARD, 1, &[], move |res| {
                assert_eq!(res, Err(ServiceError::Cancelled));
                assert!(
                    !dropped_in_cb.load(Ordering::SeqCst),
                    "host bridge MUST stay alive until app unbind cancels pending callbacks"
                );
                fired_in_cb.fetch_add(1, Ordering::SeqCst);
            })
            .expect("invoke_with_callback ok")
            .expect("must return Pending");
        assert!(pending_id >= 1);
        assert!(!dropped.load(Ordering::SeqCst));
        drop(ks);
        assert_eq!(
            callback_fired.load(Ordering::SeqCst),
            1,
            "pending callback must fire exactly once on WebAppSession drop"
        );
        assert!(dropped.load(Ordering::SeqCst));

        // Verify ColorScheme sanity from theme_from_media_flags.
        assert_eq!(
            theme_from_media_flags(true, false, false, false, 1.0).color_scheme,
            Some(ColorScheme::Dark)
        );
    }

    #[test]
    fn thousand_theme_pushes_do_not_leak_queues_and_linking_pushes_survive_studio_ingest() {
        let host_mem = MemoryHostServices::new();
        let bridge = HostCallbackBridge::new(host_mem);
        let cbs = bridge.callbacks();

        let mut ks = KitchenSinkModule::create(DEFAULT_SURFACE_ID);
        ks.bind_host(Some(cbs));
        let mut studio = StudioWorkspaceModule::create(DEFAULT_SURFACE_ID);
        studio.bind_host(Some(cbs));
        assert!(studio.host_services().is_ffi_bound());

        let dark = theme_from_media_flags(true, false, false, false, 2.0).to_repr_c_bytes();
        let light = theme_from_media_flags(false, false, false, false, 2.0).to_repr_c_bytes();
        let mut stack = [0u8; 56];

        for i in 0..1_000usize {
            let payload = if i % 2 == 0 { &dark } else { &light };
            let batch = encode_single_push_into(
                &mut stack,
                SERVICE_SYSTEM_ENV,
                EVENT_THEME_CHANGED,
                payload,
            )
            .unwrap();
            let _ = ks.dispatch_host_events(batch);
            let _ = studio.dispatch_host_events(batch);
        }

        assert_eq!(
            ks.host_services().typed_events().queued_len(),
            0,
            "KitchenSink typed_events queue must not leak on repeated theme pushes"
        );
        assert!(ks.host_services().poll_host_events().is_empty());
        assert_eq!(
            studio.host_services().typed_event_bus().unwrap().queued_len(),
            0,
            "Studio typed_events queue must not leak on repeated theme pushes"
        );
        assert!(studio.host_services().poll_host_events().is_empty());

        // Cancelled response MUST return false from KitchenSinkModule::dispatch_host_events.
        let mut cancel_b = NveHostEventBatchBuilder::new();
        cancel_b
            .push_response(SERVICE_CLIPBOARD, 1, 999, NveHostStatus::Cancelled, &[])
            .unwrap();
        assert!(
            !ks.dispatch_host_events(&cancel_b.into_bytes()),
            "Cancelled response must not trigger wasted render in KitchenSink"
        );

        // Non-SERVICE_SYSTEM_ENV push events (e.g. SERVICE_LINKING) MUST survive Studio ingest!
        let mut mixed = NveHostEventBatchBuilder::new();
        mixed
            .push_push(
                SERVICE_SYSTEM_ENV,
                EVENT_THEME_CHANGED,
                &dark,
            )
            .unwrap();
        mixed
            .push_push(SERVICE_LINKING, 1, b"https://nve.dev/deep/link")
            .unwrap();
        let report = studio
            .host_services()
            .ingest_host_batch(&mixed.into_bytes())
            .expect("ingest mixed batch");
        assert!(report.updated_theme.is_some());
        let remaining = studio
            .host_services()
            .typed_event_bus()
            .unwrap()
            .drain_typed::<HostPushEvent>();
        assert_eq!(
            remaining.len(),
            1,
            "SERVICE_LINKING push must survive StudioHostServices::ingest_host_batch"
        );
        assert_eq!(remaining[0].service_id, SERVICE_LINKING);
        assert_eq!(remaining[0].payload, b"https://nve.dev/deep/link");

        // 10,000 undrained SERVICE_LINKING pushes stay strictly bounded by DEFAULT_TYPED_EVENT_QUEUE_CAP.
        for i in 0..10_000u32 {
            let mut lb = NveHostEventBatchBuilder::new();
            let url = format!("https://nve.dev/deep/{i}");
            lb.push_push(SERVICE_LINKING, 1, url.as_bytes()).unwrap();
            let _ = studio.host_services().ingest_host_batch(&lb.into_bytes()).unwrap();
        }
        let studio_bus = studio.host_services().typed_event_bus().unwrap();
        assert!(
            studio_bus.queued_len() <= nve_host_services::DEFAULT_TYPED_EVENT_QUEUE_CAP,
            "10k undrained linking pushes must remain bounded (got {})",
            studio_bus.queued_len()
        );
        assert!(studio_bus.dropped_count() > 0);
        let drained_links = studio_bus.drain_typed::<HostPushEvent>();
        assert_eq!(drained_links.last().unwrap().payload, b"https://nve.dev/deep/9999");

        // dispatch_system_env_events + drain_system_env_events leaves typed_event_bus empty (0 snapshot leak).
        let _ = studio.host_services().dispatch_system_env_events(&[
            nve_system_env_web::SystemEnvEvent::ThemeChanged(ThemeSnapshot::default()),
        ]);
        let _ = studio.host_services().drain_system_env_events();
        assert_eq!(studio_bus.queued_len(), 0);

        // Unbind before `bridge` drops so neither module holds a dangling pointer.
        ks.bind_host(None);
        studio.bind_host(None);
        drop(ks);
        drop(studio);
        drop(bridge);

        // StudioHostServices forwards invoke_raw_with_id (both unbound memory & bound FFI) and cancel_pending.
        let mem = Arc::new(MemoryHostServices::new());
        mem.inject_async_pending(SERVICE_CLIPBOARD, 1);
        mem.inject_async_pending(SERVICE_CLIPBOARD, 1);
        let studio_svc = nve_studio_workspace::StudioHostServices::with_memory(Arc::clone(&mem));
        let r1 = studio_svc
            .invoke_raw_with_id(SERVICE_CLIPBOARD, 1, 701, &[])
            .unwrap();
        let r2 = studio_svc
            .invoke_raw_with_id(SERVICE_CLIPBOARD, 1, 702, &[])
            .unwrap();
        assert_eq!(r1, InvokeResult::Pending { request_id: 701 });
        assert_eq!(r2, InvokeResult::Pending { request_id: 702 });
        mem.complete_async(701, NveHostStatus::Ok, b"alpha".to_vec());
        mem.complete_async(702, NveHostStatus::Ok, b"beta".to_vec());
        assert_eq!(studio_svc.take_async_response(701), Some(Ok(b"alpha".to_vec())));
        assert_eq!(studio_svc.take_async_response(702), Some(Ok(b"beta".to_vec())));

        let bound_mem = Arc::new(MemoryHostServices::new());
        bound_mem.inject_async_pending(SERVICE_CLIPBOARD, 1);
        let bound_bridge = HostCallbackBridge::new(Arc::clone(&bound_mem));
        studio_svc.bind_host(Some(bound_bridge.callbacks()));
        let br = studio_svc
            .invoke_raw_with_id(SERVICE_CLIPBOARD, 1, 909, &[])
            .unwrap();
        assert_eq!(br, InvokeResult::Pending { request_id: 909 });
        assert_eq!(studio_svc.cancel_pending(), 1);
        studio_svc.bind_host(None);
    }
}
