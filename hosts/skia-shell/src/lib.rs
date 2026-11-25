//! Native View Engine — Cross-Platform Skia Desktop Host (`hosts/skia-shell`).
//!
//! Hosts any compiled NVE `cdylib` (`nve_get_interface`) inside a live `winit` desktop window
//! backed by `nve-backend-skia`, with automatic direct hardware GPU swapchains (`skia-metal`,
//! `skia-vulkan`, `skia-d3d`), zero-C++ `softbuffer` fallback, 2-tier WebSocket hot reload
//! (`HotPatchMsg`), instant-boot Frame-0 `MmapSnapshot` replay/save, and headless PPM capture.

pub mod window;

pub use window::{
    render_headless_screenshot, render_paired_headless_screenshot, replay_snapshot_into_backend,
    run_desktop_preview, run_desktop_preview_with_options, run_paired_desktop_preview,
    write_pixel_buffer_png, write_pixel_buffer_ppm, write_pixel_buffer_screenshot, CloseOutcome,
    DesktopApp, DesktopHostError, DesktopPreviewOptions, DesktopSession, DesktopWindowHost,
    HeadlessMultiWindowHost, HostWakeEvent, HotAppSlot, HotReloadClient, PairConnectConfig,
    PairTransport, PairedRemoteApp, PrewarmedSnapshot, SurfaceWindow, DEFAULT_PAIR_WS_PATH,
    DEFAULT_WS_URL,
};
