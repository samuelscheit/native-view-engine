//! Multi-OS-window lifecycle state machine (`WindowManager`), surface presentation modes
//! (`SurfacePresentation`), and zero-JSON little-endian binary `NVW1` command/event codec.

use std::collections::BTreeMap;
use std::fmt;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

pub type WindowHandleId = u32;
pub type SizeBounds = (Option<(f32, f32)>, Option<(f32, f32)>);

pub const NVW1_MAGIC: [u8; 4] = *b"NVW1";
pub const NVW1_VERSION: u16 = 1;
pub const NVW1_HEADER_SIZE: usize = 12;
const NVW1_KIND_COMMANDS: u8 = 1;
const NVW1_KIND_EVENTS: u8 = 2;
const MAX_NVW1_RECORDS: u32 = 65_536;
const MAX_NVW1_TITLE_BYTES: usize = 16_384;

#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct WindowRect { pub x: f32, pub y: f32, pub width: f32, pub height: f32 }
impl Default for WindowRect {
    fn default() -> Self { Self::new(0.0, 0.0, 800.0, 600.0) }
}
impl WindowRect {
    pub const fn new(x: f32, y: f32, width: f32, height: f32) -> Self { Self { x, y, width, height } }
    pub const fn from_size(width: f32, height: f32) -> Self { Self::new(0.0, 0.0, width, height) }
    pub fn validate(&self, allow_zero: bool) -> Result<(), WindowError> {
        let min = if allow_zero { 0.0 } else { f32::EPSILON };
        if [self.x, self.y, self.width, self.height].iter().all(|v| v.is_finite())
            && self.width >= min && self.height >= min
        { Ok(()) } else { Err(WindowError::InvalidDescriptor("invalid window rect".into())) }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState { #[default] Normal = 0, Minimized = 1, Maximized = 2, Fullscreen = 3 }
impl WindowState {
    pub fn from_u8(v: u8) -> Option<Self> {
        match v { 0 => Some(Self::Normal), 1 => Some(Self::Minimized), 2 => Some(Self::Maximized), 3 => Some(Self::Fullscreen), _ => None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WindowFlags(pub u32);
impl WindowFlags {
    pub const RESIZABLE: u32 = 1 << 0;
    pub const DECORATED: u32 = 1 << 1;
    pub const TRANSPARENT: u32 = 1 << 2;
    pub const ALWAYS_ON_TOP: u32 = 1 << 3;
    pub const MODAL: u32 = 1 << 4;
    pub const EXIT_ON_CLOSE: u32 = 1 << 5;
    pub const CLOSABLE: u32 = 1 << 6;
    pub const MINIMIZABLE: u32 = 1 << 7;
    pub const MAXIMIZABLE: u32 = 1 << 8;
    pub const fn empty() -> Self { Self(0) }
    pub const fn standard() -> Self { Self(Self::RESIZABLE | Self::DECORATED | Self::CLOSABLE | Self::MINIMIZABLE | Self::MAXIMIZABLE) }
    pub const fn bits(self) -> u32 { self.0 }
    pub const fn contains(self, f: u32) -> bool { (self.0 & f) == f }
    pub const fn with(self, f: u32) -> Self { Self(self.0 | f) }
    pub const fn without(self, f: u32) -> Self { Self(self.0 & !f) }
    pub fn set(&mut self, f: u32, on: bool) { if on { self.0 |= f; } else { self.0 &= !f; } }
}
impl Default for WindowFlags { fn default() -> Self { Self::standard() } }

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
        Self { title: "NVE Window".to_owned(), bounds: WindowRect::default(), min_size: None, max_size: None, state: WindowState::Normal, flags: WindowFlags::standard() }
    }
}
impl WindowDescriptor {
    pub fn new(title: impl Into<String>, w: f32, h: f32) -> Self { Self { title: title.into(), bounds: WindowRect::from_size(w, h), ..Default::default() } }
    pub fn with_bounds(mut self, b: WindowRect) -> Self { self.bounds = b; self }
    pub fn with_min_size(mut self, w: f32, h: f32) -> Self { self.min_size = Some((w, h)); self }
    pub fn with_max_size(mut self, w: f32, h: f32) -> Self { self.max_size = Some((w, h)); self }
    pub fn with_state(mut self, s: WindowState) -> Self { self.state = s; self }
    pub fn with_flags(mut self, f: WindowFlags) -> Self { self.flags = f; self }
    pub fn validate(&self) -> Result<(), WindowError> {
        if self.title.len() > MAX_NVW1_TITLE_BYTES { return Err(WindowError::InvalidDescriptor("title too long".into())); }
        self.bounds.validate(false)?;
        validate_constraints(self.min_size, self.max_size)
    }
}

fn validate_constraints(min: Option<(f32, f32)>, max: Option<(f32, f32)>) -> Result<(), WindowError> {
    if min.is_some_and(|(w, h)| !w.is_finite() || !h.is_finite() || w < 0.0 || h < 0.0) { return Err(WindowError::InvalidDescriptor("invalid min_size".into())); }
    if max.is_some_and(|(w, h)| !w.is_finite() || !h.is_finite() || w <= 0.0 || h <= 0.0) { return Err(WindowError::InvalidDescriptor("invalid max_size".into())); }
    if let (Some((mw, mh)), Some((xw, xh))) = (min, max) && (mw > xw || mh > xh) { return Err(WindowError::InvalidDescriptor("min_size exceeds max_size".into())); }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurfacePresentation {
    #[default]
    Embedded,
    PrimaryWindow(WindowDescriptor),
    SecondaryWindow { parent: Option<SurfaceId>, modal: bool, descriptor: WindowDescriptor },
    UtilityPanel { parent: Option<SurfaceId>, always_on_top: bool, descriptor: WindowDescriptor },
    Popover { anchor_surface: SurfaceId, anchor_rect: WindowRect, descriptor: WindowDescriptor },
    Sheet { parent: SurfaceId, descriptor: WindowDescriptor },
}
impl SurfacePresentation {
    pub fn is_embedded(&self) -> bool { matches!(self, Self::Embedded) }
    pub fn is_window(&self) -> bool { !self.is_embedded() }
    pub fn is_primary(&self) -> bool { matches!(self, Self::PrimaryWindow(_)) }
    pub fn is_modal(&self) -> bool {
        match self {
            Self::SecondaryWindow { modal, descriptor, .. } => *modal || descriptor.flags.contains(WindowFlags::MODAL),
            Self::Sheet { descriptor, .. } => descriptor.flags.contains(WindowFlags::MODAL),
            _ => false,
        }
    }
    pub fn cascades_with_parent(&self) -> bool {
        matches!(self, Self::Sheet { .. } | Self::Popover { .. } | Self::UtilityPanel { parent: Some(_), .. } | Self::SecondaryWindow { parent: Some(_), modal: true, .. })
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
            Self::PrimaryWindow(d) | Self::SecondaryWindow { descriptor: d, .. } | Self::UtilityPanel { descriptor: d, .. } | Self::Popover { descriptor: d, .. } | Self::Sheet { descriptor: d, .. } => Some(d),
        }
    }
}

#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowLifecycleState { Open = 0, ClosePending = 1, Closed = 2 }

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
pub enum WindowCodecError {
    Truncated { expected: usize, actual: usize },
    BadMagic([u8; 4]),
    UnsupportedVersion(u16),
    InvalidPayloadKind(u8),
    InvalidOpcode(u8),
    InvalidEnumValue { field: &'static str, value: u8 },
    InvalidUtf8,
    NonFiniteFloat(&'static str),
    InvalidDimensions(&'static str),
    TrailingBytes(usize),
    TooManyRecords(u32),
    TitleTooLong(usize),
}
impl fmt::Display for WindowCodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "NVW1 codec error: {self:?}") }
}
impl std::error::Error for WindowCodecError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WindowError {
    WindowNotFound(WindowHandleId),
    SurfaceAlreadyHasWindow(SurfaceId),
    ParentSurfaceNotWindow(SurfaceId),
    InvalidDescriptor(String),
    BlockedByModalChild { target: WindowHandleId, modal_child: WindowHandleId },
    NotClosePending(WindowHandleId),
    WindowAlreadyClosed(WindowHandleId),
    Codec(WindowCodecError),
}
impl fmt::Display for WindowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result { write!(f, "window error: {self:?}") }
}
impl std::error::Error for WindowError {}
impl From<WindowCodecError> for WindowError {
    fn from(v: WindowCodecError) -> Self { Self::Codec(v) }
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowCommand {
    OpenWindow { surface_id: SurfaceId, presentation: SurfacePresentation },
    CloseWindow { handle: WindowHandleId },
    SetTitle { handle: WindowHandleId, title: String },
    SetBounds { handle: WindowHandleId, bounds: WindowRect },
    SetSizeConstraints { handle: WindowHandleId, min_size: Option<(f32, f32)>, max_size: Option<(f32, f32)> },
    SetWindowState { handle: WindowHandleId, state: WindowState },
    FocusWindow { handle: WindowHandleId },
    SetDecorations { handle: WindowHandleId, decorated: bool },
    SetAlwaysOnTop { handle: WindowHandleId, always_on_top: bool },
    RespondCloseRequest { handle: WindowHandleId, allow: bool },
}

#[derive(Debug, Clone, PartialEq)]
pub enum WindowLifecycleEvent {
    Opened { handle: WindowHandleId, surface_id: SurfaceId, bounds: WindowRect, state: WindowState },
    CloseRequested { handle: WindowHandleId, surface_id: SurfaceId },
    Closed { handle: WindowHandleId, surface_id: SurfaceId },
    Focused { handle: WindowHandleId, focused: bool },
    Moved { handle: WindowHandleId, x: f32, y: f32 },
    Resized { handle: WindowHandleId, width: f32, height: f32, scale_factor: f32 },
    StateChanged { handle: WindowHandleId, state: WindowState },
}

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
    pub fn new() -> Self { Self { next_handle: 1, ..Default::default() } }

    pub fn open_window(&mut self, surface_id: SurfaceId, presentation: SurfacePresentation) -> Result<WindowHandleId, WindowError> {
        if self.surface_to_window.contains_key(&surface_id) {
            return Err(WindowError::SurfaceAlreadyHasWindow(surface_id));
        }
        let mut descriptor = presentation.descriptor()
            .ok_or_else(|| WindowError::InvalidDescriptor("Embedded presentation cannot open a window".into()))?
            .clone();
        descriptor.validate()?;
        if let SurfacePresentation::Popover { anchor_rect, .. } = &presentation { anchor_rect.validate(true)?; }
        let parent_window = match presentation.parent_surface() {
            Some(ps) => {
                let ph = *self.surface_to_window.get(&ps).ok_or(WindowError::ParentSurfaceNotWindow(ps))?;
                if self.windows.get(&ph).is_none_or(|w| w.lifecycle == WindowLifecycleState::Closed) {
                    return Err(WindowError::ParentSurfaceNotWindow(ps));
                }
                Some(ph)
            }
            None => None,
        };
        let (modal, cascade_close) = (presentation.is_modal(), presentation.cascades_with_parent());
        if modal { descriptor.flags.set(WindowFlags::MODAL, true); }
        if matches!(presentation, SurfacePresentation::UtilityPanel { always_on_top: true, .. }) {
            descriptor.flags.set(WindowFlags::ALWAYS_ON_TOP, true);
        }
        self.next_handle = self.next_handle.max(1);
        while self.windows.contains_key(&self.next_handle) { self.next_handle = self.next_handle.saturating_add(1); }
        let handle = self.next_handle;
        self.next_handle = self.next_handle.saturating_add(1);
        let (is_primary, bounds, state) = (presentation.is_primary(), descriptor.bounds, descriptor.state);
        if let Some(ph) = parent_window && let Some(p) = self.windows.get_mut(&ph) { p.children.push(handle); }
        self.windows.insert(handle, ManagedWindow {
            handle, surface_id, presentation, descriptor, parent_window,
            children: Vec::new(), lifecycle: WindowLifecycleState::Open, focused: false, modal, cascade_close,
        });
        self.surface_to_window.insert(surface_id, handle);
        if is_primary || self.primary_window.is_none() { self.primary_window = Some(handle); }
        self.events.push(WindowLifecycleEvent::Opened { handle, surface_id, bounds, state });
        self.set_focus_internal(Some(handle));
        Ok(handle)
    }

    pub fn active_modal_blocker(&self, handle: WindowHandleId) -> Option<WindowHandleId> {
        let win = self.windows.get(&handle)?;
        for &cid in win.children.iter().rev() {
            if let Some(deeper) = self.active_modal_blocker(cid) { return Some(deeper); }
            if let Some(c) = self.windows.get(&cid) && c.modal && c.lifecycle != WindowLifecycleState::Closed {
                return Some(cid);
            }
        }
        None
    }

    pub fn focus_window(&mut self, handle: WindowHandleId) -> Result<(), WindowError> {
        let win = self.windows.get(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        if win.lifecycle == WindowLifecycleState::Closed { return Err(WindowError::WindowAlreadyClosed(handle)); }
        if let Some(modal_child) = self.active_modal_blocker(handle) {
            return Err(WindowError::BlockedByModalChild { target: handle, modal_child });
        }
        self.set_focus_internal(Some(handle));
        Ok(())
    }

    pub fn set_title(&mut self, handle: WindowHandleId, title: String) -> Result<(), WindowError> {
        if title.len() > MAX_NVW1_TITLE_BYTES { return Err(WindowError::InvalidDescriptor("title too long".into())); }
        self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?.descriptor.title = title;
        Ok(())
    }

    pub fn set_bounds(&mut self, handle: WindowHandleId, bounds: WindowRect) -> Result<(), WindowError> {
        self.set_bounds_with_scale(handle, bounds, 1.0)
    }

    pub fn set_bounds_with_scale(&mut self, handle: WindowHandleId, mut bounds: WindowRect, scale: f32) -> Result<(), WindowError> {
        bounds.validate(false)?;
        if !scale.is_finite() || scale <= 0.0 { return Err(WindowError::InvalidDescriptor("invalid scale".into())); }
        let win = self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        if let Some((mw, mh)) = win.descriptor.min_size { bounds.width = bounds.width.max(mw); bounds.height = bounds.height.max(mh); }
        if let Some((xw, xh)) = win.descriptor.max_size { bounds.width = bounds.width.min(xw); bounds.height = bounds.height.min(xh); }
        let prev = win.descriptor.bounds;
        win.descriptor.bounds = bounds;
        if (prev.x - bounds.x).abs() > f32::EPSILON || (prev.y - bounds.y).abs() > f32::EPSILON {
            self.events.push(WindowLifecycleEvent::Moved { handle, x: bounds.x, y: bounds.y });
        }
        if (prev.width - bounds.width).abs() > f32::EPSILON || (prev.height - bounds.height).abs() > f32::EPSILON {
            self.events.push(WindowLifecycleEvent::Resized { handle, width: bounds.width, height: bounds.height, scale_factor: scale });
        }
        Ok(())
    }

    pub fn set_size_constraints(&mut self, handle: WindowHandleId, min: Option<(f32, f32)>, max: Option<(f32, f32)>) -> Result<(), WindowError> {
        validate_constraints(min, max)?;
        let win = self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        win.descriptor.min_size = min; win.descriptor.max_size = max; Ok(())
    }
    pub fn set_window_state(&mut self, handle: WindowHandleId, state: WindowState) -> Result<(), WindowError> {
        let win = self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        if win.descriptor.state != state { win.descriptor.state = state; self.events.push(WindowLifecycleEvent::StateChanged { handle, state }); }
        Ok(())
    }
    pub fn set_decorations(&mut self, handle: WindowHandleId, decorated: bool) -> Result<(), WindowError> {
        self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?.descriptor.flags.set(WindowFlags::DECORATED, decorated); Ok(())
    }
    pub fn set_always_on_top(&mut self, handle: WindowHandleId, on_top: bool) -> Result<(), WindowError> {
        self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?.descriptor.flags.set(WindowFlags::ALWAYS_ON_TOP, on_top); Ok(())
    }
    pub fn request_close(&mut self, handle: WindowHandleId) -> Result<bool, WindowError> {
        let win = self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        match win.lifecycle {
            WindowLifecycleState::Open => { win.lifecycle = WindowLifecycleState::ClosePending; self.events.push(WindowLifecycleEvent::CloseRequested { handle, surface_id: win.surface_id }); Ok(true) }
            WindowLifecycleState::ClosePending => Ok(false),
            WindowLifecycleState::Closed => Err(WindowError::WindowAlreadyClosed(handle)),
        }
    }
    pub fn respond_close_request(&mut self, handle: WindowHandleId, allow: bool) -> Result<Vec<WindowHandleId>, WindowError> {
        let win = self.windows.get_mut(&handle).ok_or(WindowError::WindowNotFound(handle))?;
        if win.lifecycle != WindowLifecycleState::ClosePending { return Err(WindowError::NotClosePending(handle)); }
        if !allow { win.lifecycle = WindowLifecycleState::Open; return Ok(Vec::new()); }
        self.close_window(handle)
    }

    pub fn close_window(&mut self, handle: WindowHandleId) -> Result<Vec<WindowHandleId>, WindowError> {
        if !self.windows.contains_key(&handle) { return Err(WindowError::WindowNotFound(handle)); }
        let mut order = Vec::new();
        self.collect_cascade_post_order(handle, &mut order);
        let fallback_parent = self.windows.get(&handle).and_then(|w| w.parent_window);
        let (mut closed, mut focus_closed) = (Vec::with_capacity(order.len()), false);
        for h in order {
            if let Some(win) = self.windows.remove(&h) {
                if let Some(pid) = win.parent_window && let Some(p) = self.windows.get_mut(&pid) { p.children.retain(|&c| c != h); }
                for cid in &win.children { if let Some(c) = self.windows.get_mut(cid) { c.parent_window = None; } }
                self.surface_to_window.remove(&win.surface_id);
                if self.focused_window == Some(h) { self.focused_window = None; focus_closed = true; }
                if self.primary_window == Some(h) { self.primary_window = None; }
                self.events.push(WindowLifecycleEvent::Closed { handle: h, surface_id: win.surface_id });
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

    pub fn apply_command(&mut self, cmd: WindowCommand) -> Result<Option<WindowHandleId>, WindowError> {
        match cmd {
            WindowCommand::OpenWindow { surface_id, presentation } => Ok(Some(self.open_window(surface_id, presentation)?)),
            WindowCommand::CloseWindow { handle } => { self.close_window(handle)?; Ok(None) }
            WindowCommand::SetTitle { handle, title } => { self.set_title(handle, title)?; Ok(None) }
            WindowCommand::SetBounds { handle, bounds } => { self.set_bounds(handle, bounds)?; Ok(None) }
            WindowCommand::SetSizeConstraints { handle, min_size, max_size } => { self.set_size_constraints(handle, min_size, max_size)?; Ok(None) }
            WindowCommand::SetWindowState { handle, state } => { self.set_window_state(handle, state)?; Ok(None) }
            WindowCommand::FocusWindow { handle } => { self.focus_window(handle)?; Ok(None) }
            WindowCommand::SetDecorations { handle, decorated } => { self.set_decorations(handle, decorated)?; Ok(None) }
            WindowCommand::SetAlwaysOnTop { handle, always_on_top } => { self.set_always_on_top(handle, always_on_top)?; Ok(None) }
            WindowCommand::RespondCloseRequest { handle, allow } => { self.respond_close_request(handle, allow)?; Ok(None) }
        }
    }

    fn collect_cascade_post_order(&self, handle: WindowHandleId, out: &mut Vec<WindowHandleId>) {
        if let Some(win) = self.windows.get(&handle) {
            for &cid in &win.children {
                if self.windows.get(&cid).is_some_and(|c| c.cascade_close) { self.collect_cascade_post_order(cid, out); }
            }
            out.push(handle);
        }
    }

    fn set_focus_internal(&mut self, next: Option<WindowHandleId>) {
        if self.focused_window == next { return; }
        if let Some(prev) = self.focused_window {
            if let Some(w) = self.windows.get_mut(&prev) { w.focused = false; }
            self.events.push(WindowLifecycleEvent::Focused { handle: prev, focused: false });
        }
        self.focused_window = next;
        if let Some(cur) = next {
            if let Some(w) = self.windows.get_mut(&cur) { w.focused = true; }
            self.events.push(WindowLifecycleEvent::Focused { handle: cur, focused: true });
        }
    }

    pub fn get(&self, h: WindowHandleId) -> Option<&ManagedWindow> { self.windows.get(&h) }
    pub fn handle_for_surface(&self, s: SurfaceId) -> Option<WindowHandleId> { self.surface_to_window.get(&s).copied() }
    pub fn get_by_surface(&self, s: SurfaceId) -> Option<&ManagedWindow> { self.handle_for_surface(s).and_then(|h| self.windows.get(&h)) }
    pub fn focused_window(&self) -> Option<WindowHandleId> { self.focused_window }
    pub fn primary_window(&self) -> Option<WindowHandleId> { self.primary_window }
    pub fn open_window_count(&self) -> usize { self.windows.len() }
    pub fn is_empty(&self) -> bool { self.windows.is_empty() }
    pub fn windows(&self) -> &BTreeMap<WindowHandleId, ManagedWindow> { &self.windows }
    pub fn drain_events(&mut self) -> Vec<WindowLifecycleEvent> { std::mem::take(&mut self.events) }
}

// ============================================================================
// Zero-JSON Little-Endian Binary `NVW1` Codec
// ============================================================================

fn push_u32(b: &mut Vec<u8>, v: u32) { b.extend_from_slice(&v.to_le_bytes()); }
fn push_f32(b: &mut Vec<u8>, v: f32) { b.extend_from_slice(&v.to_le_bytes()); }
fn push_header(b: &mut Vec<u8>, kind: u8, count: usize) {
    b.extend_from_slice(&NVW1_MAGIC);
    b.extend_from_slice(&NVW1_VERSION.to_le_bytes());
    b.extend_from_slice(&[kind, 0]);
    push_u32(b, count as u32);
}
fn push_rect(b: &mut Vec<u8>, r: WindowRect) { for v in [r.x, r.y, r.width, r.height] { push_f32(b, v); } }
fn push_constraints(b: &mut Vec<u8>, min: Option<(f32, f32)>, max: Option<(f32, f32)>) {
    b.push(u8::from(min.is_some()) | (u8::from(max.is_some()) << 1));
    if let Some((w, h)) = min { push_f32(b, w); push_f32(b, h); }
    if let Some((w, h)) = max { push_f32(b, w); push_f32(b, h); }
}
fn push_str(b: &mut Vec<u8>, s: &str) { push_u32(b, s.len() as u32); b.extend_from_slice(s.as_bytes()); }
fn push_descriptor(b: &mut Vec<u8>, d: &WindowDescriptor) {
    push_str(b, &d.title); push_rect(b, d.bounds); push_constraints(b, d.min_size, d.max_size);
    b.push(d.state as u8); push_u32(b, d.flags.bits());
}

pub fn encode_window_commands(commands: &[WindowCommand]) -> Vec<u8> {
    let mut b = Vec::with_capacity(NVW1_HEADER_SIZE + commands.len() * 32);
    push_header(&mut b, NVW1_KIND_COMMANDS, commands.len());
    let z = WindowRect::new(0.0, 0.0, 0.0, 0.0);
    for cmd in commands {
        match cmd {
            WindowCommand::OpenWindow { surface_id, presentation } => {
                b.push(1); push_u32(&mut b, *surface_id);
                let (k, p, ex, a, d) = match presentation {
                    SurfacePresentation::Embedded => (0u8, 0u32, 0u8, z, None),
                    SurfacePresentation::PrimaryWindow(d) => (1, 0, 0, z, Some(d)),
                    SurfacePresentation::SecondaryWindow { parent, modal, descriptor } => (2, parent.unwrap_or(0), u8::from(*modal), z, Some(descriptor)),
                    SurfacePresentation::UtilityPanel { parent, always_on_top, descriptor } => (3, parent.unwrap_or(0), u8::from(*always_on_top), z, Some(descriptor)),
                    SurfacePresentation::Popover { anchor_surface, anchor_rect, descriptor } => (4, *anchor_surface, 0, *anchor_rect, Some(descriptor)),
                    SurfacePresentation::Sheet { parent, descriptor } => (5, *parent, 0, z, Some(descriptor)),
                };
                b.push(k); push_u32(&mut b, p); b.push(ex); push_rect(&mut b, a);
                push_descriptor(&mut b, d.unwrap_or(&WindowDescriptor::default()));
            }
            WindowCommand::CloseWindow { handle } => { b.push(2); push_u32(&mut b, *handle); }
            WindowCommand::SetTitle { handle, title } => { b.push(3); push_u32(&mut b, *handle); push_str(&mut b, title); }
            WindowCommand::SetBounds { handle, bounds } => { b.push(4); push_u32(&mut b, *handle); push_rect(&mut b, *bounds); }
            WindowCommand::SetSizeConstraints { handle, min_size, max_size } => { b.push(5); push_u32(&mut b, *handle); push_constraints(&mut b, *min_size, *max_size); }
            WindowCommand::SetWindowState { handle, state } => { b.push(6); push_u32(&mut b, *handle); b.push(*state as u8); }
            WindowCommand::FocusWindow { handle } => { b.push(7); push_u32(&mut b, *handle); }
            WindowCommand::SetDecorations { handle, decorated } => { b.push(8); push_u32(&mut b, *handle); b.push(u8::from(*decorated)); }
            WindowCommand::SetAlwaysOnTop { handle, always_on_top } => { b.push(9); push_u32(&mut b, *handle); b.push(u8::from(*always_on_top)); }
            WindowCommand::RespondCloseRequest { handle, allow } => { b.push(10); push_u32(&mut b, *handle); b.push(u8::from(*allow)); }
        }
    }
    b
}

pub fn encode_window_events(events: &[WindowLifecycleEvent]) -> Vec<u8> {
    let mut b = Vec::with_capacity(NVW1_HEADER_SIZE + events.len() * 24);
    push_header(&mut b, NVW1_KIND_EVENTS, events.len());
    for ev in events {
        match ev {
            WindowLifecycleEvent::Opened { handle, surface_id, bounds, state } => { b.push(1); push_u32(&mut b, *handle); push_u32(&mut b, *surface_id); push_rect(&mut b, *bounds); b.push(*state as u8); }
            WindowLifecycleEvent::CloseRequested { handle, surface_id } => { b.push(2); push_u32(&mut b, *handle); push_u32(&mut b, *surface_id); }
            WindowLifecycleEvent::Closed { handle, surface_id } => { b.push(3); push_u32(&mut b, *handle); push_u32(&mut b, *surface_id); }
            WindowLifecycleEvent::Focused { handle, focused } => { b.push(4); push_u32(&mut b, *handle); b.push(u8::from(*focused)); }
            WindowLifecycleEvent::Moved { handle, x, y } => { b.push(5); push_u32(&mut b, *handle); push_f32(&mut b, *x); push_f32(&mut b, *y); }
            WindowLifecycleEvent::Resized { handle, width, height, scale_factor } => { b.push(6); push_u32(&mut b, *handle); push_f32(&mut b, *width); push_f32(&mut b, *height); push_f32(&mut b, *scale_factor); }
            WindowLifecycleEvent::StateChanged { handle, state } => { b.push(7); push_u32(&mut b, *handle); b.push(*state as u8); }
        }
    }
    b
}

struct Cursor<'a> { data: &'a [u8], pos: usize }

impl<'a> Cursor<'a> {
    fn new(data: &'a [u8]) -> Self { Self { data, pos: 0 } }
    fn remaining(&self) -> usize { self.data.len().saturating_sub(self.pos) }
    fn take(&mut self, n: usize) -> Result<&'a [u8], WindowCodecError> {
        let end = self.pos.checked_add(n).ok_or(WindowCodecError::Truncated { expected: usize::MAX, actual: self.data.len() })?;
        if end > self.data.len() { return Err(WindowCodecError::Truncated { expected: end, actual: self.data.len() }); }
        let s = &self.data[self.pos..end]; self.pos = end; Ok(s)
    }
    fn u8(&mut self) -> Result<u8, WindowCodecError> { Ok(self.take(1)?[0]) }
    fn u16(&mut self) -> Result<u16, WindowCodecError> { let b = self.take(2)?; Ok(u16::from_le_bytes([b[0], b[1]])) }
    fn u32(&mut self) -> Result<u32, WindowCodecError> { let b = self.take(4)?; Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]])) }
    fn f32(&mut self, f: &'static str) -> Result<f32, WindowCodecError> {
        let b = self.take(4)?;
        let v = f32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        v.is_finite().then_some(v).ok_or(WindowCodecError::NonFiniteFloat(f))
    }
    fn bool(&mut self, field: &'static str) -> Result<bool, WindowCodecError> {
        match self.u8()? { 0 => Ok(false), 1 => Ok(true), value => Err(WindowCodecError::InvalidEnumValue { field, value }) }
    }
    fn state(&mut self) -> Result<WindowState, WindowCodecError> {
        let value = self.u8()?;
        WindowState::from_u8(value).ok_or(WindowCodecError::InvalidEnumValue { field: "window_state", value })
    }
    fn rect(&mut self, f: &'static str, allow_zero: bool) -> Result<WindowRect, WindowCodecError> {
        let r = WindowRect::new(self.f32(f)?, self.f32(f)?, self.f32(f)?, self.f32(f)?);
        let min = if allow_zero { 0.0 } else { f32::EPSILON };
        (r.width >= min && r.height >= min).then_some(r).ok_or(WindowCodecError::InvalidDimensions(f))
    }
    fn constraints(&mut self) -> Result<SizeBounds, WindowCodecError> {
        let mask = self.u8()?;
        if mask & !0x03 != 0 { return Err(WindowCodecError::InvalidEnumValue { field: "size_constraints_mask", value: mask }); }
        let min = if mask & 1 != 0 {
            let (w, h) = (self.f32("min_w")?, self.f32("min_h")?);
            if w < 0.0 || h < 0.0 { return Err(WindowCodecError::InvalidDimensions("min_size")); }
            Some((w, h))
        } else { None };
        let max = if mask & 2 != 0 {
            let (w, h) = (self.f32("max_w")?, self.f32("max_h")?);
            if w <= 0.0 || h <= 0.0 { return Err(WindowCodecError::InvalidDimensions("max_size")); }
            Some((w, h))
        } else { None };
        if let (Some((mw, mh)), Some((xw, xh))) = (min, max) && (mw > xw || mh > xh) {
            return Err(WindowCodecError::InvalidDimensions("min_exceeds_max"));
        }
        Ok((min, max))
    }
    fn string(&mut self) -> Result<String, WindowCodecError> {
        let len = self.u32()? as usize;
        if len > MAX_NVW1_TITLE_BYTES { return Err(WindowCodecError::TitleTooLong(len)); }
        std::str::from_utf8(self.take(len)?).map(str::to_owned).map_err(|_| WindowCodecError::InvalidUtf8)
    }
    fn descriptor(&mut self) -> Result<WindowDescriptor, WindowCodecError> {
        let (title, bounds) = (self.string()?, self.rect("bounds", false)?);
        let (min_size, max_size) = self.constraints()?;
        Ok(WindowDescriptor { title, bounds, min_size, max_size, state: self.state()?, flags: WindowFlags(self.u32()?) })
    }
    fn read_header(&mut self, expected_kind: u8) -> Result<u32, WindowCodecError> {
        let m = self.take(4)?;
        if m != NVW1_MAGIC { return Err(WindowCodecError::BadMagic([m[0], m[1], m[2], m[3]])); }
        let ver = self.u16()?;
        if ver != NVW1_VERSION { return Err(WindowCodecError::UnsupportedVersion(ver)); }
        let kind = self.u8()?;
        if kind != expected_kind { return Err(WindowCodecError::InvalidPayloadKind(kind)); }
        let _ = self.u8()?;
        let count = self.u32()?;
        (count <= MAX_NVW1_RECORDS).then_some(count).ok_or(WindowCodecError::TooManyRecords(count))
    }
}

pub fn decode_window_commands(bytes: &[u8]) -> Result<Vec<WindowCommand>, WindowCodecError> {
    let mut cur = Cursor::new(bytes);
    let count = cur.read_header(NVW1_KIND_COMMANDS)?;
    let mut out = Vec::with_capacity((count as usize).min(256));
    for _ in 0..count {
        out.push(match cur.u8()? {
            1 => {
                let (surface_id, kind, parent_raw, extra) = (cur.u32()?, cur.u8()?, cur.u32()?, cur.bool("presentation_flag")?);
                let (anchor_rect, descriptor) = (cur.rect("anchor_rect", true)?, cur.descriptor()?);
                let parent = (parent_raw != 0).then_some(parent_raw);
                let presentation = match kind {
                    0 => SurfacePresentation::Embedded,
                    1 => SurfacePresentation::PrimaryWindow(descriptor),
                    2 => SurfacePresentation::SecondaryWindow { parent, modal: extra, descriptor },
                    3 => SurfacePresentation::UtilityPanel { parent, always_on_top: extra, descriptor },
                    4 => SurfacePresentation::Popover { anchor_surface: parent_raw, anchor_rect, descriptor },
                    5 => SurfacePresentation::Sheet { parent: parent_raw, descriptor },
                    value => return Err(WindowCodecError::InvalidEnumValue { field: "presentation_kind", value }),
                };
                WindowCommand::OpenWindow { surface_id, presentation }
            }
            2 => WindowCommand::CloseWindow { handle: cur.u32()? },
            3 => WindowCommand::SetTitle { handle: cur.u32()?, title: cur.string()? },
            4 => WindowCommand::SetBounds { handle: cur.u32()?, bounds: cur.rect("bounds", false)? },
            5 => { let handle = cur.u32()?; let (min_size, max_size) = cur.constraints()?; WindowCommand::SetSizeConstraints { handle, min_size, max_size } }
            6 => WindowCommand::SetWindowState { handle: cur.u32()?, state: cur.state()? },
            7 => WindowCommand::FocusWindow { handle: cur.u32()? },
            8 => WindowCommand::SetDecorations { handle: cur.u32()?, decorated: cur.bool("decorated")? },
            9 => WindowCommand::SetAlwaysOnTop { handle: cur.u32()?, always_on_top: cur.bool("always_on_top")? },
            10 => WindowCommand::RespondCloseRequest { handle: cur.u32()?, allow: cur.bool("allow")? },
            op => return Err(WindowCodecError::InvalidOpcode(op)),
        });
    }
    (cur.remaining() == 0).then_some(out).ok_or(WindowCodecError::TrailingBytes(cur.remaining()))
}

pub fn decode_window_events(bytes: &[u8]) -> Result<Vec<WindowLifecycleEvent>, WindowCodecError> {
    let mut cur = Cursor::new(bytes);
    let count = cur.read_header(NVW1_KIND_EVENTS)?;
    let mut out = Vec::with_capacity((count as usize).min(256));
    for _ in 0..count {
        out.push(match cur.u8()? {
            1 => WindowLifecycleEvent::Opened { handle: cur.u32()?, surface_id: cur.u32()?, bounds: cur.rect("bounds", false)?, state: cur.state()? },
            2 => WindowLifecycleEvent::CloseRequested { handle: cur.u32()?, surface_id: cur.u32()? },
            3 => WindowLifecycleEvent::Closed { handle: cur.u32()?, surface_id: cur.u32()? },
            4 => WindowLifecycleEvent::Focused { handle: cur.u32()?, focused: cur.bool("focused")? },
            5 => WindowLifecycleEvent::Moved { handle: cur.u32()?, x: cur.f32("x")?, y: cur.f32("y")? },
            6 => {
                let (handle, width, height, scale_factor) = (cur.u32()?, cur.f32("width")?, cur.f32("height")?, cur.f32("scale_factor")?);
                if width <= 0.0 || height <= 0.0 || scale_factor <= 0.0 { return Err(WindowCodecError::InvalidDimensions("resized")); }
                WindowLifecycleEvent::Resized { handle, width, height, scale_factor }
            }
            7 => WindowLifecycleEvent::StateChanged { handle: cur.u32()?, state: cur.state()? },
            op => return Err(WindowCodecError::InvalidOpcode(op)),
        });
    }
    (cur.remaining() == 0).then_some(out).ok_or(WindowCodecError::TrailingBytes(cur.remaining()))
}
