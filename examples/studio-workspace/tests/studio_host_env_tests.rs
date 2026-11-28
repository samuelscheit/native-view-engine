//! Host environment integration of the Studio module: the native entry point paints the OS
//! appearance from its first frame, and a bound `NVE_IFACE_HOST = 5` host drives clipboard and
//! theme/locale sync, repainting the mounted surface in place.

use std::sync::Arc;

mod common;

use common::register_studio_tables;
use nve_abi::{FrameId, SurfaceId};
use nve_backend_headless::HeadlessBackend;
use nve_clipboard::{ClipboardClient, MemoryClipboard};
use nve_engine::{NveApp, StoreBackend, StoreWireReplica, Viewport};
use nve_host_services::{HostCallbackBridge, MemoryHostServices};
use nve_interop::traits::{ColorScheme, HighContrast, ReducedMotion};
use nve_studio_workspace::StudioWorkspaceModule;
use nve_system_env::{
    ContrastLevel, LocaleSnapshot, MemorySystemEnv, NativeSystemEnv, SystemEnvBackend, SystemEnvEvent,
    TextDirection, ThemeSnapshot,
};

#[test]
fn native_entry_point_paints_the_os_appearance_before_a_host_binds() {
    let os = NativeSystemEnv::new();
    let (theme, locale) = (os.get_theme().expect("OS theme"), os.get_locale().expect("OS locale"));
    let mut module = StudioWorkspaceModule::create(SurfaceId::ROOT);
    let _ = module.render(FrameId(1));
    let env = module.state_snapshot().system_env;
    assert_eq!(env.theme, theme, "the first frame resolves the OS appearance, not the Studio default");
    assert_eq!(env.locale.tag, locale.tag);
}

#[test]
fn nve_iface_host_5_ffi_callbacks_clipboard_and_system_env_sync() {
    register_studio_tables();
    let mut module = common::headless_module(SurfaceId::ROOT);
    let first_frame = module.render(FrameId(1)).to_vec();
    assert!(!module.host_services().is_ffi_bound());

    // Prepare a simulated native host behind `HostCallbackBridge` (`NVE_IFACE_HOST = 5`).
    let host_mem = Arc::new(MemoryHostServices::new());
    host_mem.register(MemoryClipboard::new());
    host_mem.register(MemorySystemEnv::with_initial(
        ThemeSnapshot {
            color_scheme: Some(ColorScheme::Light),
            contrast: ContrastLevel::High,
            reduced_motion: true,
            reduced_transparency: false,
            accent_rgba: 0x38BD_F8FF,
            font_scale_x1000: 1250,
            scale_factor_x100: 200,
        },
        LocaleSnapshot {
            tag: "de-DE".into(),
            language: "de".into(),
            region: "DE".into(),
            text_direction: TextDirection::Ltr,
            uses_24h_clock: true,
            timezone: "Europe/Berlin".into(),
        },
    ));
    let bridge = HostCallbackBridge::new(Arc::clone(&host_mem));

    // Bind host callbacks across the `NVE_IFACE_HOST = 5` ABI contract.
    module.bind_host(Some(bridge.callbacks()));
    assert!(module.host_services().is_ffi_bound());

    // Binding automatically synced host theme & locale via `SystemEnvClient` into `TraitStore`!
    let snap = module.state_snapshot();
    assert_eq!(snap.color_scheme(), ColorScheme::Light);
    assert!(snap.trait_store().get::<HighContrast>());
    assert!(snap.trait_store().get::<ReducedMotion>());
    assert_eq!(snap.locale().as_str(), "de-DE");
    assert!(!snap.locale().is_rtl());

    // Render frame 2 (a delta on the still-mounted surface) and verify the live UI reflects the
    // synced Light + de-DE traits.
    let frame_bytes = module.render(FrameId(2)).to_vec();
    let mut headless = HeadlessBackend::new();
    headless.set_viewport(
        SurfaceId::ROOT,
        Viewport {
            width: 1280.0,
            height: 800.0,
            scale_factor: 1.0,
        },
    );
    let mut rep = StoreWireReplica::new(headless, nve_tables::tables::core_realization_order());
    rep.submit_nve2_frame(&first_frame).expect("frame 1");
    rep.submit_nve2_frame(&frame_bytes).expect("frame 2");
    let tree = rep.backend().render_ascii_tree(rep.store());
    assert!(
        tree.contains("Light | de-DE"),
        "Studio panel should render synced host ColorScheme + Locale:\n{tree}"
    );

    // Copying Markdown while FFI-bound routes through `NVE_IFACE_HOST = 5` into `host_mem`!
    let md = snap.active_doc_markdown();
    module
        .host_services()
        .copy_text(&md)
        .expect("FFI clipboard write");
    assert_eq!(
        ClipboardClient::new(&*host_mem)
            .read_text()
            .unwrap()
            .as_deref(),
        Some(md.as_str())
    );

    // Deliver live typed theme (Dark) and locale (ar-EG RTL) updates directly with zero serialization.
    let dark_theme = ThemeSnapshot {
        color_scheme: Some(ColorScheme::Dark),
        contrast: ContrastLevel::Standard,
        reduced_motion: false,
        reduced_transparency: false,
        accent_rgba: 0xA78B_FAFF,
        font_scale_x1000: 1000,
        scale_factor_x100: 200,
    };
    let ar_locale = LocaleSnapshot {
        tag: "ar-EG".into(),
        language: "ar".into(),
        region: "EG".into(),
        text_direction: TextDirection::Rtl,
        uses_24h_clock: true,
        timezone: "Africa/Cairo".into(),
    };
    assert!(module.dispatch_system_env_events(&[
        SystemEnvEvent::ThemeChanged(dark_theme),
        SystemEnvEvent::LocaleChanged(ar_locale),
    ]));

    let updated = module.state_snapshot();
    assert_eq!(updated.color_scheme(), ColorScheme::Dark);
    assert_eq!(updated.locale().as_str(), "ar-EG");
    assert!(updated.locale().is_rtl());
    rep.submit_nve2_frame(module.render(FrameId(3))).expect("frame 3");
    let tree = rep.backend().render_ascii_tree(rep.store());
    assert!(
        tree.contains("Dark | ar-EG"),
        "live theme/locale events must repaint in place:\n{tree}"
    );

    // Unbinding restores clean fallback to `MemoryHostServices`.
    module.bind_host(None);
    assert!(!module.host_services().is_ffi_bound());
    module
        .host_services()
        .copy_text("fallback-memory-ok")
        .expect("fallback clipboard write");
    assert_eq!(
        module
            .host_services()
            .read_clipboard_text()
            .unwrap()
            .as_deref(),
        Some("fallback-memory-ok")
    );
}
