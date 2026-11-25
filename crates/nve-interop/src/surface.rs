use std::collections::BTreeMap;

use nve_abi::SurfaceId;
use serde::{Deserialize, Serialize};

use crate::error::InteropError;
use crate::handoff::{IslandSpec, RenderHandoff};
use crate::layout::IntrinsicSize;

/// UI framework hosting or rendered within an orchestrated surface.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FrameworkKind {
    Nve = 0,
    ReactNativeFabric = 1,
    Flutter = 2,
    SwiftUI = 3,
    JetpackCompose = 4,
    WebDom = 5,
}

impl FrameworkKind {
    pub fn from_u8(value: u8) -> Option<Self> {
        match value {
            0 => Some(Self::Nve),
            1 => Some(Self::ReactNativeFabric),
            2 => Some(Self::Flutter),
            3 => Some(Self::SwiftUI),
            4 => Some(Self::JetpackCompose),
            5 => Some(Self::WebDom),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Nve => "nve",
            Self::ReactNativeFabric => "react_native_fabric",
            Self::Flutter => "flutter",
            Self::SwiftUI => "swiftui",
            Self::JetpackCompose => "jetpack_compose",
            Self::WebDom => "web_dom",
        }
    }
}

/// Lifecycle state of an orchestrated surface across framework boundaries.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LifecycleState {
    Mounted = 0,
    Paused = 1,
    Resumed = 2,
    Destroyed = 3,
}

impl LifecycleState {
    pub fn can_transition_to(self, next: Self) -> bool {
        if self == next {
            return true;
        }
        match (self, next) {
            (Self::Destroyed, _) => false,
            (Self::Mounted, Self::Resumed | Self::Paused | Self::Destroyed) => true,
            (Self::Resumed, Self::Paused | Self::Destroyed) => true,
            (Self::Paused, Self::Resumed | Self::Destroyed) => true,
            _ => false,
        }
    }
}

/// Cross-framework color scheme propagated from root hosts into embedded surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    #[default]
    Light,
    Dark,
    HighContrastLight,
    HighContrastDark,
    System,
}

/// Theme state propagated hierarchically across framework boundaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThemeContext {
    pub color_scheme: ColorScheme,
    /// Accent color packed as `0xRRGGBBAA`.
    pub accent_rgba: u32,
    /// Arbitrary semantic design tokens (e.g. `"surface.bg" -> "#0F172A"`).
    #[serde(default)]
    pub tokens: BTreeMap<String, String>,
}

impl Default for ThemeContext {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::Light,
            accent_rgba: 0x2563_EBFF,
            tokens: BTreeMap::new(),
        }
    }
}

impl ThemeContext {
    pub fn dark() -> Self {
        Self {
            color_scheme: ColorScheme::Dark,
            accent_rgba: 0x60A5_FAFF,
            tokens: BTreeMap::new(),
        }
    }

    pub fn is_dark(&self) -> bool {
        matches!(
            self.color_scheme,
            ColorScheme::Dark | ColorScheme::HighContrastDark
        )
    }
}

/// Safe-area insets (in logical points/dp) propagated from host windows into child surfaces.
#[repr(C)]
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
pub struct SafeAreaInsets {
    pub top: f32,
    pub right: f32,
    pub bottom: f32,
    pub left: f32,
}

impl SafeAreaInsets {
    pub const ZERO: Self = Self {
        top: 0.0,
        right: 0.0,
        bottom: 0.0,
        left: 0.0,
    };

    pub const fn new(top: f32, right: f32, bottom: f32, left: f32) -> Self {
        Self {
            top,
            right,
            bottom,
            left,
        }
    }
}

/// Inherited environment context propagated across framework boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceEnvironment {
    pub theme: ThemeContext,
    pub safe_area: SafeAreaInsets,
    pub scale_factor: f32,
    pub text_scale_factor: f32,
    pub locale: String,
    pub reduced_motion: bool,
}

impl Default for SurfaceEnvironment {
    fn default() -> Self {
        Self {
            theme: ThemeContext::default(),
            safe_area: SafeAreaInsets::ZERO,
            scale_factor: 1.0,
            text_scale_factor: 1.0,
            locale: "en-US".to_owned(),
            reduced_motion: false,
        }
    }
}

/// Optional per-surface overrides that stop parent propagation for the overridden fields.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EnvironmentOverrides {
    pub theme: Option<ThemeContext>,
    pub safe_area: Option<SafeAreaInsets>,
    pub scale_factor: Option<f32>,
    pub locale: Option<String>,
}

/// Registration parameters for a new surface in [`SurfaceOrchestrator`].
#[derive(Debug, Clone)]
pub struct SurfaceDescriptor {
    /// Optional explicit surface ID. When `None`, the orchestrator allocates a monotonic ID.
    pub id: Option<SurfaceId>,
    pub parent: Option<SurfaceId>,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub handoff: RenderHandoff,
    pub island: Option<IslandSpec>,
    pub environment: Option<SurfaceEnvironment>,
    pub overrides: EnvironmentOverrides,
    pub initial_size: IntrinsicSize,
}

impl SurfaceDescriptor {
    pub fn new(framework: FrameworkKind, component_name: impl Into<String>) -> Self {
        Self {
            id: None,
            parent: None,
            framework,
            component_name: component_name.into(),
            handoff: RenderHandoff::Headless,
            island: None,
            environment: None,
            overrides: EnvironmentOverrides::default(),
            initial_size: IntrinsicSize::ZERO,
        }
    }

    pub fn with_id(mut self, id: SurfaceId) -> Self {
        self.id = Some(id);
        self
    }

    pub fn with_parent(mut self, parent: SurfaceId) -> Self {
        self.parent = Some(parent);
        self
    }

    pub fn with_handoff(mut self, handoff: RenderHandoff) -> Self {
        self.handoff = handoff;
        self
    }

    pub fn with_island(mut self, island: IslandSpec) -> Self {
        self.island = Some(island);
        self
    }

    pub fn with_environment(mut self, env: SurfaceEnvironment) -> Self {
        self.environment = Some(env);
        self
    }

    pub fn with_overrides(mut self, overrides: EnvironmentOverrides) -> Self {
        self.overrides = overrides;
        self
    }

    pub fn with_initial_size(mut self, size: IntrinsicSize) -> Self {
        self.initial_size = size;
        self
    }
}

/// Metadata stored for every registered surface in the hierarchy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SurfaceRecord {
    pub id: SurfaceId,
    pub parent: Option<SurfaceId>,
    pub children: Vec<SurfaceId>,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub state: LifecycleState,
    pub environment: SurfaceEnvironment,
    pub overrides: EnvironmentOverrides,
    pub handoff: RenderHandoff,
    pub island: Option<IslandSpec>,
    pub viewport_size: IntrinsicSize,
}

/// Record describing one surface torn down during cascading destruction (bottom-up order).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TeardownRecord {
    pub surface_id: SurfaceId,
    pub framework: FrameworkKind,
    pub component_name: String,
    pub previous_state: LifecycleState,
}

/// Lifecycle/environment notification emitted by [`SurfaceOrchestrator`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SurfaceEvent {
    Mounted {
        surface_id: SurfaceId,
        parent: Option<SurfaceId>,
        framework: FrameworkKind,
    },
    LifecycleChanged {
        surface_id: SurfaceId,
        from: LifecycleState,
        to: LifecycleState,
    },
    EnvironmentPropagated {
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    },
    Reparented {
        surface_id: SurfaceId,
        old_parent: Option<SurfaceId>,
        new_parent: Option<SurfaceId>,
    },
    Destroyed {
        surface_id: SurfaceId,
        framework: FrameworkKind,
    },
}

/// Hierarchical registry of surfaces spanning heterogeneous UI frameworks
/// (e.g. Flutter root -> NVE surface -> React Native Fabric island -> SwiftUI island).
///
/// Responsibilities:
/// - Parent/child ownership graph with cycle prevention
/// - Lifecycle state machine (`Mounted`, `Paused`, `Resumed`, `Destroyed`) with subtree propagation
/// - Theme, safe-area, scale-factor, and locale cascading with per-surface override support
/// - Leak-free bottom-up cascading teardown
type TeardownHook = Box<dyn FnOnce(&TeardownRecord) + Send + 'static>;

pub struct SurfaceOrchestrator {
    next_id: SurfaceId,
    surfaces: BTreeMap<SurfaceId, SurfaceRecord>,
    roots: Vec<SurfaceId>,
    events: Vec<SurfaceEvent>,
    teardown_hooks: BTreeMap<SurfaceId, TeardownHook>,
}

impl Default for SurfaceOrchestrator {
    fn default() -> Self {
        Self::new()
    }
}

impl SurfaceOrchestrator {
    pub fn new() -> Self {
        Self {
            next_id: 1,
            surfaces: BTreeMap::new(),
            roots: Vec::new(),
            events: Vec::new(),
            teardown_hooks: BTreeMap::new(),
        }
    }

    /// Registers a surface (root or child), inheriting environment from its parent when present.
    pub fn register_surface(&mut self, desc: SurfaceDescriptor) -> Result<SurfaceId, InteropError> {
        if let Some(island) = &desc.island {
            island.validate()?;
        }

        let id = match desc.id {
            Some(explicit) => {
                if self.surfaces.contains_key(&explicit) {
                    return Err(InteropError::DuplicateSurface(explicit));
                }
                if explicit >= self.next_id {
                    self.next_id = explicit.saturating_add(1);
                }
                explicit
            }
            None => {
                while self.surfaces.contains_key(&self.next_id) {
                    self.next_id = self.next_id.saturating_add(1);
                }
                let allocated = self.next_id;
                self.next_id = self.next_id.saturating_add(1);
                allocated
            }
        };

        let inherited_env = match desc.parent {
            Some(parent_id) => {
                let parent = self
                    .surfaces
                    .get(&parent_id)
                    .ok_or(InteropError::ParentNotFound(parent_id))?;
                if parent.state == LifecycleState::Destroyed {
                    return Err(InteropError::SurfaceDestroyed(parent_id));
                }
                parent.environment.clone()
            }
            None => SurfaceEnvironment::default(),
        };

        let mut resolved_env = desc.environment.unwrap_or(inherited_env);
        apply_overrides(&mut resolved_env, &desc.overrides);

        let record = SurfaceRecord {
            id,
            parent: desc.parent,
            children: Vec::new(),
            framework: desc.framework,
            component_name: desc.component_name,
            state: LifecycleState::Mounted,
            environment: resolved_env,
            overrides: desc.overrides,
            handoff: desc.handoff,
            island: desc.island,
            viewport_size: desc.initial_size,
        };

        if let Some(parent_id) = desc.parent {
            if let Some(parent) = self.surfaces.get_mut(&parent_id) {
                parent.children.push(id);
            }
        } else {
            self.roots.push(id);
        }

        self.surfaces.insert(id, record);
        self.events.push(SurfaceEvent::Mounted {
            surface_id: id,
            parent: desc.parent,
            framework: desc.framework,
        });

        Ok(id)
    }

    /// Convenience helper to register a root surface.
    pub fn register_root(
        &mut self,
        framework: FrameworkKind,
        component_name: impl Into<String>,
        environment: SurfaceEnvironment,
    ) -> Result<SurfaceId, InteropError> {
        self.register_surface(
            SurfaceDescriptor::new(framework, component_name).with_environment(environment),
        )
    }

    /// Convenience helper to register a child surface under an existing parent.
    pub fn register_child(
        &mut self,
        parent: SurfaceId,
        framework: FrameworkKind,
        component_name: impl Into<String>,
    ) -> Result<SurfaceId, InteropError> {
        self.register_surface(SurfaceDescriptor::new(framework, component_name).with_parent(parent))
    }

    /// Registers a one-shot teardown callback invoked when `surface_id` is destroyed.
    pub fn register_teardown_hook<F>(
        &mut self,
        surface_id: SurfaceId,
        hook: F,
    ) -> Result<(), InteropError>
    where
        F: FnOnce(&TeardownRecord) + Send + 'static,
    {
        let record = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        if record.state == LifecycleState::Destroyed {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }
        self.teardown_hooks.insert(surface_id, Box::new(hook));
        Ok(())
    }

    pub fn get(&self, surface_id: SurfaceId) -> Option<&SurfaceRecord> {
        self.surfaces.get(&surface_id)
    }

    pub fn roots(&self) -> &[SurfaceId] {
        &self.roots
    }

    pub fn children(&self, surface_id: SurfaceId) -> Result<&[SurfaceId], InteropError> {
        self.surfaces
            .get(&surface_id)
            .map(|s| s.children.as_slice())
            .ok_or(InteropError::SurfaceNotFound(surface_id))
    }

    /// Returns the ancestor chain starting from `surface_id` itself up to the root.
    pub fn ancestor_chain(&self, surface_id: SurfaceId) -> Result<Vec<SurfaceId>, InteropError> {
        if !self.surfaces.contains_key(&surface_id) {
            return Err(InteropError::SurfaceNotFound(surface_id));
        }
        let mut chain = Vec::new();
        let mut cursor = Some(surface_id);
        while let Some(id) = cursor {
            chain.push(id);
            cursor = self.surfaces.get(&id).and_then(|r| r.parent);
        }
        Ok(chain)
    }

    /// Returns all descendants of `surface_id` in pre-order (excluding `surface_id`).
    pub fn descendants(&self, surface_id: SurfaceId) -> Result<Vec<SurfaceId>, InteropError> {
        if !self.surfaces.contains_key(&surface_id) {
            return Err(InteropError::SurfaceNotFound(surface_id));
        }
        let mut out = Vec::new();
        let mut stack: Vec<SurfaceId> = self.surfaces[&surface_id]
            .children
            .iter()
            .rev()
            .copied()
            .collect();
        while let Some(id) = stack.pop() {
            out.push(id);
            if let Some(node) = self.surfaces.get(&id) {
                for &child in node.children.iter().rev() {
                    stack.push(child);
                }
            }
        }
        Ok(out)
    }

    /// Active (non-destroyed) surface count.
    pub fn active_count(&self) -> usize {
        self.surfaces
            .values()
            .filter(|s| s.state != LifecycleState::Destroyed)
            .count()
    }

    /// Transitions a single surface to `next_state` (for `Destroyed`, use [`Self::destroy_surface`]).
    pub fn transition_state(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<(), InteropError> {
        if next_state == LifecycleState::Destroyed {
            self.destroy_surface(surface_id)?;
            return Ok(());
        }
        let record = self
            .surfaces
            .get_mut(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        let prev = record.state;
        if !prev.can_transition_to(next_state) {
            return Err(InteropError::InvalidLifecycleTransition {
                surface: surface_id,
                from: prev,
                to: next_state,
            });
        }
        if prev != next_state {
            record.state = next_state;
            self.events.push(SurfaceEvent::LifecycleChanged {
                surface_id,
                from: prev,
                to: next_state,
            });
        }
        Ok(())
    }

    /// Propagates a lifecycle transition (`Resumed` or `Paused`) to `surface_id` and all descendants.
    pub fn propagate_lifecycle(
        &mut self,
        surface_id: SurfaceId,
        next_state: LifecycleState,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        if next_state == LifecycleState::Destroyed {
            let torn_down = self.destroy_surface(surface_id)?;
            return Ok(torn_down.into_iter().map(|r| r.surface_id).collect());
        }
        let mut targets = vec![surface_id];
        targets.extend(self.descendants(surface_id)?);

        // Validate root first so invalid transitions fail atomically.
        let root_state = self.surfaces[&surface_id].state;
        if !root_state.can_transition_to(next_state) {
            return Err(InteropError::InvalidLifecycleTransition {
                surface: surface_id,
                from: root_state,
                to: next_state,
            });
        }

        let mut transitioned = Vec::new();
        for id in targets {
            if let Some(rec) = self.surfaces.get_mut(&id)
                && rec.state != LifecycleState::Destroyed
                && rec.state != next_state
                && rec.state.can_transition_to(next_state)
            {
                let from = rec.state;
                rec.state = next_state;
                transitioned.push(id);
                self.events.push(SurfaceEvent::LifecycleChanged {
                    surface_id: id,
                    from,
                    to: next_state,
                });
            }
        }
        Ok(transitioned)
    }

    /// Updates a surface's viewport size after layout negotiation.
    pub fn set_viewport_size(
        &mut self,
        surface_id: SurfaceId,
        size: IntrinsicSize,
    ) -> Result<(), InteropError> {
        let rec = self
            .surfaces
            .get_mut(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        if rec.state == LifecycleState::Destroyed {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }
        rec.viewport_size = size;
        Ok(())
    }

    /// Updates the environment at `surface_id` and propagates it down the subtree,
    /// respecting each descendant's [`EnvironmentOverrides`].
    pub fn update_environment(
        &mut self,
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let rec = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?;
        if rec.state == LifecycleState::Destroyed {
            return Err(InteropError::SurfaceDestroyed(surface_id));
        }

        let mut updated = Vec::new();
        self.propagate_env_recursive(surface_id, &environment, &mut updated);
        Ok(updated)
    }

    fn propagate_env_recursive(
        &mut self,
        surface_id: SurfaceId,
        incoming: &SurfaceEnvironment,
        updated: &mut Vec<SurfaceId>,
    ) {
        let (effective, children) = {
            let Some(rec) = self.surfaces.get_mut(&surface_id) else {
                return;
            };
            let mut next = incoming.clone();
            apply_overrides(&mut next, &rec.overrides);
            if rec.environment != next {
                rec.environment = next.clone();
                updated.push(surface_id);
                self.events.push(SurfaceEvent::EnvironmentPropagated {
                    surface_id,
                    environment: next.clone(),
                });
            }
            (rec.environment.clone(), rec.children.clone())
        };

        for child_id in children {
            self.propagate_env_recursive(child_id, &effective, updated);
        }
    }

    /// Convenience helper to update only the theme at `surface_id` and cascade to children.
    pub fn set_theme(
        &mut self,
        surface_id: SurfaceId,
        theme: ThemeContext,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.theme = theme;
        self.update_environment(surface_id, env)
    }

    /// Convenience helper to update safe-area insets at `surface_id` and cascade to children.
    pub fn set_safe_area(
        &mut self,
        surface_id: SurfaceId,
        safe_area: SafeAreaInsets,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.safe_area = safe_area;
        self.update_environment(surface_id, env)
    }

    /// Convenience helper to update display scale factor at `surface_id` and cascade to children.
    pub fn set_scale_factor(
        &mut self,
        surface_id: SurfaceId,
        scale_factor: f32,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        let mut env = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .clone();
        env.scale_factor = scale_factor;
        self.update_environment(surface_id, env)
    }

    /// Reparents `surface_id` under `new_parent` (or promotes it to a root if `None`),
    /// preventing cycles and re-propagating environment from the new parent.
    pub fn reparent(
        &mut self,
        surface_id: SurfaceId,
        new_parent: Option<SurfaceId>,
    ) -> Result<(), InteropError> {
        let old_parent = {
            let rec = self
                .surfaces
                .get(&surface_id)
                .ok_or(InteropError::SurfaceNotFound(surface_id))?;
            if rec.state == LifecycleState::Destroyed {
                return Err(InteropError::SurfaceDestroyed(surface_id));
            }
            rec.parent
        };

        if old_parent == new_parent {
            return Ok(());
        }

        if let Some(target_parent) = new_parent {
            let parent_rec = self
                .surfaces
                .get(&target_parent)
                .ok_or(InteropError::ParentNotFound(target_parent))?;
            if parent_rec.state == LifecycleState::Destroyed {
                return Err(InteropError::SurfaceDestroyed(target_parent));
            }
            let ancestors = self.ancestor_chain(target_parent)?;
            if ancestors.contains(&surface_id) {
                return Err(InteropError::InvalidParentCycle {
                    surface: surface_id,
                    proposed_parent: target_parent,
                });
            }
        }

        // Detach from old parent / root list.
        match old_parent {
            Some(old_pid) => {
                if let Some(old_p) = self.surfaces.get_mut(&old_pid) {
                    old_p.children.retain(|&c| c != surface_id);
                }
            }
            None => {
                self.roots.retain(|&r| r != surface_id);
            }
        }

        // Attach to new parent / root list.
        let new_parent_env = match new_parent {
            Some(new_pid) => {
                let new_p = self.surfaces.get_mut(&new_pid).unwrap();
                new_p.children.push(surface_id);
                Some(new_p.environment.clone())
            }
            None => {
                self.roots.push(surface_id);
                None
            }
        };

        if let Some(rec) = self.surfaces.get_mut(&surface_id) {
            rec.parent = new_parent;
        }

        self.events.push(SurfaceEvent::Reparented {
            surface_id,
            old_parent,
            new_parent,
        });

        if let Some(env) = new_parent_env {
            let mut updated = Vec::new();
            self.propagate_env_recursive(surface_id, &env, &mut updated);
        }

        Ok(())
    }

    /// Destroys `surface_id` and its entire subtree in strict bottom-up (post-order) sequence,
    /// firing teardown hooks, detaching from parents, and removing all entries so zero leaks remain.
    pub fn destroy_surface(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let parent_of_root = self
            .surfaces
            .get(&surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .parent;

        // Collect post-order traversal (deepest leaves first, target root last).
        let mut order = Vec::new();
        self.collect_post_order(surface_id, &mut order);

        // Detach the target surface from its parent or root list.
        match parent_of_root {
            Some(pid) => {
                if let Some(parent) = self.surfaces.get_mut(&pid) {
                    parent.children.retain(|&c| c != surface_id);
                }
            }
            None => {
                self.roots.retain(|&r| r != surface_id);
            }
        }

        let mut records = Vec::with_capacity(order.len());
        for id in order {
            if let Some(mut rec) = self.surfaces.remove(&id) {
                let previous_state = rec.state;
                rec.state = LifecycleState::Destroyed;
                let teardown = TeardownRecord {
                    surface_id: id,
                    framework: rec.framework,
                    component_name: rec.component_name,
                    previous_state,
                };
                if let Some(hook) = self.teardown_hooks.remove(&id) {
                    hook(&teardown);
                }
                self.events.push(SurfaceEvent::Destroyed {
                    surface_id: id,
                    framework: teardown.framework,
                });
                records.push(teardown);
            }
        }

        Ok(records)
    }

    fn collect_post_order(&self, surface_id: SurfaceId, out: &mut Vec<SurfaceId>) {
        if let Some(rec) = self.surfaces.get(&surface_id) {
            for &child in &rec.children {
                self.collect_post_order(child, out);
            }
            out.push(surface_id);
        }
    }

    /// Drains all orchestrator events accumulated since the last drain.
    pub fn drain_events(&mut self) -> Vec<SurfaceEvent> {
        std::mem::take(&mut self.events)
    }
}

fn apply_overrides(env: &mut SurfaceEnvironment, overrides: &EnvironmentOverrides) {
    if let Some(theme) = &overrides.theme {
        env.theme = theme.clone();
    }
    if let Some(safe_area) = overrides.safe_area {
        env.safe_area = safe_area;
    }
    if let Some(scale) = overrides.scale_factor {
        env.scale_factor = scale;
    }
    if let Some(locale) = &overrides.locale {
        env.locale = locale.clone();
    }
}
