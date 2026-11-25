//! Deep transport, C ABI `NVE_IFACE_HOST = 5`, `HostServiceRegistry`, fault-injection,
//! zero-serialization [`TypedEventBus`] / [`TypedAsyncTable`], and `NveChannelBus` tests
//! for `nve-host-services`.

use std::any::{Any, TypeId};
use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveAppHandle, NveBufferSlice, NveHostApi, NveHostCallbacks, NveHostStatus,
    NVE_CHANNEL_FLAG_RETAIN, NVE_IFACE_HOST,
};
use nve_host_services::{
    fnv1a64, pack_typed, resolve_typed, validate_url, BinReader, BinWriter, FfiHostServices,
    HostCallbackBridge, HostServiceHandler, HostServiceProvider, HostServiceRegistry, InvokeResult,
    MemoryHostServices, NativeHostServices, PermissionStatus, ServiceError, TypedEventBus,
    FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE,
};
use nve_interop::CHANNEL_ID_THEME;

const TEST_SERVICE_ECHO: u32 = 0x1001;
const TEST_METHOD_ECHO: u32 = 1;
const TEST_METHOD_BIG: u32 = 2;

struct EchoHandler {
    big_payload: Vec<u8>,
}

impl HostServiceHandler for EchoHandler {
    fn service_id(&self) -> u32 {
        TEST_SERVICE_ECHO
    }

    fn handle(&self, method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        match method_id {
            TEST_METHOD_ECHO => Ok(request.to_vec()),
            TEST_METHOD_BIG => Ok(self.big_payload.clone()),
            _ => Err(ServiceError::Unsupported),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SampleThemeEvent {
    dark: bool,
    scale_x100: u16,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SampleAlertEvent {
    id: u32,
    message: String,
}

#[test]
fn registry_and_native_host_services_compose_handlers_cleanly() {
    let reg = HostServiceRegistry::new();
    assert!(!reg.has_service(TEST_SERVICE_ECHO));
    assert_eq!(
        reg.invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"hi"),
        Err(ServiceError::Unsupported)
    );

    reg.register(EchoHandler {
        big_payload: vec![7u8; 32],
    });
    assert!(reg.has_service(TEST_SERVICE_ECHO));
    assert_eq!(reg.registered_services(), vec![TEST_SERVICE_ECHO]);
    assert_eq!(
        reg.invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"hello")
            .unwrap(),
        b"hello"
    );
    assert_eq!(
        reg.invoke_sync(TEST_SERVICE_ECHO, 999, &[]),
        Err(ServiceError::Unsupported)
    );
    assert!(reg.unregister(TEST_SERVICE_ECHO));
    assert!(!reg.unregister(TEST_SERVICE_ECHO));

    let native = NativeHostServices::new().with_handler(EchoHandler {
        big_payload: vec![1, 2, 3],
    });
    assert_eq!(
        native
            .invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_BIG, &[])
            .unwrap(),
        vec![1, 2, 3]
    );
}

#[test]
fn typed_event_bus_zero_copy_fanout_retained_replay_and_selective_drain() {
    let bus = TypedEventBus::new();

    // Publish retained event before late subscriber joins.
    let initial = SampleThemeEvent {
        dark: true,
        scale_x100: 200,
    };
    assert_eq!(bus.publish_retained(initial.clone()), 0);
    assert_eq!(bus.retained::<SampleThemeEvent>().as_deref(), Some(&initial));

    let seen_a = Arc::new(Mutex::new(Vec::<SampleThemeEvent>::new()));
    let seen_b = Arc::new(Mutex::new(Vec::<SampleThemeEvent>::new()));
    let sa = Arc::clone(&seen_a);
    let sb = Arc::clone(&seen_b);

    let sub_a = bus.subscribe::<SampleThemeEvent, _>(true, move |ev| {
        sa.lock().unwrap().push(ev.clone());
    });
    let sub_b = bus.subscribe::<SampleThemeEvent, _>(false, move |ev| {
        sb.lock().unwrap().push(ev.clone());
    });
    assert_eq!(bus.subscriber_count::<SampleThemeEvent>(), 2);
    // Late subscriber with replay_retained=true received the retained snapshot immediately.
    assert_eq!(*seen_a.lock().unwrap(), vec![initial.clone()]);
    assert!(seen_b.lock().unwrap().is_empty());

    // Publish a second theme update + an unrelated alert event.
    let updated = SampleThemeEvent {
        dark: false,
        scale_x100: 150,
    };
    assert_eq!(bus.publish_retained(updated.clone()), 2);
    assert_eq!(
        bus.publish(SampleAlertEvent {
            id: 42,
            message: "build complete".into(),
        }),
        0
    );
    assert_eq!(
        *seen_a.lock().unwrap(),
        vec![initial.clone(), updated.clone()]
    );
    assert_eq!(*seen_b.lock().unwrap(), vec![updated.clone()]);

    // Selective drain removes only SampleThemeEvent, leaving SampleAlertEvent intact.
    let drained_themes = bus.drain_typed::<SampleThemeEvent>();
    assert_eq!(drained_themes.len(), 2);
    assert_eq!(*drained_themes[0], initial);
    assert_eq!(*drained_themes[1], updated);
    assert_eq!(bus.queued_len(), 1);

    let drained_alerts = bus.drain_typed::<SampleAlertEvent>();
    assert_eq!(drained_alerts.len(), 1);
    assert_eq!(drained_alerts[0].id, 42);
    assert_eq!(bus.queued_len(), 0);

    // Unsubscribe stops further delivery cleanly.
    assert!(bus.unsubscribe(sub_a));
    assert!(!bus.unsubscribe(sub_a));
    assert_eq!(bus.subscriber_count::<SampleThemeEvent>(), 1);
    assert_eq!(bus.publish(initial), 1);
    assert!(bus.unsubscribe(sub_b));
    assert!(bus.clear_retained::<SampleThemeEvent>());
    assert!(bus.retained::<SampleThemeEvent>().is_none());
}

#[test]
fn memory_host_services_fault_injection_typed_async_and_typed_events() {
    let mem = MemoryHostServices::new().with_handler(EchoHandler {
        big_payload: vec![0xAB; 16],
    });
    assert_eq!(
        mem.invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"ok")
            .unwrap(),
        b"ok"
    );

    mem.inject_status(
        TEST_SERVICE_ECHO,
        TEST_METHOD_ECHO,
        NveHostStatus::PermissionDenied,
    );
    assert_eq!(
        mem.invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"ok"),
        Err(ServiceError::PermissionDenied)
    );
    // One-shot fault cleared on next call.
    assert_eq!(
        mem.invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"ok")
            .unwrap(),
        b"ok"
    );

    mem.inject_async_pending(TEST_SERVICE_ECHO, TEST_METHOD_ECHO);
    let InvokeResult::Pending { request_id } = mem
        .invoke_raw(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"wait")
        .unwrap()
    else {
        panic!("expected Pending");
    };
    assert_eq!(mem.take_async_response(request_id), None);
    mem.complete_async(request_id, NveHostStatus::Ok, b"done".to_vec());
    assert_eq!(
        mem.take_async_response(request_id),
        Some(Ok(b"done".to_vec()))
    );
    assert_eq!(mem.take_async_response(request_id), None);

    // Direct typed async request resolution (polling + callback, zero serialization).
    let typed_req1 = mem.register_typed_async::<SampleAlertEvent, fn(_)>(None);
    assert!(mem.take_typed_async::<SampleAlertEvent>(typed_req1).is_none());
    mem.complete_typed_async(
        typed_req1,
        Ok(SampleAlertEvent {
            id: 7,
            message: "typed-ok".into(),
        }),
    );
    let resolved = mem
        .take_typed_async::<SampleAlertEvent>(typed_req1)
        .unwrap()
        .unwrap();
    assert_eq!(resolved.id, 7);
    assert_eq!(resolved.message, "typed-ok");

    let cb_slot = Arc::new(Mutex::new(None));
    let cb_clone = Arc::clone(&cb_slot);
    let typed_req2 = mem.register_typed_async::<SampleAlertEvent, _>(Some(move |res| {
        *cb_clone.lock().unwrap() = Some(res);
    }));
    mem.complete_typed_async::<SampleAlertEvent>(typed_req2, Err(ServiceError::Cancelled));
    assert_eq!(*cb_slot.lock().unwrap(), Some(Err(ServiceError::Cancelled)));

    // Direct typed event emission & retained replay via `MemoryHostServices`.
    let received = Arc::new(Mutex::new(Vec::new()));
    let rx = Arc::clone(&received);
    mem.emit_typed_retained(SampleThemeEvent {
        dark: true,
        scale_x100: 200,
    });
    let _sub = mem.subscribe_typed::<SampleThemeEvent, _>(true, move |ev| {
        rx.lock().unwrap().push(ev.clone());
    });
    assert_eq!(received.lock().unwrap().len(), 1);
    assert_eq!(mem.drain_typed_events::<SampleThemeEvent>().len(), 1);
}

struct MockCAbiHost {
    invoke_calls: AtomicUsize,
    last_pending_req_id: AtomicUsize,
    large_payload: Vec<u8>,
    published_channels: Mutex<Vec<(u32, bool, Vec<u8>)>>,
}

unsafe extern "C" fn mock_c_invoke(
    host_ctx: *mut c_void,
    service_id: u32,
    method_id: u32,
    request_id: u64,
    _request: NveBufferSlice,
    out_buf: *mut u8,
    out_cap: usize,
    out_len: *mut usize,
) -> i32 {
    let host = unsafe { &*(host_ctx as *const MockCAbiHost) };
    host.invoke_calls.fetch_add(1, Ordering::SeqCst);

    match (service_id, method_id) {
        (TEST_SERVICE_ECHO, 1) => {
            unsafe { *out_len = 1 };
            if out_cap < 1 {
                return NveHostStatus::BufferTooSmall.as_i32();
            }
            unsafe { *out_buf = 1 };
            NveHostStatus::Ok.as_i32()
        }
        (TEST_SERVICE_ECHO, 2) => {
            let needed = host.large_payload.len();
            unsafe { *out_len = needed };
            if out_cap < needed {
                return NveHostStatus::BufferTooSmall.as_i32();
            }
            unsafe {
                std::ptr::copy_nonoverlapping(host.large_payload.as_ptr(), out_buf, needed);
            }
            NveHostStatus::Ok.as_i32()
        }
        (TEST_SERVICE_ECHO, 3) => {
            host.last_pending_req_id
                .store(request_id as usize, Ordering::SeqCst);
            unsafe { *out_len = 0 };
            NveHostStatus::Pending.as_i32()
        }
        (TEST_SERVICE_ECHO, 99) => {
            unsafe { *out_len = FFI_MAX_RESPONSE_BYTES + 1024 };
            NveHostStatus::BufferTooSmall.as_i32()
        }
        _ => NveHostStatus::Unsupported.as_i32(),
    }
}

unsafe extern "C" fn mock_c_channel_publish(
    host_ctx: *mut c_void,
    channel_id: u32,
    flags: u32,
    payload: NveBufferSlice,
) -> i32 {
    let host = unsafe { &*(host_ctx as *const MockCAbiHost) };
    let bytes = unsafe { payload.as_slice() }.to_vec();
    host.published_channels
        .lock()
        .unwrap()
        .push((channel_id, (flags & NVE_CHANNEL_FLAG_RETAIN) != 0, bytes));
    NveHostStatus::Ok.as_i32()
}

#[test]
fn ffi_host_services_fast_path_buffer_too_small_retry_and_typed_events() {
    let large_payload = vec![0x42u8; FFI_STACK_BUF_SIZE + 8192];
    let host = Box::new(MockCAbiHost {
        invoke_calls: AtomicUsize::new(0),
        last_pending_req_id: AtomicUsize::new(0),
        large_payload: large_payload.clone(),
        published_channels: Mutex::new(Vec::new()),
    });
    let callbacks = NveHostCallbacks {
        host_ctx: (&*host as *const MockCAbiHost) as *mut c_void,
        invoke: Some(mock_c_invoke),
        channel_publish: Some(mock_c_channel_publish),
    };

    let ffi = FfiHostServices::with_callbacks(callbacks);

    assert_eq!(ffi.invoke_sync(TEST_SERVICE_ECHO, 1, &[]).unwrap(), vec![1]);
    assert_eq!(host.invoke_calls.load(Ordering::SeqCst), 1);

    assert_eq!(
        ffi.invoke_sync(TEST_SERVICE_ECHO, 2, &[]).unwrap(),
        large_payload
    );
    assert_eq!(host.invoke_calls.load(Ordering::SeqCst), 3);

    assert!(matches!(
        ffi.invoke_raw(TEST_SERVICE_ECHO, 99, &[]),
        Err(ServiceError::BufferTooSmall { .. })
    ));

    let InvokeResult::Pending { request_id: req1 } =
        ffi.invoke_raw(TEST_SERVICE_ECHO, 3, &[]).unwrap()
    else {
        panic!("expected Pending");
    };
    assert_eq!(ffi.pending_request_count(), 1);
    assert_eq!(ffi.take_async_response(req1), None);

    // Typed event bus on FfiHostServices delivers typed events without byte encoding.
    ffi.typed_events().publish_retained(SampleThemeEvent {
        dark: true,
        scale_x100: 200,
    });
    assert_eq!(
        ffi.typed_event_bus()
            .unwrap()
            .drain_typed::<SampleThemeEvent>()
            .len(),
        1
    );

    ffi.channel_publish(CHANNEL_ID_THEME, true, b"outbound")
        .unwrap();
    let published = host.published_channels.lock().unwrap();
    assert_eq!(
        published.as_slice(),
        &[(CHANNEL_ID_THEME, true, b"outbound".to_vec())]
    );
}

#[test]
fn c_abi_nve_iface_host_5_roundtrip_via_host_callback_bridge() {
    struct MiniHostApp {
        ffi: FfiHostServices,
        dispatched_bytes: usize,
    }

    unsafe extern "C" fn app_bind_host(handle: NveAppHandle, callbacks: *const NveHostCallbacks) {
        let app = unsafe { &mut *(handle as *mut MiniHostApp) };
        let bound = if callbacks.is_null() {
            None
        } else {
            Some(unsafe { *callbacks })
        };
        app.ffi.bind(bound);
    }

    unsafe extern "C" fn app_dispatch_host_events(
        handle: NveAppHandle,
        batch: NveBufferSlice,
    ) -> u32 {
        let app = unsafe { &mut *(handle as *mut MiniHostApp) };
        let bytes = unsafe { batch.as_slice() };
        app.dispatched_bytes += bytes.len();
        u32::from(!bytes.is_empty())
    }

    static MINI_HOST_API: NveHostApi = NveHostApi {
        bind_host: app_bind_host,
        dispatch_host_events: app_dispatch_host_events,
    };

    unsafe extern "C" fn mini_get_interface(id: u32) -> *const c_void {
        if id == NVE_IFACE_HOST {
            (&MINI_HOST_API as *const NveHostApi).cast()
        } else {
            std::ptr::null()
        }
    }

    let host_api = unsafe { *(mini_get_interface(NVE_IFACE_HOST) as *const NveHostApi) };
    let mut app = Box::new(MiniHostApp {
        ffi: FfiHostServices::new(),
        dispatched_bytes: 0,
    });
    let app_handle = (&mut *app as *mut MiniHostApp) as NveAppHandle;

    let big = vec![0xCDu8; FFI_STACK_BUF_SIZE + 512];
    let mem = MemoryHostServices::new().with_handler(EchoHandler {
        big_payload: big.clone(),
    });
    let bridge = HostCallbackBridge::new(mem);
    let cbs = bridge.callbacks();
    unsafe { (host_api.bind_host)(app_handle, &cbs) };

    assert_eq!(
        app.ffi
            .invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_ECHO, b"across-c-abi")
            .unwrap(),
        b"across-c-abi"
    );
    assert_eq!(
        app.ffi
            .invoke_sync(TEST_SERVICE_ECHO, TEST_METHOD_BIG, &[])
            .unwrap(),
        big
    );

    let payload = [1u8, 2, 3, 4];
    let dirty = unsafe {
        (host_api.dispatch_host_events)(app_handle, NveBufferSlice::from_slice(&payload))
    };
    assert_eq!(dirty, 1);
    assert_eq!(app.dispatched_bytes, 4);
}

#[test]
fn binary_codec_primitives_and_permission_status() {
    let mut w = BinWriter::new();
    w.write_u8(7);
    w.write_bool(true);
    w.write_u16(512);
    w.write_u32(99_999);
    w.write_i32(-42);
    w.write_u64(123_456_789);
    w.write_i64(-987_654_321);
    w.write_f32(1.5);
    w.write_f64(-2.25);
    w.write_opt_str(Some("nve")).unwrap();
    w.write_opt_bytes(None).unwrap();
    w.write_str_vec(&["a".into(), "b".into()]).unwrap();
    let bytes = w.into_bytes();

    let mut r = BinReader::new(&bytes);
    assert_eq!(r.read_u8().unwrap(), 7);
    assert!(r.read_bool().unwrap());
    assert_eq!(r.read_u16().unwrap(), 512);
    assert_eq!(r.read_u32().unwrap(), 99_999);
    assert_eq!(r.read_i32().unwrap(), -42);
    assert_eq!(r.read_u64().unwrap(), 123_456_789);
    assert_eq!(r.read_i64().unwrap(), -987_654_321);
    assert_eq!(r.read_f32().unwrap(), 1.5);
    assert_eq!(r.read_f64().unwrap(), -2.25);
    assert_eq!(r.read_opt_string().unwrap().as_deref(), Some("nve"));
    assert_eq!(r.read_opt_bytes().unwrap(), None);
    assert_eq!(
        r.read_string_vec().unwrap(),
        vec!["a".to_string(), "b".to_string()]
    );
    r.finish().unwrap();

    assert_eq!(fnv1a64(b"hello"), fnv1a64(b"hello"));
    assert_ne!(fnv1a64(b"hello"), fnv1a64(b"world"));
    assert!(validate_url("https://nve.dev").is_ok());
    assert!(validate_url("bad url").is_err());
    assert!(validate_url("https://bad\nurl").is_err());

    for status in [
        PermissionStatus::NotDetermined,
        PermissionStatus::Denied,
        PermissionStatus::Authorized,
        PermissionStatus::Restricted,
    ] {
        assert_eq!(PermissionStatus::decode(&status.encode()).unwrap(), status);
    }
    assert!(PermissionStatus::decode(&[99]).is_err());
}

trait DirectGreetBackend: Send + Sync {
    fn greet(&self, name: &str) -> String;
}

struct CountingGreetHandler {
    wire_calls: AtomicUsize,
}

impl DirectGreetBackend for CountingGreetHandler {
    fn greet(&self, name: &str) -> String {
        format!("hello {name}")
    }
}

impl HostServiceHandler for CountingGreetHandler {
    fn service_id(&self) -> u32 {
        0x1099
    }

    fn handle(&self, _method_id: u32, request: &[u8]) -> Result<Vec<u8>, ServiceError> {
        self.wire_calls.fetch_add(1, Ordering::SeqCst);
        let name = std::str::from_utf8(request).map_err(|e| ServiceError::Codec(e.to_string()))?;
        Ok(self.greet(name).into_bytes())
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn DirectGreetBackend>(self))
    }
}

#[test]
fn typed_in_process_dispatch_bypasses_wire_serialization_completely() {
    let handler = Arc::new(CountingGreetHandler {
        wire_calls: AtomicUsize::new(0),
    });
    let mem = MemoryHostServices::new().with_handler_arc(handler.clone());

    let direct = resolve_typed::<dyn DirectGreetBackend>(&mem, 0x1099, 1)
        .expect("in-process typed backend must resolve");
    assert_eq!(direct.greet("NVE"), "hello NVE");
    // Proves zero wire serialization / handle() calls occurred on in-process dispatch!
    assert_eq!(handler.wire_calls.load(Ordering::SeqCst), 0);

    // Fault injection forces fallback to wire path so tests still intercept calls cleanly.
    mem.inject_status(0x1099, 1, NveHostStatus::PermissionDenied);
    assert!(resolve_typed::<dyn DirectGreetBackend>(&mem, 0x1099, 1).is_none());
    assert_eq!(
        mem.invoke_sync(0x1099, 1, b"NVE"),
        Err(ServiceError::PermissionDenied)
    );
    // Once one-shot fault is consumed, zero-serialization direct dispatch resumes immediately!
    assert!(resolve_typed::<dyn DirectGreetBackend>(&mem, 0x1099, 1).is_some());
    assert_eq!(handler.wire_calls.load(Ordering::SeqCst), 0);
}
