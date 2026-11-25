//! Deep unit & integration tests for `NVE_IFACE_HOST = 5` (`SkiaHostServices`),
//! `SERVICE_WINDOW = 0x0002` (`NVW1` / `NVH1`), modal close-blocking, cascading child cleanup,
//! live OS theme (`NVT2` on `CHANNEL_ID_THEME`) / locale (`NVL1` on `CHANNEL_ID_LOCALE`) pushes,
//! and Tier-2 `cdylib` hot-reload callback re-binding with zero dangling pointers.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, FrameId, NveHostEventBatch,
    NveHostEventRecordRef, NveHostStatus, SurfaceId, NVE_CHANNEL_FLAG_RETAIN, ROOT_NODE_ID,
    SERVICE_SYSTEM_ENV, SERVICE_WINDOW,
};
use nve_clipboard::{ClipboardClient, MemoryClipboard};
use nve_engine::Viewport;
use nve_filesystem::{FilesystemClient, MemoryFilesystem, StandardPathKind};
use nve_haptics::{HapticsClient, ImpactStyle, MemoryHaptics, NotificationHaptic};
use nve_host_services::{FfiHostServices, HostServiceProvider, MemoryHostServices, ServiceError};
use nve_interop::{
    decode_window_events, encode_window_commands, NveChannelBus, SurfacePresentation,
    WindowCommand, WindowDescriptor, WindowError, WindowLifecycleEvent, WindowLifecycleState,
    WindowRect, CHANNEL_ID_LOCALE, CHANNEL_ID_THEME,
};
use nve_linking::{LinkingClient, MemoryLinking};
use nve_notifications::{
    MemoryNotifications, NotificationPermission, NotificationRequest, NotificationsClient,
};
use nve_secure_store::{MemorySecureStore, SecureStoreClient};
use nve_system_env::{
    ColorScheme, ContrastLevel, LocaleSnapshot, MemorySystemEnv, SystemEnvBridge, SystemEnvClient,
    TextDirection, ThemeSnapshot,
};
use winit::window::Theme as WinitTheme;

use super::multi_window::HeadlessMultiWindowHost;
use super::{DesktopApp, DesktopHostError, DesktopSession, HotAppSlot};
use crate::host_services::{
    encode_locale_channel_batch, encode_system_env_batch, encode_theme_channel_batch,
    encode_window_lifecycle_batch, winit_theme_to_color_scheme, SkiaHostServices,
    EVENT_WINDOW_LIFECYCLE, METHOD_WINDOW_DISPATCH_BATCH,
};

static TEST_DIR_SEQ: AtomicU64 = AtomicU64::new(1);

struct TempDirGuard(PathBuf);

impl TempDirGuard {
    fn new(prefix: &str) -> Self {
        let seq = TEST_DIR_SEQ.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "nve_skia_host_tests_{prefix}_{}_{seq}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).expect("create temp test dir");
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn compile_test_cdylib(dir: &Path, name: &str, source: &str) -> PathBuf {
    let src_path = dir.join(format!("{name}.rs"));
    std::fs::write(&src_path, source).expect("write test cdylib source");
    let ext = if cfg!(target_os = "macos") {
        "dylib"
    } else if cfg!(target_os = "windows") {
        "dll"
    } else {
        "so"
    };
    let out_path = dir.join(format!("lib{name}.{ext}"));
    let status = Command::new("rustc")
        .arg("--crate-type=cdylib")
        .arg("--edition=2024")
        .arg(&src_path)
        .arg("-o")
        .arg(&out_path)
        .status()
        .expect("invoke rustc for test cdylib");
    assert!(status.success(), "rustc failed compiling {name}");
    out_path
}

struct TestMemoryHandles {
    linking: Arc<MemoryLinking>,
    haptics: Arc<MemoryHaptics>,
}

fn make_memory_services() -> (TestMemoryHandles, Arc<SkiaHostServices>) {
    let mem = Arc::new(MemoryHostServices::new());
    let fs = Arc::new(MemoryFilesystem::new());
    fs.set_standard_path(StandardPathKind::Cache, "/tmp/nve-test-cache");
    let linking = Arc::new(MemoryLinking::new());
    let notifs = Arc::new(MemoryNotifications::new());
    notifs.set_permission(NotificationPermission::Authorized);
    let haptics = Arc::new(MemoryHaptics::new());
    let sys_env = Arc::new(MemorySystemEnv::with_initial(
        ThemeSnapshot {
            color_scheme: ColorScheme::Light,
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
    mem.register_arc(notifs);
    mem.register_arc(haptics.clone());

    let bus = mem.channel_bus();
    let svc = SkiaHostServices::with_provider(mem, bus);
    (TestMemoryHandles { linking, haptics }, svc)
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
    let notifications = NotificationsClient::new(&ffi);
    let haptics = HapticsClient::new(&ffi);

    // 1. Clipboard round-trip (including > 4 KiB payload forcing BufferTooSmall retry).
    assert!(!clipboard.has_text().unwrap());
    let large_text = "NVE-ZERO-JSON-CLIPBOARD-".repeat(300); // > 4096 bytes
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

    // 5. SystemEnv (NVT2 theme + NVL1 locale)
    let theme = system_env.get_theme().unwrap();
    assert_eq!(theme.color_scheme, ColorScheme::Light);
    assert_eq!(theme.accent_rgba, 0x3B82_F6FF);
    let locale = system_env.get_locale().unwrap();
    assert_eq!(locale.tag, "en-US");
    assert_eq!(locale.text_direction, TextDirection::Ltr);

    // 6. Notifications & Haptics
    assert_eq!(
        notifications.request_permission().unwrap(),
        NotificationPermission::Authorized
    );
    let mut notif = NotificationRequest::new("n1", "Build Ready", "Hot swap completed");
    notif.badge = Some(1);
    notifications.show(&notif).unwrap();
    notifications.cancel("n1").unwrap();
    haptics.impact(ImpactStyle::Medium).unwrap();
    haptics.notification(NotificationHaptic::Success).unwrap();
    haptics.selection().unwrap();
    assert_eq!(handles.haptics.log().len(), 3);

    // 7. NveChannelBus publish from app through C ABI callback
    let received = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let rx_clone = Arc::clone(&received);
    let custom_channel_id = {
        let bus_arc = services.channel_bus();
        let mut bus = bus_arc.lock().unwrap();
        let id = bus.ensure_channel("skia.custom").unwrap();
        bus.subscribe_by_id(id, None, false, move |msg| {
            rx_clone.lock().unwrap().push(msg.as_bytes().to_vec());
        })
        .unwrap();
        id
    };
    ffi.channel_publish(custom_channel_id, true, b"binary-channel-frame").unwrap();
    assert_eq!(received.lock().unwrap().as_slice(), &[b"binary-channel-frame".to_vec()]);

    // 8. Corrupt / unsupported error paths
    assert!(matches!(
        ffi.invoke_sync(SERVICE_WINDOW, METHOD_WINDOW_DISPATCH_BATCH, b"BAD!"),
        Err(ServiceError::InvalidArg(_))
    ));
    assert!(matches!(
        ffi.invoke_sync(SERVICE_WINDOW, 999, &[]),
        Err(ServiceError::Unsupported)
    ));
    assert!(matches!(
        ffi.invoke_sync(0x9999, 1, &[]),
        Err(ServiceError::Unsupported)
    ));

    // 9. Raw C-ABI null host_ctx / null out_buf safety checks
    let cb = services.callbacks();
    let invoke = cb.invoke.unwrap();
    let publish = cb.channel_publish.unwrap();
    let mut out_len = 0usize;
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
    assert_eq!(NveHostStatus::from_i32(status_null_ctx), NveHostStatus::InvalidArg);
    let status_null_pub = unsafe {
        publish(std::ptr::null_mut(), 1, 0, nve_abi::NveBufferSlice::EMPTY)
    };
    assert_eq!(NveHostStatus::from_i32(status_null_pub), NveHostStatus::InvalidArg);

    // Small buffer returns BufferTooSmall and writes exact required byte count (16 for NVT2).
    let mut tiny = [0u8; 4];
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
    assert_eq!(NveHostStatus::from_i32(status_small), NveHostStatus::BufferTooSmall);
    assert_eq!(out_len, 16);
}

#[test]
fn service_window_queues_before_sink_and_flushes_fifo_into_sink() {
    let (_mem, services) = make_memory_services();
    let ffi = FfiHostServices::with_callbacks(services.callbacks());

    let cmd1 = WindowCommand::SetTitle {
        handle: 1,
        title: "First".into(),
    };
    let cmd2 = WindowCommand::FocusWindow { handle: 2 };
    let wire = encode_window_commands(&[cmd1.clone(), cmd2.clone()]);
    let resp = ffi
        .invoke_sync(SERVICE_WINDOW, METHOD_WINDOW_DISPATCH_BATCH, &wire)
        .expect("dispatch NVW1 batch");
    assert_eq!(u32::from_le_bytes(resp.try_into().unwrap()), 2);

    // Attaching a sink (like EventLoopProxy<HostWakeEvent>) immediately flushes buffered commands in FIFO order.
    let forwarded = Arc::new(Mutex::new(Vec::<WindowCommand>::new()));
    let fwd_clone = Arc::clone(&forwarded);
    services.attach_window_sink(Arc::new(move |cmd| {
        fwd_clone.lock().unwrap().push(cmd);
    }));
    assert_eq!(forwarded.lock().unwrap().as_slice(), &[cmd1.clone(), cmd2.clone()]);

    // Subsequent commands go straight to the sink without buffering.
    let cmd3 = WindowCommand::CloseWindow { handle: 2 };
    let wire3 = encode_window_commands(std::slice::from_ref(&cmd3));
    let _ = ffi
        .invoke_sync(SERVICE_WINDOW, METHOD_WINDOW_DISPATCH_BATCH, &wire3)
        .unwrap();
    assert_eq!(forwarded.lock().unwrap().as_slice(), &[cmd1, cmd2, cmd3]);
    assert!(services.drain_queued_window_commands().is_empty());
}

/// Host-aware app mock backed by `FfiHostServices` to test `SERVICE_WINDOW`, live theme/locale
/// `NVH1` delivery, and app-driven `CloseRequested` veto/allow end-to-end.
struct HostAwareMockApp {
    surface_id: SurfaceId,
    ffi: FfiHostServices,
    bus: Arc<Mutex<NveChannelBus>>,
    frame_buf: Vec<u8>,
    themes_seen: Arc<Mutex<Vec<ThemeSnapshot>>>,
    locales_seen: Arc<Mutex<Vec<LocaleSnapshot>>>,
    lifecycle_events: Arc<Mutex<Vec<WindowLifecycleEvent>>>,
    veto_next_close: Arc<AtomicBool>,
    allow_close_via_service: Arc<AtomicBool>,
}

impl HostAwareMockApp {
    fn new(surface_id: SurfaceId) -> Self {
        let bus = Arc::new(Mutex::new(NveChannelBus::new()));
        let themes_seen = Arc::new(Mutex::new(Vec::new()));
        let locales_seen = Arc::new(Mutex::new(Vec::new()));
        let t_sink = Arc::clone(&themes_seen);
        let l_sink = Arc::clone(&locales_seen);
        {
            let mut b = bus.lock().unwrap();
            SystemEnvBridge::subscribe_theme(&mut b, Some(surface_id), true, move |t| {
                t_sink.lock().unwrap().push(t);
            })
            .unwrap();
            SystemEnvBridge::subscribe_locale(&mut b, Some(surface_id), true, move |l| {
                l_sink.lock().unwrap().push(l);
            })
            .unwrap();
        }
        let ffi = FfiHostServices::new();
        ffi.attach_channel_bus(Arc::clone(&bus));
        Self {
            surface_id,
            ffi,
            bus,
            frame_buf: Vec::new(),
            themes_seen,
            locales_seen,
            lifecycle_events: Arc::new(Mutex::new(Vec::new())),
            veto_next_close: Arc::new(AtomicBool::new(false)),
            allow_close_via_service: Arc::new(AtomicBool::new(false)),
        }
    }

    fn send_window_commands(&self, commands: &[WindowCommand]) -> u32 {
        let payload = encode_window_commands(commands);
        let resp = self
            .ffi
            .invoke_sync(SERVICE_WINDOW, METHOD_WINDOW_DISPATCH_BATCH, &payload)
            .expect("SERVICE_WINDOW invoke");
        u32::from_le_bytes(resp.try_into().unwrap())
    }
}

impl DesktopApp for HostAwareMockApp {
    fn generation(&self) -> u64 {
        1
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        let mut b = CommandBufferBuilder::new(self.surface_id, frame_id);
        b.create_node(2, ComponentKind::View, None);
        b.set_children(ROOT_NODE_ID, &[2]);
        b.commit(true);
        self.frame_buf = b.into_bytes();
        &self.frame_buf
    }

    fn dispatch_events(&mut self, _encoded_events: &[u8]) -> bool {
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
        match services {
            Some(svc) => {
                self.ffi.bind(Some(svc.callbacks()));
                let init = svc.encode_initial_env_batch();
                let _ = self.dispatch_host_events(&init);
                true
            }
            None => {
                self.ffi.bind(None);
                true
            }
        }
    }

    fn dispatch_host_events(&mut self, batch: &[u8]) -> bool {
        if self.ffi.dispatch_host_events(batch).is_err() {
            return false;
        }
        let mut intercepted_close = false;
        if let Ok(records) = NveHostEventBatch::decode_refs(batch) {
            for rec in records {
                if let NveHostEventRecordRef::HostPushEvent {
                    service_id: SERVICE_WINDOW,
                    event_id: EVENT_WINDOW_LIFECYCLE,
                    payload,
                } = rec
                    && let Ok(events) = decode_window_events(payload)
                {
                    for ev in events {
                        if let WindowLifecycleEvent::CloseRequested { handle, surface_id } = ev
                            && surface_id == self.surface_id
                        {
                            if self.veto_next_close.swap(false, Ordering::SeqCst) {
                                let _ = self.send_window_commands(&[
                                    WindowCommand::RespondCloseRequest {
                                        handle,
                                        allow: false,
                                    },
                                ]);
                                intercepted_close = true;
                            } else if self.allow_close_via_service.swap(false, Ordering::SeqCst) {
                                let _ = self.send_window_commands(&[
                                    WindowCommand::RespondCloseRequest {
                                        handle,
                                        allow: true,
                                    },
                                ]);
                                intercepted_close = true;
                            }
                        }
                        self.lifecycle_events.lock().unwrap().push(ev);
                    }
                }
            }
        }
        let _ = &self.bus;
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

    let primary_app = HostAwareMockApp::new(1);
    let primary_themes = Arc::clone(&primary_app.themes_seen);
    let primary_locales = Arc::clone(&primary_app.locales_seen);
    let primary_events = Arc::clone(&primary_app.lifecycle_events);
    let veto_flag = Arc::clone(&primary_app.veto_next_close);
    let allow_flag = Arc::clone(&primary_app.allow_close_via_service);

    let primary_session = DesktopSession::new(
        primary_app,
        1,
        Viewport { width: 640.0, height: 480.0, scale_factor: 1.0 },
        ColorRgba::BLACK,
        false,
    );
    let primary_h = host
        .open_window(
            1,
            1,
            SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Primary", 640.0, 480.0)),
            primary_session,
        )
        .unwrap();

    // 1. Initial bind pushed retained NVT2 theme + NVL1 locale AND OpenWindow pushed Opened+Focused!
    assert_eq!(primary_themes.lock().unwrap().len(), 1);
    assert_eq!(primary_themes.lock().unwrap()[0].color_scheme, ColorScheme::Light);
    assert_eq!(primary_locales.lock().unwrap().len(), 1);
    assert_eq!(primary_locales.lock().unwrap()[0].tag, "en-US");
    assert!(primary_events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, WindowLifecycleEvent::Opened { handle, surface_id: 1, .. } if *handle == primary_h)));

    // 2. App invokes SERVICE_WINDOW to open a Modal dialog, Sheet, Popover, and update title/bounds!
    let count = host.window(1).unwrap().session.app().send_window_commands(&[
        WindowCommand::OpenWindow {
            surface_id: 2,
            presentation: SurfacePresentation::SecondaryWindow {
                parent: Some(1),
                modal: true,
                descriptor: WindowDescriptor::new("Modal", 320.0, 200.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: 3,
            presentation: SurfacePresentation::Sheet {
                parent: 1,
                descriptor: WindowDescriptor::new("Inspector Sheet", 300.0, 240.0),
            },
        },
        WindowCommand::OpenWindow {
            surface_id: 4,
            presentation: SurfacePresentation::Popover {
                anchor_surface: 3,
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

    let results = host.drain_window_commands(u64::from);
    assert_eq!(results.len(), 5);
    let modal_h = results[0].as_ref().unwrap().unwrap();
    let sheet_h = results[1].as_ref().unwrap().unwrap();
    let popover_h = results[2].as_ref().unwrap().unwrap();
    assert_eq!(host.window_count(), 4);
    assert_eq!(
        host.window_manager().get(primary_h).unwrap().descriptor.title,
        "Primary Renamed"
    );
    assert!(primary_events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, WindowLifecycleEvent::Resized { handle, width, height, .. } if *handle == primary_h && (*width - 720.0).abs() < 0.1 && (*height - 540.0).abs() < 0.1)));

    // 3. Modal parent close blocking: BOTH handle_close_requested AND SERVICE_WINDOW::CloseWindow
    //    must fail with BlockedByModalChild while modal is active!
    assert_eq!(
        host.handle_close_requested(1),
        Err(WindowError::BlockedByModalChild {
            target: primary_h,
            modal_child: modal_h,
        })
    );
    host.window(1).unwrap().session.app().send_window_commands(&[
        WindowCommand::CloseWindow { handle: primary_h },
    ]);
    let blocked_res = host.drain_window_commands(u64::from);
    assert_eq!(
        blocked_res,
        vec![Err(WindowError::BlockedByModalChild {
            target: primary_h,
            modal_child: modal_h,
        })]
    );
    assert_eq!(host.window_count(), 4);

    // 4. Close modal via SERVICE_WINDOW -> modal closes & Primary receives Closed event!
    host.window(1).unwrap().session.app().send_window_commands(&[
        WindowCommand::CloseWindow { handle: modal_h },
    ]);
    let close_modal_res = host.drain_window_commands(u64::from);
    assert_eq!(close_modal_res, vec![Ok(None)]);
    assert_eq!(host.window_count(), 3);
    assert!(primary_events
        .lock()
        .unwrap()
        .iter()
        .any(|e| matches!(e, WindowLifecycleEvent::Closed { handle, surface_id: 2 } if *handle == modal_h)));

    // 5. Live theme change (Dark @ 2x) and locale change (ar-EG RTL) push NVT2/NVL1 via NVH1!
    let _ = host.simulate_theme_change(WinitTheme::Dark, 2.0);
    assert_eq!(primary_themes.lock().unwrap().last().unwrap().color_scheme, ColorScheme::Dark);
    assert_eq!(primary_themes.lock().unwrap().last().unwrap().scale_factor_x100, 200);
    assert_eq!(
        SystemEnvClient::new(&*services).get_theme().unwrap().color_scheme,
        ColorScheme::Dark
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

    // 6. App-driven CloseRequested VETO via SERVICE_WINDOW::RespondCloseRequest { allow: false }!
    veto_flag.store(true, Ordering::SeqCst);
    let veto_outcome = host.handle_close_requested(1).unwrap();
    assert!(veto_outcome.vetoed);
    assert_eq!(host.window_manager().get(primary_h).unwrap().lifecycle, WindowLifecycleState::ClosePending);
    let veto_drain = host.drain_window_commands(u64::from);
    assert_eq!(veto_drain, vec![Ok(None)]);
    assert_eq!(host.window_manager().get(primary_h).unwrap().lifecycle, WindowLifecycleState::Open);
    assert_eq!(host.window_count(), 3);

    // 7. App-driven CloseRequested ALLOW via SERVICE_WINDOW::RespondCloseRequest { allow: true }
    //    cascades Popover + Sheet + Primary cleanly!
    allow_flag.store(true, Ordering::SeqCst);
    let allow_pending = host.handle_close_requested(1).unwrap();
    assert!(allow_pending.vetoed); // deferred to queued RespondCloseRequest
    let allow_drain = host.drain_window_commands(u64::from);
    assert_eq!(allow_drain, vec![Ok(None)]);
    assert_eq!(host.window_count(), 0);
    assert!(host.window_manager().get(popover_h).is_none());
    assert!(host.window_manager().get(sheet_h).is_none());
    assert!(host.window_manager().get(primary_h).is_none());
}

#[test]
fn hot_app_slot_binds_host_services_rebinds_across_tier2_reload_without_dangling_pointers() {
    let dir = TempDirGuard::new("skia-host-reload");
    let cdylib_src = r#"
        use std::ffi::c_void;

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveBufferSlice { pub ptr: *const u8, pub len: usize }

        #[repr(C)]
        #[derive(Clone, Copy)]
        pub struct NveHostCallbacks {
            pub host_ctx: *mut c_void,
            pub invoke: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u64, NveBufferSlice, *mut u8, usize, *mut usize) -> i32>,
            pub channel_publish: Option<unsafe extern "C" fn(*mut c_void, u32, u32, NveBufferSlice) -> i32>,
        }

        #[repr(C)]
        pub struct NveSurfaceApi {
            pub create: unsafe extern "C" fn(u32) -> *mut c_void,
            pub destroy: unsafe extern "C" fn(*mut c_void),
            pub render: unsafe extern "C" fn(*mut c_void, u64) -> NveBufferSlice,
            pub dispatch_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub dispatch_events_direct: Option<unsafe extern "C" fn(*mut c_void, *const c_void, u32) -> u32>,
        }

        #[repr(C)]
        pub struct NveDevApi {
            pub apply_hot_patch: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
            pub snapshot_state: unsafe extern "C" fn(*mut c_void) -> NveBufferSlice,
            pub restore_state: unsafe extern "C" fn(*mut c_void, NveBufferSlice),
        }

        #[repr(C)]
        pub struct NveHostApi {
            pub bind_host: unsafe extern "C" fn(*mut c_void, *const NveHostCallbacks),
            pub dispatch_host_events: unsafe extern "C" fn(*mut c_void, NveBufferSlice) -> u32,
        }

        struct LiveHostCdylib {
            cb: Option<NveHostCallbacks>,
            env_batches: u32,
            reloads_preserved: u32,
            last_theme_scheme: u8,
            frame_buf: Vec<u8>,
            state_buf: Vec<u8>,
        }

        unsafe extern "C" fn create(_: u32) -> *mut c_void {
            Box::into_raw(Box::new(LiveHostCdylib {
                cb: None,
                env_batches: 0,
                reloads_preserved: 1,
                last_theme_scheme: 0,
                frame_buf: Vec::new(),
                state_buf: Vec::new(),
            })) as *mut c_void
        }

        unsafe extern "C" fn destroy(h: *mut c_void) {
            if !h.is_null() {
                unsafe { drop(Box::from_raw(h as *mut LiveHostCdylib)); }
            }
        }

        unsafe extern "C" fn render(h: *mut c_void, frame_id: u64) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            // Invoke SERVICE_SYSTEM_ENV::METHOD_GET_THEME (0x0014, 1) through bound callbacks
            // on every render to verify host_ctx is live and non-dangling across reloads!
            let mut scheme = 0u8;
            if let Some(cb) = app.cb
                && let Some(invoke) = cb.invoke
            {
                let mut out = [0u8; 32];
                let mut out_len = 0usize;
                let st = unsafe {
                    invoke(
                        cb.host_ctx,
                        0x0014,
                        1,
                        frame_id,
                        NveBufferSlice { ptr: std::ptr::null(), len: 0 },
                        out.as_mut_ptr(),
                        out.len(),
                        &mut out_len,
                    )
                };
                if st == 0 && out_len == 16 && &out[0..4] == b"NVT2" {
                    scheme = out[4];
                }
            }
            app.last_theme_scheme = scheme;
            app.frame_buf = format!(
                "f={frame_id};bound={};batches={};state={};scheme={}",
                u8::from(app.cb.is_some()),
                app.env_batches,
                app.reloads_preserved,
                app.last_theme_scheme,
            )
            .into_bytes();
            NveBufferSlice { ptr: app.frame_buf.as_ptr(), len: app.frame_buf.len() }
        }

        unsafe extern "C" fn dispatch_events(h: *mut c_void, ev: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            // Forward raw NVW1 bytes from `ev` directly to SERVICE_WINDOW (0x0002, method 1)!
            if ev.len > 0
                && let Some(cb) = app.cb
                && let Some(invoke) = cb.invoke
            {
                let mut out = [0u8; 8];
                let mut out_len = 0usize;
                let st = unsafe {
                    invoke(
                        cb.host_ctx,
                        0x0002,
                        1,
                        77,
                        ev,
                        out.as_mut_ptr(),
                        out.len(),
                        &mut out_len,
                    )
                };
                if st == 0 {
                    app.reloads_preserved += 1;
                    return 1;
                }
            }
            0
        }

        unsafe extern "C" fn apply_hot_patch(_: *mut c_void, _: NveBufferSlice) -> u32 { 0 }

        unsafe extern "C" fn snapshot_state(h: *mut c_void) -> NveBufferSlice {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            app.state_buf = app.reloads_preserved.to_le_bytes().to_vec();
            NveBufferSlice { ptr: app.state_buf.as_ptr(), len: app.state_buf.len() }
        }

        unsafe extern "C" fn restore_state(h: *mut c_void, s: NveBufferSlice) {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            if s.len == 4 && !s.ptr.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(s.ptr, 4) };
                app.reloads_preserved = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            }
        }

        unsafe extern "C" fn bind_host(h: *mut c_void, cb: *const NveHostCallbacks) {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            app.cb = if cb.is_null() { None } else { Some(unsafe { *cb }) };
        }

        unsafe extern "C" fn dispatch_host_events(h: *mut c_void, batch: NveBufferSlice) -> u32 {
            let app = unsafe { &mut *(h as *mut LiveHostCdylib) };
            if batch.len >= 4 && !batch.ptr.is_null() {
                let slice = unsafe { std::slice::from_raw_parts(batch.ptr, batch.len) };
                if &slice[0..4] == b"NVH1" {
                    app.env_batches += 1;
                    return 1;
                }
            }
            0
        }

        static SURFACE_API: NveSurfaceApi = NveSurfaceApi {
            create, destroy, render, dispatch_events, dispatch_events_direct: None,
        };
        static DEV_API: NveDevApi = NveDevApi { apply_hot_patch, snapshot_state, restore_state };
        static HOST_API: NveHostApi = NveHostApi { bind_host, dispatch_host_events };

        #[unsafe(no_mangle)]
        pub unsafe extern "C" fn nve_get_interface(id: u32) -> *const c_void {
            match id {
                1 => &SURFACE_API as *const NveSurfaceApi as *const c_void,
                4 => &DEV_API as *const NveDevApi as *const c_void,
                5 => &HOST_API as *const NveHostApi as *const c_void,
                _ => std::ptr::null(),
            }
        }
    "#;

    let dylib = compile_test_cdylib(dir.path(), "skia_host_reload_app", cdylib_src);
    let (_mem, services) = make_memory_services();

    let mut slot = HotAppSlot::load(&dylib, 1).expect("load cdylib into HotAppSlot");
    assert!(slot.has_host_interface());
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));

    // Gen 1 received initial NVH1 (NVT2+NVL1) batch and invokes SERVICE_SYSTEM_ENV::GET_THEME live!
    assert_eq!(slot.render(1), b"f=1;bound=1;batches=1;state=1;scheme=1");

    // Gen 1 invokes SERVICE_WINDOW over the bound C-ABI callback!
    let focus_wire = encode_window_commands(&[WindowCommand::FocusWindow { handle: 1 }]);
    assert!(slot.dispatch_events(&focus_wire));
    assert_eq!(
        services.drain_queued_window_commands(),
        vec![WindowCommand::FocusWindow { handle: 1 }]
    );

    // Update theme to Dark -> push NVH1 -> Tier-2 reload to Gen 2!
    let dark_batch = services.update_theme_from_winit(WinitTheme::Dark, 2.0).1;
    assert!(slot.dispatch_host_events(&dark_batch));
    assert_eq!(slot.render(2), b"f=2;bound=1;batches=2;state=2;scheme=2");

    // Tier-2 reload: preserves state=2, re-binds SkiaHostServices onto Gen 2 without dangling pointers,
    // and pushes retained Dark theme + locale!
    let gen2 = slot.reload(&dylib, 1).expect("reload gen 2");
    assert_eq!(gen2, 2);
    assert_eq!(slot.render(3), b"f=3;bound=1;batches=1;state=2;scheme=2");

    // Gen 2 still invokes SERVICE_WINDOW cleanly through the re-bound callback!
    let title_wire = encode_window_commands(&[WindowCommand::SetTitle {
        handle: 1,
        title: "From Gen 2".into(),
    }]);
    assert!(slot.dispatch_events(&title_wire));
    assert_eq!(
        services.drain_queued_window_commands(),
        vec![WindowCommand::SetTitle {
            handle: 1,
            title: "From Gen 2".into(),
        }]
    );

    // Unbind cleanly clears callbacks (scheme becomes 0, bound=0).
    assert!(slot.bind_host_services(None));
    assert_eq!(slot.render(4), b"f=4;bound=0;batches=1;state=3;scheme=0");

    // Re-bind + drop slot safely without dangling pointers.
    assert!(slot.bind_host_services(Some(Arc::clone(&services))));
    drop(slot);
    assert_eq!(Arc::strong_count(&services), 1);
}

#[test]
fn native_skia_host_services_and_nvh1_helpers_encode_binary_without_json() {
    let services = SkiaHostServices::new();
    let ffi = FfiHostServices::with_callbacks(services.callbacks());
    let system_env = SystemEnvClient::new(&ffi);
    let filesystem = FilesystemClient::new(&ffi);

    // Live NativeHostServices returns valid NVT2 theme, NVL1 locale, and OS cache path.
    let theme = system_env.get_theme().expect("native theme");
    let locale = system_env.get_locale().expect("native locale");
    assert!(!locale.tag.is_empty());
    let cache = filesystem.standard_path(StandardPathKind::Cache).expect("native cache dir");
    assert!(!cache.is_empty());

    // Helper batches decode strictly as binary NVH1 with retained flags and zero JSON.
    assert_eq!(winit_theme_to_color_scheme(WinitTheme::Light), ColorScheme::Light);
    assert_eq!(winit_theme_to_color_scheme(WinitTheme::Dark), ColorScheme::Dark);

    let env_batch = encode_system_env_batch(&theme, &locale);
    let recs = NveHostEventBatch::decode_refs(&env_batch).unwrap();
    assert_eq!(recs.len(), 2);
    assert!(matches!(
        recs[0],
        NveHostEventRecordRef::ChannelMessage {
            channel_id: CHANNEL_ID_THEME,
            flags: NVE_CHANNEL_FLAG_RETAIN,
            payload,
        } if ThemeSnapshot::decode_nvt2(payload).unwrap() == theme
    ));
    assert!(matches!(
        recs[1],
        NveHostEventRecordRef::ChannelMessage {
            channel_id: CHANNEL_ID_LOCALE,
            flags: NVE_CHANNEL_FLAG_RETAIN,
            payload,
        } if LocaleSnapshot::decode_nvl1(payload).unwrap() == locale
    ));

    let theme_only = encode_theme_channel_batch(&theme);
    assert_eq!(NveHostEventBatch::decode_refs(&theme_only).unwrap().len(), 1);
    let locale_only = encode_locale_channel_batch(&locale);
    assert_eq!(NveHostEventBatch::decode_refs(&locale_only).unwrap().len(), 1);

    let lc_batch = encode_window_lifecycle_batch(&[WindowLifecycleEvent::Closed {
        handle: 9,
        surface_id: 3,
    }]);
    let lc_recs = NveHostEventBatch::decode_refs(&lc_batch).unwrap();
    assert_eq!(lc_recs.len(), 1);
    match lc_recs[0] {
        NveHostEventRecordRef::HostPushEvent {
            service_id: SERVICE_WINDOW,
            event_id: EVENT_WINDOW_LIFECYCLE,
            payload,
        } => {
            assert_eq!(
                decode_window_events(payload).unwrap(),
                vec![WindowLifecycleEvent::Closed {
                    handle: 9,
                    surface_id: 3,
                }]
            );
        }
        other => panic!("unexpected record: {other:?}"),
    }
}
