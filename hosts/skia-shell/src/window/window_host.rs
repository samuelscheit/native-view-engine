//! Live `winit` multi-window desktop host with per-window GPU/softbuffer presenters,
//! independent [`DesktopSession`]s, strict `WindowId` event routing, `WindowManager` close veto &
//! cascading child cleanup, IME positioning, hot reload, and Frame-0 snapshot persistence.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::Arc;

use nve_abi::SurfaceId;
use nve_engine::{Frame0Meta, Viewport};
use nve_interop::{
    SurfacePresentation, WindowDescriptor, WindowFlags, WindowHandleId, WindowManager,
};
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop, EventLoopProxy};
use winit::keyboard::ModifiersState;
use winit::window::WindowId;

use super::hot_reload_client::HotReloadClient;
use super::multi_window::{
    CloseVetoHandler, HostWakeEvent, SecondaryAppFactory, SurfaceWindow, apply_window_state,
    build_window_attributes, create_presenter,
};
use super::pair_client::{PairConnectConfig, PairedRemoteApp};
use super::session::{DesktopSession, prewarm_frame0_into_replica};
use super::{DesktopApp, DesktopHostError, DesktopPreviewOptions, HotAppSlot};
use crate::host_services::{SkiaHostEvent, SkiaHostServices};

/// Live multi-window `winit` desktop host supporting direct hardware GPU swapchains (`skia-metal`,
/// `skia-vulkan`, `skia-d3d`) with `softbuffer` fallback, optional Frame-0 `frame0.nve2` pre-warm,
/// `WindowManager` lifecycle/modal/veto rules, `NVE_IFACE_HOST = 5` host services, and thin-client
/// `NVD1` pairing.
pub struct DesktopWindowHost<A: DesktopApp = HotAppSlot> {
    pub(crate) options: DesktopPreviewOptions,
    primary_session: Option<DesktopSession<A>>,
    pub(crate) primary_window_id: Option<WindowId>,
    primary_handle: Option<WindowHandleId>,
    pub(crate) windows: HashMap<WindowId, SurfaceWindow<A>>,
    pub(crate) surface_to_window: BTreeMap<SurfaceId, WindowId>,
    pub(crate) window_manager: WindowManager,
    pub(crate) host_services: Arc<SkiaHostServices>,
    exit_on_primary_close: bool,
    pub(crate) close_veto_handler: Option<CloseVetoHandler>,
    pub(crate) secondary_app_factory: Option<SecondaryAppFactory<A>>,
    hot_reload: Option<HotReloadClient>,
    pub(crate) proxy: Option<EventLoopProxy<HostWakeEvent>>,
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

        let meta = Frame0Meta::default_core();
        let prewarmed = options.frame0_path.as_deref().and_then(|path| {
            prewarm_frame0_into_replica(
                path,
                meta,
                options.surface_id,
                initial_viewport,
                options.clear_color,
                options.layout_engine,
            )
            .ok()
        });

        let host_services = SkiaHostServices::new();
        let mut app = HotAppSlot::load(&dylib_path, options.surface_id)?;
        let _ = app.bind_host_services(Some(Arc::clone(&host_services)));
        let mut session = if let Some((replica, pixel_buffer, _footer)) = prewarmed {
            DesktopSession::from_parts(
                app,
                replica,
                pixel_buffer,
                Vec::new(),
                options.surface_id,
                initial_viewport,
                options.clear_color,
                options.show_hud,
            )
            .with_layout_choice(options.layout_engine)
        } else {
            DesktopSession::new_with_layout(
                app,
                options.surface_id,
                initial_viewport,
                options.clear_color,
                options.show_hud,
                options.layout_engine,
            )?
        };
        let _ = session.notify_viewport_resize();
        session.render_and_rasterize()?;

        let factory_dylib = dylib_path;
        let factory_host = Arc::clone(&host_services);
        let factory: SecondaryAppFactory<HotAppSlot> = Box::new(move |surface_id, _desc| {
            let mut app = HotAppSlot::load(&factory_dylib, surface_id)?;
            let _ = app.bind_host_services(Some(Arc::clone(&factory_host)));
            Ok(app)
        });

        let mut host =
            Self::from_primary_session_with_host_services(options, session, host_services);
        host.secondary_app_factory = Some(factory);
        Ok(host)
    }
}

impl DesktopWindowHost<PairedRemoteApp> {
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
        let mut session = DesktopSession::new_with_layout(
            app,
            options.surface_id,
            initial_viewport,
            options.clear_color,
            options.show_hud,
            options.layout_engine,
        )?;
        let _ = session.notify_viewport_resize();
        session.app_mut().wait_for_frame(initial_timeout)?;
        if let Some(banner) = session.app().remote_status_banner() {
            session.set_status_banner(Some(banner));
        }
        session.render_and_rasterize()?;
        Ok(Self::from_primary_session(options, session))
    }
}

impl<A: DesktopApp> DesktopWindowHost<A> {
    pub(crate) fn from_primary_session(
        options: DesktopPreviewOptions,
        mut session: DesktopSession<A>,
    ) -> Self {
        let host_services = SkiaHostServices::new();
        let _ = session
            .app_mut()
            .bind_host_services(Some(Arc::clone(&host_services)));
        Self::from_primary_session_with_host_services(options, session, host_services)
    }

    pub(crate) fn from_primary_session_with_host_services(
        options: DesktopPreviewOptions,
        session: DesktopSession<A>,
        host_services: Arc<SkiaHostServices>,
    ) -> Self {
        Self {
            options,
            primary_session: Some(session),
            primary_window_id: None,
            primary_handle: None,
            windows: HashMap::new(),
            surface_to_window: BTreeMap::new(),
            window_manager: WindowManager::new(),
            host_services,
            exit_on_primary_close: true,
            close_veto_handler: None,
            secondary_app_factory: None,
            hot_reload: None,
            proxy: None,
        }
    }

    pub fn session(&self) -> &DesktopSession<A> {
        if let Some(wid) = self.primary_window_id
            && let Some(win) = self.windows.get(&wid)
        {
            return &win.session;
        }
        self.primary_session
            .as_ref()
            .or_else(|| self.windows.values().next().map(|w| &w.session))
            .expect("DesktopWindowHost has no active session")
    }

    pub fn session_mut(&mut self) -> &mut DesktopSession<A> {
        if let Some(wid) = self.primary_window_id
            && self.windows.contains_key(&wid)
        {
            return &mut self
                .windows
                .get_mut(&wid)
                .expect("primary_window_id verified present in self.windows")
                .session;
        }
        if let Some(primary) = self.primary_session.as_mut() {
            return primary;
        }
        &mut self
            .windows
            .values_mut()
            .next()
            .expect("DesktopWindowHost has no active session")
            .session
    }

    pub fn save_frame0_if_configured(&mut self) {
        let Some(frame0_path) = self.options.frame0_path.clone() else {
            return;
        };
        let meta = Frame0Meta::default_core();
        let saved = if let Some(wid) = self.primary_window_id
            && let Some(win) = self.windows.get(&wid)
        {
            Some(win.session.save_frame0(&frame0_path, meta))
        } else {
            self.primary_session
                .as_ref()
                .map(|s| s.save_frame0(&frame0_path, meta))
        };
        if let Some(Err(err)) = saved {
            eprintln!(
                "[nve-skia-shell] Warning: failed to save frame0.nve2 to '{}': {err}",
                frame0_path.display()
            );
        }
    }

    pub fn run(mut self) -> Result<(), DesktopHostError> {
        let event_loop = EventLoop::<HostWakeEvent>::with_user_event()
            .build()
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()))?;
        let proxy = event_loop.create_proxy();
        self.proxy = Some(proxy.clone());
        self.host_services.attach_event_loop_proxy(proxy.clone());

        if let Some(ws_url) = self.options.ws_url.clone() {
            let wake_proxy = proxy.clone();
            self.hot_reload = Some(HotReloadClient::spawn(ws_url, move || {
                let _ = wake_proxy.send_event(HostWakeEvent::HotReload);
            }));
        }

        if let Some(primary) = self.primary_session.as_mut() {
            let remote_proxy = proxy;
            primary.app_mut().install_wake_callback(Arc::new(move || {
                let _ = remote_proxy.send_event(HostWakeEvent::RemoteFrame);
            }));
        }

        let res = event_loop
            .run_app(&mut self)
            .map_err(|e| DesktopHostError::EventLoop(e.to_string()));
        self.save_frame0_if_configured();
        res
    }

    pub(crate) fn window_id_for_handle(&self, handle: WindowHandleId) -> Option<WindowId> {
        self.window_manager
            .get(handle)
            .and_then(|w| self.surface_to_window.get(&w.surface_id).copied())
            .or_else(|| {
                self.windows
                    .iter()
                    .find_map(|(&wid, w)| (w.handle_id == handle).then_some(wid))
            })
    }

    pub(crate) fn spawn_os_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        surface_id: SurfaceId,
        presentation: SurfacePresentation,
        mut session: DesktopSession<A>,
    ) -> Result<WindowId, DesktopHostError> {
        let desc = presentation
            .descriptor()
            .ok_or_else(|| DesktopHostError::Window("cannot spawn Embedded presentation".into()))?
            .clone();
        let is_primary = presentation.is_primary();
        let handle_id = self
            .window_manager
            .open_window(surface_id, presentation)
            .map_err(|e| DesktopHostError::Window(e.to_string()))?;

        let window = match event_loop.create_window(build_window_attributes(&desc)) {
            Ok(w) => Arc::new(w),
            Err(err) => {
                let _ = self.window_manager.close_window(handle_id);
                return Err(DesktopHostError::Window(err.to_string()));
            }
        };
        let presenter =
            match create_presenter(&window, self.options.preferred_backend, self.options.vsync) {
                Ok(p) => p,
                Err(err) => {
                    let _ = self.window_manager.close_window(handle_id);
                    return Err(err);
                }
            };

        let size = window.inner_size();
        let scale = window.scale_factor() as f32;
        if let Err(err) = session.resize(size.width.max(1), size.height.max(1), scale) {
            eprintln!(
                "[nve-skia-shell] initial resize/render failed for surface {surface_id}: {err}"
            );
        }
        apply_window_state(&window, desc.state);

        let initial_theme = window
            .theme()
            .map(|t| self.host_services.update_theme_from_winit(t, scale));
        let window_id = window.id();
        let mut sw = SurfaceWindow {
            window: window.clone(),
            presenter,
            session,
            surface_id,
            handle_id,
            base_title: desc.title,
            modifiers: ModifiersState::empty(),
            ime_allowed: false,
            last_ime_cursor_rect: None,
            last_cursor: nve_backend_skia::CursorKind::Auto,
            last_telemetry: None,
        };
        if let Some(theme) = initial_theme
            && sw
                .session
                .dispatch_host_event(&SkiaHostEvent::ThemeChanged(theme))
        {
            let _ = sw.session.render_and_rasterize();
        }
        sw.sync_title();
        sw.sync_ime_state();
        self.surface_to_window.insert(surface_id, window_id);
        self.windows.insert(window_id, sw);
        if is_primary || self.primary_window_id.is_none() {
            self.primary_window_id = Some(window_id);
            self.primary_handle = Some(handle_id);
        }
        let _ = self.flush_window_lifecycle_events();
        window.request_redraw();
        Ok(window_id)
    }

    pub(crate) fn broadcast_host_event(&mut self, event: &SkiaHostEvent) -> bool {
        let mut any_handled = false;
        for sw in self.windows.values_mut() {
            if sw.session.dispatch_host_event(event) {
                any_handled = true;
                if let Err(err) = sw.session.render_and_rasterize() {
                    eprintln!("[nve-skia-shell] host event render error: {err}");
                }
                sw.sync_ime_state();
                sw.window.request_redraw();
            }
        }
        if let Some(primary) = self.primary_session.as_mut()
            && primary.dispatch_host_event(event)
        {
            any_handled = true;
            let _ = primary.render_and_rasterize();
        }
        any_handled
    }

    pub(crate) fn flush_window_lifecycle_events(&mut self) -> bool {
        let events = self.window_manager.drain_events();
        if events.is_empty() {
            return false;
        }
        self.broadcast_host_event(&SkiaHostEvent::WindowLifecycle(events))
    }

    pub(crate) fn finalize_closed_handles(
        &mut self,
        event_loop: &ActiveEventLoop,
        closed: &[WindowHandleId],
    ) {
        if closed.is_empty() {
            return;
        }
        let primary_closed = self.primary_handle.is_some_and(|p| closed.contains(&p));
        if primary_closed {
            self.save_frame0_if_configured();
        }
        let wids: Vec<(WindowId, SurfaceId)> = self
            .windows
            .iter()
            .filter(|(_, w)| closed.contains(&w.handle_id))
            .map(|(&wid, w)| (wid, w.surface_id))
            .collect();
        for (wid, sid) in wids {
            self.windows.remove(&wid);
            self.surface_to_window.remove(&sid);
        }
        if primary_closed {
            self.primary_window_id = None;
            self.primary_handle = None;
        }
        if self.windows.is_empty() || (primary_closed && self.exit_on_primary_close) {
            self.save_frame0_if_configured();
            self.hot_reload.take();
            self.windows.clear();
            self.surface_to_window.clear();
            event_loop.exit();
        }
    }

    pub(crate) fn process_pending_hot_patches(&mut self) {
        let msgs = self
            .hot_reload
            .as_ref()
            .map(HotReloadClient::try_drain)
            .unwrap_or_default();
        if msgs.is_empty() {
            return;
        }
        for sw in self.windows.values_mut() {
            let mut needs_redraw = false;
            for msg in &msgs {
                match sw.session.handle_hot_patch(msg.clone()) {
                    Ok(updated) => needs_redraw |= updated,
                    Err(err) => {
                        eprintln!("[nve-skia-shell] {err}");
                        needs_redraw = true;
                    }
                }
            }
            sw.sync_title();
            sw.sync_ime_state();
            if needs_redraw {
                sw.window.request_redraw();
            }
        }
    }

    pub(crate) fn process_pending_remote_frames(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let mut reschedule = false;
        for sw in self.windows.values_mut() {
            let has_remote = sw.session.app().has_pending_remote_frame();
            let mut banner_changed = false;
            if let Some(banner) = sw.session.app().remote_status_banner()
                && sw.session.status_banner() != Some(banner.as_str())
            {
                sw.session.set_status_banner(Some(banner));
                banner_changed = true;
            }
            let (rendered, exhausted) = Self::pump_window_async(sw, has_remote);
            reschedule |= exhausted;
            if rendered {
                sw.sync_title();
                sw.sync_ime_state();
                sw.window.request_redraw();
            } else if banner_changed {
                sw.session.rasterize_current();
                sw.sync_title();
                sw.window.request_redraw();
            }
        }
        if reschedule {
            self.host_services.request_frame_wake();
        }
    }

    pub(crate) fn process_app_wakes(&mut self) {
        if self.windows.is_empty() {
            return;
        }
        let host_woke = self.host_services.take_pending_app_wake();
        let mut reschedule = false;
        for sw in self.windows.values_mut() {
            let (rendered, exhausted) = Self::pump_window_async(sw, host_woke);
            reschedule |= exhausted;
            if rendered {
                sw.sync_title();
                sw.sync_ime_state();
                sw.window.request_redraw();
            }
        }
        if reschedule {
            self.host_services.request_frame_wake();
        }
    }
}

impl<A: DesktopApp> Drop for DesktopWindowHost<A> {
    fn drop(&mut self) {
        self.save_frame0_if_configured();
    }
}

impl<A: DesktopApp> ApplicationHandler<HostWakeEvent> for DesktopWindowHost<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if !self.windows.is_empty() {
            return;
        }
        let Some(session) = self.primary_session.take() else {
            return;
        };
        let mut flags = WindowFlags::standard();
        if self.exit_on_primary_close {
            flags.set(WindowFlags::EXIT_ON_CLOSE, true);
        }
        let desc = WindowDescriptor::new(
            self.options.title.clone(),
            self.options.width,
            self.options.height,
        )
        .with_flags(flags);
        if let Err(err) = self.spawn_os_window(
            event_loop,
            self.options.surface_id,
            SurfacePresentation::PrimaryWindow(desc),
            session,
        ) {
            eprintln!("[nve-skia-shell] failed to create primary window: {err}");
            event_loop.exit();
            return;
        }
        // Pump initial async tasks spawned during mount and arm host wakers immediately.
        self.process_app_wakes();
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: HostWakeEvent) {
        match event {
            HostWakeEvent::HotReload => self.process_pending_hot_patches(),
            HostWakeEvent::RemoteFrame => self.process_pending_remote_frames(),
            HostWakeEvent::AppWake => self.process_app_wakes(),
            HostWakeEvent::WindowCommand(cmd) => self.apply_window_command(event_loop, cmd),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.dispatch_window_event(event_loop, window_id, event);
    }
}
