//! Cross-platform desktop preview window (`winit` + `softbuffer` / hardware GPU swapchains)
//! with live hit-testing, interactive `TextInput` focus/IME/caret editing, 2-tier WebSocket
//! hot reload (`HotPatchMsg` from `ws://127.0.0.1:8089`), and optional Frame-0 `frame0.nve2`
//! keyframe pre-warm & atomic refresh.

pub mod a11y;
pub mod clipboard;
#[cfg(test)]
pub mod dioxus_app;
pub mod hot_reload_client;
mod ime;
pub mod input;
mod keyboard;
pub mod multi_window;
pub mod pair_client;
mod pointer;
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
mod pointer_focus_tests;
#[cfg(test)]
mod scroll_keydown_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod text_maturity_tests;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use nve_abi::{
    encode_single_push_into, ColorRgba, FrameId, SurfaceId, ValidatedEvents, SERVICE_SYSTEM_ENV,
};
use nve_backend_skia::encode_rgb_png;
use nve_backend_skia::gpu::GpuBackendKind;
use nve_backend_skia::raster::PixelBuffer;
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::{BackendError, Frame0Miss, PersistenceError};
use nve_layout::{DynLayoutEngine, TaffyStoreLayout};
use nve_system_env::{ThemeSnapshot, EVENT_THEME_CHANGED};

use crate::host_services::{SkiaHostEvent, SkiaHostServices, SkiaSlotHostBinding};

#[cfg(test)]
pub use dioxus_app::DioxusDesktopApp;
#[cfg(test)]
pub use hot_reload_client::HotReloadClient;
pub use multi_window::HostWakeEvent;
pub use pair_client::PairConnectConfig;
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

const THEME_PUSH_BATCH_LEN: usize = 56;

#[inline]
fn encode_theme_push_batch(theme: ThemeSnapshot, buf: &mut [u8; THEME_PUSH_BATCH_LEN]) -> &[u8] {
    let raw = theme.to_repr_c_bytes();
    encode_single_push_into(buf, SERVICE_SYSTEM_ENV, EVENT_THEME_CHANGED, &raw).unwrap_or(&[])
}

/// Owns a live [`HotAppInstance`] and preserves the last state snapshot and bound
/// [`SkiaHostServices`] across Tier-2 reloads (including recovery if an intermediate reload fails).
pub struct HotAppSlot {
    // Field order is load-bearing: `instance` MUST drop before `slot_binding` and `host_services`
    // so the loaded cdylib never observes a dangling `host_ctx` pointer during `destroy`.
    instance: Option<HotAppInstance>,
    generation: u64,
    last_saved_state: Vec<u8>,
    slot_binding: Option<Arc<SkiaSlotHostBinding>>,
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
            slot_binding: None,
            host_services: None,
        })
    }
}

impl Drop for HotAppSlot {
    fn drop(&mut self) {
        if let Some(inst) = self.instance.as_mut() {
            let _ = inst.bind_host(None);
        }
        if let (Some(svc), Some(binding)) =
            (self.host_services.as_ref(), self.slot_binding.as_ref())
        {
            let _ = svc.discard_pending_for_owner(binding.owner_tag());
        }
        self.instance.take();
        self.slot_binding.take();
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

    fn dispatch_events(&mut self, events: ValidatedEvents<'_>) -> bool {
        match self.instance.as_mut() {
            Some(inst) => inst.dispatch_events(events),
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
            let next_binding = self
                .host_services
                .as_ref()
                .map(SkiaHostServices::new_slot_binding);
            let next_cbs = next_binding.as_ref().map(SkiaSlotHostBinding::callbacks);
            // Transactional reload: keep Gen 1's owner_tag & pending requests untouched until
            // Gen 2 loads and binds ONCE inside `reload_with_host` (zero double-bind, zero
            // cross-generation cancellation on failed reload).
            let outcome = match inst.reload_with_host(new_dylib_path, next_cbs) {
                Ok(outcome) => outcome,
                Err(err) => {
                    if let (Some(svc), Some(nb)) =
                        (self.host_services.as_ref(), next_binding.as_ref())
                    {
                        let _ = svc.discard_pending_for_owner(nb.owner_tag());
                    }
                    return Err(err.into());
                }
            };
            self.generation = outcome.generation;
            let prev_binding = std::mem::replace(&mut self.slot_binding, next_binding);
            if let Some(svc) = self.host_services.as_ref() {
                if let Some(old) = prev_binding.as_ref() {
                    let _ = svc.discard_pending_for_owner(old.owner_tag());
                }
                if inst.has_host_interface() {
                    let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                    let _ = inst.dispatch_host_events(encode_theme_push_batch(
                        svc.current_theme(),
                        &mut batch,
                    ));
                }
            }
            Ok(self.generation)
        } else {
            let next_gen = self.generation + 1;
            let prior = (!self.last_saved_state.is_empty())
                .then_some(self.last_saved_state.as_slice());
            let mut loaded = HotAppInstance::load(new_dylib_path, surface_id, next_gen, prior)?;
            let next_binding = self
                .host_services
                .as_ref()
                .map(SkiaHostServices::new_slot_binding);
            if let (Some(svc), Some(nb)) = (self.host_services.as_ref(), next_binding.as_ref())
                && loaded.bind_host(Some(nb.callbacks()))
            {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                let _ = loaded.dispatch_host_events(encode_theme_push_batch(
                    svc.current_theme(),
                    &mut batch,
                ));
            }
            let old_binding = std::mem::replace(&mut self.slot_binding, next_binding);
            if let (Some(svc), Some(old)) = (self.host_services.as_ref(), old_binding.as_ref()) {
                let _ = svc.discard_pending_for_owner(old.owner_tag());
            }
            self.generation = loaded.generation();
            self.instance = Some(loaded);
            Ok(self.generation)
        }
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
                let next_binding = svc.new_slot_binding();
                let cb = next_binding.callbacks();
                let bound = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(Some(cb)));
                // Store the new binding & Arc AFTER `bind_host` updates the dylib's pointer so any
                // previous binding stays alive during the swap (zero dangling-pointer window), and
                // discard ONLY this slot's previous owner_tag (never cancelling sibling windows).
                let prev_binding = self.slot_binding.replace(next_binding);
                let prev_svc = self.host_services.replace(svc);
                if let (Some(old_svc), Some(old_binding)) =
                    (prev_svc.as_ref(), prev_binding.as_ref())
                {
                    let _ = old_svc.discard_pending_for_owner(old_binding.owner_tag());
                }
                if bound && let Some(svc_ref) = self.host_services.as_ref() {
                    let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                    let encoded = encode_theme_push_batch(svc_ref.current_theme(), &mut batch);
                    if let Some(inst) = self.instance.as_mut() {
                        let _ = inst.dispatch_host_events(encoded);
                    }
                }
                bound
            }
            None => {
                let had = self
                    .instance
                    .as_mut()
                    .is_some_and(|inst| inst.bind_host(None));
                let prev_binding = self.slot_binding.take();
                let prev_svc = self.host_services.take();
                if let (Some(old_svc), Some(old_binding)) =
                    (prev_svc.as_ref(), prev_binding.as_ref())
                {
                    let _ = old_svc.discard_pending_for_owner(old_binding.owner_tag());
                }
                had
            }
        }
    }

    fn dispatch_host_batch(&mut self, batch: &[u8]) -> bool {
        self.instance
            .as_mut()
            .is_some_and(|inst| inst.dispatch_host_events(batch))
    }

    fn host_owner_tag(&self) -> u64 {
        self.slot_binding
            .as_ref()
            .map_or(0, |binding| binding.owner_tag())
    }

    fn dispatch_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        let Some(inst) = self.instance.as_mut() else {
            return false;
        };
        match event {
            SkiaHostEvent::ThemeChanged(theme) => {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                inst.dispatch_host_events(encode_theme_push_batch(*theme, &mut batch))
            }
            #[cfg(test)]
            SkiaHostEvent::SystemEnv { theme, .. } => {
                let mut batch = [0u8; THEME_PUSH_BATCH_LEN];
                inst.dispatch_host_events(encode_theme_push_batch(*theme, &mut batch))
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
