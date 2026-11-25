//! Cross-platform desktop preview window (`winit` + `softbuffer`) with live hit-testing and
//! 2-tier WebSocket hot reload (`HotPatchMsg` from `ws://127.0.0.1:8089`).

use std::fmt;
use std::io::ErrorKind;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crossbeam_channel::{unbounded, Receiver};
use nve_abi::{
    encode_events, ColorRgba, EventPayload, FrameId, NativeEvent, NativeEventKind, SurfaceId,
};
use nve_engine::loader::{HotAppInstance, LoaderError};
use nve_engine::{BackendError, HotPatchMsg, RenderBackend, Viewport};
use softbuffer::{Context, Surface};
use tungstenite::stream::MaybeTlsStream;
use tungstenite::Message;
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::window::{Window, WindowId};

use crate::gpu::{
    FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig, GpuSurfaceError,
    GpuSwapchain,
};
use crate::raster::{draw_hud_banner, PixelBuffer};
#[cfg(feature = "skia")]
use crate::{GpuSurface, GpuSurfaceBackend, SkiaCanvasRenderer};
use crate::SkiaBackend;

pub const DEFAULT_WS_URL: &str = "ws://127.0.0.1:8089";

#[derive(Debug)]
pub enum DesktopHostError {
    Loader(LoaderError),
    Backend(BackendError),
    EventLoop(String),
    Window(String),
    Surface(String),
}

impl fmt::Display for DesktopHostError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Loader(err) => write!(f, "failed to load NVE app cdylib: {err}"),
            Self::Backend(err) => write!(f, "render backend error: {err}"),
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

/// Configuration for the live desktop preview window.
#[derive(Debug, Clone)]
pub struct DesktopPreviewOptions {
    pub title: String,
    pub width: f32,
    pub height: f32,
    pub clear_color: ColorRgba,
    pub ws_url: Option<String>,
    pub surface_id: SurfaceId,
    pub show_hud: bool,
    pub preferred_backend: GpuBackendKind,
    pub vsync: bool,
}

impl Default for DesktopPreviewOptions {
    fn default() -> Self {
        Self {
            title: "Native View Engine — Desktop Preview".to_string(),
            width: 420.0,
            height: 780.0,
            clear_color: ColorRgba::rgb(15, 23, 42),
            ws_url: Some(DEFAULT_WS_URL.to_string()),
            surface_id: 1,
            show_hud: true,
            preferred_backend: GpuBackendKind::platform_preferred(),
            vsync: true,
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
        let instance = HotAppInstance::load(dylib_path, surface_id, 1, None)?;
        let generation = instance.generation();
        Ok(Self {
            instance: Some(instance),
            generation,
            last_saved_state: Vec::new(),
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
        if let Some(mut inst) = self.instance.take() {
            self.last_saved_state = inst.snapshot_state();
            let next_gen = inst.generation() + 1;
            match inst.reload(new_dylib_path, surface_id) {
                Ok(reloaded) => {
                    self.generation = reloaded.generation();
                    self.instance = Some(reloaded);
                    Ok(self.generation)
                }
                Err(err) => {
                    self.generation = next_gen;
                    Err(DesktopHostError::Loader(err))
                }
            }
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
}

/// Window-independent controller driving rendering, hit-test -> event dispatch, and 2-tier hot
/// reload for any [`DesktopApp`].
pub struct DesktopSession<A: DesktopApp> {
    app: A,
    backend: SkiaBackend,
    surface_id: SurfaceId,
    next_frame_id: FrameId,
    viewport: Viewport,
    pixel_buffer: PixelBuffer,
    clear_color: ColorRgba,
    cursor_logical: (f32, f32),
    status_banner: Option<String>,
    show_hud: bool,
    last_frame_bytes: Vec<u8>,
    frame_counter: u64,
    last_telemetry: Option<FrameTelemetry>,
    #[cfg(feature = "skia")]
    skia_renderer: SkiaCanvasRenderer,
    #[cfg(feature = "skia")]
    gpu_surface: Option<GpuSurface>,
}

impl<A: DesktopApp> DesktopSession<A> {
    pub fn new(
        app: A,
        surface_id: SurfaceId,
        viewport: Viewport,
        clear_color: ColorRgba,
        show_hud: bool,
    ) -> Self {
        let mut backend = SkiaBackend::new();
        backend.set_viewport(surface_id, viewport);
        let phys_w = ((viewport.width * viewport.scale_factor).round() as u32).max(1);
        let phys_h = ((viewport.height * viewport.scale_factor).round() as u32).max(1);
        #[cfg(feature = "skia")]
        let gpu_surface = GpuSurface::new(GpuSurfaceConfig {
            width: phys_w,
            height: phys_h,
            scale_factor: viewport.scale_factor,
            ..Default::default()
        })
        .ok();
        Self {
            app,
            backend,
            surface_id,
            next_frame_id: 1,
            viewport,
            pixel_buffer: PixelBuffer::new(phys_w, phys_h),
            clear_color,
            cursor_logical: (0.0, 0.0),
            status_banner: None,
            show_hud,
            last_frame_bytes: Vec::new(),
            frame_counter: 0,
            last_telemetry: None,
            #[cfg(feature = "skia")]
            skia_renderer: SkiaCanvasRenderer::new(),
            #[cfg(feature = "skia")]
            gpu_surface,
        }
    }

    pub fn app(&self) -> &A {
        &self.app
    }

    pub fn backend(&self) -> &SkiaBackend {
        &self.backend
    }

    pub fn pixel_buffer(&self) -> &PixelBuffer {
        &self.pixel_buffer
    }

    pub fn viewport(&self) -> Viewport {
        self.viewport
    }

    pub fn status_banner(&self) -> Option<&str> {
        self.status_banner.as_deref()
    }

    pub fn active_backend(&self) -> GpuBackendKind {
        #[cfg(feature = "skia")]
        if let Some(surface) = &self.gpu_surface {
            return surface.kind();
        }
        GpuBackendKind::Softbuffer
    }

    pub fn last_frame_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry.as_ref()
    }

    pub fn clear_color(&self) -> ColorRgba {
        self.clear_color
    }

    pub fn hud_banner_if_enabled(&self) -> Option<&str> {
        if self.show_hud {
            self.status_banner.as_deref()
        } else {
            None
        }
    }

    /// Presents the current retained `DisplayList` directly through a hardware or offscreen
    /// [`GpuSwapchain`] without touching the CPU software `PixelBuffer`.
    pub fn present_to_swapchain(
        &mut self,
        swapchain: &mut GpuSwapchain,
    ) -> Result<FrameTelemetry, GpuSurfaceError> {
        let hud = if self.show_hud {
            self.status_banner.as_deref()
        } else {
            None
        };
        let telemetry = swapchain.execute_frame(
            self.backend.display_list(),
            self.viewport.scale_factor,
            self.clear_color,
            hud,
        )?;
        self.last_telemetry = Some(telemetry.clone());
        Ok(telemetry)
    }

    pub fn set_cursor_logical(&mut self, x: f32, y: f32) {
        self.cursor_logical = (x, y);
    }

    pub fn cursor_logical(&self) -> (f32, f32) {
        self.cursor_logical
    }

    /// Updates physical surface dimensions and HiDPI scale factor, then re-layouts and re-blits.
    pub fn resize(
        &mut self,
        physical_width: u32,
        physical_height: u32,
        scale_factor: f32,
    ) -> Result<(), DesktopHostError> {
        let pw = physical_width.max(1);
        let ph = physical_height.max(1);
        let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
            scale_factor
        } else {
            1.0
        };
        self.viewport = Viewport {
            width: pw as f32 / scale,
            height: ph as f32 / scale,
            scale_factor: scale,
        };
        self.backend.set_viewport(self.surface_id, self.viewport);
        self.pixel_buffer.resize(pw, ph);

        // Re-submit last command buffer if we have one (recomputes Flexbox for new viewport),
        // otherwise pull a fresh frame from the app.
        if !self.last_frame_bytes.is_empty() {
            let bytes = self.last_frame_bytes.clone();
            self.backend.submit_frame(&bytes)?;
            self.rasterize_current();
        } else {
            self.render_and_rasterize()?;
        }
        Ok(())
    }

    /// Renders a frame from the app into `SkiaBackend` and rasterizes into `pixel_buffer`.
    pub fn render_and_rasterize(&mut self) -> Result<&PixelBuffer, DesktopHostError> {
        let frame_id = self.next_frame_id;
        self.next_frame_id += 1;
        let bytes = self.app.render(frame_id);
        if !bytes.is_empty() {
            self.backend.submit_frame(&bytes)?;
            self.last_frame_bytes = bytes;
        }
        self.rasterize_current();
        Ok(&self.pixel_buffer)
    }

    fn rasterize_current(&mut self) {
        let t0 = Instant::now();
        #[cfg(feature = "skia")]
        let (painted_via_skia, active_kind, fallback_reason) =
            if let Some(surface) = self.gpu_surface.as_mut() {
                let pw = self.pixel_buffer.width();
                let ph = self.pixel_buffer.height();
                let ok = surface.resize(pw, ph).is_ok()
                    && self
                        .backend
                        .paint_to_surface(&self.skia_renderer, surface, self.clear_color)
                        .is_ok()
                    && surface.copy_to_pixel_buffer(&mut self.pixel_buffer).is_ok();
                (
                    ok,
                    surface.kind(),
                    surface.fallback_reason().map(str::to_owned),
                )
            } else {
                (false, GpuBackendKind::Softbuffer, None)
            };
        #[cfg(not(feature = "skia"))]
        let (painted_via_skia, active_kind, fallback_reason): (bool, GpuBackendKind, Option<String>) =
            (false, GpuBackendKind::Softbuffer, None);

        if !painted_via_skia {
            self.backend
                .rasterize(&mut self.pixel_buffer, self.clear_color);
        }
        if self.show_hud {
            if let Some(banner) = &self.status_banner {
                draw_hud_banner(&mut self.pixel_buffer, self.viewport.scale_factor, banner);
            }
        }
        let elapsed_us = t0.elapsed().as_micros() as u64;
        self.frame_counter += 1;
        self.last_telemetry = Some(FrameTelemetry {
            backend: if painted_via_skia {
                active_kind
            } else {
                GpuBackendKind::Softbuffer
            },
            frame_index: self.frame_counter,
            width: self.pixel_buffer.width(),
            height: self.pixel_buffer.height(),
            scale_factor: self.viewport.scale_factor,
            acquire_us: 0,
            record_us: elapsed_us,
            submit_us: 0,
            present_us: 0,
            total_us: elapsed_us,
            swapchain_recreations: 0,
            fallback_reason,
        });
    }

    /// Translates a logical mouse click into hit-tested `NativeEventKind::Press` ->
    /// `encode_events` -> `app.dispatch_events` -> immediate re-render.
    pub fn handle_click_logical(&mut self, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        self.cursor_logical = (x, y);
        let Some(node_id) = self.backend.hit_test(x, y) else {
            return Ok(false);
        };
        let timestamp_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0);
        let event = NativeEvent {
            surface_id: self.surface_id,
            node_id,
            timestamp_us,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x, y },
        };
        let encoded = encode_events(&[event]);
        let handled = self.app.dispatch_events(&encoded);
        if handled {
            self.render_and_rasterize()?;
        }
        Ok(handled)
    }

    /// Processes a 2-tier hot-reload message from `nve dev` and returns `true` when the frame
    /// buffer was updated and should be presented.
    pub fn handle_hot_patch(&mut self, msg: HotPatchMsg) -> Result<bool, DesktopHostError> {
        match &msg {
            HotPatchMsg::TemplatePatch { generation, .. } => {
                let raw = msg.to_json();
                let changed = self.app.apply_hot_patch(&raw);
                self.status_banner = Some(format!("Tier-1 patch gen #{generation}"));
                if changed {
                    self.render_and_rasterize()?;
                } else {
                    self.rasterize_current();
                }
                Ok(true)
            }
            HotPatchMsg::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => {
                let t0 = Instant::now();
                let resolved = resolve_dylib_path(dylib_path);
                match self.app.reload(&resolved, self.surface_id) {
                    Ok(new_gen) => {
                        // Reset retained layout tree so the reloaded app mounts cleanly without
                        // stale node IDs from the previous generation.
                        self.backend = SkiaBackend::new();
                        self.backend.set_viewport(self.surface_id, self.viewport);
                        self.last_frame_bytes.clear();
                        let swap_ms = t0.elapsed().as_millis();
                        self.status_banner = Some(format!(
                            "Hot reload gen #{new_gen} (build {build_ms}ms, swap {swap_ms}ms)"
                        ));
                        self.render_and_rasterize()?;
                        Ok(true)
                    }
                    Err(err) => {
                        self.status_banner =
                            Some(format!("Reload gen #{generation} failed: {err}"));
                        self.rasterize_current();
                        Err(err)
                    }
                }
            }
            HotPatchMsg::DevStatus {
                message,
                latency_ms,
            } => {
                self.status_banner = if *latency_ms > 0 {
                    Some(format!("{message} ({latency_ms}ms)"))
                } else {
                    Some(message.clone())
                };
                self.rasterize_current();
                Ok(true)
            }
        }
    }
}

fn resolve_dylib_path(raw_path: &str) -> PathBuf {
    let p = PathBuf::from(raw_path);
    if p.is_absolute() || p.exists() {
        return p;
    }
    if let Ok(cwd) = std::env::current_dir() {
        let candidate = cwd.join(&p);
        if candidate.exists() {
            return candidate;
        }
    }
    p
}

/// Background WebSocket client subscribing to `nve dev` (`HotPatchMsg` JSON frames) and waking
/// the UI event loop whenever a patch arrives.
pub struct HotReloadClient {
    rx: Receiver<HotPatchMsg>,
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl HotReloadClient {
    pub fn spawn<F>(ws_url: String, on_wake: F) -> Self
    where
        F: Fn() + Send + 'static,
    {
        let (tx, rx) = unbounded();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_flag = Arc::clone(&stop);

        let worker = thread::Builder::new()
            .name("nve-hot-reload-ws".to_string())
            .spawn(move || {
                let mut backoff = Duration::from_millis(150);
                let max_backoff = Duration::from_secs(2);

                while !stop_flag.load(Ordering::Relaxed) {
                    match tungstenite::connect(ws_url.as_str()) {
                        Ok((mut ws, _response)) => {
                            backoff = Duration::from_millis(150);
                            if let MaybeTlsStream::Plain(stream) = ws.get_ref() {
                                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                            }
                            while !stop_flag.load(Ordering::Relaxed) {
                                match ws.read() {
                                    Ok(Message::Text(text)) => {
                                        if let Ok(msg) = HotPatchMsg::from_json(text.as_str()) {
                                            if tx.send(msg).is_err() {
                                                return;
                                            }
                                            on_wake();
                                        }
                                    }
                                    Ok(Message::Binary(bytes)) => {
                                        if let Ok(text) = std::str::from_utf8(&bytes) {
                                            if let Ok(msg) = HotPatchMsg::from_json(text) {
                                                if tx.send(msg).is_err() {
                                                    return;
                                                }
                                                on_wake();
                                            }
                                        }
                                    }
                                    Ok(Message::Ping(payload)) => {
                                        let _ = ws.send(Message::Pong(payload));
                                    }
                                    Ok(Message::Close(_)) => break,
                                    Ok(_) => {}
                                    Err(tungstenite::Error::Io(ref err))
                                        if err.kind() == ErrorKind::WouldBlock
                                            || err.kind() == ErrorKind::TimedOut =>
                                    {
                                        continue;
                                    }
                                    Err(_) => break,
                                }
                            }
                        }
                        Err(_) => {
                            // Server not up yet or restarting: sleep in short slices so Drop stays responsive.
                            let deadline = Instant::now() + backoff;
                            while Instant::now() < deadline && !stop_flag.load(Ordering::Relaxed) {
                                thread::sleep(Duration::from_millis(50));
                            }
                            backoff = (backoff * 2).min(max_backoff);
                        }
                    }
                }
            })
            .ok();

        Self { rx, stop, worker }
    }

    pub fn try_drain(&self) -> Vec<HotPatchMsg> {
        let mut out = Vec::new();
        while let Ok(msg) = self.rx.try_recv() {
            out.push(msg);
        }
        out
    }

    pub fn receiver(&self) -> &Receiver<HotPatchMsg> {
        &self.rx
    }
}

impl Drop for HotReloadClient {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.worker.take() {
            let _ = handle.join();
        }
    }
}

#[derive(Debug, Clone, Copy)]
enum HostWakeEvent {
    HotReload,
}

enum WindowPresenter {
    /// Direct hardware GPU swapchain (`CAMetalLayer` / `VkSwapchainKHR` / `IDXGISwapChain3`).
    Hardware(GpuSwapchain),
    /// Software `softbuffer` presenter (zero-C++ default, or automatic fallback when a hardware
    /// GPU device/swapchain cannot be initialized).
    Softbuffer {
        _context: Context<Arc<Window>>,
        surface: Surface<Arc<Window>, Arc<Window>>,
        fallback_reason: Option<String>,
    },
}

/// Live `winit` desktop preview host supporting direct hardware GPU swapchains (`skia-metal`,
/// `skia-vulkan`, `skia-d3d`) with automatic graceful fallback to `softbuffer`.
pub struct DesktopWindowHost {
    options: DesktopPreviewOptions,
    session: DesktopSession<HotAppSlot>,
    window: Option<Arc<Window>>,
    presenter: Option<WindowPresenter>,
    hot_reload: Option<HotReloadClient>,
    proxy: Option<EventLoopProxy<HostWakeEvent>>,
    last_telemetry: Option<FrameTelemetry>,
}

impl DesktopWindowHost {
    pub fn new(
        dylib_path: PathBuf,
        options: DesktopPreviewOptions,
    ) -> Result<Self, DesktopHostError> {
        let app = HotAppSlot::load(&dylib_path, options.surface_id)?;
        let initial_viewport = Viewport {
            width: options.width,
            height: options.height,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            app,
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
        );
        session.render_and_rasterize()?;

        Ok(Self {
            options,
            session,
            window: None,
            presenter: None,
            hot_reload: None,
            proxy: None,
            last_telemetry: None,
        })
    }

    pub fn session(&self) -> &DesktopSession<HotAppSlot> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DesktopSession<HotAppSlot> {
        &mut self.session
    }

    pub fn active_backend(&self) -> GpuBackendKind {
        match &self.presenter {
            Some(WindowPresenter::Hardware(sw)) => sw.kind(),
            Some(WindowPresenter::Softbuffer { .. }) => self.session.active_backend(),
            None => self.session.active_backend(),
        }
    }

    pub fn last_frame_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry
            .as_ref()
            .or_else(|| self.session.last_frame_telemetry())
    }

    /// Runs the native `winit` event loop until the user closes the window.
    pub fn run(mut self) -> Result<(), DesktopHostError> {
        let event_loop = EventLoop::<HostWakeEvent>::with_user_event()
            .build()
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()))?;
        let proxy = event_loop.create_proxy();
        self.proxy = Some(proxy.clone());

        if let Some(ws_url) = self.options.ws_url.clone() {
            let wake_proxy = proxy;
            self.hot_reload = Some(HotReloadClient::spawn(ws_url, move || {
                let _ = wake_proxy.send_event(HostWakeEvent::HotReload);
            }));
        }

        event_loop
            .run_app(&mut self)
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()))
    }

    fn present_pixels(&mut self) {
        let Some(presenter) = self.presenter.as_mut() else {
            return;
        };
        match presenter {
            WindowPresenter::Hardware(swapchain) => {
                let res = self.session.present_to_swapchain(swapchain);
                match res {
                    Ok(telemetry) => {
                        self.last_telemetry = Some(telemetry);
                    }
                    Err(GpuSurfaceError::SwapchainOutOfDate(_)) => {
                        let pw = self.session.pixel_buffer().width();
                        let ph = self.session.pixel_buffer().height();
                        let scale = self.session.viewport().scale_factor;
                        if swapchain.resize(pw, ph, scale).is_ok() {
                            if let Ok(telemetry) = self.session.present_to_swapchain(swapchain) {
                                self.last_telemetry = Some(telemetry);
                            }
                        }
                    }
                    Err(err) => {
                        eprintln!("[nve-desktop] hardware swapchain present error: {err}");
                    }
                }
            }
            WindowPresenter::Softbuffer {
                surface,
                fallback_reason,
                ..
            } => {
                let buf = self.session.pixel_buffer();
                let (Some(w), Some(h)) =
                    (NonZeroU32::new(buf.width()), NonZeroU32::new(buf.height()))
                else {
                    return;
                };
                if surface.resize(w, h).is_err() {
                    return;
                }
                let t_present = Instant::now();
                if let Ok(mut frame) = surface.buffer_mut() {
                    let src = buf.pixels();
                    let len = frame.len().min(src.len());
                    frame[..len].copy_from_slice(&src[..len]);
                    let _ = frame.present();
                    let present_us = t_present.elapsed().as_micros() as u64;
                    if let Some(mut tel) = self.session.last_frame_telemetry().cloned() {
                        tel.present_us = present_us;
                        tel.total_us = tel.total_us.saturating_add(present_us);
                        if tel.fallback_reason.is_none() {
                            tel.fallback_reason = fallback_reason.clone();
                        }
                        self.last_telemetry = Some(tel);
                    }
                }
            }
        }
    }

    fn sync_window_title(&self) {
        if let Some(window) = &self.window {
            let gen_num = self.session.app().generation();
            let backend_label = self.active_backend().as_str();
            let title = match self.session.status_banner() {
                Some(banner) => format!("{} [{backend_label}] — {banner}", self.options.title),
                None => format!("{} [{backend_label}] — gen #{gen_num}", self.options.title),
            };
            window.set_title(&title);
        }
    }

    fn process_pending_hot_patches(&mut self) {
        let msgs = self
            .hot_reload
            .as_ref()
            .map(HotReloadClient::try_drain)
            .unwrap_or_default();
        if msgs.is_empty() {
            return;
        }
        let mut needs_redraw = false;
        for msg in msgs {
            match self.session.handle_hot_patch(msg) {
                Ok(updated) => needs_redraw |= updated,
                Err(err) => {
                    eprintln!("[nve-desktop] {err}");
                    needs_redraw = true;
                }
            }
        }
        self.sync_window_title();
        if needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler<HostWakeEvent> for DesktopWindowHost {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes()
            .with_title(self.options.title.clone())
            .with_inner_size(LogicalSize::new(
                self.options.width as f64,
                self.options.height as f64,
            ));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(err) => {
                eprintln!("[nve-desktop] failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let phys_w = size.width.max(1);
        let phys_h = size.height.max(1);

        // Prefer a direct hardware GPU swapchain whenever compiled for this platform; fall back
        // gracefully to `softbuffer` if no GPU feature is enabled or adapter init fails.
        let mut fallback_reason = None;
        let presenter = if self.options.preferred_backend.is_hardware()
            && GpuBackendKind::selection_priority(self.options.preferred_backend, true)
                .iter()
                .any(|k| k.is_hardware() && k.is_compiled())
        {
            let gpu_cfg = GpuSurfaceConfig {
                width: phys_w,
                height: phys_h,
                scale_factor: scale,
                preferred_backend: self.options.preferred_backend,
                allow_cpu_fallback: false,
                vsync: self.options.vsync,
                ..Default::default()
            };
            match GpuSwapchain::attach_window(window.clone(), gpu_cfg) {
                Ok(sw) if sw.presents_directly() => Some(WindowPresenter::Hardware(sw)),
                Ok(sw) => {
                    fallback_reason = sw.fallback_reason().map(str::to_owned);
                    None
                }
                Err(err) => {
                    eprintln!(
                        "[nve-desktop] hardware GPU swapchain unavailable ({err}); falling back to softbuffer"
                    );
                    fallback_reason = Some(err.to_string());
                    None
                }
            }
        } else {
            None
        };

        let presenter = match presenter {
            Some(hw) => hw,
            None => {
                let context = match Context::new(window.clone()) {
                    Ok(c) => c,
                    Err(err) => {
                        eprintln!("[nve-desktop] failed to create softbuffer context: {err}");
                        event_loop.exit();
                        return;
                    }
                };
                let surface = match Surface::new(&context, window.clone()) {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("[nve-desktop] failed to create softbuffer surface: {err}");
                        event_loop.exit();
                        return;
                    }
                };
                WindowPresenter::Softbuffer {
                    _context: context,
                    surface,
                    fallback_reason,
                }
            }
        };

        if let Err(err) = self.session.resize(phys_w, phys_h, scale) {
            eprintln!("[nve-desktop] initial resize/render failed: {err}");
        }

        self.window = Some(window.clone());
        self.presenter = Some(presenter);
        self.sync_window_title();
        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostWakeEvent) {
        match event {
            HostWakeEvent::HotReload => self.process_pending_hot_patches(),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                self.hot_reload.take();
                self.presenter.take();
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    // Window minimized: skip swapchain recreation until restored.
                    return;
                }
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor() as f32)
                    .unwrap_or(self.session.viewport().scale_factor);
                if let Err(err) = self.session.resize(new_size.width, new_size.height, scale) {
                    eprintln!("[nve-desktop] resize error: {err}");
                }
                if let Some(WindowPresenter::Hardware(sw)) = self.presenter.as_mut() {
                    if let Err(err) = sw.resize(new_size.width, new_size.height, scale) {
                        eprintln!("[nve-desktop] hardware swapchain resize error: {err}");
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let size = self
                    .window
                    .as_ref()
                    .map(|w| w.inner_size())
                    .unwrap_or_else(|| {
                        winit::dpi::PhysicalSize::new(
                            self.session.pixel_buffer().width(),
                            self.session.pixel_buffer().height(),
                        )
                    });
                if size.width == 0 || size.height == 0 {
                    return;
                }
                if let Err(err) = self
                    .session
                    .resize(size.width, size.height, scale_factor as f32)
                {
                    eprintln!("[nve-desktop] scale change error: {err}");
                }
                if let Some(WindowPresenter::Hardware(sw)) = self.presenter.as_mut() {
                    if let Err(err) = sw.resize(size.width, size.height, scale_factor as f32) {
                        eprintln!("[nve-desktop] hardware swapchain scale resize error: {err}");
                    }
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = self.session.viewport().scale_factor.max(0.001);
                self.session
                    .set_cursor_logical(position.x as f32 / scale, position.y as f32 / scale);
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let (x, y) = self.session.cursor_logical();
                match self.session.handle_click_logical(x, y) {
                    Ok(true) => {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    Ok(false) => {}
                    Err(err) => eprintln!("[nve-desktop] click dispatch error: {err}"),
                }
            }
            WindowEvent::RedrawRequested => {
                self.process_pending_hot_patches();
                self.present_pixels();
            }
            _ => {}
        }
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

/// Opens a live cross-platform desktop preview window with custom window and HUD options.
pub fn run_desktop_preview_with_options(
    dylib_path: PathBuf,
    options: DesktopPreviewOptions,
) -> Result<(), DesktopHostError> {
    let host = DesktopWindowHost::new(dylib_path, options)?;
    host.run()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;
    use std::sync::atomic::AtomicUsize;

    use nve_abi::{
        decode_events, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, EventMask,
        FlexDirection, LayoutProps, StyleProps, ROOT_NODE_ID,
    };

    /// Pure-Rust in-memory mock NVE app implementing `DesktopApp` to test the full
    /// hit-test -> `encode_events` -> `dispatch_events` -> re-render -> pixel buffer and
    /// 2-tier `HotPatchMsg` lifecycle without requiring a GUI display server.
    struct MockCounterApp {
        generation: u64,
        count: i32,
        label: String,
        button_color: ColorRgba,
        reloaded_paths: Vec<PathBuf>,
    }

    impl MockCounterApp {
        fn new() -> Self {
            Self {
                generation: 1,
                count: 0,
                label: "Counter".to_string(),
                button_color: ColorRgba::rgb(37, 99, 235),
                reloaded_paths: Vec::new(),
            }
        }
    }

    impl DesktopApp for MockCounterApp {
        fn generation(&self) -> u64 {
            self.generation
        }

        fn render(&mut self, frame_id: FrameId) -> Vec<u8> {
            let mut b = CommandBufferBuilder::new(1, frame_id);
            b.create_node(2, ComponentKind::View, None);
            b.set_layout(
                2,
                &LayoutProps {
                    width: Dimension::Percent(100.0),
                    height: Dimension::Percent(100.0),
                    padding: EdgeInsets::all(20.0),
                    gap: 12.0,
                    flex_direction: FlexDirection::Column,
                    ..Default::default()
                },
            );
            b.set_style(
                2,
                &StyleProps {
                    background: ColorRgba::rgb(15, 23, 42),
                    ..Default::default()
                },
            );

            b.create_node(3, ComponentKind::Text, None);
            b.set_style(
                3,
                &StyleProps {
                    text_color: ColorRgba::WHITE,
                    font_size: 20.0,
                    ..Default::default()
                },
            );
            b.set_text(3, &format!("{}: {}", self.label, self.count));

            b.create_node(4, ComponentKind::Pressable, None);
            b.set_layout(
                4,
                &LayoutProps {
                    width: Dimension::Points(160.0),
                    height: Dimension::Points(48.0),
                    ..Default::default()
                },
            );
            b.set_style(
                4,
                &StyleProps {
                    background: self.button_color,
                    border_radius: 10.0,
                    ..Default::default()
                },
            );
            b.set_event_mask(4, EventMask::PRESS);

            b.set_children(2, &[3, 4]);
            b.set_children(ROOT_NODE_ID, &[2]);
            b.commit(true);
            b.into_bytes()
        }

        fn dispatch_events(&mut self, encoded_events: &[u8]) -> bool {
            let events = decode_events(encoded_events).expect("valid encoded events");
            let mut handled = false;
            for ev in events {
                if ev.node_id == 4 && ev.kind == NativeEventKind::Press {
                    self.count += 1;
                    // Visually change button color on click so pixel assertions verify re-render.
                    self.button_color = ColorRgba::rgb(16, 185, 129);
                    handled = true;
                }
            }
            handled
        }

        fn apply_hot_patch(&mut self, patch_json: &str) -> bool {
            match HotPatchMsg::from_json(patch_json) {
                Ok(HotPatchMsg::TemplatePatch {
                    text_updates,
                    attr_updates,
                    ..
                }) => {
                    for (_, text) in text_updates {
                        self.label = text;
                    }
                    for (k, v) in attr_updates {
                        if k == "button_color" && v == "orange" {
                            self.button_color = ColorRgba::rgb(249, 115, 22);
                        }
                    }
                    true
                }
                _ => false,
            }
        }

        fn reload(
            &mut self,
            new_dylib_path: &Path,
            _surface_id: SurfaceId,
        ) -> Result<u64, DesktopHostError> {
            self.reloaded_paths.push(new_dylib_path.to_path_buf());
            // State (`self.count`) is preserved across reload; generation increments.
            self.generation += 1;
            self.button_color = ColorRgba::rgb(168, 85, 247);
            Ok(self.generation)
        }
    }

    #[test]
    fn hit_test_click_dispatches_press_and_rerenders_pixels() {
        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            MockCounterApp::new(),
            1,
            viewport,
            ColorRgba::BLACK,
            false,
        );
        session.render_and_rasterize().unwrap();

        let btn = session.backend().layout_of(4).expect("button layout");
        let cx = btn.x + btn.width * 0.5;
        let cy = btn.y + btn.height * 0.5;

        // Initial button color is blue (#2563eb).
        assert_eq!(
            session.pixel_buffer().get_rgb(cx as u32, cy as u32),
            Some((37, 99, 235))
        );

        // Clicking outside the button is a miss and does not increment count.
        assert!(!session.handle_click_logical(280.0, 180.0).unwrap());
        assert_eq!(session.app().count, 0);

        // Clicking inside the button hits Node#4, encodes Press, dispatches, and re-renders green!
        assert!(session.handle_click_logical(cx, cy).unwrap());
        assert_eq!(session.app().count, 1);
        assert_eq!(
            session.pixel_buffer().get_rgb(cx as u32, cy as u32),
            Some((16, 185, 129))
        );
    }

    #[test]
    fn handles_tier1_template_patch_tier2_dylib_reload_and_dev_status() {
        let viewport = Viewport {
            width: 300.0,
            height: 200.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            MockCounterApp::new(),
            1,
            viewport,
            ColorRgba::BLACK,
            true,
        );
        session.render_and_rasterize().unwrap();

        // Click once so count == 1 before hot reload.
        let btn = session.backend().layout_of(4).unwrap();
        let cx = btn.x + btn.width * 0.5;
        let cy = btn.y + btn.height * 0.5;
        assert!(session.handle_click_logical(cx, cy).unwrap());
        assert_eq!(session.app().count, 1);

        // Tier 1: TemplatePatch updates button color to orange without bumping generation.
        let updated = session
            .handle_hot_patch(HotPatchMsg::TemplatePatch {
                generation: 2,
                template_id: "counter".into(),
                attr_updates: vec![("button_color".into(), "orange".into())],
                text_updates: vec![(0, "Live".into())],
            })
            .unwrap();
        assert!(updated);
        assert_eq!(session.app().generation(), 1);
        assert_eq!(session.app().label, "Live");
        assert_eq!(
            session.pixel_buffer().get_rgb(cx as u32, cy as u32),
            Some((249, 115, 22))
        );

        // Tier 2: DylibReload reloads while preserving count == 1 and re-renders purple.
        let updated = session
            .handle_hot_patch(HotPatchMsg::DylibReload {
                generation: 2,
                dylib_path: "target/debug/libnve_example_counter.dylib".into(),
                build_ms: 185,
            })
            .unwrap();
        assert!(updated);
        assert_eq!(session.app().generation(), 2);
        assert_eq!(session.app().count, 1);
        assert_eq!(session.app().reloaded_paths.len(), 1);
        assert_eq!(
            session.pixel_buffer().get_rgb(cx as u32, cy as u32),
            Some((168, 85, 247))
        );
        assert!(session
            .status_banner()
            .unwrap()
            .contains("Hot reload gen #2"));

        // DevStatus updates HUD banner.
        let updated = session
            .handle_hot_patch(HotPatchMsg::DevStatus {
                message: "Connected".into(),
                latency_ms: 12,
            })
            .unwrap();
        assert!(updated);
        assert_eq!(session.status_banner(), Some("Connected (12ms)"));
    }

    #[test]
    fn websocket_hot_reload_client_receives_messages_and_invokes_wake() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind ephemeral ws port");
        let addr = listener.local_addr().unwrap();

        let server = thread::spawn(move || {
            let (stream, _) = listener.accept().expect("accept client");
            let mut ws = tungstenite::accept(stream).expect("ws handshake");
            let m1 = HotPatchMsg::DevStatus {
                message: "Ready".into(),
                latency_ms: 4,
            };
            let m2 = HotPatchMsg::DylibReload {
                generation: 3,
                dylib_path: "target/debug/libdemo.dylib".into(),
                build_ms: 210,
            };
            ws.send(Message::Text(m1.to_json().into())).unwrap();
            ws.send(Message::Text(m2.to_json().into())).unwrap();
            let _ = ws.close(None);
        });

        let wake_count = Arc::new(AtomicUsize::new(0));
        let wake_counter = Arc::clone(&wake_count);
        let client = HotReloadClient::spawn(format!("ws://{addr}"), move || {
            wake_counter.fetch_add(1, Ordering::SeqCst);
        });

        let first = client
            .receiver()
            .recv_timeout(Duration::from_secs(3))
            .expect("first WS message");
        let second = client
            .receiver()
            .recv_timeout(Duration::from_secs(3))
            .expect("second WS message");

        assert_eq!(
            first,
            HotPatchMsg::DevStatus {
                message: "Ready".into(),
                latency_ms: 4,
            }
        );
        assert_eq!(
            second,
            HotPatchMsg::DylibReload {
                generation: 3,
                dylib_path: "target/debug/libdemo.dylib".into(),
                build_ms: 210,
            }
        );
        assert!(wake_count.load(Ordering::SeqCst) >= 2);

        drop(client);
        server.join().unwrap();
    }

    #[test]
    fn resize_updates_hidpi_viewport_and_scales_pixel_buffer() {
        let viewport = Viewport {
            width: 200.0,
            height: 150.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            MockCounterApp::new(),
            1,
            viewport,
            ColorRgba::BLACK,
            false,
        );
        session.render_and_rasterize().unwrap();
        assert_eq!(session.pixel_buffer().width(), 200);
        assert_eq!(session.pixel_buffer().height(), 150);

        // Switch to 2x Retina (600x400 physical == 300x200 logical).
        session.resize(600, 400, 2.0).unwrap();
        assert_eq!(session.viewport().width, 300.0);
        assert_eq!(session.viewport().height, 200.0);
        assert_eq!(session.viewport().scale_factor, 2.0);
        assert_eq!(session.pixel_buffer().width(), 600);
        assert_eq!(session.pixel_buffer().height(), 400);

        // Button center in physical pixels should be at (2x logical center) and colored blue.
        let btn = session.backend().layout_of(4).unwrap();
        let px = ((btn.x + btn.width * 0.5) * 2.0) as u32;
        let py = ((btn.y + btn.height * 0.5) * 2.0) as u32;
        assert_eq!(session.pixel_buffer().get_rgb(px, py), Some((37, 99, 235)));
    }

    #[test]
    fn desktop_session_presents_to_swapchain_and_records_frame_telemetry() {
        let viewport = Viewport {
            width: 240.0,
            height: 160.0,
            scale_factor: 1.0,
        };
        let mut session = DesktopSession::new(
            MockCounterApp::new(),
            1,
            viewport,
            ColorRgba::BLACK,
            true,
        );
        session.render_and_rasterize().unwrap();
        let init_tel = session
            .last_frame_telemetry()
            .expect("render_and_rasterize should record telemetry");
        assert_eq!(init_tel.backend, session.active_backend());
        assert_eq!(init_tel.frame_index, 1);

        let mut sw = GpuSwapchain::new_offscreen(GpuSurfaceConfig {
            width: 240,
            height: 160,
            scale_factor: 1.0,
            preferred_backend: GpuBackendKind::platform_preferred(),
            allow_cpu_fallback: true,
            ..Default::default()
        })
        .unwrap();
        let tel = session.present_to_swapchain(&mut sw).unwrap();
        assert_eq!(tel.backend, sw.kind());
        assert_eq!(tel.width, 240);
        assert_eq!(tel.height, 160);
        assert_eq!(tel.frame_index, 1);

        // Resize both session and swapchain to 2x Retina and verify presented button pixel.
        session.resize(480, 320, 2.0).unwrap();
        sw.resize(480, 320, 2.0).unwrap();
        let tel2 = session.present_to_swapchain(&mut sw).unwrap();
        assert_eq!(tel2.frame_index, 2);
        assert_eq!(tel2.swapchain_recreations, 1);

        let mut readback = PixelBuffer::new(1, 1);
        sw.copy_to_pixel_buffer(&mut readback).unwrap();
        let btn = session.backend().layout_of(4).unwrap();
        let px = ((btn.x + btn.width * 0.5) * 2.0) as u32;
        let py = ((btn.y + btn.height * 0.5) * 2.0) as u32;
        assert_eq!(readback.get_rgb(px, py), Some((37, 99, 235)));
    }

    #[test]
    fn hot_app_slot_reports_loader_error_for_missing_dylib() {
        let err = HotAppSlot::load(Path::new("/nonexistent/path/libmissing_nve.dylib"), 1)
            .err()
            .expect("expected loader error for nonexistent dylib");
        assert!(matches!(err, DesktopHostError::Loader(LoaderError::Io(_))));
    }

    #[test]
    #[ignore = "renders 2x HiDPI visual verification PPMs from compiled example cdylibs"]
    fn render_visual_verification_ppms_at_2x_hidpi() {
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let workspace_root = manifest_dir.parent().unwrap().parent().unwrap();
        let dylib_ext = if cfg!(target_os = "macos") {
            "dylib"
        } else if cfg!(target_os = "windows") {
            "dll"
        } else {
            "so"
        };

        let targets = [
            (
                "libnve_studio_workspace",
                960.0f32,
                640.0f32,
                ColorRgba::rgb(11, 17, 32),
                "/tmp/nve_skia_cosmic_studio.ppm",
            ),
            (
                "libnve_example_counter",
                640.0f32,
                420.0f32,
                ColorRgba::rgb(15, 23, 42),
                "/tmp/nve_skia_cosmic_counter.ppm",
            ),
        ];

        for (lib_stem, w, h, clear, out_path) in targets {
            let dylib = workspace_root.join(format!("target/debug/{lib_stem}.{dylib_ext}"));
            let app = HotAppSlot::load(&dylib, 1)
                .unwrap_or_else(|e| panic!("failed to load {dylib:?}: {e}"));
            let viewport = Viewport {
                width: w,
                height: h,
                scale_factor: 2.0,
            };
            let mut session = DesktopSession::new(app, 1, viewport, clear, false);
            session
                .render_and_rasterize()
                .expect("render and rasterize at 2x HiDPI");
            let buf = session.pixel_buffer();
            assert_eq!(buf.width(), (w * 2.0) as u32);
            assert_eq!(buf.height(), (h * 2.0) as u32);

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
            std::fs::write(out_path, out).expect("write ppm");
        }
    }
}
