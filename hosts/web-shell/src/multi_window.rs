//! Cross-target multi-surface & popup window coordination for `hosts/web-shell`.
//!
//! Coordinates multiple independent [`WebAppSession`] surfaces alongside [`WindowManager`]
//! lifecycle rules, popup window feature formatting, graceful popup-blocker errors, and
//! direct typed [`WindowCommand`] / [`WindowLifecycleEvent`] synchronization.

use std::collections::BTreeMap;
use std::fmt;

use nve_abi::SurfaceId;
use nve_interop::{
    SurfacePresentation, WindowCommand, WindowDescriptor, WindowError, WindowHandleId,
    WindowLifecycleEvent, WindowManager,
};

use crate::session::WebAppSession;

/// Default `BroadcastChannel` name used to synchronize window state across browser windows.
pub const DEFAULT_WINDOW_SYNC_CHANNEL: &str = "nve-window-sync-v1";

/// Validated specification for opening a secondary browser popup window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PopupWindowSpec {
    pub surface_id: SurfaceId,
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub channel_name: String,
}

impl PopupWindowSpec {
    pub fn new(
        surface_id: SurfaceId,
        title: impl Into<String>,
        width: u32,
        height: u32,
    ) -> Result<Self, PopupWindowError> {
        if surface_id == 0 {
            return Err(PopupWindowError::InvalidSurfaceId);
        }
        if width == 0 || height == 0 {
            return Err(PopupWindowError::InvalidDimensions { width, height });
        }
        Ok(Self {
            surface_id,
            title: title.into(),
            width,
            height,
            channel_name: DEFAULT_WINDOW_SYNC_CHANNEL.to_owned(),
        })
    }

    pub fn target_name(&self) -> String {
        format!("nve-surface-{}", self.surface_id)
    }

    pub fn mount_root_id(&self) -> String {
        format!("nve-popup-root-{}", self.surface_id)
    }

    pub fn format_window_features(&self) -> String {
        format!(
            "popup=yes,width={},height={},resizable=yes,scrollbars=yes",
            self.width, self.height
        )
    }

    pub fn to_presentation(&self, parent: Option<SurfaceId>) -> SurfacePresentation {
        SurfacePresentation::SecondaryWindow {
            parent,
            modal: false,
            descriptor: WindowDescriptor::new(
                self.title.clone(),
                self.width as f32,
                self.height as f32,
            ),
        }
    }
}

/// Errors produced by multi-surface mounting, popup creation, or `BroadcastChannel` sync.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PopupWindowError {
    InvalidSurfaceId,
    InvalidDimensions { width: u32, height: u32 },
    DuplicateSurface(SurfaceId),
    PopupBlocked { surface_id: SurfaceId, title: String },
    BroadcastChannelUnavailable(String),
    WindowManager(WindowError),
}

impl fmt::Display for PopupWindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSurfaceId => write!(f, "surface_id must be non-zero"),
            Self::InvalidDimensions { width, height } => {
                write!(f, "invalid popup dimensions {width}x{height} (must be > 0)")
            }
            Self::DuplicateSurface(sid) => write!(f, "surface {sid} is already mounted"),
            Self::PopupBlocked { surface_id, title } => write!(
                f,
                "browser popup blocker prevented opening window '{title}' for surface {surface_id}; allow popups for this origin or invoke from a user gesture"
            ),
            Self::BroadcastChannelUnavailable(msg) => {
                write!(f, "BroadcastChannel unavailable: {msg}")
            }
            Self::WindowManager(err) => write!(f, "window manager error: {err}"),
        }
    }
}

impl std::error::Error for PopupWindowError {}

impl From<WindowError> for PopupWindowError {
    fn from(value: WindowError) -> Self {
        Self::WindowManager(value)
    }
}

/// Cross-target multi-surface web host pairing independent [`WebAppSession`]s with [`WindowManager`]
/// lifecycle state and direct typed [`WindowCommand`] / [`WindowLifecycleEvent`] APIs.
pub struct WebMultiWindowHost {
    sessions: BTreeMap<SurfaceId, WebAppSession>,
    window_manager: WindowManager,
}

impl Default for WebMultiWindowHost {
    fn default() -> Self {
        Self::new()
    }
}

impl WebMultiWindowHost {
    pub fn new() -> Self {
        Self {
            sessions: BTreeMap::new(),
            window_manager: WindowManager::new(),
        }
    }

    pub fn mount_surface(
        &mut self,
        surface_id: SurfaceId,
        session: WebAppSession,
        presentation: SurfacePresentation,
    ) -> Result<WindowHandleId, PopupWindowError> {
        if surface_id == 0 {
            return Err(PopupWindowError::InvalidSurfaceId);
        }
        if self.sessions.contains_key(&surface_id) {
            return Err(PopupWindowError::DuplicateSurface(surface_id));
        }
        let handle = self.window_manager.open_window(surface_id, presentation)?;
        self.sessions.insert(surface_id, session);
        Ok(handle)
    }

    pub fn unmount_surface(&mut self, surface_id: SurfaceId) -> Vec<SurfaceId> {
        let Some(handle) = self.window_manager.handle_for_surface(surface_id) else {
            return if self.sessions.remove(&surface_id).is_some() {
                vec![surface_id]
            } else {
                Vec::new()
            };
        };
        let surface_by_handle: BTreeMap<WindowHandleId, SurfaceId> = self
            .window_manager
            .windows()
            .iter()
            .map(|(&h, w)| (h, w.surface_id))
            .collect();
        let closed_handles = self.window_manager.close_window(handle).unwrap_or_default();
        let mut removed = Vec::with_capacity(closed_handles.len());
        for h in closed_handles {
            if let Some(&sid) = surface_by_handle.get(&h) {
                self.sessions.remove(&sid);
                removed.push(sid);
            }
        }
        removed
    }

    pub fn session(&self, surface_id: SurfaceId) -> Option<&WebAppSession> {
        self.sessions.get(&surface_id)
    }

    pub fn session_mut(&mut self, surface_id: SurfaceId) -> Option<&mut WebAppSession> {
        self.sessions.get_mut(&surface_id)
    }

    pub fn active_surface_ids(&self) -> Vec<SurfaceId> {
        self.sessions.keys().copied().collect()
    }

    pub fn surface_count(&self) -> usize {
        self.sessions.len()
    }

    pub fn window_manager(&self) -> &WindowManager {
        &self.window_manager
    }

    pub fn window_manager_mut(&mut self) -> &mut WindowManager {
        &mut self.window_manager
    }

    /// Applies a single typed [`WindowCommand`] directly with zero serialization.
    pub fn apply_window_command(
        &mut self,
        command: WindowCommand,
    ) -> Result<Option<WindowHandleId>, PopupWindowError> {
        Ok(self.window_manager.apply_command(command)?)
    }

    /// Applies a slice of typed [`WindowCommand`]s directly in FIFO order.
    pub fn apply_window_commands(
        &mut self,
        commands: &[WindowCommand],
    ) -> Result<usize, PopupWindowError> {
        self.window_manager.apply_commands(commands)?;
        Ok(commands.len())
    }

    /// Drains all pending typed [`WindowLifecycleEvent`]s with zero serialization.
    pub fn drain_lifecycle_events(&mut self) -> Vec<WindowLifecycleEvent> {
        self.window_manager.drain_events()
    }
}

#[cfg(all(test, feature = "app-counter"))]
mod tests {
    use nve_abi::{EventMask, EventPayload, NativeEvent, NativeEventKind};
    use nve_backend_web::WebBackend;
    use nve_engine::Viewport;
    use nve_interop::WindowRect;

    use super::*;

    #[test]
    fn popup_spec_validates_inputs_and_formats_features_and_blocker_error() {
        assert_eq!(
            PopupWindowSpec::new(0, "Bad", 400, 300),
            Err(PopupWindowError::InvalidSurfaceId)
        );
        assert_eq!(
            PopupWindowSpec::new(2, "Bad", 0, 300),
            Err(PopupWindowError::InvalidDimensions { width: 0, height: 300 })
        );

        let spec = PopupWindowSpec::new(7, "Inspector", 480, 640).unwrap();
        assert_eq!(spec.target_name(), "nve-surface-7");
        assert_eq!(spec.mount_root_id(), "nve-popup-root-7");
        assert_eq!(
            spec.format_window_features(),
            "popup=yes,width=480,height=640,resizable=yes,scrollbars=yes"
        );

        let blocked = PopupWindowError::PopupBlocked {
            surface_id: 7,
            title: "Inspector".into(),
        };
        let msg = blocked.to_string();
        assert!(msg.contains("popup blocker"));
        assert!(msg.contains("Inspector"));
        assert!(msg.contains("surface 7"));
    }

    #[test]
    fn multi_surface_web_host_isolates_sessions_cascades_popovers_and_applies_typed_commands() {
        let mut host = WebMultiWindowHost::new();

        let mut studio = WebAppSession::new_studio_workspace(1, WebBackend::new(), Viewport::default());
        studio.render_initial().unwrap();
        let mut counter = WebAppSession::new_counter(2, WebBackend::new(), Viewport::default());
        counter.render_initial().unwrap();

        let h1 = host
            .mount_surface(
                1,
                studio,
                SurfacePresentation::PrimaryWindow(WindowDescriptor::new("Studio", 1280.0, 800.0)),
            )
            .unwrap();
        let spec2 = PopupWindowSpec::new(2, "Counter Popup", 420, 360).unwrap();
        let h2 = host
            .mount_surface(2, counter, spec2.to_presentation(Some(1)))
            .unwrap();

        // Mount a popover anchored to surface 1 that must cascade when surface 1 unmounts.
        let mut popover = WebAppSession::new_counter(3, WebBackend::new(), Viewport::default());
        popover.render_initial().unwrap();
        let _h3 = host
            .mount_surface(
                3,
                popover,
                SurfacePresentation::Popover {
                    anchor_surface: 1,
                    anchor_rect: WindowRect::new(10.0, 10.0, 80.0, 24.0),
                    descriptor: WindowDescriptor::new("Quick Menu", 200.0, 140.0),
                },
            )
            .unwrap();

        assert_eq!(host.active_surface_ids(), vec![1, 2, 3]);
        assert!(matches!(
            host.mount_surface(
                1,
                WebAppSession::new_counter(1, WebBackend::new(), Viewport::default()),
                SurfacePresentation::PrimaryWindow(WindowDescriptor::default()),
            ),
            Err(PopupWindowError::DuplicateSurface(1))
        ));

        // Clicking Increment on surface 2 updates surface 2 only, leaving surface 3 untouched.
        let inc_node = (1..=64)
            .find(|&id| (host.session(2).unwrap().backend().event_mask_of(id) & EventMask::PRESS) != 0)
            .unwrap();
        host.session_mut(2).unwrap().backend().push_event(NativeEvent {
            surface_id: 2,
            node_id: inc_node,
            timestamp_us: 1000,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 20.0, y: 20.0 },
        });
        host.session_mut(2).unwrap().pump_events().unwrap().unwrap();
        assert!(host.session(2).unwrap().backend().render_html().contains("Count: 1"));
        assert!(host.session(3).unwrap().backend().render_html().contains("Count: 0"));

        // Direct typed WindowCommand dispatch updates title & focus cleanly.
        assert_eq!(
            host.apply_window_commands(&[
                WindowCommand::SetTitle {
                    handle: h2,
                    title: "Counter Synced".into(),
                },
                WindowCommand::FocusWindow { handle: h1 },
            ])
            .unwrap(),
            2
        );
        assert_eq!(host.window_manager().get(h2).unwrap().descriptor.title, "Counter Synced");
        assert_eq!(host.window_manager().focused_window(), Some(h1));

        let events = host.drain_lifecycle_events();
        assert!(events
            .iter()
            .any(|e| matches!(e, WindowLifecycleEvent::Opened { surface_id: 2, .. })));

        // Unmounting primary surface 1 cascades popover 3 while leaving popup 2 mounted.
        let removed = host.unmount_surface(1);
        assert_eq!(removed, vec![3, 1]);
        assert_eq!(host.active_surface_ids(), vec![2]);
        assert_eq!(host.surface_count(), 1);
    }
}
