//! Deep unit & integration tests for `SkiaHostServices`, direct typed `WindowServiceBackend`,
//! modal close-blocking, cascading child cleanup, live typed OS theme / locale updates over
//! `TypedEventBus`, and app-driven `CloseRequested` veto/allow flows.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    ColorRgba, FrameId, NveHostStatus, SurfaceId, METHOD_WINDOW_REQUEST_FRAME, SERVICE_SYSTEM_ENV,
    SERVICE_WINDOW,
};
use nve_components::tables::register_core_tables;
use nve_store::wire::WireEncoder;
use nve_store::{ChangeSet, NodeId, NodeStore};
use nve_clipboard::{ClipboardClient, MemoryClipboard};
use nve_engine::Viewport;
use nve_filesystem::{FilesystemClient, MemoryFilesystem, StandardPathKind};
use nve_haptics::{HapticsClient, ImpactStyle, MemoryHaptics, NotificationHaptic};
use nve_host_services::{
    resolve_typed, FfiHostServices, HostServiceProvider, MemoryHostServices, ServiceError,
};
use nve_interop::{
    NveChannelBus, SurfacePresentation, WindowCommand, WindowDescriptor, WindowError,
    WindowLifecycleEvent, WindowLifecycleState, WindowRect,
};
use nve_linking::{LinkingClient, MemoryLinking};
use nve_secure_store::{MemorySecureStore, SecureStoreClient};
use nve_system_env::{
    ColorScheme, ContrastLevel, LocaleSnapshot, MemorySystemEnv, SystemEnvBridge, SystemEnvClient,
    TextDirection, ThemeSnapshot, THEME_SNAPSHOT_REPR_C_LEN,
};
use winit::window::Theme as WinitTheme;

use super::multi_window::HeadlessMultiWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopSession};
use crate::host_services::{
    winit_theme_to_color_scheme, SkiaHostEvent, SkiaHostServices, WindowServiceBackend,
};

pub(super) struct TestMemoryHandles {
    pub(super) linking: Arc<MemoryLinking>,
    pub(super) haptics: Arc<MemoryHaptics>,
    pub(super) channel_bus: Arc<Mutex<NveChannelBus>>,
}

pub(super) fn make_memory_services() -> (TestMemoryHandles, Arc<SkiaHostServices>) {
    let mem = Arc::new(MemoryHostServices::new());
    let fs = Arc::new(MemoryFilesystem::new());
    fs.set_standard_path(StandardPathKind::Cache, "/tmp/nve-test-cache");
    let linking = Arc::new(MemoryLinking::new());
    let haptics = Arc::new(MemoryHaptics::new());
    let sys_env = Arc::new(MemorySystemEnv::with_initial(
        ThemeSnapshot {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::Standard,
            reduced_motion: false,
            reduced_transparency: false,
            accent_rgba: 0x3B82_F6FF,
            font_scale_x1000: 1000,
            scale_factor_x100: 100,
        },
        LocaleSnapshot {
            tag: "en-US".into(),
            language: "en".into(),
            region: "US".into(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: false,
            timezone: "UTC".into(),
        },
    ));
    mem.register(MemoryClipboard::new());
    mem.register_arc(fs);
    mem.register(MemorySecureStore::new());
    mem.register_arc(linking.clone());
    mem.register_arc(sys_env);
    mem.register_arc(haptics.clone());

    let channel_bus = mem.channel_bus();
    let svc = SkiaHostServices::with_provider(mem);
    (
        TestMemoryHandles {
            linking,
            haptics,
            channel_bus,
        },
        svc,
    )
}

#[test]
fn skia_host_services_dispatches_all_sdk_capabilities_buffer_retry_and_channel_bus() {
    let (handles, services) = make_memory_services();
    let ffi = FfiHostServices::with_callbacks(services.callbacks());
    let clipboard = ClipboardClient::new(&ffi);
    let filesystem = FilesystemClient::new(&ffi);
    let secure_store = SecureStoreClient::new(&ffi);
    let linking = LinkingClient::new(&ffi);
    let system_env = SystemEnvClient::new(&ffi);
    let haptics = HapticsClient::new(&ffi);

    // 1. Clipboard round-trip (including > 4 KiB payload forcing BufferTooSmall retry).
    assert!(!clipboard.has_text().unwrap());
    let large_text = "NVE-BINARY-CLIPBOARD-".repeat(300);
    clipboard.write_text(&large_text).unwrap();
    assert!(clipboard.has_text().unwrap());
    assert_eq!(
        clipboard.read_text().unwrap().as_deref(),
        Some(large_text.as_str())
    );
    clipboard.clear().unwrap();
    assert!(!clipboard.has_text().unwrap());

    // 2. Filesystem standard path
    assert_eq!(
        filesystem.standard_path(StandardPathKind::Cache).unwrap(),
        "/tmp/nve-test-cache"
    );

    // 3. Secure store set/get/delete
    secure_store
        .set("nve.studio", "token", b"top-secret-bytes")
        .unwrap();
    assert_eq!(
        secure_store
            .get("nve.studio", "token")
            .unwrap()
            .as_deref(),
        Some(b"top-secret-bytes".as_slice())
    );
    assert!(secure_store.delete("nve.studio", "token").unwrap());
    assert_eq!(secure_store.get("nve.studio", "token").unwrap(), None);

    // 4. Linking
    assert!(linking.can_open_url("https://nve.dev").unwrap());
    linking.open_url("https://nve.dev/docs").unwrap();
    assert_eq!(
        handles.linking.opened_urls(),
        vec!["https://nve.dev/docs".to_owned()]
    );

    // 5. SystemEnv (direct #[repr(C)] ThemeSnapshot + LocaleSnapshot across C-ABI & in-process)
    let theme = system_env.get_theme().unwrap();
    assert_eq!(theme.color_scheme, Some(ColorScheme::Light));
    assert_eq!(theme.accent_rgba, 0x3B82_F6FF);
    let locale = system_env.get_locale().unwrap();
    assert_eq!(locale.tag, "en-US");
    assert_eq!(locale.text_direction, TextDirection::Ltr);

    // 6. Direct typed Haptics
    haptics.impact(ImpactStyle::Medium).unwrap();
    haptics.notification(NotificationHaptic::Success).unwrap();
    haptics.selection().unwrap();
    assert_eq!(handles.haptics.log().len(), 3);

    // 7. NveChannelBus publish from app through C ABI callback
    let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let rx_clone = Arc::clone(&received);
    let custom_channel_id = {
        let mut bus = handles.channel_bus.lock().unwrap();
        let id = bus.ensure_channel("skia.custom").unwrap();
        bus.subscribe_by_id(id, None, false, move |msg| {
            rx_clone.lock().unwrap().push(msg.as_bytes().to_vec());
        })
        .unwrap();
        id
    };
    ffi.channel_publish(custom_channel_id, true, b"binary-channel-frame")
        .unwrap();
    assert_eq!(
        received.lock().unwrap().as_slice(),
        &[b"binary-channel-frame".to_vec()]
    );

    // 8. Async frame wakeup (`METHOD_WINDOW_REQUEST_FRAME = 1`) + unsupported raw C-ABI service error paths
    assert!(!services.take_pending_app_wake());
    let wake_resp = ffi
        .invoke_sync(SERVICE_WINDOW, METHOD_WINDOW_REQUEST_FRAME, &[])
        .expect("METHOD_WINDOW_REQUEST_FRAME must succeed");
    assert!(wake_resp.is_empty());
    assert!(services.take_pending_app_wake());
    assert!(!services.take_pending_app_wake());
    assert!(matches!(
        ffi.invoke_sync(SERVICE_WINDOW, 99, &[]),
        Err(ServiceError::Unsupported)
    ));
    assert!(matches!(
        ffi.invoke_sync(0x9999, 1, &[]),
        Err(ServiceError::Unsupported)
    ));

    // 9. Raw C-ABI null host_ctx / small buffer safety checks
    let cb = services.callbacks();
    let invoke = cb.invoke.unwrap();
    let publish = cb.channel_publish.unwrap();
    let mut out_len = 0usize;
    // SAFETY: Verifying null `host_ctx` guard with valid `out_len` pointer.
    let status_null_ctx = unsafe {
        invoke(
            std::ptr::null_mut(),
            SERVICE_SYSTEM_ENV,
            1,
            1,
            nve_abi::NveBufferSlice::EMPTY,
            std::ptr::null_mut(),
            0,
            &mut out_len,
        )
    };
    assert_eq!(
        NveHostStatus::from_i32(status_null_ctx),
        NveHostStatus::InvalidArg
    );
    // SAFETY: Verifying null `host_ctx` guard on `channel_publish` with empty slice.
    let status_null_pub =
        unsafe { publish(std::ptr::null_mut(), 1, 0, nve_abi::NveBufferSlice::EMPTY) };
    assert_eq!(
        NveHostStatus::from_i32(status_null_pub),
        NveHostStatus::InvalidArg
    );

    // Small buffer returns BufferTooSmall and writes exact #[repr(C)] ThemeSnapshot size (12B).
    let mut tiny = [0u8; 4];
    // SAFETY: `cb.host_ctx` belongs to live `host`, and `tiny`/`out_len` are valid writable buffers.
    let status_small = unsafe {
        invoke(
            cb.host_ctx,
            SERVICE_SYSTEM_ENV,
            1,
            2,
            nve_abi::NveBufferSlice::EMPTY,
            tiny.as_mut_ptr(),
            tiny.len(),
            &mut out_len,
        )
    };
    assert_eq!(
        NveHostStatus::from_i32(status_small),
        NveHostStatus::BufferTooSmall
    );
    assert_eq!(out_len, THEME_SNAPSHOT_REPR_C_LEN);
}

#[test]
fn service_window_queues_typed_commands_before_sink_and_flushes_fifo_into_sink() {
    let (_mem, services) = make_memory_services();
    let window_backend = resolve_typed::<dyn WindowServiceBackend>(&*services, SERVICE_WINDOW, 1)
        .expect("in-process WindowServiceBackend must resolve");

    let cmd1 = WindowCommand::SetTitle {
        handle: 1,
        title: "First".into(),
    };
    let cmd2 = WindowCommand::FocusWindow { handle: 2 };
    let count = window_backend.submit_commands(&[cmd1.clone(), cmd2.clone()]);
    assert_eq!(count, 2);

    // Attaching a sink (like EventLoopProxy<HostWakeEvent>) immediately flushes buffered commands in FIFO order.
    let forwarded = Arc::new(Mutex::new(Vec::<WindowCommand>::new()));
    let fwd_clone = Arc::clone(&forwarded);
    services.attach_window_sink(Arc::new(move |cmd| {
        fwd_clone.lock().unwrap().push(cmd);
    }));
    assert_eq!(
        forwarded.lock().unwrap().as_slice(),
        &[cmd1.clone(), cmd2.clone()]
    );

    // Subsequent commands go straight to the sink without buffering or serialization.
    let cmd3 = WindowCommand::CloseWindow { handle: 2 };
    services.submit_window_command(cmd3.clone());
    assert_eq!(forwarded.lock().unwrap().as_slice(), &[cmd1, cmd2, cmd3]);
    assert!(services.drain_queued_window_commands().is_empty());
}

/// Host-aware app mock backed directly by `SkiaHostServices` to test typed `WindowCommand`
/// submission, live `TypedEventBus` theme/locale delivery, and `CloseRequested` veto/allow.
struct HostAwareMockApp {
    surface_id: SurfaceId,
    services: Option<Arc<SkiaHostServices>>,
    frame_buf: Vec<u8>,
    themes_seen: Arc<Mutex<Vec<ThemeSnapshot>>>,
    locales_seen: Arc<Mutex<Vec<LocaleSnapshot>>>,
    lifecycle_events: Arc<Mutex<Vec<WindowLifecycleEvent>>>,
    veto_next_close: Arc<AtomicBool>,
    allow_close_via_service: Arc<AtomicBool>,
}

impl HostAwareMockApp {
    fn new(surface_id: SurfaceId) -> Self {
        Self {
            surface_id,
            services: None,
            frame_buf: Vec::new(),
            themes_seen: Arc::new(Mutex::new(Vec::new())),
            locales_seen: Arc::new(Mutex::new(Vec::new())),
            lifecycle_events: Arc::new(Mutex::new(Vec::new())),
            veto_next_close: Arc::new(AtomicBool::new(false)),
            allow_close_via_service: Arc::new(AtomicBool::new(false)),
        }
    }

    fn send_window_commands(&self, commands: &[WindowCommand]) -> usize {
        self.services
            .as_ref()
            .expect("host services bound")
            .submit_window_commands(commands)
    }
}

impl DesktopApp for HostAwareMockApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        register_core_tables();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        {
            let mut tx = store.tx();
            let _ = tx.create_at(NodeId::ROOT);
            let _ = tx.create_at(NodeId::new(2, 0));
            tx.set_children(NodeId::ROOT, &[NodeId::new(2, 0)]);
            tx.commit_into(&mut changes);
        }
        self.frame_buf.clear();
        WireEncoder::default().encode(&store, &changes, &mut self.frame_buf);
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _events: nve_abi::NveEventBatchRef<'_>) -> bool {
        false
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(&mut self, _path: &Path, _sid: SurfaceId) -> Result<u64, DesktopHostError> {
        Ok(1)
    }

    fn has_host_interface(&self) -> bool {
        true
    }

    fn bind_host_services(&mut self, services: Option<Arc<SkiaHostServices>>) -> bool {
        self.services = services.clone();
        if let Some(svc) = services {
            let t_sink = Arc::clone(&self.themes_seen);
            let l_sink = Arc::clone(&self.locales_seen);
            let _ = SystemEnvBridge::subscribe_theme(svc.typed_events(), true, move |t| {
                t_sink.lock().unwrap().push(*t);
            });
            let _ = SystemEnvBridge::subscribe_locale(svc.typed_events(), true, move |l| {
                l_sink.lock().unwrap().push(l.clone());
            });
        }
        true
    }

    fn dispatch_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        let mut intercepted_close = false;
        if let SkiaHostEvent::WindowLifecycle(events) = event {
            for ev in events {
                if let WindowLifecycleEvent::CloseRequested { handle, surface_id } = *ev
                    && surface_id == self.surface_id
                {
                    if self.veto_next_close.swap(false, Ordering::SeqCst) {
                        let _ = self.send_window_commands(&[WindowCommand::RespondCloseRequest {
                            handle,
                            allow: false,
                        }]);
                        intercepted_close = true;
                    } else if self.allow_close_via_service.swap(false, Ordering::SeqCst) {
                        let _ = self.send_window_commands(&[WindowCommand::RespondCloseRequest {
                            handle,
                            allow: true,
                        }]);
                        intercepted_close = true;
                    }
                }
                self.lifecycle_events.lock().unwrap().push(ev.clone());
            }
        }
        intercepted_close
    }
}

#[test]
fn multi_window_service_roundtrip_modal_close_block_cascade_veto_and_theme_locale_pushes() {
    let (_mem, services) = make_memory_services();
    let mut host = HeadlessMultiWindowHost::<u64, HostAwareMockApp>::with_host_services(
        Arc::clone(&services),
    );
    host.set_exit_on_primary_close(false);
    host.set_secondary_app_factory(|surface_id, _desc| Ok(HostAwareMockApp::new(surface_id)));

    let primary_app = HostAwareMockApp::new(SurfaceId::ROOT);
    let primary_themes = Arc::clone(&primary_app.themes_seen);
    let primary_locales = Arc::clone(&primary_app.locales_seen);
    let primary_events = Arc::clone(&primary_app.lifecycle_events);
    let veto_flag = Arc::clone(&primary_app.veto_next_close);
    let allow_flag = Arc::clone(&primary_app.allow_close_via_service);

    let primary_session = DesktopSession::new(
        primary_app,
        SurfaceId::ROOT,
        Viewport {
            width: 640.0,
            height: 480.0,
            scale_factor: 1.0,
        },
        ColorRgba::BLACK,
        false,
    );
    let primary_h = host
        .open_window(
            1,
            SurfaceId::ROOT,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 640.0, 480.0)),
            primary_session,
        )
        .unwrap();

    // 1. Initial bind replayed retained typed theme + locale AND OpenWindow delivered Opened+Focused.
    assert_eq!(primary_themes.lock().unwrap().len(), 1);
    assert_eq!(
        primary_themes.lock().unwrap()[0].color_scheme,
        Some(ColorScheme::Light)
    );
    assert_eq!(primary_locales.lock().unwrap().len(), 1);
    assert_eq!(primary_locales.lock().unwrap()[0].tag, "en-US");
    assert!(primary_events.lock().unwrap().iter().any(
        |e| matches!(e, WindowLifecycleEvent::Opened { handle, surface_id, .. } if *handle == primary_h && *surface_id == SurfaceId::ROOT)
    ));

    // 2. App submits typed WindowCommands directly to open Modal, Sheet, Popover, and update title/bounds.
    let count = host.window(1).unwrap().session.app().send_window_commands(&[
        WindowCommand::OpenWindow {
            surface_id: SurfaceId::new(2, 0),
            presentation: SurfacePresentation::SecondaryWindow {
                parent: Some(SurfaceId::ROOT),
                modal: true,
                descriptor: WindowDescriptor::new("Modal", 320.0, 200.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: SurfaceId::new(3, 0),
            presentation: SurfacePresentation::Sheet {
                parent: SurfaceId::ROOT,
                descriptor: WindowDescriptor::new("Inspector Sheet", 300.0, 240.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: SurfaceId::new(4, 0),
            presentation: SurfacePresentation::Popover {
                anchor_surface: SurfaceId::new(3, 0),
                anchor_rect: WindowRect::new(10.0, 10.0, 80.0, 24.0),
                descriptor: WindowDescriptor::new("Tooltip Popover", 180.0, 120.0),
            },
        },
        WindowCommand::SetTitle {
            handle: primary_h,
            title: "Primary Renamed".into(),
        },
        WindowCommand::SetBounds {
            handle: primary_h,
            bounds: WindowRect::new(24.0, 48.0, 720.0, 540.0),
        },
    ]);
    assert_eq!(count, 5);

    let results = host.drain_window_commands(|s| u64::from(s.raw()));
    assert_eq!(results.len(), 5);
    let modal_h = results[0].as_ref().unwrap().unwrap();
    let sheet_h = results[1].as_ref().unwrap().unwrap();
    let popover_h = results[2].as_ref().unwrap().unwrap();
    assert_eq!(host.window_count(), 4);
    assert_eq!(
        host.window_manager().get(primary_h).unwrap().descriptor.title,
        "Primary Renamed"
    );
    assert!(primary_events.lock().unwrap().iter().any(|e| matches!(
        e,
        WindowLifecycleEvent::Resized { handle, width, height, .. }
            if *handle == primary_h && (*width - 720.0).abs() < 0.1 && (*height - 540.0).abs() < 0.1
    )));

    // 3. Modal parent close blocking works for both handle_close_requested and CloseWindow command.
    assert_eq!(
        host.handle_close_requested(1),
        Err(WindowError::BlockedByModalChild {
            target: primary_h,
            modal_child: modal_h,
        })
    );
    host.window(1)
        .unwrap()
        .session
        .app()
        .send_window_commands(&[WindowCommand::CloseWindow { handle: primary_h }]);
    assert_eq!(
        host.drain_window_commands(|s| u64::from(s.raw())),
        vec![Err(WindowError::BlockedByModalChild {
            target: primary_h,
            modal_child: modal_h,
        })]
    );
    assert_eq!(host.window_count(), 4);

    // 4. Close modal -> Primary receives Closed event directly.
    host.window(1)
        .unwrap()
        .session
        .app()
        .send_window_commands(&[WindowCommand::CloseWindow { handle: modal_h }]);
    assert_eq!(host.drain_window_commands(|s| u64::from(s.raw())), vec![Ok(None)]);
    assert_eq!(host.window_count(), 3);
    assert!(primary_events.lock().unwrap().iter().any(
        |e| matches!(e, WindowLifecycleEvent::Closed { handle, surface_id } if *handle == modal_h && *surface_id == SurfaceId::new(2, 0))
    ));

    // 5. Live theme change (Dark @ 2x) and locale change (ar-EG RTL) flow directly via TypedEventBus.
    let _ = host.simulate_theme_change(WinitTheme::Dark, 2.0);
    assert_eq!(
        primary_themes.lock().unwrap().last().unwrap().color_scheme,
        Some(ColorScheme::Dark)
    );
    assert_eq!(
        primary_themes
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .scale_factor_x100,
        200
    );
    assert_eq!(
        SystemEnvClient::new(&*services)
            .get_theme()
            .unwrap()
            .color_scheme,
        Some(ColorScheme::Dark)
    );

    let rtl_locale = LocaleSnapshot {
        tag: "ar-EG".into(),
        language: "ar".into(),
        region: "EG".into(),
        text_direction: TextDirection::Rtl,
        uses_24h_clock: true,
        timezone: "Africa/Cairo".into(),
    };
    let _ = host.simulate_locale_change(rtl_locale.clone());
    assert_eq!(primary_locales.lock().unwrap().last().unwrap(), &rtl_locale);
    assert_eq!(
        SystemEnvClient::new(&*services).get_locale().unwrap(),
        rtl_locale
    );

    // 6. App-driven CloseRequested VETO via RespondCloseRequest { allow: false }.
    veto_flag.store(true, Ordering::SeqCst);
    let veto_outcome = host.handle_close_requested(1).unwrap();
    assert!(veto_outcome.vetoed);
    assert_eq!(
        host.window_manager().get(primary_h).unwrap().lifecycle,
        WindowLifecycleState::ClosePending
    );
    assert_eq!(host.drain_window_commands(|s| u64::from(s.raw())), vec![Ok(None)]);
    assert_eq!(
        host.window_manager().get(primary_h).unwrap().lifecycle,
        WindowLifecycleState::Open
    );
    assert_eq!(host.window_count(), 3);

    // 7. App-driven CloseRequested ALLOW cascades Popover + Sheet + Primary cleanly.
    allow_flag.store(true, Ordering::SeqCst);
    let allow_pending = host.handle_close_requested(1).unwrap();
    assert!(allow_pending.vetoed);
    assert_eq!(host.drain_window_commands(|s| u64::from(s.raw())), vec![Ok(None)]);
    assert_eq!(host.window_count(), 0);
    assert!(host.window_manager().get(popover_h).is_none());
    assert!(host.window_manager().get(sheet_h).is_none());
    assert!(host.window_manager().get(primary_h).is_none());
}

#[test]
fn native_skia_host_services_delivers_typed_system_env_without_wire_codecs() {
    let services = SkiaHostServices::new();
    let system_env = SystemEnvClient::new(&*services);
    let filesystem = FilesystemClient::new(&*services);

    let theme = system_env.get_theme().expect("native theme");
    let locale = system_env.get_locale().expect("native locale");
    assert!(!locale.tag.is_empty());
    let cache = filesystem
        .standard_path(StandardPathKind::Cache)
        .expect("native cache dir");
    assert!(!cache.is_empty());

    assert_eq!(
        winit_theme_to_color_scheme(WinitTheme::Light),
        ColorScheme::Light
    );
    assert_eq!(
        winit_theme_to_color_scheme(WinitTheme::Dark),
        ColorScheme::Dark
    );

    assert_eq!(
        services.initial_env_event(),
        SkiaHostEvent::SystemEnv {
            theme,
            locale: locale.clone(),
        }
    );
    assert_eq!(
        services
            .typed_events()
            .retained::<ThemeSnapshot>()
            .as_deref(),
        Some(&theme)
    );
    assert_eq!(
        services
            .typed_events()
            .retained::<LocaleSnapshot>()
            .as_deref(),
        Some(&locale)
    );
}
