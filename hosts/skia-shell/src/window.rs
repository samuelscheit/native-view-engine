//! Cross-platform desktop preview window (`winit` + `softbuffer` / hardware GPU swapchains)
//! with live hit-testing, interactive `TextInput` focus/IME/caret editing, 2-tier WebSocket
//! hot reload (`HotPatchMsg` from `ws://127.0.0.1:8089`), and instant-boot Frame-0
//! `MmapSnapshot` replay + atomic save on exit.

pub mod hot_reload_client;
pub mod input;
pub mod pair_client;
pub mod session;
pub mod window_host;

#[cfg(test)]
mod input_tests;
#[cfg(test)]
mod pair_tests;
#[cfg(test)]
mod tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use nve_abi::{ColorRgba, FrameId, SurfaceId};
use nve_backend_skia::encode_rgb_png;
use nve_backend_skia::gpu::GpuBackendKind;
use nve_backend_skia::raster::PixelBuffer;
use nve_backend_skia::SkiaBackend;
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::{BackendError, MmapSnapshot, RenderBackend, SnapshotError, Viewport};

pub use hot_reload_client::HotReloadClient;
pub use input::format_keydown_shorthand;
pub use pair_client::{PairConnectConfig, PairTransport, PairedRemoteApp, DEFAULT_PAIR_WS_PATH};
pub use session::DesktopSession;
pub use window_host::DesktopWindowHost;

pub const DEFAULT_WS_URL: &str = "ws://127.0.0.1:8089";

#[derive(Debug)]
pub enum DesktopHostError {
    Loader(LoaderError),
    Backend(BackendError),
    Snapshot(SnapshotError),
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
            Self::Snapshot(err) => write!(f, "snapshot error: {err}"),
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

impl From<SnapshotError> for DesktopHostError {
    fn from(value: SnapshotError) -> Self {
        Self::Snapshot(value)
    }
}

impl From<std::io::Error> for DesktopHostError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Configuration for the live desktop preview window and optional instant-boot snapshot.
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
    pub schema_version: u32,
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
            schema_version: 1,
        }
    }
}

/// Runtime interface for an NVE app hosted inside [`DesktopSession`].
///
/// Implemented for [`HotAppSlot`] (backed by `HotAppInstance` cdylib hot-swapping) and mockable
/// in unit tests without spawning an OS window.
pub trait DesktopApp {
    fn generation(&self) -> u64;
    fn render(&mut self, frame_id: FrameId) -> Vec<u8>;
    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool;
    fn apply_hot_patch(&mut self, patch_json: &str) -> bool;
    fn reload(
        &mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError>;

    /// Serializes the current app state for snapshot persistence or hot-swap preservation.
    fn snapshot_state(&mut self) -> Vec<u8> {
        Vec::new()
    }

    /// Registers an optional event-loop wake callback invoked when asynchronous remote frames
    /// arrive (used by [`PairedRemoteApp`]).
    fn install_wake_callback(&mut self, _wake: Arc<dyn Fn() + Send + Sync>) {}

    /// Returns `true` when an asynchronous remote frame is queued and ready to be rendered.
    fn has_pending_remote_frame(&self) -> bool {
        false
    }

    /// Optional live status banner for remote/paired sessions (shown in HUD & window title).
    fn remote_status_banner(&self) -> Option<String> {
        None
    }
}

/// Owns a live [`HotAppInstance`] and preserves the last state snapshot across Tier-2 reloads
/// (including recovery if an intermediate reload fails).
pub struct HotAppSlot {
    instance: Option<HotAppInstance>,
    generation: u64,
    last_saved_state: Vec<u8>,
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
        })
    }
}

impl DesktopApp for HotAppSlot {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> Vec<u8> {
        match self.instance.as_mut() {
            Some(inst) => inst.render(frame_id),
            None => Vec::new(),
        }
    }

    fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.dispatch_events(encoded_events),
            None => false,
        }
    }

    fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.apply_hot_patch(patch_json),
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
                self.last_saved_state = state;
            }
            let outcome = inst.reload(new_dylib_path)?;
            self.generation = outcome.generation;
            Ok(self.generation)
        } else {
            let next_gen = self.generation + 1;
            let prior = if self.last_saved_state.is_empty() {
                None
            } else {
                Some(self.last_saved_state.as_slice())
            };
            let loaded = HotAppInstance::load(new_dylib_path, surface_id, next_gen, prior)?;
            self.generation = loaded.generation();
            self.instance = Some(loaded);
            Ok(self.generation)
        }
    }

    fn snapshot_state(&mut self) -> Vec<u8> {
        if let Some(inst) = self.instance.as_mut() {
            let state = inst.snapshot_state();
            if !state.is_empty() {
                self.last_saved_state = state.clone();
            }
            state
        } else {
            self.last_saved_state.clone()
        }
    }
}

/// Pre-warmed Frame-0 backend state decoded directly from a zero-copy [`MmapSnapshot`] prior to
/// loading any app `cdylib`.
pub struct PrewarmedSnapshot {
    pub backend: SkiaBackend,
    pub pixel_buffer: PixelBuffer,
    pub frame0_bytes: Vec<u8>,
    pub state_bytes: Vec<u8>,
    pub restore_us: u64,
}

/// Opens an `NVE_SNAP` snapshot with schema validation, replays its zero-copy Frame-0
/// `CommandBuffer` straight into a [`SkiaBackend`], and rasterizes Frame 0 before any app `cdylib`
/// is loaded.
pub fn replay_snapshot_into_backend(
    path: &Path,
    schema_version: u32,
    surface_id: SurfaceId,
    viewport: Viewport,
    clear_color: ColorRgba,
) -> Result<PrewarmedSnapshot, DesktopHostError> {
    let t0 = Instant::now();
    let snap = MmapSnapshot::open_with_schema(path, schema_version)?;
    let frame0 = snap.frame0_command_buffer();
    let mut backend = SkiaBackend::new();
    backend.set_viewport(surface_id, viewport);
    if !frame0.is_empty() {
        backend.submit_frame(frame0)?;
    }
    let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
    let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
    let mut pixel_buffer = PixelBuffer::new(phys_w, phys_h);
    backend.rasterize(&mut pixel_buffer, clear_color);
    let restore_us = t0.elapsed().as_micros() as u64;

    Ok(PrewarmedSnapshot {
        backend,
        pixel_buffer,
        frame0_bytes: frame0.to_vec(),
        state_bytes: snap.state_bytes().to_vec(),
        restore_us,
    })
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

/// Opens a live cross-platform desktop preview window for the given NVE app `cdylib`.
///
/// Pass `ws_url = Some("ws://127.0.0.1:8089".into())` (or `Some(DEFAULT_WS_URL.into())`) to enable
/// 2-tier WebSocket hot reload from `nve dev`, or `None` to run standalone.
pub fn run_desktop_preview(
    dylib_path: PathBuf,
    ws_url: Option<String>,
) -> Result<(), DesktopHostError> {
    let options = DesktopPreviewOptions {
        ws_url,
        ..Default::default()
    };
    run_desktop_preview_with_options(dylib_path, options)
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
    options: DesktopPreviewOptions,
    screenshot_path: PathBuf,
) -> Result<(), DesktopHostError> {
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
