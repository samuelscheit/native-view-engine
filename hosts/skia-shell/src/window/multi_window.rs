//! Multi-window helper primitives for `hosts/skia-shell`:
//! - [`HostWakeEvent`] (including `WindowCommand`)
//! - [`SurfaceWindow`] owning per-window `winit::Window`, presenter, `DesktopSession`, and input/IME state
//! - [`HeadlessMultiWindowHost`] sharing the exact [`WindowManager`] routing, close-veto, modal
//!   focus blocking, and cascading child cleanup logic for headless unit tests.

use std::collections::{BTreeMap, HashMap};
use std::hash::Hash;
use std::num::NonZeroU32;
use std::sync::Arc;
use std::time::Instant;

use nve_abi::SurfaceId;
use nve_backend_skia::gpu::{
    FrameTelemetry, GpuBackendKind, GpuFramePresenter, GpuSurfaceConfig, GpuSurfaceError,
    GpuSwapchain,
};
use nve_engine::RenderBackend;
use nve_interop::{
    SurfacePresentation, WindowCommand, WindowDescriptor, WindowError, WindowFlags, WindowHandleId,
    WindowManager, WindowRect, WindowState,
};
use softbuffer::{Context, Surface};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::keyboard::ModifiersState;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowLevel};

use super::session::DesktopSession;
use super::{DesktopApp, DesktopHostError};

/// Cross-thread event loop wake signals for [`super::DesktopWindowHost`].
#[derive(Debug, Clone)]
pub enum HostWakeEvent {
    HotReload,
    RemoteFrame,
    WindowCommand(WindowCommand),
}

pub(crate) type CloseVetoHandler = Box<dyn FnMut(WindowHandleId, SurfaceId) -> bool>;
pub(crate) type SecondaryAppFactory<A> =
    Box<dyn FnMut(SurfaceId, &WindowDescriptor) -> Result<A, DesktopHostError>>;

pub(crate) enum WindowPresenter {
    Hardware(GpuSwapchain),
    Softbuffer {
        _context: Context<Arc<Window>>,
        surface: Surface<Arc<Window>, Arc<Window>>,
        fallback_reason: Option<String>,
    },
}

/// Per-OS-window state managed by [`super::DesktopWindowHost`].
pub struct SurfaceWindow<A: DesktopApp> {
    pub(crate) window: Arc<Window>,
    pub(crate) presenter: WindowPresenter,
    pub(crate) session: DesktopSession<A>,
    pub(crate) surface_id: SurfaceId,
    pub(crate) handle_id: WindowHandleId,
    pub(crate) base_title: String,
    pub(crate) modifiers: ModifiersState,
    pub(crate) ime_allowed: bool,
    pub(crate) last_telemetry: Option<FrameTelemetry>,
}

impl<A: DesktopApp> SurfaceWindow<A> {
    pub fn window(&self) -> &Arc<Window> { &self.window }
    pub fn session(&self) -> &DesktopSession<A> { &self.session }
    pub fn session_mut(&mut self) -> &mut DesktopSession<A> { &mut self.session }
    pub fn surface_id(&self) -> SurfaceId { self.surface_id }
    pub fn handle_id(&self) -> WindowHandleId { self.handle_id }
    pub fn modifiers(&self) -> ModifiersState { self.modifiers }
    pub fn ime_allowed(&self) -> bool { self.ime_allowed }

    pub fn active_backend(&self) -> GpuBackendKind {
        match &self.presenter {
            WindowPresenter::Hardware(sw) => sw.kind(),
            WindowPresenter::Softbuffer { .. } => self.session.active_backend(),
        }
    }

    pub fn last_frame_telemetry(&self) -> Option<&FrameTelemetry> {
        self.last_telemetry
            .as_ref()
            .or_else(|| self.session.last_frame_telemetry())
    }

    pub(crate) fn sync_ime_state(&mut self) {
        let focused = self
            .session
            .focused_node()
            .filter(|&id| self.session.backend().is_text_input(id));
        self.ime_allowed = focused.is_some();
        self.window.set_ime_allowed(self.ime_allowed);
        if let Some(node_id) = focused
            && let Some(rect) = self.session.backend().layout_of(node_id)
        {
            self.window.set_ime_cursor_area(
                LogicalPosition::new(rect.x as f64, rect.y as f64),
                LogicalSize::new(rect.width as f64, rect.height as f64),
            );
        }
    }

    pub(crate) fn sync_title(&self) {
        let gen_num = self.session.app().generation();
        let backend_label = self.active_backend().as_str();
        let title = match self.session.status_banner() {
            Some(banner) => format!("{} [{backend_label}] — {banner}", self.base_title),
            None => format!("{} [{backend_label}] — gen #{gen_num}", self.base_title),
        };
        self.window.set_title(&title);
    }

    pub(crate) fn present_pixels(&mut self) {
        match &mut self.presenter {
            WindowPresenter::Hardware(swapchain) => {
                match self.session.present_to_swapchain(swapchain) {
                    Ok(telemetry) => self.last_telemetry = Some(telemetry),
                    Err(GpuSurfaceError::SwapchainOutOfDate(_)) => {
                        let pw = self.session.pixel_buffer().width();
                        let ph = self.session.pixel_buffer().height();
                        let scale = self.session.viewport().scale_factor;
                        if swapchain.resize(pw, ph, scale).is_ok()
                            && let Ok(telemetry) = self.session.present_to_swapchain(swapchain)
                        {
                            self.last_telemetry = Some(telemetry);
                        }
                    }
                    Err(err) => {
                        eprintln!("[nve-skia-shell] hardware swapchain present error: {err}");
                    }
                }
            }
            WindowPresenter::Softbuffer { surface, fallback_reason, .. } => {
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
}

pub(crate) fn build_window_attributes(desc: &WindowDescriptor) -> WindowAttributes {
    let mut attrs = Window::default_attributes()
        .with_title(desc.title.clone())
        .with_inner_size(LogicalSize::new(desc.bounds.width as f64, desc.bounds.height as f64))
        .with_resizable(desc.flags.contains(WindowFlags::RESIZABLE))
        .with_decorations(desc.flags.contains(WindowFlags::DECORATED))
        .with_transparent(desc.flags.contains(WindowFlags::TRANSPARENT));
    if desc.bounds.x != 0.0 || desc.bounds.y != 0.0 {
        attrs = attrs.with_position(LogicalPosition::new(desc.bounds.x as f64, desc.bounds.y as f64));
    }
    if let Some((min_w, min_h)) = desc.min_size {
        attrs = attrs.with_min_inner_size(LogicalSize::new(min_w as f64, min_h as f64));
    }
    if let Some((max_w, max_h)) = desc.max_size {
        attrs = attrs.with_max_inner_size(LogicalSize::new(max_w as f64, max_h as f64));
    }
    if desc.flags.contains(WindowFlags::ALWAYS_ON_TOP) {
        attrs = attrs.with_window_level(WindowLevel::AlwaysOnTop);
    }
    attrs
}

pub(crate) fn apply_window_state(window: &Window, state: WindowState) {
    match state {
        WindowState::Normal => {
            window.set_fullscreen(None);
            window.set_maximized(false);
            window.set_minimized(false);
        }
        WindowState::Minimized => {
            window.set_fullscreen(None);
            window.set_minimized(true);
        }
        WindowState::Maximized => {
            window.set_fullscreen(None);
            window.set_maximized(true);
        }
        WindowState::Fullscreen => {
            window.set_fullscreen(Some(Fullscreen::Borderless(None)));
        }
    }
}

pub(crate) fn create_presenter(
    window: &Arc<Window>,
    preferred_backend: GpuBackendKind,
    vsync: bool,
) -> Result<WindowPresenter, DesktopHostError> {
    let size = window.inner_size();
    let scale = window.scale_factor() as f32;
    let phys_w = size.width.max(1);
    let phys_h = size.height.max(1);

    let mut fallback_reason = None;
    if preferred_backend.is_hardware()
        && GpuBackendKind::selection_priority(preferred_backend, true)
            .iter()
            .any(|k| k.is_hardware() && k.is_compiled())
    {
        let gpu_cfg = GpuSurfaceConfig {
            width: phys_w,
            height: phys_h,
            scale_factor: scale,
            preferred_backend,
            allow_cpu_fallback: false,
            vsync,
            ..Default::default()
        };
        match GpuSwapchain::attach_window(window.as_ref(), gpu_cfg) {
            Ok(sw) if sw.presents_directly() => return Ok(WindowPresenter::Hardware(sw)),
            Ok(sw) => fallback_reason = sw.fallback_reason().map(str::to_owned),
            Err(err) => {
                eprintln!(
                    "[nve-skia-shell] hardware GPU swapchain unavailable ({err}); falling back to softbuffer"
                );
                fallback_reason = Some(err.to_string());
            }
        }
    }

    let context = Context::new(window.clone())
        .map_err(|e| DesktopHostError::Surface(format!("softbuffer context: {e}")))?;
    let surface = Surface::new(&context, window.clone())
        .map_err(|e| DesktopHostError::Surface(format!("softbuffer surface: {e}")))?;
    Ok(WindowPresenter::Softbuffer {
        _context: context,
        surface,
        fallback_reason,
    })
}

/// Outcome of handling a window close request or close command.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CloseOutcome {
    pub vetoed: bool,
    pub closed_handles: Vec<WindowHandleId>,
    pub should_exit_loop: bool,
}

/// Headless multi-window session host sharing the exact [`WindowManager`] lifecycle, per-window
/// session isolation, event routing, close veto, modal blocking, and cascading child cleanup
/// semantics as [`super::DesktopWindowHost`] without requiring a native display server.
pub struct HeadlessMultiWindowHost<K: Eq + Hash + Copy, A: DesktopApp> {
    windows: HashMap<K, HeadlessSurfaceWindow<A>>,
    surface_to_window: BTreeMap<SurfaceId, K>,
    window_manager: WindowManager,
    primary_handle: Option<WindowHandleId>,
    exit_on_primary_close: bool,
    close_veto_handler: Option<CloseVetoHandler>,
}

pub struct HeadlessSurfaceWindow<A: DesktopApp> {
    pub session: DesktopSession<A>,
    pub surface_id: SurfaceId,
    pub handle_id: WindowHandleId,
    pub modifiers: ModifiersState,
    pub ime_allowed: bool,
}

impl<K: Eq + Hash + Copy, A: DesktopApp> Default for HeadlessMultiWindowHost<K, A> {
    fn default() -> Self { Self::new() }
}

impl<K: Eq + Hash + Copy, A: DesktopApp> HeadlessMultiWindowHost<K, A> {
    pub fn new() -> Self {
        Self {
            windows: HashMap::new(),
            surface_to_window: BTreeMap::new(),
            window_manager: WindowManager::new(),
            primary_handle: None,
            exit_on_primary_close: true,
            close_veto_handler: None,
        }
    }

    pub fn set_exit_on_primary_close(&mut self, exit: bool) {
        self.exit_on_primary_close = exit;
    }

    pub fn set_close_veto_handler<F>(&mut self, handler: F)
    where
        F: FnMut(WindowHandleId, SurfaceId) -> bool + 'static,
    {
        self.close_veto_handler = Some(Box::new(handler));
    }

    pub fn open_window(
        &mut self,
        window_key: K,
        surface_id: SurfaceId,
        presentation: SurfacePresentation,
        mut session: DesktopSession<A>,
    ) -> Result<WindowHandleId, DesktopHostError> {
        let is_primary = presentation.is_primary();
        let handle_id = self
            .window_manager
            .open_window(surface_id, presentation)
            .map_err(|e| DesktopHostError::Window(e.to_string()))?;
        session.render_and_rasterize()?;
        let ime_allowed = session
            .focused_node()
            .is_some_and(|id| session.backend().is_text_input(id));
        if is_primary || self.primary_handle.is_none() {
            self.primary_handle = Some(handle_id);
        }
        self.surface_to_window.insert(surface_id, window_key);
        self.windows.insert(
            window_key,
            HeadlessSurfaceWindow {
                session,
                surface_id,
                handle_id,
                modifiers: ModifiersState::empty(),
                ime_allowed,
            },
        );
        Ok(handle_id)
    }

    pub fn route_click(&mut self, window_key: K, x: f32, y: f32) -> Result<bool, DesktopHostError> {
        let Some(win) = self.windows.get_mut(&window_key) else {
            return Ok(false);
        };
        if self.window_manager.active_modal_blocker(win.handle_id).is_some() {
            return Ok(false);
        }
        let handled = win.session.handle_click_logical(x, y)?;
        win.ime_allowed = win
            .session
            .focused_node()
            .is_some_and(|id| win.session.backend().is_text_input(id));
        Ok(handled)
    }

    pub fn route_resize(
        &mut self,
        window_key: K,
        width_px: u32,
        height_px: u32,
        scale_factor: f32,
    ) -> Result<bool, DesktopHostError> {
        let Some(win) = self.windows.get_mut(&window_key) else {
            return Ok(false);
        };
        win.session.resize(width_px, height_px, scale_factor)?;
        let cur = self
            .window_manager
            .get(win.handle_id)
            .map(|w| w.descriptor.bounds)
            .unwrap_or_default();
        let next_bounds = WindowRect::new(
            cur.x,
            cur.y,
            width_px as f32 / scale_factor.max(0.001),
            height_px as f32 / scale_factor.max(0.001),
        );
        let _ = self
            .window_manager
            .set_bounds_with_scale(win.handle_id, next_bounds, scale_factor);
        Ok(true)
    }

    pub fn route_focus(&mut self, window_key: K) -> Result<(), WindowError> {
        let Some(win) = self.windows.get(&window_key) else {
            return Ok(());
        };
        self.window_manager.focus_window(win.handle_id)
    }

    pub fn handle_close_requested(&mut self, window_key: K) -> Result<CloseOutcome, WindowError> {
        let Some((handle_id, surface_id)) = self
            .windows
            .get(&window_key)
            .map(|w| (w.handle_id, w.surface_id))
        else {
            return Ok(CloseOutcome {
                vetoed: false,
                closed_handles: Vec::new(),
                should_exit_loop: self.windows.is_empty(),
            });
        };
        self.window_manager.request_close(handle_id)?;
        let allow = match self.close_veto_handler.as_mut() {
            Some(cb) => cb(handle_id, surface_id),
            None => true,
        };
        self.finish_close_response(handle_id, allow)
    }

    pub fn respond_close_request(
        &mut self,
        handle_id: WindowHandleId,
        allow: bool,
    ) -> Result<CloseOutcome, WindowError> {
        self.finish_close_response(handle_id, allow)
    }

    fn finish_close_response(
        &mut self,
        handle_id: WindowHandleId,
        allow: bool,
    ) -> Result<CloseOutcome, WindowError> {
        let closed_handles = self.window_manager.respond_close_request(handle_id, allow)?;
        if !allow {
            return Ok(CloseOutcome {
                vetoed: true,
                closed_handles,
                should_exit_loop: false,
            });
        }
        let primary_closed = self.primary_handle.is_some_and(|p| closed_handles.contains(&p));
        self.remove_closed_handles(&closed_handles);
        if primary_closed {
            self.primary_handle = None;
        }
        let should_exit_loop =
            self.windows.is_empty() || (primary_closed && self.exit_on_primary_close);
        Ok(CloseOutcome {
            vetoed: false,
            closed_handles,
            should_exit_loop,
        })
    }

    fn remove_closed_handles(&mut self, closed_handles: &[WindowHandleId]) {
        let keys: Vec<(K, SurfaceId)> = self
            .windows
            .iter()
            .filter(|(_, w)| closed_handles.contains(&w.handle_id))
            .map(|(&k, w)| (k, w.surface_id))
            .collect();
        for (k, sid) in keys {
            self.windows.remove(&k);
            self.surface_to_window.remove(&sid);
        }
    }

    pub fn window(&self, key: K) -> Option<&HeadlessSurfaceWindow<A>> { self.windows.get(&key) }
    pub fn window_by_surface(&self, sid: SurfaceId) -> Option<&HeadlessSurfaceWindow<A>> {
        self.surface_to_window.get(&sid).and_then(|k| self.windows.get(k))
    }
    pub fn window_count(&self) -> usize { self.windows.len() }
    pub fn window_manager(&self) -> &WindowManager { &self.window_manager }
    pub fn window_manager_mut(&mut self) -> &mut WindowManager { &mut self.window_manager }
}
