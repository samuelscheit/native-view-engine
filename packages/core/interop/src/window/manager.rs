//! [`WindowManager`] multi-window lifecycle state machine operating directly on typed
//! [`WindowCommand`] and [`WindowLifecycleEvent`] values.

use std::collections::BTreeMap;

use nve_abi::SurfaceId;

use super::{
    validate_constraints, ManagedWindow, SurfacePresentation, WindowCommand, WindowError,
    WindowFlags, WindowHandleId, WindowLifecycleEvent, WindowLifecycleState, WindowRect,
    WindowState, MAX_WINDOW_TITLE_BYTES,
};

#[derive(Debug, Default)]
pub struct WindowManager {
    next_handle: WindowHandleId,
    windows: BTreeMap<WindowHandleId, ManagedWindow>,
    surface_to_window: BTreeMap<SurfaceId, WindowHandleId>,
    focused_window: Option<WindowHandleId>,
    primary_window: Option<WindowHandleId>,
    events: Vec<WindowLifecycleEvent>,
}

impl WindowManager {
    pub fn new() -> Self {
        Self {
            next_handle: 1,
            ..Default::default()
        }
    }

    pub fn open_window(
        &mut self,
        surface_id: SurfaceId,
        presentation: SurfacePresentation,
    ) -> Result<WindowHandleId, WindowError> {
        if self.surface_to_window.contains_key(&surface_id) {
            return Err(WindowError::SurfaceAlreadyHasWindow(surface_id));
        }
        let mut descriptor = presentation
            .descriptor()
            .ok_or_else(|| {
                WindowError::InvalidDescriptor("Embedded presentation cannot open a window".into())
            })?
            .clone();
        descriptor.validate()?;
        if let SurfacePresentation::Popover { anchor_rect, .. } = &presentation {
            anchor_rect.validate(true)?;
        }
        let parent_window = match presentation.parent_surface() {
            Some(ps) => {
                let ph = *self
                    .surface_to_window
                    .get(&ps)
                    .ok_or(WindowError::ParentSurfaceNotWindow(ps))?;
                if self
                    .windows
                    .get(&ph)
                    .is_none_or(|w| w.lifecycle == WindowLifecycleState::Closed)
                {
                    return Err(WindowError::ParentSurfaceNotWindow(ps));
                }
                Some(ph)
            }
            None => None,
        };
        let (modal, cascade_close) = (presentation.is_modal(), presentation.cascades_with_parent());
        if modal {
            descriptor.flags.set(WindowFlags::MODAL, true);
        }
        if matches!(
            presentation,
            SurfacePresentation::UtilityPanel {
                always_on_top: true,
                ..
            }
        ) {
            descriptor.flags.set(WindowFlags::ALWAYS_ON_TOP, true);
        }
        self.next_handle = self.next_handle.max(1);
        while self.windows.contains_key(&self.next_handle) {
            self.next_handle = self.next_handle.saturating_add(1);
        }
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        let (is_primary, bounds, state) =
            (presentation.is_primary(), descriptor.bounds, descriptor.state);
        if let Some(ph) = parent_window
            && let Some(p) = self.windows.get_mut(&ph)
        {
            p.children.push(handle);
        }
        self.windows.insert(
            handle,
            ManagedWindow {
                handle,
                surface_id,
                presentation,
                descriptor,
                parent_window,
                children: Vec::new(),
                lifecycle: WindowLifecycleState::Open,
                focused: false,
                modal,
                cascade_close,
            },
        );
        self.surface_to_window.insert(surface_id, handle);
        if is_primary || self.primary_window.is_none() {
            self.primary_window = Some(handle);
        }
        self.events.push(WindowLifecycleEvent::Opened {
            handle,
            surface_id,
            bounds,
            state,
        });
        self.set_focus_internal(Some(handle));
        Ok(handle)
    }

    pub fn active_modal_blocker(&self, handle: WindowHandleId) -> Option<WindowHandleId> {
        let win = self.windows.get(&handle)?;
        for &cid in win.children.iter().rev() {
            if let Some(deeper) = self.active_modal_blocker(cid) {
                return Some(deeper);
            }
            if let Some(c) = self.windows.get(&cid)
                && c.modal
                && c.lifecycle != WindowLifecycleState::Closed
            {
                return Some(cid);
            }
        }
        None
    }

    pub fn focus_window(&mut self, handle: WindowHandleId) -> Result<(), WindowError> {
        let win = self
            .windows
            .get(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        if win.lifecycle == WindowLifecycleState::Closed {
            return Err(WindowError::WindowAlreadyClosed(handle));
        }
        if let Some(modal_child) = self.active_modal_blocker(handle) {
            return Err(WindowError::BlockedByModalChild {
                target: handle,
                modal_child,
            });
        }
        self.set_focus_internal(Some(handle));
        Ok(())
    }

    pub fn set_title(&mut self, handle: WindowHandleId, title: String) -> Result<(), WindowError> {
        if title.len() > MAX_WINDOW_TITLE_BYTES {
            return Err(WindowError::InvalidDescriptor("title too long".into()));
        }
        self.windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?
            .descriptor
            .title = title;
        Ok(())
    }

    pub fn set_bounds(
        &mut self,
        handle: WindowHandleId,
        bounds: WindowRect,
    ) -> Result<(), WindowError> {
        self.set_bounds_with_scale(handle, bounds, 1.0)
    }

    pub fn set_bounds_with_scale(
        &mut self,
        handle: WindowHandleId,
        mut bounds: WindowRect,
        scale: f32,
    ) -> Result<(), WindowError> {
        bounds.validate(false)?;
        if !scale.is_finite() || scale <= 0.0 {
            return Err(WindowError::InvalidDescriptor("invalid scale".into()));
        }
        let win = self
            .windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        if let Some((mw, mh)) = win.descriptor.min_size {
            bounds.width = bounds.width.max(mw);
            bounds.height = bounds.height.max(mh);
        }
        if let Some((xw, xh)) = win.descriptor.max_size {
            bounds.width = bounds.width.min(xw);
            bounds.height = bounds.height.min(xh);
        }
        let prev = win.descriptor.bounds;
        win.descriptor.bounds = bounds;
        if (prev.x - bounds.x).abs() > f32::EPSILON || (prev.y - bounds.y).abs() > f32::EPSILON {
            self.events.push(WindowLifecycleEvent::Moved {
                handle,
                x: bounds.x,
                y: bounds.y,
            });
        }
        if (prev.width - bounds.width).abs() > f32::EPSILON
            || (prev.height - bounds.height).abs() > f32::EPSILON
        {
            self.events.push(WindowLifecycleEvent::Resized {
                handle,
                width: bounds.width,
                height: bounds.height,
                scale_factor: scale,
            });
        }
        Ok(())
    }

    pub fn set_size_constraints(
        &mut self,
        handle: WindowHandleId,
        min: Option<(f32, f32)>,
        max: Option<(f32, f32)>,
    ) -> Result<(), WindowError> {
        validate_constraints(min, max)?;
        let win = self
            .windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        win.descriptor.min_size = min;
        win.descriptor.max_size = max;
        Ok(())
    }

    pub fn set_window_state(
        &mut self,
        handle: WindowHandleId,
        state: WindowState,
    ) -> Result<(), WindowError> {
        let win = self
            .windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        if win.descriptor.state != state {
            win.descriptor.state = state;
            self.events
                .push(WindowLifecycleEvent::StateChanged { handle, state });
        }
        Ok(())
    }

    pub fn set_decorations(
        &mut self,
        handle: WindowHandleId,
        decorated: bool,
    ) -> Result<(), WindowError> {
        self.windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?
            .descriptor
            .flags
            .set(WindowFlags::DECORATED, decorated);
        Ok(())
    }

    pub fn set_always_on_top(
        &mut self,
        handle: WindowHandleId,
        on_top: bool,
    ) -> Result<(), WindowError> {
        self.windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?
            .descriptor
            .flags
            .set(WindowFlags::ALWAYS_ON_TOP, on_top);
        Ok(())
    }

    pub fn request_close(&mut self, handle: WindowHandleId) -> Result<bool, WindowError> {
        let win = self
            .windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        match win.lifecycle {
            WindowLifecycleState::Open => {
                win.lifecycle = WindowLifecycleState::ClosePending;
                self.events.push(WindowLifecycleEvent::CloseRequested {
                    handle,
                    surface_id: win.surface_id,
                });
                Ok(true)
            }
            WindowLifecycleState::ClosePending => Ok(false),
            WindowLifecycleState::Closed => Err(WindowError::WindowAlreadyClosed(handle)),
        }
    }

    pub fn respond_close_request(
        &mut self,
        handle: WindowHandleId,
        allow: bool,
    ) -> Result<Vec<WindowHandleId>, WindowError> {
        let win = self
            .windows
            .get_mut(&handle)
            .ok_or(WindowError::WindowNotFound(handle))?;
        if win.lifecycle != WindowLifecycleState::ClosePending {
            return Err(WindowError::NotClosePending(handle));
        }
        if !allow {
            win.lifecycle = WindowLifecycleState::Open;
            return Ok(Vec::new());
        }
        self.close_window(handle)
    }

    pub fn close_window(
        &mut self,
        handle: WindowHandleId,
    ) -> Result<Vec<WindowHandleId>, WindowError> {
        if !self.windows.contains_key(&handle) {
            return Err(WindowError::WindowNotFound(handle));
        }
        let mut order = Vec::new();
        self.collect_cascade_post_order(handle, &mut order);
        let fallback_parent = self.windows.get(&handle).and_then(|w| w.parent_window);
        let (mut closed, mut focus_closed) = (Vec::with_capacity(order.len()), false);
        for h in order {
            if let Some(win) = self.windows.remove(&h) {
                if let Some(pid) = win.parent_window
                    && let Some(p) = self.windows.get_mut(&pid)
                {
                    p.children.retain(|&c| c != h);
                }
                for cid in &win.children {
                    if let Some(c) = self.windows.get_mut(cid) {
                        c.parent_window = None;
                    }
                }
                self.surface_to_window.remove(&win.surface_id);
                if self.focused_window == Some(h) {
                    self.focused_window = None;
                    focus_closed = true;
                }
                if self.primary_window == Some(h) {
                    self.primary_window = None;
                }
                self.events.push(WindowLifecycleEvent::Closed {
                    handle: h,
                    surface_id: win.surface_id,
                });
                closed.push(h);
            }
        }
        if focus_closed {
            let next = fallback_parent
                .filter(|p| self.windows.contains_key(p) && self.active_modal_blocker(*p).is_none())
                .or_else(|| self.windows.keys().next_back().copied());
            self.set_focus_internal(next);
        }
        Ok(closed)
    }

    pub fn apply_command(
        &mut self,
        cmd: WindowCommand,
    ) -> Result<Option<WindowHandleId>, WindowError> {
        match cmd {
            WindowCommand::OpenWindow {
                surface_id,
                presentation,
            } => Ok(Some(self.open_window(surface_id, presentation)?)),
            WindowCommand::CloseWindow { handle } => {
                self.close_window(handle)?;
                Ok(None)
            }
            WindowCommand::SetTitle { handle, title } => {
                self.set_title(handle, title)?;
                Ok(None)
            }
            WindowCommand::SetBounds { handle, bounds } => {
                self.set_bounds(handle, bounds)?;
                Ok(None)
            }
            WindowCommand::SetSizeConstraints {
                handle,
                min_size,
                max_size,
            } => {
                self.set_size_constraints(handle, min_size, max_size)?;
                Ok(None)
            }
            WindowCommand::SetWindowState { handle, state } => {
                self.set_window_state(handle, state)?;
                Ok(None)
            }
            WindowCommand::FocusWindow { handle } => {
                self.focus_window(handle)?;
                Ok(None)
            }
            WindowCommand::SetDecorations { handle, decorated } => {
                self.set_decorations(handle, decorated)?;
                Ok(None)
            }
            WindowCommand::SetAlwaysOnTop {
                handle,
                always_on_top,
            } => {
                self.set_always_on_top(handle, always_on_top)?;
                Ok(None)
            }
            WindowCommand::RespondCloseRequest { handle, allow } => {
                self.respond_close_request(handle, allow)?;
                Ok(None)
            }
        }
    }

    /// Applies a slice of typed [`WindowCommand`]s in FIFO order with zero serialization.
    pub fn apply_commands(
        &mut self,
        cmds: &[WindowCommand],
    ) -> Result<Vec<Option<WindowHandleId>>, WindowError> {
        let mut results = Vec::with_capacity(cmds.len());
        for cmd in cmds {
            results.push(self.apply_command(cmd.clone())?);
        }
        Ok(results)
    }

    fn collect_cascade_post_order(&self, handle: WindowHandleId, out: &mut Vec<WindowHandleId>) {
        if let Some(win) = self.windows.get(&handle) {
            for &cid in &win.children {
                if self.windows.get(&cid).is_some_and(|c| c.cascade_close) {
                    self.collect_cascade_post_order(cid, out);
                }
            }
            out.push(handle);
        }
    }

    fn set_focus_internal(&mut self, next: Option<WindowHandleId>) {
        if self.focused_window == next {
            return;
        }
        if let Some(prev) = self.focused_window {
            if let Some(w) = self.windows.get_mut(&prev) {
                w.focused = false;
            }
            self.events.push(WindowLifecycleEvent::Focused {
                handle: prev,
                focused: false,
            });
        }
        self.focused_window = next;
        if let Some(cur) = next {
            if let Some(w) = self.windows.get_mut(&cur) {
                w.focused = true;
            }
            self.events.push(WindowLifecycleEvent::Focused {
                handle: cur,
                focused: true,
            });
        }
    }

    pub fn get(&self, h: WindowHandleId) -> Option<&ManagedWindow> {
        self.windows.get(&h)
    }
    pub fn handle_for_surface(&self, s: SurfaceId) -> Option<WindowHandleId> {
        self.surface_to_window.get(&s).copied()
    }
    pub fn get_by_surface(&self, s: SurfaceId) -> Option<&ManagedWindow> {
        self.handle_for_surface(s)
            .and_then(|h| self.windows.get(&h))
    }
    pub fn focused_window(&self) -> Option<WindowHandleId> {
        self.focused_window
    }
    pub fn primary_window(&self) -> Option<WindowHandleId> {
        self.primary_window
    }
    pub fn open_window_count(&self) -> usize {
        self.windows.len()
    }
    pub fn is_empty(&self) -> bool {
        self.windows.is_empty()
    }
    pub fn windows(&self) -> &BTreeMap<WindowHandleId, ManagedWindow> {
        &self.windows
    }
    pub fn drain_events(&mut self) -> Vec<WindowLifecycleEvent> {
        std::mem::take(&mut self.events)
    }
}
