//! Cross-platform desktop preview window (`winit` + `softbuffer` / hardware GPU swapchains)
//! with live hit-testing, interactive `TextInput` focus/IME/caret editing, 2-tier WebSocket
//! hot reload (`HotPatchMsg` from `ws://127.0.0.1:8089`), and optional Frame-0 `frame0.nve2`
//! keyframe pre-warm & atomic refresh.

pub mod a11y;
pub mod clipboard;
mod hot_app_slot;
pub mod hot_reload_client;
mod ime;
pub mod input;
mod keyboard;
pub mod multi_window;
pub mod session;
pub mod text_edit;
mod window_commands;
pub mod window_host;

#[cfg(test)]
mod a11y_tests;
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

use nve_abi::{ColorRgba, FrameId, SurfaceId, ValidatedEvents};
use nve_backend_skia::encode_rgb_png;
use nve_backend_skia::gpu::GpuBackendKind;
use nve_backend_skia::raster::PixelBuffer;
use nve_engine::loader::LoaderError;
use nve_engine::{
    AppFrame, BackendError, Frame0Miss, HotPatchMsg, PersistenceError, RuntimeApp, WakeBridge,
};
use nve_layout::{DynLayoutEngine, TaffyStoreLayout};
use nve_runtime::PairReplicaClient;
use nve_store::wire::WireEncoder;

use crate::host_services::{SkiaHostEvent, SkiaHostServices};

pub use hot_app_slot::HotAppSlot;
#[cfg(test)]
pub use hot_reload_client::HotReloadClient;
pub use multi_window::HostWakeEvent;
pub use nve_wire::PairConnectConfig;
#[cfg(test)]
pub use session::DesktopSession;
pub use window_host::DesktopWindowHost;

pub const DEFAULT_WS_URL: &str = "ws://127.0.0.1:8089";

#[derive(Debug)]
pub enum DesktopHostError {
    Loader(LoaderError),
    Backend(BackendError),
    Frame0(Frame0Miss),
    Persistence(PersistenceError),
    Io(std::io::Error),
    Pairing(String),
    EventLoop(String),
    Window(String),
    Surface(String),
    Reload(String),
    Layout(String),
}

impl fmt::Display for DesktopHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader(err) => write!(f, "failed to load NVE app cdylib: {err}"),
            Self::Backend(err) => write!(f, "render backend error: {err}"),
            Self::Frame0(err) => write!(f, "frame0.nve2 error: {err}"),
            Self::Persistence(err) => write!(f, "persistence error: {err}"),
            Self::Io(err) => write!(f, "I/O error: {err}"),
            Self::Pairing(msg) => write!(f, "live pairing error: {msg}"),
            Self::EventLoop(msg) => write!(f, "event loop error: {msg}"),
            Self::Window(msg) => write!(f, "window error: {msg}"),
            Self::Surface(msg) => write!(f, "softbuffer surface error: {msg}"),
            Self::Reload(msg) => write!(f, "hot reload error: {msg}"),
            Self::Layout(msg) => write!(f, "layout engine error: {msg}"),
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

impl From<Frame0Miss> for DesktopHostError {
    fn from(value: Frame0Miss) -> Self {
        Self::Frame0(value)
    }
}

impl From<PersistenceError> for DesktopHostError {
    fn from(value: PersistenceError) -> Self {
        Self::Persistence(value)
    }
}

impl From<std::io::Error> for DesktopHostError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

/// Pluggable layout engine selected by `--layout <taffy|yoga>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LayoutEngineChoice {
    #[default]
    Taffy,
    Yoga,
}

impl LayoutEngineChoice {
    pub fn parse(raw: &str) -> Result<Self, String> {
        let s = raw.trim();
        if s.eq_ignore_ascii_case("taffy") {
            Ok(Self::Taffy)
        } else if s.eq_ignore_ascii_case("yoga") {
            Ok(Self::Yoga)
        } else {
            Err(format!("invalid --layout '{s}' (expected 'taffy' or 'yoga')"))
        }
    }

    pub fn build_layout_engine(self) -> Result<DynLayoutEngine, DesktopHostError> {
        match self {
            Self::Taffy => Ok(Box::new(TaffyStoreLayout::new())),
            Self::Yoga => {
                #[cfg(feature = "yoga")]
                {
                    nve_layout_yoga::YogaLayoutEngine::new()
                        .map(|engine| Box::new(engine) as DynLayoutEngine)
                        .map_err(|err| DesktopHostError::Layout(err.to_string()))
                }
                #[cfg(not(feature = "yoga"))]
                {
                    Err(DesktopHostError::Layout(
                        "layout engine 'yoga' requires building nve-skia-shell with `--features yoga`"
                            .to_string(),
                    ))
                }
            }
        }
    }
}

/// Configuration for the live desktop preview window and optional Frame-0 `frame0.nve2` cache.
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
    pub layout_engine: LayoutEngineChoice,
    pub vsync: bool,
    pub frame0_path: Option<PathBuf>,
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
            surface_id: SurfaceId::ROOT,
            show_hud: true,
            preferred_backend: GpuBackendKind::platform_preferred(),
            layout_engine: LayoutEngineChoice::Taffy,
            vsync: true,
            frame0_path: None,
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
    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool;
    /// Forwards one binary `NVP1` hot-patch frame to the app's `NveDevApi::apply_hot_patch`.
    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool;
    fn reload(
        &mut self,
        new_dylib_path: &Path,
        surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError>;

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

    /// Dispatches a canonical `NveHostEventBatch` (`NVHB` v1) drained from `CompletionSink`
    /// into the mounted app's `NveHostApi::dispatch_host_events`.
    fn dispatch_host_batch(&mut self, _batch: &[u8]) -> bool {
        false
    }

    /// Returns this app slot's active host owner tag (non-zero for bound [`HotAppSlot`]s) so
    /// [`DesktopWindowHost`] drains only this slot's async `Response` records.
    fn host_owner_tag(&self) -> u64 {
        0
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

    /// Returns `true` when this app delivers frames exclusively via [`Self::take_queued_wire_frame`]
    /// rather than synchronous [`Self::render`] calls (e.g. [`PairedRemoteApp`]).
    fn uses_queued_wire_frames(&self) -> bool {
        false
    }

    /// Takes the next queued `(is_keyframe, wire_bytes)` frame in delivery order for paired/remote
    /// apps so incremental `CommandDelta` batches are never overwritten before rendering.
    fn take_queued_wire_frame(&mut self) -> Option<(bool, Vec<u8>)> {
        None
    }

    /// Invoked after the most recently dequeued wire frame applied cleanly into `StoreWireReplica`.
    fn on_queued_wire_frame_applied(&mut self, _is_keyframe: bool) {}

    /// Invoked when the most recently dequeued wire frame failed to decode/apply, allowing paired
    /// remote apps to enter `waiting_for_keyframe` and request a rate-limited `SceneKeyframe`.
    fn on_queued_wire_frame_error(&mut self) {}

    /// Optional live status banner for remote/paired sessions (shown in HUD & window title).
    fn remote_status_banner(&self) -> Option<String> {
        None
    }
}

impl DesktopApp for PairReplicaClient {
    fn generation(&self) -> u64 {
        self.frames_received().max(1)
    }

    fn render(&mut self, _frame_id: FrameId) -> &[u8] {
        &[]
    }

    fn uses_queued_wire_frames(&self) -> bool {
        true
    }

    fn take_queued_wire_frame(&mut self) -> Option<(bool, Vec<u8>)> {
        PairReplicaClient::take_queued_wire_frame(self)
    }

    fn on_queued_wire_frame_applied(&mut self, is_keyframe: bool) {
        PairReplicaClient::on_queued_wire_frame_applied(self, is_keyframe);
    }

    fn on_queued_wire_frame_error(&mut self) {
        PairReplicaClient::on_queued_wire_frame_error(self);
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        self.send_events(events)
    }

    fn apply_hot_patch(&mut self, _patch_nvp1: &[u8]) -> bool {
        false
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Err(DesktopHostError::Reload(
            "paired remote session does not support local dylib reload".into(),
        ))
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.set_wake_callback(Some(wake));
    }

    fn has_pending_remote_frame(&self) -> bool {
        self.has_pending_frame()
    }

    fn remote_status_banner(&self) -> Option<String> {
        Some(PairReplicaClient::remote_status_banner(self))
    }
}

/// Generic [`DesktopApp`] adapter hosting any [`nve_engine::RuntimeApp`] (such as
/// `nve_dioxus::DioxusRuntimeApp`) inside [`DesktopSession`] / [`DesktopWindowHost`].
#[cfg_attr(not(test), allow(dead_code))]
pub struct RuntimeAppDesktopBridge<A: RuntimeApp> {
    // Declared first so `app` drops before `wake` clears.
    app: A,
    wake: Arc<WakeBridge>,
    encoder: WireEncoder,
    changes_scratch: nve_store::ChangeSet,
    wire_buf: Vec<u8>,
    generation: u64,
}

#[cfg_attr(not(test), allow(dead_code))]
impl<A: RuntimeApp> RuntimeAppDesktopBridge<A> {
    pub fn new(mut app: A) -> Self {
        let wake = Arc::new(WakeBridge::new());
        if app.install_wake_bridge(Arc::clone(&wake)) {
            wake.trigger();
        }
        Self {
            app,
            wake,
            encoder: WireEncoder::default(),
            changes_scratch: nve_store::ChangeSet::default(),
            wire_buf: Vec::new(),
            generation: 1,
        }
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn app_mut(&mut self) -> &mut A {
        &mut self.app
    }
}

impl<A: RuntimeApp> Drop for RuntimeAppDesktopBridge<A> {
    fn drop(&mut self) {
        self.app.bind_host(None);
        self.wake.clear_sink();
    }
}

impl<A: RuntimeApp> DesktopApp for RuntimeAppDesktopBridge<A> {
    fn generation(&self) -> u64 {
        self.generation
    }

    fn render(&mut self, frame_id: FrameId) -> &[u8] {
        self.wire_buf.clear();
        let direct_ready = match self.app.render_frame(frame_id) {
            AppFrame::Clean => return &[],
            AppFrame::Wire(bytes) => {
                self.wire_buf.extend_from_slice(bytes);
                false
            }
            AppFrame::Direct { changes } => {
                self.changes_scratch.copy_from(changes);
                true
            }
        };
        if direct_ready && let Some(store) = self.app.direct_store() {
            self.encoder
                .encode(store, &self.changes_scratch, &mut self.wire_buf);
        }
        &self.wire_buf
    }

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        if events.is_empty() {
            return false;
        }
        self.app.dispatch_events(events)
    }

    fn apply_hot_patch(&mut self, patch_nvp1: &[u8]) -> bool {
        let Ok(msg) = HotPatchMsg::from_bytes(patch_nvp1) else {
            return false;
        };
        if self.app.apply_hot_patch(&msg) == Ok(true) {
            self.generation = self.generation.saturating_add(1);
            true
        } else {
            false
        }
    }

    fn reload(
        &mut self,
        _new_dylib_path: &Path,
        _surface_id: SurfaceId,
    ) -> Result<u64, DesktopHostError> {
        Err(DesktopHostError::Reload(
            "in-process RuntimeApp does not support dylib reload".into(),
        ))
    }

    fn install_wake_callback(&mut self, wake: Arc<dyn Fn() + Send + Sync>) {
        self.wake.set_sink(wake);
    }

    fn poll_async_work(&mut self) -> bool {
        let _ = self.wake.take_pending();
        self.app.poll_async_work()
    }

    fn dispatch_host_batch(&mut self, batch: &[u8]) -> bool {
        self.app.dispatch_host_events(batch)
    }
}

#[cfg_attr(not(test), allow(dead_code))]
impl<A: RuntimeApp> DesktopWindowHost<RuntimeAppDesktopBridge<A>> {
    /// Creates a [`DesktopWindowHost`] directly hosting any [`nve_engine::RuntimeApp`]
    /// (such as `nve_dioxus::DioxusRuntimeApp`) with zero `cdylib` boundary.
    pub fn new_runtime_app(
        app: A,
        options: DesktopPreviewOptions,
    ) -> Result<Self, DesktopHostError> {
        let scale = if options.scale_factor.is_finite() && options.scale_factor > 0.0 {
            options.scale_factor
        } else {
            1.0
        };
        let initial_viewport = nve_engine::Viewport {
            width: options.width,
            height: options.height,
            scale_factor: scale,
        };
        let mut session = session::DesktopSession::new_with_layout(
            RuntimeAppDesktopBridge::new(app),
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
            options.layout_engine,
        )?;
        let _ = session.notify_viewport_resize();
        session.render_and_rasterize()?;
        Ok(Self::from_primary_session(options, session))
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

/// Headlessly renders one frame of `dylib_path` (with optional Frame-0 `frame0.nve2` pre-warm
/// before load and atomic refresh on exit) and writes the result as PNG or `P6` PPM.
pub fn render_headless_screenshot(
    dylib_path: PathBuf,
    options: DesktopPreviewOptions,
    screenshot_path: PathBuf,
) -> Result<(), DesktopHostError> {
    let mut host = DesktopWindowHost::new(dylib_path, options)?;
    host.session().write_screenshot(&screenshot_path)?;
    host.save_frame0_if_configured();
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
