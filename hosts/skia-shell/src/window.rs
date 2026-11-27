//! Cross-platform desktop preview window (`winit` + `softbuffer` / hardware GPU swapchains)
//! with live hit-testing, interactive `TextInput` focus/IME/caret editing, 2-tier WebSocket
//! hot reload (`HotPatchMsg` from `ws://127.0.0.1:8089`), and instant-boot Frame-0
//! `NVERELOC` + `MmapOffsetArena` (`NativeInstantBootStore`) replay & atomic save on exit.

pub mod clipboard;
pub mod dioxus_app;
pub mod hot_reload_client;
mod ime;
pub mod input;
pub mod instant_boot;
mod keyboard;
pub mod multi_window;
pub mod pair_client;
pub mod session;
pub mod text_edit;
mod window_commands;
pub mod window_host;

#[cfg(test)]
mod dioxus_app_tests;
#[cfg(test)]
mod host_reload_tests;
#[cfg(test)]
mod host_services_tests;
#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod multi_window_tests;
#[cfg(test)]
mod pair_tests;
#[cfg(test)]
mod scroll_keydown_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_maturity_tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nve_abi::{ColorRgba, FrameId, NveEventBatchRef, SurfaceId};
use nve_backend_skia::encode_rgb_png;
use nve_backend_skia::gpu::GpuBackendKind;
use nve_backend_skia::raster::PixelBuffer;
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::{BackendError, InstantBootError, RelocError};

use crate::host_services::{SkiaHostEvent, SkiaHostServices};

#[cfg(test)]
pub use dioxus_app::DioxusDesktopApp;
#[cfg(test)]
pub use hot_reload_client::HotReloadClient;
pub(crate) use instant_boot::prewarm_from_options;
#[cfg(test)]
pub use instant_boot::{replay_instant_boot_store_into_backend, replay_snapshot_into_backend};
pub use multi_window::HostWakeEvent;
pub use pair_client::{PairConnectConfig, PairTransport};
#[cfg(test)]
pub use pair_client::PairedRemoteApp;
#[cfg(test)]
pub use session::DesktopSession;
pub use window_host::DesktopWindowHost;

pub const DEFAULT_WS_URL: &str = "ws://127.0.0.1:8089";

#[derive(Debug)]
pub enum DesktopHostError {
    Loader(LoaderError),
    Backend(BackendError),
    Reloc(RelocError),
    InstantBoot(InstantBootError),
    Io(std::io::Error),
    Pairing(String),
    EventLoop(String),
    Window(String),
    Surface(String),
}

impl fmt::Display for DesktopHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader(err) => write!(f, "failed to load NVE app cdylib: {err}"),
            Self::Backend(err) => write!(f, "render backend error: {err}"),
            Self::Reloc(err) => write!(f, "NVERELOC error: {err}"),
            Self::InstantBoot(err) => write!(f, "instant-boot error: {err}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Pairing(msg) => write!(f, "live pairing error: {msg}"),
            Self::EventLoop(msg) => write!(f, "event loop error: {msg}"),
            Self::Window(msg) => write!(f, "window error: {msg}"),
            Self::Surface(msg) => write!(f, "softbuffer surface error: {msg}"),
        }
    }
}

impl std::error::Error for DesktopHostError {}

impl From<LoaderError> for DesktopHostError {
    fn from(value: LoaderError) -> Self {
        Self::Loader(value)
    }
}

impl From<BackendError> for DesktopHostError {
    fn from(value: BackendError) -> Self {
        Self::Backend(value)
    }
}

impl From<RelocError> for DesktopHostError {
    fn from(value: RelocError) -> Self {
        Self::Reloc(value)
    }
}

impl From<InstantBootError> for DesktopHostError {
    fn from(value: InstantBootError) -> Self {
        Self::InstantBoot(value)
    }
}

impl From<std::io::Error> for DesktopHostError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Configuration for the live desktop preview window and optional instant-boot store/snapshot.
#[derive(Debug, Clone, PartialEq)]
pub struct DesktopPreviewOptions {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub scale_factor: f32,
    pub clear_color: ColorRgba,
    pub ws_url: Option<String>,
    pub surface_id: SurfaceId,
    pub show_hud: bool,
    pub preferred_backend: GpuBackendKind,
    pub vsync: bool,
    pub snapshot_path: Option<PathBuf>,
    pub instant_boot_dir: Option<PathBuf>,
    pub default_frame0_path: Option<PathBuf>,
    pub schema_version: u32,
    pub disable_instant_boot: bool,
    pub reset_state: bool,
}

impl Default for DesktopPreviewOptions {
    fn default() -> Self {
        Self {
            title: "Native View Engine — Skia Desktop Shell".to_string(),
            width: 420.0,
            height: 780.0,
            scale_factor: 1.0,
            clear_color: ColorRgba::rgb(15, 23, 42),
            ws_url: Some(DEFAULT_WS_URL.to_string()),
            surface_id: 1,
            show_hud: true,
            preferred_backend: GpuBackendKind::platform_preferred(),
            vsync: true,
            snapshot_path: None,
            instant_boot_dir: None,
            default_frame0_path: None,
            schema_version: 1,
            disable_instant_boot: false,
            reset_state: false,
        }
    }
}

/// Runtime interface for an NVE app hosted inside [`DesktopSession`].
///
/// Implemented for [`HotAppSlot`] (backed by `HotAppInstance` cdylib hot-swapping) and mockable
/// in unit tests without spawning an OS window.
pub trait DesktopApp {
    fn generation(&self) -> u64;
    fn render(&mut self, frame_id: FrameId) -> &[u8];
    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool;
    /// Forwards one binary `NVP1` hot-patch frame to the app's `NveDevApi::apply_hot_patch`.
    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool;
    fn reload(
        &mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError>;

    /// Serializes the current app state for snapshot persistence or hot-swap preservation.
    fn snapshot_state(&mut self) -> &[u8] {
        &[]
    }

    /// Returns `true` when this app exposes `NVE_IFACE_HOST = 5`.
    #[cfg(test)]
    fn has_host_interface(&self) -> bool {
        false
    }

    /// Binds (or unbinds when `None`) the host's [`SkiaHostServices`] callbacks onto this app
    /// via `NveHostApi::bind_host`. Returns `true` if the app bound the host callbacks.
    fn bind_host_services(&mut self, _services: Option<Arc<SkiaHostServices>>) -> bool {
        false
    }

    /// Dispatches a direct typed [`SkiaHostEvent`] (theme, locale, or window lifecycle) with
    /// zero in-process serialization.
    fn dispatch_host_event(&mut self, _event: &SkiaHostEvent) -> bool {
        false
    }

    /// Registers an optional event-loop wake callback invoked when asynchronous remote frames
    /// arrive (used by [`PairedRemoteApp`]).
    fn install_wake_callback(&mut self, _wake: Arc<dyn Fn() + Send + Sync>) {}

    /// Polls any in-process asynchronous tasks (such as `DioxusSurface::poll_work`) and returns
    /// `true` when dirty state requires rendering a fresh frame.
    fn poll_async_work(&mut self) -> bool {
        false
    }

    /// Returns `true` when an asynchronous remote frame is queued and ready to be rendered.
    fn has_pending_remote_frame(&self) -> bool {
        false
    }

    /// Optional live status banner for remote/paired sessions (shown in HUD & window title).
    fn remote_status_banner(&self) -> Option<String> {
        None
    }
}

/// Owns a live [`HotAppInstance`] and preserves the last state snapshot and bound
/// [`SkiaHostServices`] across Tier-2 reloads (including recovery if an intermediate reload fails).
pub struct HotAppSlot {
    // Field order is load-bearing: `instance` MUST drop before `host_services` so the loaded
    // cdylib never observes a dangling `host_ctx` pointer during `destroy`.
    instance: Option<HotAppInstance>,
    generation: u64,
    last_saved_state: Vec<u8>,
    host_services: Option<Arc<SkiaHostServices>>,
}

impl HotAppSlot {
    pub fn load(dylib_path: &Path, surface_id: SurfaceId) -> Result<Self, DesktopHostError> {
        Self::load_with_state(dylib_path, surface_id, None)
    }

    pub fn load_with_state(
        dylib_path: &Path,
        surface_id: SurfaceId,
        prior_state: Option<&[u8]>,
    ) -> Result<Self, DesktopHostError> {
        let instance = HotAppInstance::load(dylib_path, surface_id, 1, prior_state)?;
        let generation = instance.generation();
        Ok(Self {
            instance: Some(instance),
            generation,
            last_saved_state: prior_state.map(|s| s.to_vec()).unwrap_or_default(),
            host_services: None,
        })
    }
}

impl Drop for HotAppSlot {
    fn drop(&mut self) {
        if let Some(inst) = self.instance.as_mut() {
            let _ = inst.bind_host(None);
        }
        self.instance.take();
        self.host_services.take();
    }
}

impl DesktopApp for HotAppSlot {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        match self.instance.as_mut() {
            Some(inst) => inst.render(frame_id),
            None => &[],
        }
    }

    fn dispatch_events(&mut self, events: NveEventBatchRef<'_>) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.dispatch_events(events.as_slice()),
            None => false,
        }
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.apply_hot_patch(patch_nvp1),
            None => false,
        }
    }

    fn reload(
        &mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        if let Some(inst) = self.instance.as_mut() {
            let state = inst.snapshot_state();
            if !state.is_empty() {
                self.last_saved_state.clear();
                self.last_saved_state.extend_from_slice(state);
            }
            let outcome = inst.reload(new_dylib_path)?;
            self.generation = outcome.generation;
            // Re-bind explicitly (also covers gen 1 lacking NVE_IFACE_HOST while gen 2 exposes it)
            // and push the direct #[repr(C)] ThemeSnapshot across the cdylib boundary.
            if let Some(svc) = self.host_services.as_ref()
                && inst.bind_host(Some(svc.callbacks()))
            {
                let theme_c = svc.current_theme().to_repr_c_bytes();
                let _ = inst.dispatch_host_events(&theme_c);
            }
            Ok(self.generation)
        } else {
            let next_gen = self.generation + 1;
            let prior = (!self.last_saved_state.is_empty())
                .then_some(self.last_saved_state.as_slice());
            let mut loaded = HotAppInstance::load(new_dylib_path, surface_id, next_gen, prior)?;
            if let Some(svc) = self.host_services.as_ref()
                && loaded.bind_host(Some(svc.callbacks()))
            {
                let theme_c = svc.current_theme().to_repr_c_bytes();
                let _ = loaded.dispatch_host_events(&theme_c);
            }
            self.generation = loaded.generation();
            self.instance = Some(loaded);
            Ok(self.generation)
        }
    }

    fn snapshot_state(&mut self) -> &[u8] {
        if let Some(inst) = self.instance.as_mut() {
            let state = inst.snapshot_state();
            if !state.is_empty() {
                self.last_saved_state.clear();
                self.last_saved_state.extend_from_slice(state);
            }
        }
        &self.last_saved_state
    }

    #[cfg(test)]
    fn has_host_interface(&self) -> bool {
        self.instance
            .as_ref()
            .is_some_and(HotAppInstance::has_host_interface)
    }

    fn bind_host_services(&mut self, services: Option<Arc<SkiaHostServices>>) -> bool {
        match services {
            Some(svc) => {
                let cb = svc.callbacks();
                let bound = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(Some(cb)));
                // Store the new Arc AFTER `bind_host` updates the dylib's pointer so any previous
                // Arc stays alive during the swap (zero dangling-pointer window).
                self.host_services = Some(svc);
                if bound && let Some(svc_ref) = self.host_services.as_ref() {
                    let theme_c = svc_ref.current_theme().to_repr_c_bytes();
                    if let Some(inst) = self.instance.as_mut() {
                        let _ = inst.dispatch_host_events(&theme_c);
                    }
                }
                bound
            }
            None => {
                let had = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(None));
                self.host_services = None;
                had
            }
        }
    }

    fn dispatch_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        let Some(inst) = self.instance.as_mut() else {
            return false;
        };
        match event {
            SkiaHostEvent::ThemeChanged(theme) => {
                let theme_c = theme.to_repr_c_bytes();
                inst.dispatch_host_events(&theme_c)
            }
            #[cfg(test)]
            SkiaHostEvent::SystemEnv { theme, .. } => {
                let theme_c = theme.to_repr_c_bytes();
                inst.dispatch_host_events(&theme_c)
            }
            #[cfg(test)]
            SkiaHostEvent::LocaleChanged(_) => false,
            SkiaHostEvent::WindowLifecycle(_) => false,
        }
    }
}

/// Writes a [`PixelBuffer`] to disk as a binary `P6` PPM image.
pub fn write_pixel_buffer_ppm(buf: &PixelBuffer, path: &Path) -> std::io::Result<()> {
    let mut out = Vec::with_capacity((buf.width() * buf.height() * 3) as usize + 32);
    out.extend_from_slice(format!("P6\n{} {}\n255\n", buf.width(), buf.height()).as_bytes());
    for y in 0..buf.height() {
        for x in 0..buf.width() {
            let (r, g, b) = buf.get_rgb(x, y).unwrap_or((0, 0, 0));
            out.push(r);
            out.push(g);
            out.push(b);
        }
    }
    std::fs::write(path, out)
}

/// Writes a [`PixelBuffer`] to disk as a standard RGB PNG image.
pub fn write_pixel_buffer_png(buf: &PixelBuffer, path: &Path) -> std::io::Result<()> {
    let mut rgb = Vec::with_capacity((buf.width() * buf.height() * 3) as usize);
    for y in 0..buf.height() {
        for x in 0..buf.width() {
            let (r, g, b) = buf.get_rgb(x, y).unwrap_or((0, 0, 0));
            rgb.push(r);
            rgb.push(g);
            rgb.push(b);
        }
    }
    let png = encode_rgb_png(buf.width(), buf.height(), &rgb)?;
    std::fs::write(path, png)
}

/// Writes a [`PixelBuffer`] as PNG when `path` has a `.png` extension, or binary `P6` PPM otherwise.
pub fn write_pixel_buffer_screenshot(buf: &PixelBuffer, path: &Path) -> std::io::Result<()> {
    let is_png = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("png"));
    if is_png {
        write_pixel_buffer_png(buf, path)
    } else {
        write_pixel_buffer_ppm(buf, path)
    }
}

/// Opens a live cross-platform desktop preview window with custom window, snapshot, and HUD options.
pub fn run_desktop_preview_with_options(
    dylib_path: PathBuf,
    options: DesktopPreviewOptions,
) -> Result<(), DesktopHostError> {
    let host = DesktopWindowHost::new(dylib_path, options)?;
    host.run()
}

/// Headlessly renders one frame of `dylib_path` (with optional Frame-0 `MmapSnapshot` replay
/// before load and atomic snapshot save on exit) and writes the result as PNG or `P6` PPM.
pub fn render_headless_screenshot(
    dylib_path: PathBuf,
    mut options: DesktopPreviewOptions,
    screenshot_path: PathBuf,
) -> Result<(), DesktopHostError> {
    if options.instant_boot_dir.is_none() && options.snapshot_path.is_none() {
        options.disable_instant_boot = true;
    }
    let mut host = DesktopWindowHost::new(dylib_path, options)?;
    host.session().write_screenshot(&screenshot_path)?;
    host.save_snapshot_if_configured();
    Ok(())
}

/// Opens a live thin-client desktop preview window paired directly to a remote `nve pair` server
/// over UDP, TCP, or WebSocket (`NVT1`/`NVD1`), with zero local `cdylib` required.
pub fn run_paired_desktop_preview(
    pair_config: PairConnectConfig,
    options: DesktopPreviewOptions,
) -> Result<(), DesktopHostError> {
    let host = DesktopWindowHost::new_paired(pair_config, options)?;
    host.run()
}

/// Headlessly connects to a remote `nve pair` server, receives Frame 1 over `NVT1`/`NVD1`,
/// rasterizes it to PNG or `P6` PPM, and disconnects cleanly.
pub fn render_paired_headless_screenshot(
    pair_config: PairConnectConfig,
    options: DesktopPreviewOptions,
    screenshot_path: PathBuf,
) -> Result<(), DesktopHostError> {
    let mut host = DesktopWindowHost::new_paired(pair_config, options)?;
    host.session().write_screenshot(&screenshot_path)?;
    host.session_mut().app_mut().disconnect();
    Ok(())
}
