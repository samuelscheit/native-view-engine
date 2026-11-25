//! Multi-OS-window lifecycle state machine (`WindowManager`), surface presentation modes
//! (`SurfacePresentation`), and direct typed [`WindowCommand`] / [`WindowLifecycleEvent`] APIs.

mod manager;

use std::fmt;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

pub use manager::WindowManager;

pub type WindowHandleId = u32;
pub type SizeBounds = (Option<(f32, f32)>, Option<(f32, f32)>);

pub(crate) const MAX_WINDOW_TITLE_BYTES: usize = 16_384;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowRect {
    pub x: f32,
    pub y: f32,
    pub width: f32,
    pub height: f32,
}

impl Default for WindowRect {
    fn default() -> Self {
        Self::new(0.0, 0.0, 800.0, 600.0)
    }
}

impl WindowRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self {
        Self { x, y, width, height }
    }

    pub const fn from_size(width: f32, height: f32) -> Self {
        Self::new(0.0, 0.0, width, height)
    }

    pub fn validate(&self, allow_zero: bool) -> Result<(), WindowError> {
        let min = if allow_zero { 0.0 } else { f32::EPSILON };
        if [self.x, self.y, self.width, self.height]
            .iter()
            .all(|v| v.is_finite())
            && self.width >= min
            && self.height >= min
        {
            Ok(())
        } else {
            Err(WindowError::InvalidDescriptor("invalid window rect".into()))
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    #[default]
    Normal = 0,
    Minimized = 1,
    Maximized = 2,
    Fullscreen = 3,
}

impl WindowState {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v {
            0 => Some(Self::Normal),
            1 => Some(Self::Minimized),
            2 => Some(Self::Maximized),
            3 => Some(Self::Fullscreen),
            _ => None,
        }
    }
}

nve_abi::define_bitflags! {
    #[derive(Serialize, Deserialize)]
    pub struct WindowFlags(u32) {
        const RESIZABLE = 1 << 0;
        const DECORATED = 1 << 1;
        const TRANSPARENT = 1 << 2;
        const ALWAYS_ON_TOP = 1 << 3;
        const MODAL = 1 << 4;
        const EXIT_ON_CLOSE = 1 << 5;
        const CLOSABLE = 1 << 6;
        const MINIMIZABLE = 1 << 7;
        const MAXIMIZABLE = 1 << 8;
    }
}

impl WindowFlags {
    pub const fn standard() -> Self {
        Self::RESIZABLE
            .union(Self::DECORATED)
            .union(Self::CLOSABLE)
            .union(Self::MINIMIZABLE)
            .union(Self::MAXIMIZABLE)
    }
}

impl Default for WindowFlags {
    fn default() -> Self {
        Self::standard()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WindowDescriptor {
    pub title: String,
    pub bounds: WindowRect,
    pub min_size: Option<(f32, f32)>,
    pub max_size: Option<(f32, f32)>,
    pub state: WindowState,
    pub flags: WindowFlags,
}

impl Default for WindowDescriptor {
    fn default() -> Self {
        Self {
            title: "NVE Window".to_owned(),
            bounds: WindowRect::default(),
            min_size: None,
            max_size: None,
            state: WindowState::Normal,
            flags: WindowFlags::standard(),
        }
    }
}

impl WindowDescriptor {
    pub fn new(title: impl Into<String>, w: f32, h: f32) -> Self {
        Self {
            title: title.into(),
            bounds: WindowRect::from_size(w, h),
            ..Default::default()
        }
    }
    pub fn with_bounds(mut self, b: WindowRect) -> Self {
        self.bounds = b;
        self
    }
    pub fn with_min_size(mut self, w: f32, h: f32) -> Self {
        self.min_size = Some((w, h));
        self
    }
    pub fn with_max_size(mut self, w: f32, h: f32) -> Self {
        self.max_size = Some((w, h));
        self
    }
    pub fn with_state(mut self, s: WindowState) -> Self {
        self.state = s;
        self
    }
    pub fn with_flags(mut self, f: WindowFlags) -> Self {
        self.flags = f;
        self
    }
    pub fn validate(&self) -> Result<(), WindowError> {
        if self.title.len() > MAX_WINDOW_TITLE_BYTES {
            return Err(WindowError::InvalidDescriptor("title too long".into()));
        }
        self.bounds.validate(false)?;
        validate_constraints(self.min_size, self.max_size)
    }
}

pub(crate) fn validate_constraints(
    min: Option<(f32, f32)>,
    max: Option<(f32, f32)>,
) -> Result<(), WindowError> {
    if min.is_some_and(|(w, h)| !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0) {
        return Err(WindowError::InvalidDescriptor("invalid min_size".into()));
    }
    if max.is_some_and(|(w, h)| !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0) {
        return Err(WindowError::InvalidDescriptor("invalid max_size".into()));
    }
    if let (Some((mw, mh)), Some((xw, xh))) = (min, max)
        && (mw > xw || mh > xh)
    {
        return Err(WindowError::InvalidDescriptor(
            "min_size exceeds max_size".into(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacePresentation {
    #[default]
    Embedded,
    PrimaryWindow(WindowDescriptor),
    SecondaryWindow {
        parent: Option<SurfaceId>,
        modal: bool,
        descriptor: WindowDescriptor,
    },
    UtilityPanel {
        parent: Option<SurfaceId>,
        always_on_top: bool,
        descriptor: WindowDescriptor,
    },
    Popover {
        anchor_surface: SurfaceId,
        anchor_rect: WindowRect,
        descriptor: WindowDescriptor,
    },
    Sheet {
        parent: SurfaceId,
        descriptor: WindowDescriptor,
    },
}

impl SurfacePresentation {
    pub fn is_embedded(&self) -> bool {
        matches!(self, Self::Embedded)
    }
    pub fn is_window(&self) -> bool {
        !self.is_embedded()
    }
    pub fn is_primary(&self) -> bool {
        matches!(self, Self::PrimaryWindow(_))
    }
    pub fn is_modal(&self) -> bool {
        match self {
            Self::SecondaryWindow {
                modal, descriptor, ..
            } => *modal || descriptor.flags.contains(WindowFlags::MODAL),
            Self::Sheet { descriptor, .. } => descriptor.flags.contains(WindowFlags::MODAL),
            _ => false,
        }
    }
    pub fn cascades_with_parent(&self) -> bool {
        matches!(
            self,
            Self::Sheet { .. }
                | Self::Popover { .. }
                | Self::UtilityPanel {
                    parent: Some(_),
                    ..
                }
                | Self::SecondaryWindow {
                    parent: Some(_),
                    modal: true,
                    ..
                }
        )
    }
    pub fn parent_surface(&self) -> Option<SurfaceId> {
        match self {
            Self::Embedded | Self::PrimaryWindow(_) => None,
            Self::SecondaryWindow { parent, .. } | Self::UtilityPanel { parent, .. } => *parent,
            Self::Popover { anchor_surface, .. } => Some(*anchor_surface),
            Self::Sheet { parent, .. } => Some(*parent),
        }
    }
    pub fn descriptor(&self) -> Option<&WindowDescriptor> {
        match self {
            Self::Embedded => None,
            Self::PrimaryWindow(d)
            | Self::SecondaryWindow { descriptor: d, .. }
            | Self::UtilityPanel { descriptor: d, .. }
            | Self::Popover { descriptor: d, .. }
            | Self::Sheet { descriptor: d, .. } => Some(d),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowLifecycleState {
    Open = 0,
    ClosePending = 1,
    Closed = 2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ManagedWindow {
    pub handle: WindowHandleId,
    pub surface_id: SurfaceId,
    pub presentation: SurfacePresentation,
    pub descriptor: WindowDescriptor,
    pub parent_window: Option<WindowHandleId>,
    pub children: Vec<WindowHandleId>,
    pub lifecycle: WindowLifecycleState,
    pub focused: bool,
    pub modal: bool,
    pub cascade_close: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowError {
    WindowNotFound(WindowHandleId),
    SurfaceAlreadyHasWindow(SurfaceId),
    ParentSurfaceNotWindow(SurfaceId),
    InvalidDescriptor(String),
    BlockedByModalChild {
        target: WindowHandleId,
        modal_child: WindowHandleId,
    },
    NotClosePending(WindowHandleId),
    WindowAlreadyClosed(WindowHandleId),
}

impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "window error: {self:?}")
    }
}

impl std::error::Error for WindowError {}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    OpenWindow {
        surface_id: SurfaceId,
        presentation: SurfacePresentation,
    },
    CloseWindow {
        handle: WindowHandleId,
    },
    SetTitle {
        handle: WindowHandleId,
        title: String,
    },
    SetBounds {
        handle: WindowHandleId,
        bounds: WindowRect,
    },
    SetSizeConstraints {
        handle: WindowHandleId,
        min_size: Option<(f32, f32)>,
        max_size: Option<(f32, f32)>,
    },
    SetWindowState {
        handle: WindowHandleId,
        state: WindowState,
    },
    FocusWindow {
        handle: WindowHandleId,
    },
    SetDecorations {
        handle: WindowHandleId,
        decorated: bool,
    },
    SetAlwaysOnTop {
        handle: WindowHandleId,
        always_on_top: bool,
    },
    RespondCloseRequest {
        handle: WindowHandleId,
        allow: bool,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowLifecycleEvent {
    Opened {
        handle: WindowHandleId,
        surface_id: SurfaceId,
        bounds: WindowRect,
        state: WindowState,
    },
    CloseRequested {
        handle: WindowHandleId,
        surface_id: SurfaceId,
    },
    Closed {
        handle: WindowHandleId,
        surface_id: SurfaceId,
    },
    Focused {
        handle: WindowHandleId,
        focused: bool,
    },
    Moved {
        handle: WindowHandleId,
        x: f32,
        y: f32,
    },
    Resized {
        handle: WindowHandleId,
        width: f32,
        height: f32,
        scale_factor: f32,
    },
    StateChanged {
        handle: WindowHandleId,
        state: WindowState,
    },
}
