//! `WindowCommand` dispatch and per-window `WindowEvent` routing helpers for [`DesktopWindowHost`].

use std::sync::Arc;

use nve_backend_skia::gpu::GpuFramePresenter;
use nve_engine::Viewport;
use nve_interop::{WindowCommand, WindowError, WindowRect};
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::window::{WindowId, WindowLevel};

use super::multi_window::{apply_window_state, HostWakeEvent, SurfaceWindow, WindowPresenter};
use super::session::DesktopSession;
use super::window_host::DesktopWindowHost;
use super::DesktopApp;
use crate::host_services::SkiaHostEvent;

impl<A: DesktopApp> DesktopWindowHost<A> {
    /// Polls ready async tasks on `sw` and re-renders in a bounded loop until quiescent so
    /// newly spawned tasks start immediately and arm their host waker. Returns
    /// `(rendered_any, hit_pump_cap)`.
    pub(crate) fn pump_window_async(
        sw: &mut SurfaceWindow<A>,
        force_initial_render: bool,
    ) -> (bool, bool) {
        const MAX_PUMPS: usize = 8;
        let mut rendered = false;
        let mut force = force_initial_render;
        let mut exhausted = true;
        for _ in 0..MAX_PUMPS {
            let polled = sw.session.app_mut().poll_async_work();
            if !polled && !force {
                exhausted = false;
                break;
            }
            force = false;
            if let Err(err) = sw.session.render_and_rasterize() {
                eprintln!("[nve-skia-shell] async frame error: {err}");
            }
            rendered = true;
        }
        (rendered, exhausted)
    }
    pub(crate) fn apply_window_command(&mut self, event_loop: &ActiveEventLoop, cmd: WindowCommand) {
        match cmd {
            WindowCommand::OpenWindow { surface_id, presentation } => {
                let Some(desc) = presentation.descriptor().cloned() else { return };
                let Some(factory) = self.secondary_app_factory.as_mut() else {
                    eprintln!("[nve-skia-shell] cannot open secondary surface {surface_id}: no app factory");
                    return;
                };
                match factory(surface_id, &desc) {
                    Ok(mut app) => {
                        if let Some(proxy) = self.proxy.clone() {
                            app.install_wake_callback(Arc::new(move || {
                                let _ = proxy.send_event(HostWakeEvent::RemoteFrame);
                            }));
                        }
                        let vp = Viewport {
                            width: desc.bounds.width,
                            height: desc.bounds.height,
                            scale_factor: self.options.scale_factor.max(1.0),
                        };
                        let mut session = DesktopSession::new(
                            app,
                            surface_id,
                            vp,
                            self.options.clear_color,
                            self.options.show_hud,
                        );
                        let _ = session.notify_viewport_resize();
                        let _ = session.render_and_rasterize();
                        if let Err(err) = self.spawn_os_window(event_loop, surface_id, presentation, session) {
                            eprintln!("[nve-skia-shell] failed to open secondary window: {err}");
                        }
                    }
                    Err(err) => eprintln!("[nve-skia-shell] secondary app creation failed: {err}"),
                }
            }
            WindowCommand::CloseWindow { handle } => {
                if let Some(modal_child) = self.window_manager.active_modal_blocker(handle) {
                    if let Some(wid) = self.window_id_for_handle(modal_child)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        sw.window.focus_window();
                    }
                    return;
                }
                if let Ok(closed) = self.window_manager.close_window(handle) {
                    self.finalize_closed_handles(event_loop, &closed);
                    let _ = self.flush_window_lifecycle_events();
                }
            }
            WindowCommand::SetTitle { handle, title } => {
                if self.window_manager.set_title(handle, title.clone()).is_ok()
                    && let Some(wid) = self.window_id_for_handle(handle)
                    && let Some(sw) = self.windows.get_mut(&wid)
                {
                    sw.base_title = title;
                    sw.sync_title();
                }
            }
            WindowCommand::SetBounds { handle, bounds } => {
                if self.window_manager.set_bounds(handle, bounds).is_ok() {
                    if let Some(wid) = self.window_id_for_handle(handle)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        sw.window.set_outer_position(LogicalPosition::new(bounds.x as f64, bounds.y as f64));
                        let _ = sw.window.request_inner_size(LogicalSize::new(
                            bounds.width as f64,
                            bounds.height as f64,
                        ));
                    }
                    let _ = self.flush_window_lifecycle_events();
                }
            }
            WindowCommand::SetSizeConstraints { handle, min_size, max_size } => {
                if self.window_manager.set_size_constraints(handle, min_size, max_size).is_ok()
                    && let Some(wid) = self.window_id_for_handle(handle)
                    && let Some(sw) = self.windows.get(&wid)
                {
                    sw.window.set_min_inner_size(min_size.map(|(w, h)| LogicalSize::new(w as f64, h as f64)));
                    sw.window.set_max_inner_size(max_size.map(|(w, h)| LogicalSize::new(w as f64, h as f64)));
                }
            }
            WindowCommand::SetWindowState { handle, state } => {
                if self.window_manager.set_window_state(handle, state).is_ok() {
                    if let Some(wid) = self.window_id_for_handle(handle)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        apply_window_state(&sw.window, state);
                    }
                    let _ = self.flush_window_lifecycle_events();
                }
            }
            WindowCommand::FocusWindow { handle } => match self.window_manager.focus_window(handle) {
                Ok(()) => {
                    if let Some(wid) = self.window_id_for_handle(handle)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        sw.window.focus_window();
                    }
                    let _ = self.flush_window_lifecycle_events();
                }
                Err(WindowError::BlockedByModalChild { modal_child, .. }) => {
                    if let Some(wid) = self.window_id_for_handle(modal_child)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        sw.window.focus_window();
                    }
                }
                Err(_) => {}
            },
            WindowCommand::SetDecorations { handle, decorated } => {
                if self.window_manager.set_decorations(handle, decorated).is_ok()
                    && let Some(wid) = self.window_id_for_handle(handle)
                    && let Some(sw) = self.windows.get(&wid)
                {
                    sw.window.set_decorations(decorated);
                }
            }
            WindowCommand::SetAlwaysOnTop { handle, always_on_top } => {
                if self.window_manager.set_always_on_top(handle, always_on_top).is_ok()
                    && let Some(wid) = self.window_id_for_handle(handle)
                    && let Some(sw) = self.windows.get(&wid)
                {
                    sw.window.set_window_level(if always_on_top {
                        WindowLevel::AlwaysOnTop
                    } else {
                        WindowLevel::Normal
                    });
                }
            }
            WindowCommand::RespondCloseRequest { handle, allow } => {
                self.host_services.take_pending_close_response(handle);
                if allow && let Some(modal_child) = self.window_manager.active_modal_blocker(handle) {
                    let _ = self.window_manager.respond_close_request(handle, false);
                    if let Some(wid) = self.window_id_for_handle(modal_child)
                        && let Some(sw) = self.windows.get(&wid)
                    {
                        sw.window.focus_window();
                    }
                    return;
                }
                if let Ok(closed) = self.window_manager.respond_close_request(handle, allow) {
                    if allow {
                        self.finalize_closed_handles(event_loop, &closed);
                    }
                    let _ = self.flush_window_lifecycle_events();
                }
            }
        }
    }

    pub(crate) fn dispatch_window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                let Some((handle_id, surface_id)) =
                    self.windows.get(&window_id).map(|w| (w.handle_id, w.surface_id))
                else {
                    return;
                };
                if let Some(modal_child) = self.window_manager.active_modal_blocker(handle_id) {
                    if let Some(modal_wid) = self.window_id_for_handle(modal_child)
                        && let Some(modal_win) = self.windows.get(&modal_wid)
                    {
                        modal_win.window.focus_window();
                    }
                    return;
                }
                if self.window_manager.request_close(handle_id).is_err() {
                    return;
                }
                let app_intercepted = self.flush_window_lifecycle_events();
                if app_intercepted || self.host_services.has_pending_close_response(handle_id) {
                    return;
                }
                let allow = match self.close_veto_handler.as_mut() {
                    Some(cb) => cb(handle_id, surface_id),
                    None => true,
                };
                if let Ok(closed) = self.window_manager.respond_close_request(handle_id, allow) {
                    if allow {
                        self.finalize_closed_handles(event_loop, &closed);
                    }
                    let _ = self.flush_window_lifecycle_events();
                }
            }
            WindowEvent::Focused(true) => {
                let Some(handle_id) = self.windows.get(&window_id).map(|w| w.handle_id) else {
                    return;
                };
                match self.window_manager.focus_window(handle_id) {
                    Ok(()) => {
                        let _ = self.flush_window_lifecycle_events();
                    }
                    Err(WindowError::BlockedByModalChild { modal_child, .. }) => {
                        if let Some(modal_wid) = self.window_id_for_handle(modal_child)
                            && let Some(modal_win) = self.windows.get(&modal_wid)
                        {
                            modal_win.window.focus_window();
                        }
                    }
                    Err(_) => {}
                }
            }
            WindowEvent::Focused(false) => {
                if self.primary_window_id == Some(window_id) {
                    self.save_snapshot_if_configured();
                }
            }
            WindowEvent::Moved(pos) => {
                if let Some(sw) = self.windows.get(&window_id) {
                    let scale = sw.window.scale_factor().max(0.001) as f32;
                    let cur = self.window_manager.get(sw.handle_id).map(|w| w.descriptor.bounds).unwrap_or_default();
                    let _ = self.window_manager.set_bounds_with_scale(
                        sw.handle_id,
                        WindowRect::new(pos.x as f32 / scale, pos.y as f32 / scale, cur.width, cur.height),
                        scale,
                    );
                    let _ = self.flush_window_lifecycle_events();
                }
            }
            WindowEvent::Resized(new_size) => {
                if new_size.width == 0 || new_size.height == 0 { return; }
                let Some(sw) = self.windows.get_mut(&window_id) else { return };
                let scale = sw.window.scale_factor() as f32;
                if let Err(err) = sw.session.resize(new_size.width, new_size.height, scale) {
                    eprintln!("[nve-skia-shell] resize error: {err}");
                }
                if let WindowPresenter::Hardware(ref mut hw) = sw.presenter
                    && let Err(err) = hw.resize(new_size.width, new_size.height, scale)
                {
                    eprintln!("[nve-skia-shell] hardware swapchain resize error: {err}");
                }
                let handle_id = sw.handle_id;
                sw.sync_ime_state();
                sw.window.request_redraw();
                let cur = self.window_manager.get(handle_id).map(|w| w.descriptor.bounds).unwrap_or_default();
                let _ = self.window_manager.set_bounds_with_scale(
                    handle_id,
                    WindowRect::new(cur.x, cur.y, new_size.width as f32 / scale.max(0.001), new_size.height as f32 / scale.max(0.001)),
                    scale,
                );
                let _ = self.flush_window_lifecycle_events();
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let Some(sw) = self.windows.get_mut(&window_id) else { return };
                let size = sw.window.inner_size();
                if size.width == 0 || size.height == 0 { return; }
                let scale = scale_factor as f32;
                if let Err(err) = sw.session.resize(size.width, size.height, scale) {
                    eprintln!("[nve-skia-shell] scale change error: {err}");
                }
                if let WindowPresenter::Hardware(ref mut hw) = sw.presenter
                    && let Err(err) = hw.resize(size.width, size.height, scale)
                {
                    eprintln!("[nve-skia-shell] hardware swapchain scale resize error: {err}");
                }
                let handle_id = sw.handle_id;
                sw.sync_ime_state();
                sw.window.request_redraw();
                let cur = self.window_manager.get(handle_id).map(|w| w.descriptor.bounds).unwrap_or_default();
                let _ = self.window_manager.set_bounds_with_scale(
                    handle_id,
                    WindowRect::new(cur.x, cur.y, size.width as f32 / scale.max(0.001), size.height as f32 / scale.max(0.001)),
                    scale,
                );
                let _ = self.flush_window_lifecycle_events();
                if let Some(snap) = self.host_services.update_scale_factor(scale) {
                    let _ = self.broadcast_host_event(&SkiaHostEvent::ThemeChanged(snap));
                }
            }
            WindowEvent::ThemeChanged(theme) => {
                let scale = self
                    .windows
                    .get(&window_id)
                    .map(|sw| sw.window.scale_factor() as f32)
                    .unwrap_or(self.options.scale_factor);
                let snap = self.host_services.update_theme_from_winit(theme, scale);
                let _ = self.broadcast_host_event(&SkiaHostEvent::ThemeChanged(snap));
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(sw) = self.windows.get_mut(&window_id) else { return };
                let scale = sw.session.viewport().scale_factor.max(0.001);
                match sw.session.handle_pointer_move_logical(position.x as f32 / scale, position.y as f32 / scale) {
                    Ok(true) => { sw.sync_ime_state(); sw.sync_cursor_state(); sw.window.request_redraw(); }
                    Ok(false) => sw.sync_cursor_state(),
                    Err(err) => eprintln!("[nve-skia-shell] pointer move error: {err}"),
                }
            }
            WindowEvent::CursorLeft { .. } => {
                let Some(sw) = self.windows.get_mut(&window_id) else { return };
                let dirty = sw.session.handle_pointer_leave();
                sw.sync_cursor_state();
                if dirty {
                    sw.window.request_redraw();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Pressed, button: MouseButton::Left, .. } => {
                let mut reschedule = false;
                {
                    let Some(sw) = self.windows.get_mut(&window_id) else { return };
                    if self.window_manager.active_modal_blocker(sw.handle_id).is_some() { return; }
                    let (x, y) = sw.session.cursor_logical();
                    match sw.session.handle_click_logical(x, y) {
                        Ok(true) => {
                            let (_, exhausted) = Self::pump_window_async(sw, false);
                            reschedule = exhausted;
                            sw.sync_ime_state();
                            sw.sync_cursor_state();
                            sw.window.request_redraw();
                        }
                        Ok(false) => sw.sync_cursor_state(),
                        Err(err) => eprintln!("[nve-skia-shell] click dispatch error: {err}"),
                    }
                }
                if reschedule {
                    self.host_services.request_frame_wake();
                }
            }
            WindowEvent::MouseInput { state: ElementState::Released, button: MouseButton::Left, .. } => {
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    let dirty = sw.session.handle_pointer_up_with_redraw();
                    sw.sync_cursor_state();
                    if dirty {
                        sw.window.request_redraw();
                    }
                }
            }
            WindowEvent::ModifiersChanged(new_mods) => {
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    sw.modifiers = new_mods.state();
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let mut reschedule = false;
                {
                    let Some(sw) = self.windows.get_mut(&window_id) else { return };
                    let scale = sw.session.viewport().scale_factor.max(0.001);
                    let (dx, dy) = match delta {
                        MouseScrollDelta::LineDelta(lx, ly) => (-lx * 40.0, -ly * 40.0),
                        MouseScrollDelta::PixelDelta(pos) => (-(pos.x as f32) / scale, -(pos.y as f32) / scale),
                    };
                    let (x, y) = sw.session.cursor_logical();
                    match sw.session.handle_scroll_logical(x, y, dx, dy) {
                        Ok(true) => {
                            let (_, exhausted) = Self::pump_window_async(sw, false);
                            reschedule = exhausted;
                            sw.sync_ime_state();
                            sw.window.request_redraw();
                        }
                        Ok(false) => {}
                        Err(err) => eprintln!("[nve-skia-shell] scroll dispatch error: {err}"),
                    }
                }
                if reschedule {
                    self.host_services.request_frame_wake();
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                let mut reschedule = false;
                {
                    let Some(sw) = self.windows.get_mut(&window_id) else { return };
                    if self.window_manager.active_modal_blocker(sw.handle_id).is_some() { return; }
                    match sw.session.handle_keyboard_input(&event.logical_key, event.text.as_deref(), sw.modifiers) {
                        Ok(true) => {
                            let (_, exhausted) = Self::pump_window_async(sw, false);
                            reschedule = exhausted;
                            sw.sync_ime_state();
                            sw.window.request_redraw();
                        }
                        Ok(false) => {}
                        Err(err) => eprintln!("[nve-skia-shell] keyboard dispatch error: {err}"),
                    }
                }
                if reschedule {
                    self.host_services.request_frame_wake();
                }
            }
            WindowEvent::Ime(ime) => {
                let mut reschedule = false;
                {
                    let Some(sw) = self.windows.get_mut(&window_id) else { return };
                    match sw.session.handle_ime(&ime) {
                        Ok(true) => {
                            let (_, exhausted) = Self::pump_window_async(sw, false);
                            reschedule = exhausted;
                            sw.sync_ime_state();
                            sw.window.request_redraw();
                        }
                        Ok(false) => {}
                        Err(err) => eprintln!("[nve-skia-shell] IME dispatch error: {err}"),
                    }
                }
                if reschedule {
                    self.host_services.request_frame_wake();
                }
            }
            WindowEvent::RedrawRequested => {
                self.process_pending_hot_patches();
                self.process_pending_remote_frames();
                self.process_app_wakes();
                if let Some(sw) = self.windows.get_mut(&window_id) {
                    sw.present_pixels();
                }
            }
            _ => {}
        }
    }
}
