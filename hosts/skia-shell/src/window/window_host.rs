//! Live `winit` desktop preview window host with hardware GPU swapchains (`skia-metal`,
//! `skia-vulkan`, `skia-d3d`), `softbuffer` fallback, IME cursor positioning, and snapshot save.

use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use nve_backend_skia::gpu::{
    FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig, GpuSurfaceError,
    GpuSwapchain,
};
use nve_engine::{RenderBackend, Viewport};
use softbuffer::{Context, Surface};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::{Window, WindowId};

use super::hot_reload_client::HotReloadClient;
use super::pair_client::{PairConnectConfig, PairedRemoteApp};
use super::session::DesktopSession;
use super::{
    replay_snapshot_into_backend, DesktopApp, DesktopHostError, DesktopPreviewOptions, HotAppSlot,
};

#[derive(Debug, Clone, Copy)]
enum HostWakeEvent {
    HotReload,
    RemoteFrame,
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
/// `skia-vulkan`, `skia-d3d`) with automatic graceful fallback to `softbuffer`, plus Frame-0
/// `MmapSnapshot` instant boot, atomic persistence on exit, and thin-client `NVD1` pairing.
pub struct DesktopWindowHost<A: DesktopApp = HotAppSlot> {
    options: DesktopPreviewOptions,
    session: DesktopSession<A>,
    window: Option<Arc<Window>>,
    presenter: Option<WindowPresenter>,
    hot_reload: Option<HotReloadClient>,
    proxy: Option<EventLoopProxy<HostWakeEvent>>,
    last_telemetry: Option<FrameTelemetry>,
    modifiers: ModifiersState,
    snapshot_saved: bool,
}

impl DesktopWindowHost<HotAppSlot> {
    pub fn new(
        dylib_path: PathBuf,
        options: DesktopPreviewOptions,
    ) -> Result<Self, DesktopHostError> {
        let scale = if options.scale_factor.is_finite() && options.scale_factor > 0.0 {
            options.scale_factor
        } else {
            1.0
        };
        let initial_viewport = Viewport {
            width: options.width,
            height: options.height,
            scale_factor: scale,
        };

        // Instant-boot Frame-0 replay BEFORE dlopen of the app cdylib when a snapshot exists.
        let prewarmed = if let Some(snap_path) = options.snapshot_path.as_deref() {
            if snap_path.exists() {
                match replay_snapshot_into_backend(
                    snap_path,
                    options.schema_version,
                    options.surface_id,
                    initial_viewport,
                    options.clear_color,
                ) {
                    Ok(prewarmed) => {
                        eprintln!(
                            "[nve-skia-shell] Instant-boot Frame-0 mmap restore from '{}' in {} us",
                            snap_path.display(),
                            prewarmed.restore_us
                        );
                        Some(prewarmed)
                    }
                    Err(err) => {
                        eprintln!(
                            "[nve-skia-shell] Warning: ignoring snapshot '{}' ({err}); cold-booting app",
                            snap_path.display()
                        );
                        None
                    }
                }
            } else {
                None
            }
        } else {
            None
        };

        let mut session = if let Some(prewarmed) = prewarmed {
            let prior_state = if prewarmed.state_bytes.is_empty() {
                None
            } else {
                Some(prewarmed.state_bytes.as_slice())
            };
            let app = HotAppSlot::load_with_state(&dylib_path, options.surface_id, prior_state)?;
            DesktopSession::from_parts(
                app,
                prewarmed.backend,
                prewarmed.pixel_buffer,
                prewarmed.frame0_bytes,
                options.surface_id,
                initial_viewport,
                options.clear_color,
                options.show_hud,
            )
        } else {
            let app = HotAppSlot::load(&dylib_path, options.surface_id)?;
            DesktopSession::new(
                app,
                options.surface_id,
                initial_viewport,
                options.clear_color,
                options.show_hud,
            )
        };
        let _ = session.notify_viewport_resize();
        session.render_and_rasterize()?;

        Ok(Self {
            options,
            session,
            window: None,
            presenter: None,
            hot_reload: None,
            proxy: None,
            last_telemetry: None,
            modifiers: ModifiersState::empty(),
            snapshot_saved: false,
        })
    }
}

impl DesktopWindowHost<PairedRemoteApp> {
    /// Connects to a remote `nve pair` server over `NVT1`/`NVD1` (UDP, TCP, or WebSocket) with no
    /// local `cdylib`, sends the initial viewport `Layout` event, waits for Frame 1, and
    /// rasterizes it into the session pixel buffer.
    pub fn new_paired(
        pair_config: PairConnectConfig,
        options: DesktopPreviewOptions,
    ) -> Result<Self, DesktopHostError> {
        let scale = if options.scale_factor.is_finite() && options.scale_factor > 0.0 {
            options.scale_factor
        } else {
            1.0
        };
        let initial_viewport = Viewport {
            width: options.width,
            height: options.height,
            scale_factor: scale,
        };

        let initial_timeout = pair_config.initial_frame_timeout;
        let app = PairedRemoteApp::connect(pair_config)?;
        let mut session = DesktopSession::new(
            app,
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
        );
        let _ = session.notify_viewport_resize();
        session.app_mut().wait_for_frame(initial_timeout)?;
        if let Some(banner) = session.app().remote_status_banner() {
            session.set_status_banner(Some(banner));
        }
        session.render_and_rasterize()?;

        Ok(Self {
            options,
            session,
            window: None,
            presenter: None,
            hot_reload: None,
            proxy: None,
            last_telemetry: None,
            modifiers: ModifiersState::empty(),
            snapshot_saved: false,
        })
    }
}

impl<A: DesktopApp> DesktopWindowHost<A> {
    pub fn session(&self) -> &DesktopSession<A> {
        &self.session
    }

    pub fn session_mut(&mut self) -> &mut DesktopSession<A> {
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

    /// Atomically saves the current Frame-0 `CommandBuffer` and live app state if `--snapshot`
    /// was configured. Safe to call multiple times (only persists once).
    pub fn save_snapshot_if_configured(&mut self) {
        if self.snapshot_saved {
            return;
        }
        let Some(snap_path) = self.options.snapshot_path.clone() else {
            return;
        };
        self.snapshot_saved = true;
        match self
            .session
            .save_snapshot(&snap_path, self.options.schema_version)
        {
            Ok(_) => {
                eprintln!(
                    "[nve-skia-shell] Saved Frame-0 + state snapshot to '{}' (schema v{})",
                    snap_path.display(),
                    self.options.schema_version
                );
            }
            Err(err) => {
                eprintln!(
                    "[nve-skia-shell] Warning: failed to save snapshot to '{}': {err}",
                    snap_path.display()
                );
            }
        }
    }

    /// Runs the native `winit` event loop until the user closes the window.
    pub fn run(mut self) -> Result<(), DesktopHostError> {
        let event_loop = EventLoop::<HostWakeEvent>::with_user_event()
            .build()
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()))?;
        let proxy = event_loop.create_proxy();
        self.proxy = Some(proxy.clone());

        if let Some(ws_url) = self.options.ws_url.clone() {
            let wake_proxy = proxy.clone();
            self.hot_reload = Some(HotReloadClient::spawn(ws_url, move || {
                let _ = wake_proxy.send_event(HostWakeEvent::HotReload);
            }));
        }

        let remote_wake_proxy = proxy;
        self.session
            .app_mut()
            .install_wake_callback(Arc::new(move || {
                let _ = remote_wake_proxy.send_event(HostWakeEvent::RemoteFrame);
            }));

        let res = event_loop
            .run_app(&mut self)
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()));
        self.save_snapshot_if_configured();
        res
    }

    fn sync_ime_state(&self) {
        let Some(window) = &self.window else {
            return;
        };
        let focused = self
            .session
            .focused_node()
            .filter(|&id| self.session.backend().is_text_input(id));
        window.set_ime_allowed(focused.is_some());
        if let Some(node_id) = focused {
            if let Some(rect) = self.session.backend().layout_of(node_id) {
                window.set_ime_cursor_area(
                    LogicalPosition::new(rect.x as f64, rect.y as f64),
                    LogicalSize::new(rect.width as f64, rect.height as f64),
                );
            }
        }
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
                        eprintln!("[nve-skia-shell] hardware swapchain present error: {err}");
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
                    eprintln!("[nve-skia-shell] {err}");
                    needs_redraw = true;
                }
            }
        }
        self.sync_window_title();
        self.sync_ime_state();
        if needs_redraw {
            if let Some(window) = &self.window {
                window.request_redraw();
            }
        }
    }

    fn process_pending_remote_frames(&mut self) {
        if !self.session.app().has_pending_remote_frame() {
            return;
        }
        if let Some(banner) = self.session.app().remote_status_banner() {
            self.session.set_status_banner(Some(banner));
        }
        if let Err(err) = self.session.render_and_rasterize() {
            eprintln!("[nve-skia-shell] paired remote frame error: {err}");
        }
        self.sync_window_title();
        self.sync_ime_state();
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl<A: DesktopApp> Drop for DesktopWindowHost<A> {
    fn drop(&mut self) {
        self.save_snapshot_if_configured();
    }
}

impl<A: DesktopApp> ApplicationHandler<HostWakeEvent> for DesktopWindowHost<A> {
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
                eprintln!("[nve-skia-shell] failed to create window: {err}");
                event_loop.exit();
                return;
            }
        };

        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        let phys_w = size.width.max(1);
        let phys_h = size.height.max(1);

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
            match GpuSwapchain::attach_window(window.as_ref(), gpu_cfg) {
                Ok(sw) if sw.presents_directly() => Some(WindowPresenter::Hardware(sw)),
                Ok(sw) => {
                    fallback_reason = sw.fallback_reason().map(str::to_owned);
                    None
                }
                Err(err) => {
                    eprintln!(
                        "[nve-skia-shell] hardware GPU swapchain unavailable ({err}); falling back to softbuffer"
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
                        eprintln!("[nve-skia-shell] failed to create softbuffer context: {err}");
                        event_loop.exit();
                        return;
                    }
                };
                let surface = match Surface::new(&context, window.clone()) {
                    Ok(s) => s,
                    Err(err) => {
                        eprintln!("[nve-skia-shell] failed to create softbuffer surface: {err}");
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
            eprintln!("[nve-skia-shell] initial resize/render failed: {err}");
        }

        self.window = Some(window.clone());
        self.presenter = Some(presenter);
        self.sync_window_title();
        self.sync_ime_state();
        window.request_redraw();
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, event: HostWakeEvent) {
        match event {
            HostWakeEvent::HotReload => self.process_pending_hot_patches(),
            HostWakeEvent::RemoteFrame => self.process_pending_remote_frames(),
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
                self.save_snapshot_if_configured();
                self.hot_reload.take();
                self.presenter.take();
                event_loop.exit();
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 {
                    return;
                }
                let scale = self
                    .window
                    .as_ref()
                    .map(|w| w.scale_factor() as f32)
                    .unwrap_or(self.session.viewport().scale_factor);
                if let Err(err) = self.session.resize(new_size.width, new_size.height, scale) {
                    eprintln!("[nve-skia-shell] resize error: {err}");
                }
                if let Some(WindowPresenter::Hardware(sw)) = self.presenter.as_mut() {
                    if let Err(err) = sw.resize(new_size.width, new_size.height, scale) {
                        eprintln!("[nve-skia-shell] hardware swapchain resize error: {err}");
                    }
                }
                self.sync_ime_state();
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
                    eprintln!("[nve-skia-shell] scale change error: {err}");
                }
                if let Some(WindowPresenter::Hardware(sw)) = self.presenter.as_mut() {
                    if let Err(err) = sw.resize(size.width, size.height, scale_factor as f32) {
                        eprintln!("[nve-skia-shell] hardware swapchain scale resize error: {err}");
                    }
                }
                self.sync_ime_state();
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
                        self.sync_ime_state();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    Ok(false) => {}
                    Err(err) => eprintln!("[nve-skia-shell] click dispatch error: {err}"),
                }
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                self.modifiers = new_mods.state();
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scale = self.session.viewport().scale_factor.max(0.001);
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(lx, ly) => (-lx * 40.0, -ly * 40.0),
                    MouseScrollDelta::PixelDelta(pos) => {
                        (-(pos.x as f32) / scale, -(pos.y as f32) / scale)
                    }
                };
                let (x, y) = self.session.cursor_logical();
                match self.session.handle_scroll_logical(x, y, dx, dy) {
                    Ok(true) => {
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    Ok(false) => {}
                    Err(err) => eprintln!("[nve-skia-shell] scroll dispatch error: {err}"),
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                match self.session.handle_keyboard_input(
                    &event.logical_key,
                    event.text.as_deref(),
                    self.modifiers,
                ) {
                    Ok(true) => {
                        self.sync_ime_state();
                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    Ok(false) => {}
                    Err(err) => eprintln!("[nve-skia-shell] keyboard dispatch error: {err}"),
                }
            }
            WindowEvent::Ime(ime) => match self.session.handle_ime(&ime) {
                Ok(true) => {
                    self.sync_ime_state();
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                }
                Ok(false) => {}
                Err(err) => eprintln!("[nve-skia-shell] IME dispatch error: {err}"),
            },
            WindowEvent::RedrawRequested => {
                self.process_pending_hot_patches();
                self.process_pending_remote_frames();
                self.present_pixels();
            }
            _ => {}
        }
    }
}
