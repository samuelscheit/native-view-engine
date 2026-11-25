//! Deep integration, C ABI `NVE_IFACE_HOST = 5`, `NveChannelBus` retained replay,
//! fault-injection, safe native OS, and adversarial binary fuzz tests for `nve-sdk`.

use std::ffi::c_void;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveAppHandle, NveBufferSlice, NveHostApi, NveHostCallbacks, NveHostEventBatch, NveHostStatus,
    NVE_CHANNEL_FLAG_RETAIN, NVE_IFACE_HOST, SERVICE_CLIPBOARD, SERVICE_FILESYSTEM,
    SERVICE_LINKING, SERVICE_NOTIFICATIONS, SERVICE_SECURE_STORE, SERVICE_SYSTEM_ENV,
};
use nve_interop::{NveChannelBus, CHANNEL_ID_LOCALE, CHANNEL_ID_THEME};
use nve_sdk::{
    clipboard, filesystem, linking, notifications, parse_locale_tag, secure_store, system_env,
    ColorScheme, ContrastLevel, FfiHostServices, FileFilter, FilePickerOptions, HostCallbackBridge,
    HostEvent, HostServiceProvider, ImpactStyle, InvokeResult, LinkingClient, LinkingEvent,
    LocaleSnapshot, MemoryHostServices, NativeHostServices, NotificationHaptic,
    NotificationPermission, NotificationRequest, NotificationTapped, NotificationsClient,
    RecordedHaptic, SaveFileOptions, SdkClient, SdkError, StandardPathKind, SystemEnvBridge,
    TextDirection, ThemeSnapshot, FFI_MAX_RESPONSE_BYTES, FFI_STACK_BUF_SIZE, THEME_WIRE_LEN,
};

#[test]
fn all_seven_services_roundtrip_and_fault_injection_via_memory_host() {
    let mem = MemoryHostServices::new();
    let sdk = SdkClient::new(&mem);

    // 1. Clipboard
    assert!(!sdk.clipboard().has_text().unwrap());
    assert_eq!(sdk.clipboard().read_text().unwrap(), None);
    sdk.clipboard().write_text("Hello NVE 🚀").unwrap();
    assert!(sdk.clipboard().has_text().unwrap());
    assert_eq!(
        sdk.clipboard().read_text().unwrap().as_deref(),
        Some("Hello NVE 🚀")
    );
    sdk.clipboard().clear().unwrap();
    assert!(!sdk.clipboard().has_text().unwrap());
    assert_eq!(sdk.clipboard().read_text().unwrap(), None);

    // 2. Filesystem (standard paths + pick_files + save_file + Cancelled / PermissionDenied)
    for (kind, expected) in [
        (StandardPathKind::Data, "/mem/data"),
        (StandardPathKind::Cache, "/mem/cache"),
        (StandardPathKind::Config, "/mem/config"),
        (StandardPathKind::Temp, "/mem/tmp"),
        (StandardPathKind::Documents, "/mem/documents"),
        (StandardPathKind::Downloads, "/mem/downloads"),
    ] {
        assert_eq!(sdk.filesystem().standard_path(kind).unwrap(), expected);
    }
    let pick_opts = FilePickerOptions {
        title: Some("Select Assets".into()),
        initial_dir: Some("/mem/documents".into()),
        filters: vec![FileFilter::new("Images", ["png", "webp"])],
        allow_multiple: true,
        pick_directories: false,
    };
    assert_eq!(
        sdk.filesystem().pick_files(&pick_opts).unwrap(),
        vec!["/mem/documents/sample.txt".to_string()]
    );
    mem.set_pick_files_result(Err(SdkError::Cancelled));
    assert_eq!(
        sdk.filesystem().pick_files(&pick_opts),
        Err(SdkError::Cancelled)
    );
    let save_opts = SaveFileOptions {
        title: Some("Export Scene".into()),
        default_name: Some("scene.nve".into()),
        initial_dir: None,
        filters: vec![FileFilter::new("NVE Binary", ["nve"])],
    };
    assert_eq!(
        sdk.filesystem().save_file(&save_opts).unwrap().as_deref(),
        Some("/mem/documents/output.bin")
    );

    // 3. Secure Store (binary + UTF-8 + validation + delete)
    assert_eq!(sdk.secure_store().get("app.nve", "token").unwrap(), None);
    sdk.secure_store()
        .set("app.nve", "raw", &[0x00, 0xFF, 0x10, 0x20])
        .unwrap();
    assert_eq!(
        sdk.secure_store().get("app.nve", "raw").unwrap(),
        Some(vec![0x00, 0xFF, 0x10, 0x20])
    );
    sdk.secure_store()
        .set_string("app.nve", "token", "s3cr3t-jwt")
        .unwrap();
    assert_eq!(
        sdk.secure_store().get_string("app.nve", "token").unwrap().as_deref(),
        Some("s3cr3t-jwt")
    );
    assert!(sdk.secure_store().delete("app.nve", "token").unwrap());
    assert!(!sdk.secure_store().delete("app.nve", "token").unwrap());
    assert!(matches!(
        sdk.secure_store().get("", "token"),
        Err(SdkError::InvalidArg(_))
    ));
    mem.inject_status(
        SERVICE_SECURE_STORE,
        secure_store::METHOD_GET,
        NveHostStatus::PermissionDenied,
    );
    assert_eq!(
        sdk.secure_store().get("app.nve", "raw"),
        Err(SdkError::PermissionDenied)
    );

    // 4. Linking (open_url, can_open_url, initial_url, push UrlReceived)
    mem.set_initial_url(Some("nve://workspace/42".into()));
    assert_eq!(
        sdk.linking().initial_url().unwrap().as_deref(),
        Some("nve://workspace/42")
    );
    assert!(sdk.linking().can_open_url("https://nve.dev").unwrap());
    assert!(!sdk.linking().can_open_url("customscheme://x").unwrap());
    sdk.linking().open_url("https://nve.dev/docs").unwrap();
    assert_eq!(mem.opened_urls(), vec!["https://nve.dev/docs".to_string()]);
    assert!(matches!(
        sdk.linking().open_url("not a valid url"),
        Err(SdkError::InvalidArg(_))
    ));
    let push_bytes = linking::encode_url_received_event("nve://doc/99?tab=inspect").unwrap();
    mem.push_host_event(SERVICE_LINKING, linking::EVENT_URL_RECEIVED, push_bytes);
    let polled = mem.poll_host_events();
    assert_eq!(polled.len(), 1);
    let HostEvent::Push(ref push) = polled[0] else {
        panic!("expected push event");
    };
    assert_eq!(
        LinkingClient::<MemoryHostServices>::decode_push_event(push).unwrap(),
        Some(LinkingEvent::UrlReceived {
            url: "nve://doc/99?tab=inspect".into()
        })
    );

    // 5. System Env (`NVT2` 16B fixed + `NVL1`)
    let custom_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Dark,
        contrast: ContrastLevel::High,
        reduced_motion: true,
        reduced_transparency: true,
        accent_rgba: 0x3B82_F6FF,
        font_scale_x1000: 1250,
        scale_factor_x100: 200,
    };
    assert_eq!(custom_theme.encode_nvt2().len(), THEME_WIRE_LEN);
    let custom_locale = LocaleSnapshot {
        tag: "ar-EG".into(),
        language: "ar".into(),
        region: "EG".into(),
        text_direction: TextDirection::Rtl,
        uses_24h_clock: true,
        timezone: "Africa/Cairo".into(),
    };
    mem.set_theme(custom_theme);
    mem.set_locale(custom_locale.clone());
    assert_eq!(sdk.system_env().get_theme().unwrap(), custom_theme);
    assert_eq!(sdk.system_env().get_locale().unwrap(), custom_locale);

    // 6. Notifications (permission, show, cancel, permission-denied, push NotificationTapped)
    assert_eq!(
        sdk.notifications().request_permission().unwrap(),
        NotificationPermission::Authorized
    );
    let mut notif = NotificationRequest::new("build-1", "Build Finished", "Release binary ready");
    notif.subtitle = Some("nve-sdk".into());
    notif.badge = Some(3);
    notif.action_url = Some("nve://builds/1".into());
    sdk.notifications().show(&notif).unwrap();
    assert_eq!(mem.active_notifications(), vec![notif]);
    sdk.notifications().cancel("build-1").unwrap();
    assert!(mem.active_notifications().is_empty());
    assert_eq!(mem.cancelled_notifications(), vec!["build-1".to_string()]);

    mem.set_notification_permission(NotificationPermission::Denied);
    assert_eq!(
        sdk.notifications()
            .show(&NotificationRequest::new("n2", "Blocked", "Denied")),
        Err(SdkError::PermissionDenied)
    );

    let tapped = NotificationTapped {
        id: "build-1".into(),
        action_url: Some("nve://builds/1".into()),
    };
    let tapped_bytes = notifications::encode_tapped_event(&tapped).unwrap();
    mem.push_host_event(
        SERVICE_NOTIFICATIONS,
        notifications::EVENT_NOTIFICATION_TAPPED,
        tapped_bytes,
    );
    let events = mem.poll_host_events();
    let HostEvent::Push(ref push) = events[0] else {
        panic!("expected push");
    };
    assert_eq!(
        NotificationsClient::<MemoryHostServices>::decode_push_event(push).unwrap(),
        Some(tapped)
    );

    // 7. Haptics (impact, notification, selection)
    sdk.haptics().impact(ImpactStyle::Heavy).unwrap();
    sdk.haptics()
        .notification(NotificationHaptic::Success)
        .unwrap();
    sdk.haptics().selection().unwrap();
    assert_eq!(
        mem.haptics_log(),
        vec![
            RecordedHaptic::Impact(ImpactStyle::Heavy),
            RecordedHaptic::Notification(NotificationHaptic::Success),
            RecordedHaptic::Selection,
        ]
    );
}

#[test]
fn system_env_bridge_publishes_retained_binary_snapshots_onto_nve_channel_bus() {
    let mem = MemoryHostServices::new();
    let initial_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Dark,
        contrast: ContrastLevel::Standard,
        reduced_motion: false,
        reduced_transparency: false,
        accent_rgba: 0x10B9_81FF,
        font_scale_x1000: 1100,
        scale_factor_x100: 200,
    };
    let initial_locale = LocaleSnapshot {
        tag: "de-DE".into(),
        language: "de".into(),
        region: "DE".into(),
        text_direction: TextDirection::Ltr,
        uses_24h_clock: true,
        timezone: "Europe/Berlin".into(),
    };
    mem.set_theme(initial_theme);
    mem.set_locale(initial_locale.clone());

    let mut bus = NveChannelBus::new();
    let bridge = SystemEnvBridge::new();
    let (t, l) = bridge.sync_from_provider(&mem, &mut bus).unwrap();
    assert_eq!(t, initial_theme);
    assert_eq!(l, initial_locale);

    // Late subscriber with `replay_retained = true` immediately receives the binary NVT2 & NVL1 snapshots!
    let seen_themes = Arc::new(Mutex::new(Vec::new()));
    let seen_locales = Arc::new(Mutex::new(Vec::new()));
    let st_clone = seen_themes.clone();
    let sl_clone = seen_locales.clone();

    SystemEnvBridge::subscribe_theme(&mut bus, Some(1), true, move |snap| {
        st_clone.lock().unwrap().push(snap);
    })
    .unwrap();
    SystemEnvBridge::subscribe_locale(&mut bus, Some(1), true, move |snap| {
        sl_clone.lock().unwrap().push(snap);
    })
    .unwrap();

    assert_eq!(*seen_themes.lock().unwrap(), vec![initial_theme]);
    assert_eq!(*seen_locales.lock().unwrap(), vec![initial_locale.clone()]);

    // Live host theme change arrives via NVH1 -> SystemEnvBridge updates retained bus state & fans out.
    let updated_theme = ThemeSnapshot {
        color_scheme: ColorScheme::Light,
        contrast: ContrastLevel::High,
        reduced_motion: true,
        ..initial_theme
    };
    let mut nvh1 = NveHostEventBatch::new();
    nvh1.push_host_event(
        SERVICE_SYSTEM_ENV,
        system_env::EVENT_THEME_CHANGED,
        updated_theme.encode_nvt2().to_vec(),
    );
    nvh1.push_channel_message(
        CHANNEL_ID_LOCALE,
        NVE_CHANNEL_FLAG_RETAIN,
        LocaleSnapshot {
            tag: "ja-JP".into(),
            language: "ja".into(),
            region: "JP".into(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: true,
            timezone: "Asia/Tokyo".into(),
        }
        .encode_nvl1()
        .unwrap(),
    );
    assert_eq!(mem.dispatch_host_events(&nvh1.encode()).unwrap(), 2);

    for ev in mem.poll_host_events() {
        assert!(SystemEnvBridge::handle_host_event(&mut bus, &ev).unwrap());
    }
    assert_eq!(
        *seen_themes.lock().unwrap(),
        vec![initial_theme, updated_theme]
    );
    assert_eq!(seen_locales.lock().unwrap().len(), 2);
    assert_eq!(seen_locales.lock().unwrap()[1].tag, "ja-JP");

    // Verify retained state on raw `NveChannelBus` has exact 16-byte NVT2 bytes (zero JSON!).
    let retained_theme = bus.retained("theme").expect("retained theme message");
    assert_eq!(retained_theme.channel_id, CHANNEL_ID_THEME);
    assert_eq!(retained_theme.as_bytes().len(), THEME_WIRE_LEN);
    assert_eq!(&retained_theme.as_bytes()[..4], b"NVT2");
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
        // Fast-path (< 4 KiB): single call succeeds in stack buffer.
        (SERVICE_CLIPBOARD, clipboard::METHOD_HAS_TEXT) => {
            unsafe { *out_len = 1 };
            if out_cap < 1 {
                return NveHostStatus::BufferTooSmall.as_i32();
            }
            unsafe { *out_buf = 1 };
            NveHostStatus::Ok.as_i32()
        }
        // Oversized (> 4 KiB): first call returns BUFFER_TOO_SMALL with exact required size,
        // second call fills heap buffer and returns OK.
        (SERVICE_CLIPBOARD, clipboard::METHOD_READ_TEXT) => {
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
        // Async PENDING path: returns PENDING and records request_id for NVH1 resolution.
        (SERVICE_FILESYSTEM, filesystem::METHOD_PICK_FILES) => {
            host.last_pending_req_id
                .store(request_id as usize, Ordering::SeqCst);
            unsafe { *out_len = 0 };
            NveHostStatus::Pending.as_i32()
        }
        // Hostile oversized claim (> FFI_MAX_RESPONSE_BYTES) must be rejected without OOM!
        (SERVICE_SYSTEM_ENV, 99) => {
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
fn ffi_host_services_fast_path_buffer_too_small_retry_and_async_nvh1_resolution() {
    let big_text = "A".repeat(FFI_STACK_BUF_SIZE + 8192);
    let large_payload = clipboard::encode_read_text_response(Some(&big_text)).unwrap();
    assert!(large_payload.len() > FFI_STACK_BUF_SIZE);

    let host = Box::new(MockCAbiHost {
        invoke_calls: AtomicUsize::new(0),
        last_pending_req_id: AtomicUsize::new(0),
        large_payload,
        published_channels: Mutex::new(Vec::new()),
    });

    let callbacks = NveHostCallbacks {
        host_ctx: (&*host as *const MockCAbiHost) as *mut c_void,
        invoke: Some(mock_c_invoke),
        channel_publish: Some(mock_c_channel_publish),
    };

    let ffi = FfiHostServices::with_callbacks(callbacks);
    let bus = Arc::new(Mutex::new(NveChannelBus::new()));
    ffi.attach_channel_bus(bus.clone());
    let sdk = SdkClient::new(&ffi);

    // 1. 4 KiB stack-buffer fast-path -> exactly 1 invoke call.
    assert!(sdk.clipboard().has_text().unwrap());
    assert_eq!(host.invoke_calls.load(Ordering::SeqCst), 1);

    // 2. > 4 KiB payload -> BUFFER_TOO_SMALL auto-grow retry -> exactly 2 additional invoke calls.
    let read_back = sdk.clipboard().read_text().unwrap();
    assert_eq!(read_back.as_deref(), Some(big_text.as_str()));
    assert_eq!(host.invoke_calls.load(Ordering::SeqCst), 3);

    // 3. Hostile >64 MiB BufferTooSmall claim rejected defensively without allocating.
    assert!(matches!(
        ffi.invoke_raw(SERVICE_SYSTEM_ENV, 99, &[]),
        Err(SdkError::BufferTooSmall { .. })
    ));

    // 4. Async PENDING + polling resolution via NVH1 batch.
    let req_bytes = filesystem::encode_file_picker_options(&FilePickerOptions::default()).unwrap();
    let InvokeResult::Pending { request_id: req1 } = ffi
        .invoke_raw(SERVICE_FILESYSTEM, filesystem::METHOD_PICK_FILES, &req_bytes)
        .unwrap()
    else {
        panic!("expected Pending");
    };
    assert_eq!(ffi.pending_request_count(), 1);
    assert_eq!(ffi.take_async_response(req1), None);

    // 5. Async PENDING with one-shot callback + ChannelMessage auto-forward in the SAME NVH1 batch!
    let cb_outcome = Arc::new(Mutex::new(None));
    let cb_slot = cb_outcome.clone();
    let req2 = ffi
        .invoke_with_callback(
            SERVICE_FILESYSTEM,
            filesystem::METHOD_PICK_FILES,
            &req_bytes,
            move |res| {
                *cb_slot.lock().unwrap() = Some(res);
            },
        )
        .unwrap()
        .expect("second call should be pending");
    assert_eq!(ffi.pending_request_count(), 2);

    let mut batch = NveHostEventBatch::new();
    batch.push_async_response(
        req1,
        NveHostStatus::Ok,
        filesystem::encode_pick_files_response(&["/tmp/a.rs".into()]).unwrap(),
    );
    batch.push_async_response(req2, NveHostStatus::Cancelled, Vec::new());
    batch.push_channel_message(
        CHANNEL_ID_THEME,
        NVE_CHANNEL_FLAG_RETAIN,
        ThemeSnapshot::default().encode_nvt2().to_vec(),
    );
    assert_eq!(ffi.dispatch_host_events(&batch.encode()).unwrap(), 3);
    assert_eq!(ffi.pending_request_count(), 0);

    let resolved1 = ffi.take_async_response(req1).expect("req1 resolved").unwrap();
    assert_eq!(
        filesystem::decode_pick_files_response(&resolved1).unwrap(),
        vec!["/tmp/a.rs".to_string()]
    );
    assert_eq!(*cb_outcome.lock().unwrap(), Some(Err(SdkError::Cancelled)));

    // Verify ChannelMessage auto-forwarded to attached NveChannelBus as retained binary state!
    let bus_guard = bus.lock().unwrap();
    let retained = bus_guard.retained("theme").expect("theme retained on bus");
    assert_eq!(retained.as_bytes(), &ThemeSnapshot::default().encode_nvt2());

    // 6. Outbound channel_publish crosses C ABI cleanly.
    ffi.channel_publish(CHANNEL_ID_THEME, true, b"NVT2test")
        .unwrap();
    let published = host.published_channels.lock().unwrap();
    assert_eq!(published.len(), 1);
    assert_eq!(published[0], (CHANNEL_ID_THEME, true, b"NVT2test".to_vec()));
}

#[test]
fn c_abi_nve_iface_host_5_end_to_end_roundtrip_via_host_callback_bridge() {
    // Simulates an NVE app exposing `NveHostApi` (`NVE_IFACE_HOST = 5`) and binding a live
    // `HostCallbackBridge<MemoryHostServices>` across the real `extern "C"` ABI boundary.
    struct MiniHostApp {
        ffi: FfiHostServices,
        dispatched: usize,
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
        match app.ffi.dispatch_host_events(bytes) {
            Ok(n) => {
                app.dispatched += n;
                u32::from(n > 0)
            }
            Err(_) => 0,
        }
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
        dispatched: 0,
    });
    let app_handle = (&mut *app as *mut MiniHostApp) as NveAppHandle;

    let bridge = HostCallbackBridge::new(MemoryHostServices::new());
    let cbs = bridge.callbacks();
    unsafe { (host_api.bind_host)(app_handle, &cbs) };

    let sdk = SdkClient::new(&app.ffi);
    sdk.clipboard().write_text("across-c-abi").unwrap();
    assert_eq!(
        sdk.clipboard().read_text().unwrap().as_deref(),
        Some("across-c-abi")
    );
    sdk.secure_store()
        .set_string("svc.cabi", "user", "tok-123")
        .unwrap();
    assert_eq!(
        sdk.secure_store().get_string("svc.cabi", "user").unwrap().as_deref(),
        Some("tok-123")
    );

    let mut batch = NveHostEventBatch::new();
    batch.push_host_event(
        SERVICE_LINKING,
        linking::EVENT_URL_RECEIVED,
        linking::encode_url_received_event("nve://cabi/ok").unwrap(),
    );
    let encoded = batch.encode();
    let dirty = unsafe {
        (host_api.dispatch_host_events)(app_handle, NveBufferSlice::from_slice(&encoded))
    };
    assert_eq!(dirty, 1);
    assert_eq!(app.dispatched, 1);
    let events = app.ffi.poll_host_events();
    assert_eq!(events.len(), 1);
}

#[test]
fn native_host_services_safe_non_modal_capabilities_and_locale_parser() {
    // Pure locale tag parser coverage (POSIX, BCP-47, RTL, C/POSIX fallback).
    let en = parse_locale_tag("en_US.UTF-8", Some("America/New_York".into()), false);
    assert_eq!(en.tag, "en-US");
    assert_eq!(en.language, "en");
    assert_eq!(en.region, "US");
    assert_eq!(en.text_direction, TextDirection::Ltr);

    let ar = parse_locale_tag("ar_EG.UTF-8", None, true);
    assert_eq!(ar.tag, "ar-EG");
    assert_eq!(ar.text_direction, TextDirection::Rtl);
    assert!(ar.uses_24h_clock);

    let posix = parse_locale_tag("POSIX", None, false);
    assert_eq!(posix.tag, "en-US");

    // Safe non-modal native queries (NEVER triggers Keychain modal or GUI panels!).
    let native = NativeHostServices::new();
    let sdk = SdkClient::new(&native);

    for kind in [
        StandardPathKind::Data,
        StandardPathKind::Cache,
        StandardPathKind::Config,
        StandardPathKind::Temp,
        StandardPathKind::Documents,
        StandardPathKind::Downloads,
    ] {
        let p = sdk.filesystem().standard_path(kind).unwrap();
        assert!(!p.is_empty(), "standard_path({kind:?}) must be non-empty");
    }

    let theme = sdk.system_env().get_theme().unwrap();
    assert_eq!(
        ThemeSnapshot::decode_nvt2(&theme.encode_nvt2()).unwrap(),
        theme
    );

    let locale = sdk.system_env().get_locale().unwrap();
    assert!(!locale.tag.is_empty());
    assert_eq!(
        LocaleSnapshot::decode_nvl1(&locale.encode_nvl1().unwrap()).unwrap(),
        locale
    );

    // Unbundled `cargo test` binary must NEVER crash via ObjC `UNUserNotificationCenter` exception!
    assert_eq!(
        sdk.notifications().request_permission(),
        Err(SdkError::Unsupported)
    );
    assert_eq!(
        sdk.notifications()
            .show(&NotificationRequest::new("t", "Title", "Body")),
        Err(SdkError::Unsupported)
    );
}
