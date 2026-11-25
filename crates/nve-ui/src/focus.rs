//! Deterministic focus management and keyboard navigation engine (`FocusScope`, `FocusTrap`,
//! and `RovingFocus`) for accessible dialogs, popovers, tabs, menus, and toolbars.

use dioxus_core::{provide_context, try_consume_context, use_hook, Element};
use dioxus_core_macro::{component, rsx, Props};
use dioxus_signals::{ReadableExt, Signal, WritableExt};

/// Axis orientation for roving keyboard navigation and layout primitives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Orientation {
    #[default]
    Horizontal,
    Vertical,
}

/// Ordered entry registered inside a [`FocusScope`] or [`RovingFocus`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusableEntry {
    pub id: String,
    pub disabled: bool,
}

impl FocusableEntry {
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            disabled: false,
        }
    }

    pub fn disabled(id: impl Into<String>, disabled: bool) -> Self {
        Self {
            id: id.into(),
            disabled,
        }
    }
}

/// Ordered focus scope tracking focusable elements, disabled state, and active focus ID.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusScope {
    items: Vec<FocusableEntry>,
    active: Option<String>,
}

impl FocusScope {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_items(items: impl IntoIterator<Item = FocusableEntry>) -> Self {
        Self {
            items: items.into_iter().collect(),
            active: None,
        }
    }

    pub fn items(&self) -> &[FocusableEntry] {
        &self.items
    }

    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Registers an enabled focusable ID (updates disabled flag to `false` if already present).
    pub fn register(&mut self, id: impl Into<String>) {
        self.register_with_state(id, false);
    }

    /// Registers or updates a focusable ID with its `disabled` flag.
    pub fn register_with_state(&mut self, id: impl Into<String>, disabled: bool) {
        let id = id.into();
        if let Some(entry) = self.items.iter_mut().find(|e| e.id == id) {
            entry.disabled = disabled;
            if disabled && self.active.as_deref() == Some(entry.id.as_str()) {
                self.active = self.first_enabled_id().map(str::to_owned);
            }
        } else {
            self.items.push(FocusableEntry { id, disabled });
        }
    }

    /// Replaces the ordered item list while preserving active focus if still enabled.
    pub fn set_items(&mut self, items: Vec<FocusableEntry>) {
        self.items = items;
        if let Some(ref active_id) = self.active {
            let valid = self
                .items
                .iter()
                .any(|e| !e.disabled && e.id == *active_id);
            if !valid {
                self.active = self.first_enabled_id().map(str::to_owned);
            }
        }
    }

    pub fn unregister(&mut self, id: &str) {
        self.items.retain(|e| e.id != id);
        if self.active.as_deref() == Some(id) {
            self.active = self.first_enabled_id().map(str::to_owned);
        }
    }

    pub fn set_disabled(&mut self, id: &str, disabled: bool) {
        if let Some(entry) = self.items.iter_mut().find(|e| e.id == id) {
            entry.disabled = disabled;
            if disabled && self.active.as_deref() == Some(id) {
                self.active = self.first_enabled_id().map(str::to_owned);
            }
        }
    }

    pub fn focus(&mut self, id: &str) -> bool {
        if self.items.iter().any(|e| !e.disabled && e.id == id) {
            self.active = Some(id.to_owned());
            true
        } else {
            false
        }
    }

    pub fn blur(&mut self) {
        self.active = None;
    }

    fn first_enabled_id(&self) -> Option<&str> {
        self.items
            .iter()
            .find(|e| !e.disabled)
            .map(|e| e.id.as_str())
    }

    fn last_enabled_id(&self) -> Option<&str> {
        self.items
            .iter()
            .rev()
            .find(|e| !e.disabled)
            .map(|e| e.id.as_str())
    }

    pub fn focus_first(&mut self) -> Option<&str> {
        self.active = self.first_enabled_id().map(str::to_owned);
        self.active.as_deref()
    }

    pub fn focus_last(&mut self) -> Option<&str> {
        self.active = self.last_enabled_id().map(str::to_owned);
        self.active.as_deref()
    }

    /// Moves focus forward to the next enabled item, wrapping to the first item when `wrap == true`.
    pub fn focus_next(&mut self, wrap: bool) -> Option<&str> {
        let len = self.items.len();
        if len == 0 {
            self.active = None;
            return None;
        }
        let start = match self.active.as_deref() {
            Some(cur) => match self.items.iter().position(|e| e.id == cur) {
                Some(idx) => idx + 1,
                None => 0,
            },
            None => 0,
        };
        for offset in 0..len {
            let idx = start + offset;
            if idx >= len && !wrap {
                break;
            }
            let wrapped = idx % len;
            if !self.items[wrapped].disabled {
                self.active = Some(self.items[wrapped].id.clone());
                return self.active.as_deref();
            }
        }
        self.active.as_deref()
    }

    /// Moves focus backward to the previous enabled item, wrapping to the last item when `wrap == true`.
    pub fn focus_prev(&mut self, wrap: bool) -> Option<&str> {
        let len = self.items.len();
        if len == 0 {
            self.active = None;
            return None;
        }
        let start = match self.active.as_deref() {
            Some(cur) => self
                .items
                .iter()
                .position(|e| e.id == cur)
                .unwrap_or(0),
            None => return self.focus_last(),
        };
        for step in 1..=len {
            if step > start && !wrap {
                break;
            }
            let idx = (start + len - (step % len)) % len;
            if !self.items[idx].disabled {
                self.active = Some(self.items[idx].id.clone());
                return self.active.as_deref();
            }
        }
        self.active.as_deref()
    }

    pub fn tab_index_for(&self, id: &str) -> i32 {
        match self.items.iter().find(|e| e.id == id) {
            Some(e) if !e.disabled => 0,
            _ => -1,
        }
    }
}

/// Outcome produced by [`FocusTrap::handle_key`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FocusTrapAction {
    /// Focus moved within the trap to this element ID.
    FocusChanged(String),
    /// `Escape` dismissed the trap; caller should close the overlay and restore focus if present.
    Dismiss { restore_focus: Option<String> },
    /// Key was not handled by the trap.
    Ignored,
}

/// Focus trap for modal overlays (`Dialog`, `Popover`, `DropdownMenu`).
///
/// Traps `Tab` / `Shift+Tab` inside an ordered [`FocusScope`], handles `Escape` dismissal,
/// and restores the previously focused ID upon deactivation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FocusTrap {
    scope: FocusScope,
    open: bool,
    previous_focus: Option<String>,
    close_on_escape: bool,
}

impl Default for FocusTrap {
    fn default() -> Self {
        Self {
            scope: FocusScope::new(),
            open: false,
            previous_focus: None,
            close_on_escape: true,
        }
    }
}

impl FocusTrap {
    pub fn new(items: impl IntoIterator<Item = FocusableEntry>) -> Self {
        Self {
            scope: FocusScope::with_items(items),
            open: false,
            previous_focus: None,
            close_on_escape: true,
        }
    }

    pub fn from_ids<I, S>(ids: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self::new(ids.into_iter().map(FocusableEntry::new))
    }

    pub fn with_close_on_escape(mut self, enabled: bool) -> Self {
        self.close_on_escape = enabled;
        self
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    pub fn active(&self) -> Option<&str> {
        self.scope.active()
    }

    pub fn previous_focus(&self) -> Option<&str> {
        self.previous_focus.as_deref()
    }

    pub fn scope(&self) -> &FocusScope {
        &self.scope
    }

    pub fn scope_mut(&mut self) -> &mut FocusScope {
        &mut self.scope
    }

    /// Activates the trap, remembering `previous_focus` and focusing the first enabled child.
    pub fn activate(&mut self, previous_focus: Option<String>) -> Option<&str> {
        self.open = true;
        self.previous_focus = previous_focus;
        self.scope.focus_first()
    }

    /// Deactivates the trap, clearing active trap focus and returning the focus ID to restore.
    pub fn deactivate(&mut self) -> Option<String> {
        self.open = false;
        self.scope.blur();
        self.previous_focus.take()
    }

    /// Processes a keyboard event (`Tab`, `Shift+Tab`, `Escape`) while the trap is open.
    pub fn handle_key(&mut self, key: &str, shift: bool) -> FocusTrapAction {
        if !self.open {
            return FocusTrapAction::Ignored;
        }
        match key {
            "Tab" => {
                let next = if shift {
                    self.scope.focus_prev(true)
                } else {
                    self.scope.focus_next(true)
                };
                match next {
                    Some(id) => FocusTrapAction::FocusChanged(id.to_owned()),
                    None => FocusTrapAction::Ignored,
                }
            }
            "Escape" | "Esc" if self.close_on_escape => {
                let restore_focus = self.deactivate();
                FocusTrapAction::Dismiss { restore_focus }
            }
            _ => FocusTrapAction::Ignored,
        }
    }
}

/// Roving tabindex keyboard navigator for composite widgets (`Tabs`, `DropdownMenu`).
///
/// Supports `ArrowLeft`/`ArrowRight` (horizontal) or `ArrowUp`/`ArrowDown` (vertical) plus
/// `Home`/`End`, skipping disabled items automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RovingFocus {
    items: Vec<FocusableEntry>,
    active_index: Option<usize>,
    orientation: Orientation,
    loop_navigation: bool,
}

impl RovingFocus {
    pub fn new(orientation: Orientation) -> Self {
        Self {
            items: Vec::new(),
            active_index: None,
            orientation,
            loop_navigation: true,
        }
    }

    pub fn with_items(
        orientation: Orientation,
        items: impl IntoIterator<Item = FocusableEntry>,
    ) -> Self {
        let items: Vec<FocusableEntry> = items.into_iter().collect();
        let active_index = items.iter().position(|e| !e.disabled);
        Self {
            items,
            active_index,
            orientation,
            loop_navigation: true,
        }
    }

    pub fn with_loop(mut self, enabled: bool) -> Self {
        self.loop_navigation = enabled;
        self
    }

    pub fn orientation(&self) -> Orientation {
        self.orientation
    }

    pub fn items(&self) -> &[FocusableEntry] {
        &self.items
    }

    pub fn active_index(&self) -> Option<usize> {
        self.active_index
    }

    pub fn active_id(&self) -> Option<&str> {
        self.active_index
            .and_then(|i| self.items.get(i))
            .filter(|e| !e.disabled)
            .map(|e| e.id.as_str())
    }

    pub fn set_items(&mut self, items: Vec<FocusableEntry>) {
        let prev_id = self.active_id().map(str::to_owned);
        self.items = items;
        self.active_index = prev_id
            .as_deref()
            .and_then(|id| self.items.iter().position(|e| !e.disabled && e.id == id))
            .or_else(|| self.first_enabled());
    }

    pub fn set_active_index(&mut self, idx: usize) -> Option<usize> {
        if self.items.get(idx).is_some_and(|e| !e.disabled) {
            self.active_index = Some(idx);
            Some(idx)
        } else {
            None
        }
    }

    pub fn set_active_id(&mut self, id: &str) -> Option<usize> {
        let idx = self.items.iter().position(|e| !e.disabled && e.id == id)?;
        self.active_index = Some(idx);
        Some(idx)
    }

    pub fn first_enabled(&self) -> Option<usize> {
        self.items.iter().position(|e| !e.disabled)
    }

    pub fn last_enabled(&self) -> Option<usize> {
        self.items.iter().rposition(|e| !e.disabled)
    }

    fn step_enabled(&self, forward: bool) -> Option<usize> {
        let len = self.items.len();
        if len == 0 {
            return None;
        }
        let Some(cur) = self.active_index else {
            return if forward {
                self.first_enabled()
            } else {
                self.last_enabled()
            };
        };
        for delta in 1..=len {
            let candidate = if forward {
                let raw = cur + delta;
                if raw >= len {
                    if !self.loop_navigation {
                        return Some(cur);
                    }
                    raw % len
                } else {
                    raw
                }
            } else if delta > cur {
                if !self.loop_navigation {
                    return Some(cur);
                }
                (cur + len - (delta % len)) % len
            } else {
                cur - delta
            };
            if !self.items[candidate].disabled {
                return Some(candidate);
            }
        }
        None
    }

    /// Processes directional / `Home` / `End` keys, skipping disabled items.
    pub fn handle_key(&mut self, key: &str) -> Option<usize> {
        let next = match (self.orientation, key) {
            (Orientation::Horizontal, "ArrowRight") | (Orientation::Vertical, "ArrowDown") => {
                self.step_enabled(true)
            }
            (Orientation::Horizontal, "ArrowLeft") | (Orientation::Vertical, "ArrowUp") => {
                self.step_enabled(false)
            }
            (_, "Home") => self.first_enabled(),
            (_, "End") => self.last_enabled(),
            _ => return None,
        };
        if let Some(idx) = next {
            self.active_index = Some(idx);
        }
        next
    }

    /// Returns `0` for the active enabled item (or first enabled if unset) and `-1` otherwise.
    pub fn tab_index_for(&self, index: usize) -> i32 {
        let target = self.active_index.or_else(|| self.first_enabled());
        if target == Some(index) && self.items.get(index).is_some_and(|e| !e.disabled) {
            0
        } else {
            -1
        }
    }

    pub fn tab_index_for_id(&self, id: &str) -> i32 {
        self.items
            .iter()
            .position(|e| e.id == id)
            .map(|i| self.tab_index_for(i))
            .unwrap_or(-1)
    }
}

/// Provides a shared [`FocusScope`] signal to descendant components for cross-widget focus tracking.
#[component]
pub fn FocusScopeProvider(children: Element) -> Element {
    let _scope = use_hook(|| provide_context(Signal::new(FocusScope::new())));
    rsx! { {children} }
}

/// Returns the shared [`FocusScope`] signal from the nearest [`FocusScopeProvider`], if mounted.
pub fn use_focus_scope() -> Option<Signal<FocusScope>> {
    try_consume_context::<Signal<FocusScope>>()
}

/// Helper to read the currently focused ID from the nearest [`FocusScopeProvider`].
pub fn current_focused_id() -> Option<String> {
    use_focus_scope().and_then(|sig| sig.read().active().map(str::to_owned))
}

/// Helper to update active focus inside the nearest [`FocusScopeProvider`], if present.
pub fn notify_focus_scope(id: &str) {
    if let Some(mut sig) = use_focus_scope() {
        let mut scope = sig.write();
        scope.register(id);
        scope.focus(id);
    }
}
