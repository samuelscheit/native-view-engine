//! Universal Framework Compatibility Orchestration Layer for Native View Engine (NVE).
//!
//! Enables embedding NVE components natively inside any UI framework (React Native Fabric,
//! Flutter, SwiftUI/UIKit, Jetpack Compose, Web DOM) with unified surface lifecycle,
//! layout negotiation, gesture arbitration, typed native views, and shared binary channels.

#![cfg_attr(all(doctest, feature = "channel"), doc = include_str!("../README.md"))]

#[cfg(feature = "channel")]
pub mod channel;
#[cfg(feature = "crdt")]
pub mod crdt;
pub mod error;
#[cfg(feature = "gesture")]
pub mod gesture;
#[cfg(feature = "layout")]
pub mod layout;
#[cfg(feature = "plugin-host")]
pub mod plugin_host;
#[cfg(feature = "reconcile")]
pub mod reconcile;
pub mod surface;
#[cfg(feature = "traits")]
pub mod traits;
#[cfg(feature = "window")]
pub mod window;

#[cfg(feature = "channel")]
pub use channel::{
    CHANNEL_ID_AUTH, CHANNEL_ID_LOCALE, CHANNEL_ID_NAVIGATION, CHANNEL_ID_THEME, ChannelId,
    ChannelMessage, ChannelPayload, NveChannelBus, PayloadEncoding, SubscriptionId, TOPIC_AUTH,
    TOPIC_LOCALE, TOPIC_NAVIGATION, TOPIC_THEME,
};
pub use error::InteropError;
#[cfg(feature = "gesture")]
pub use gesture::{
    ArbitrationDecision, ArbitrationReason, GestureArbitrator, GestureCapabilities,
    GestureDisposition, PointerPhase, PointerSample, ScrollAxes, ScrollEdgeState,
};
#[cfg(feature = "layout")]
pub use layout::{
    BaselineMetrics, IntrinsicSize, LayoutNegotiator, LayoutProposal, LayoutResult,
    MeasurableSurface, SizeConstraints,
};
#[cfg(feature = "plugin-host")]
pub use plugin_host::{
    NvePluginHost, NvePluginSlot, PluginCapabilities, PluginError, PluginId, PluginMetadata,
    PluginRequirement, PluginSourceKind,
};
#[cfg(all(feature = "reconcile", feature = "crdt"))]
pub use reconcile::{CrdtReconcileAdapter, hlc_to_logical_clock};
#[cfg(feature = "reconcile")]
pub use reconcile::{
    ConflictPolicy, DivergenceKind, LogicalClock, ReconcileBatch, ReconcileDecision,
    StateReconciler, ThreeWayEntry, reconcile_three_way,
};
#[cfg(feature = "window")]
pub use surface::SurfacePresentation;
pub use surface::{
    ColorScheme, ContrastLevel, EnvironmentOverrides, FrameworkKind, LifecycleState,
    SafeAreaInsets, SurfaceDescriptor, SurfaceEnvironment, SurfaceEvent, SurfaceOrchestrator,
    SurfaceRecord, TeardownRecord, ThemeContext,
};
#[cfg(feature = "window")]
pub use window::{
    ManagedWindow, WindowCommand, WindowDescriptor, WindowError, WindowFlags, WindowHandleId,
    WindowLifecycleEvent, WindowLifecycleState, WindowManager, WindowRect, WindowState,
};
#[cfg(feature = "traits")]
pub use traits::{
    BUILTIN_TRAIT_COUNT, BuiltinTraitSlot, BuiltinTraits, CanHover, ContainerSize,
    DeviceFormFactor, HasHardwareKeyboard, HighContrast, HostToolkit, KeyboardInsets, Locale,
    Orientation, PlatformOs, PointerKind, ReducedMotion, ScaleFactor, SizeClass, TextScale,
    TraitEnvironment, TraitKey, TraitOverrides, TraitResolver, TraitStore, UiDensity,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX, VerticalSizeClass, ViewportChangeMask,
    effective_bottom_inset, keyboard_insets_from_visual_viewport,
};

pub use nve_abi::SurfaceId;

/// Unified facade coordinating [`SurfaceOrchestrator`] and any enabled interop subsystems
/// so teardown and arbitration stay strictly synchronized with zero leaked subscriptions or locks.
///
/// `surfaces` and `plugins` are kept private so every destructive teardown goes exclusively
/// through [`Self::destroy_surface_cascade`] or [`Self::unload_plugin`].
pub struct InteropRuntime {
    surfaces: SurfaceOrchestrator,
    #[cfg(feature = "layout")]
    pub layout: LayoutNegotiator,
    #[cfg(feature = "gesture")]
    pub gestures: GestureArbitrator,
    #[cfg(feature = "channel")]
    pub channels: NveChannelBus,
    #[cfg(feature = "window")]
    pub windows: WindowManager,
    #[cfg(feature = "plugin-host")]
    plugins: NvePluginHost,
}

impl Default for InteropRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl InteropRuntime {
    pub fn new() -> Self {
        Self {
            surfaces: SurfaceOrchestrator::new(),
            #[cfg(feature = "layout")]
            layout: LayoutNegotiator::new(),
            #[cfg(feature = "gesture")]
            gestures: GestureArbitrator::new(),
            #[cfg(feature = "channel")]
            channels: NveChannelBus::new(),
            #[cfg(feature = "window")]
            windows: WindowManager::new(),
            #[cfg(feature = "plugin-host")]
            plugins: NvePluginHost::new(),
        }
    }

    /// Read-only view of the unified surface hierarchy.
    #[inline]
    pub fn surfaces(&self) -> &SurfaceOrchestrator {
        &self.surfaces
    }

    /// Read-only view of mounted plugin slots.
    #[cfg(feature = "plugin-host")]
    #[inline]
    pub fn plugins(&self) -> &NvePluginHost {
        &self.plugins
    }

    /// Returns mutable access to one mounted plugin slot (for `render`/`dispatch_events`/`set_props`).
    #[cfg(feature = "plugin-host")]
    #[inline]
    pub fn plugin_mut(&mut self, plugin_id: PluginId) -> Option<&mut NvePluginSlot> {
        self.plugins.get_mut(plugin_id)
    }

    /// Registers a surface (root or child), inheriting environment from its parent when present.
    pub fn register_surface(&mut self, desc: SurfaceDescriptor) -> Result<SurfaceId, InteropError> {
        self.surfaces.register_surface(desc)
    }

    /// Convenience helper to register a root surface.
    pub fn register_root(
        &mut self,
        framework: FrameworkKind,
        component_name: impl Into<String>,
        environment: SurfaceEnvironment,
    ) -> Result<SurfaceId, InteropError> {
        self.surfaces.register_root(framework, component_name, environment)
    }

    /// Convenience helper to register a child surface inheriting its parent's environment.
    pub fn register_child(
        &mut self,
        parent: SurfaceId,
        framework: FrameworkKind,
        component_name: impl Into<String>,
    ) -> Result<SurfaceId, InteropError> {
        self.surfaces.register_child(parent, framework, component_name)
    }

    /// Reparents `surface_id` under `new_parent` (or promotes it to a root when `None`).
    pub fn reparent_surface(
        &mut self,
        surface_id: SurfaceId,
        new_parent: Option<SurfaceId>,
    ) -> Result<(), InteropError> {
        self.surfaces.reparent(surface_id, new_parent)
    }

    /// Transitions lifecycle state on a single surface (routing `Destroyed` through
    /// [`Self::destroy_surface_cascade`] so plugins, gestures, layouts, channels, and windows stay
    /// strictly synchronized).
    pub fn transition_surface_state(
        &mut self,
        surface_id: SurfaceId,
        state: LifecycleState,
    ) -> Result<(), InteropError> {
        if state == LifecycleState::Destroyed {
            let _ = self.destroy_surface_cascade(surface_id)?;
            return Ok(());
        }
        self.surfaces.transition_state(surface_id, state)
    }

    /// Cascades a lifecycle transition down `surface_id`'s subtree (routing `Destroyed` through
    /// [`Self::destroy_surface_cascade`]).
    pub fn propagate_lifecycle(
        &mut self,
        surface_id: SurfaceId,
        state: LifecycleState,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        if state == LifecycleState::Destroyed {
            let torn_down = self.destroy_surface_cascade(surface_id)?;
            return Ok(torn_down.into_iter().map(|r| r.surface_id).collect());
        }
        self.surfaces.propagate_lifecycle(surface_id, state)
    }

    /// Updates environment at `surface_id` and cascades down its subtree.
    pub fn update_environment(
        &mut self,
        surface_id: SurfaceId,
        environment: SurfaceEnvironment,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.surfaces.update_environment(surface_id, environment)
    }

    /// Updates theme at `surface_id` and cascades down its subtree.
    pub fn set_theme(
        &mut self,
        surface_id: SurfaceId,
        theme: ThemeContext,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.surfaces.set_theme(surface_id, theme)
    }

    /// Updates safe-area insets at `surface_id` and cascades down its subtree.
    pub fn set_safe_area(
        &mut self,
        surface_id: SurfaceId,
        safe_area: SafeAreaInsets,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.surfaces.set_safe_area(surface_id, safe_area)
    }

    /// Updates display scale factor at `surface_id` and cascades down its subtree.
    pub fn set_scale_factor(
        &mut self,
        surface_id: SurfaceId,
        scale_factor: f32,
    ) -> Result<Vec<SurfaceId>, InteropError> {
        self.surfaces.set_scale_factor(surface_id, scale_factor)
    }

    /// Records the latest negotiated viewport size for `surface_id`.
    #[cfg(feature = "layout")]
    pub fn set_surface_viewport_size(
        &mut self,
        surface_id: SurfaceId,
        size: IntrinsicSize,
    ) -> Result<(), InteropError> {
        self.surfaces.set_viewport_size(surface_id, size)
    }

    /// Registers a teardown hook to fire when `surface_id` is destroyed.
    pub fn on_surface_teardown(
        &mut self,
        surface_id: SurfaceId,
        hook: impl FnOnce(&TeardownRecord) + Send + 'static,
    ) -> Result<(), InteropError> {
        self.surfaces.register_teardown_hook(surface_id, hook)
    }

    /// Drains queued surface lifecycle/environment events.
    pub fn drain_surface_events(&mut self) -> Vec<SurfaceEvent> {
        self.surfaces.drain_events()
    }

    /// Registers an in-process plugin allocating its surface from `self.surfaces`.
    #[cfg(feature = "plugin-host")]
    pub fn register_plugin_in_process(
        &mut self,
        name: impl Into<String>,
        get_interface: nve_abi::NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        self.plugins
            .register_in_process(&mut self.surfaces, name, get_interface, req)
    }

    /// Loads a dynamic (`cdylib`) plugin allocating its surface from `self.surfaces`.
    #[cfg(all(feature = "plugin-host", not(target_arch = "wasm32")))]
    pub fn load_plugin_dynamic(
        &mut self,
        name: impl Into<String>,
        dylib_path: &std::path::Path,
        req: PluginRequirement,
    ) -> Result<PluginId, PluginError> {
        self.plugins
            .load_dynamic(&mut self.surfaces, name, dylib_path, req)
    }

    /// Transactionally hot-swaps an existing plugin to a new in-process `nve_get_interface`
    /// implementation after verifying its surface is still live in `self.surfaces`.
    #[cfg(feature = "plugin-host")]
    pub fn hot_swap_plugin_in_process(
        &mut self,
        plugin_id: PluginId,
        new_get_interface: nve_abi::NveGetInterfaceFn,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        self.plugins
            .hot_swap_in_process(&self.surfaces, plugin_id, new_get_interface, req)
    }

    /// Transactionally hot-swaps an existing plugin to a freshly built `cdylib` after verifying
    /// its surface is still live in `self.surfaces`.
    #[cfg(all(feature = "plugin-host", not(target_arch = "wasm32")))]
    pub fn hot_swap_plugin_dynamic(
        &mut self,
        plugin_id: PluginId,
        new_dylib_path: &std::path::Path,
        req: PluginRequirement,
    ) -> Result<u64, PluginError> {
        self.plugins
            .hot_swap_dynamic(&self.surfaces, plugin_id, new_dylib_path, req)
    }

    /// Unloads a plugin and cascades teardown of its surface (and any reparented child surfaces,
    /// gesture capabilities, layout caches, channel subscriptions, windows, and child plugins).
    #[cfg(feature = "plugin-host")]
    pub fn unload_plugin(&mut self, plugin_id: PluginId) -> Result<PluginMetadata, PluginError> {
        let meta = self.plugins.take_for_unload(plugin_id)?;
        let res = self.destroy_surface_cascade(meta.surface_id);
        debug_assert!(
            res.is_ok(),
            "plugin surface {:?} unexpectedly absent during unload_plugin: {res:?}",
            meta.surface_id
        );
        Ok(meta)
    }

    /// Arbitrates a pointer event starting at `innermost_surface` by resolving its live
    /// ancestor chain directly from [`SurfaceOrchestrator`].
    #[cfg(feature = "gesture")]
    pub fn arbitrate_pointer(
        &mut self,
        innermost_surface: SurfaceId,
        sample: PointerSample,
    ) -> Result<ArbitrationDecision, InteropError> {
        let mut stack_chain = [SurfaceId::NULL; SurfaceOrchestrator::INLINE_ANCESTOR_CAP];
        match self
            .surfaces
            .ancestor_chain_into(innermost_surface, &mut stack_chain)?
        {
            Some(len) => Ok(self.gestures.arbitrate(&stack_chain[..len], sample)),
            None => {
                let chain = self.surfaces.ancestor_chain(innermost_surface)?;
                Ok(self.gestures.arbitrate(&chain, sample))
            }
        }
    }

    /// Negotiates layout for `surface_id` using its inherited display scale factor,
    /// updating the surface's recorded viewport size upon success.
    #[cfg(feature = "layout")]
    pub fn negotiate_layout<S: MeasurableSurface + ?Sized>(
        &mut self,
        surface_id: SurfaceId,
        surface: &S,
        constraints: SizeConstraints,
    ) -> Result<LayoutResult, InteropError> {
        let scale_factor = self
            .surfaces
            .get(surface_id)
            .ok_or(InteropError::SurfaceNotFound(surface_id))?
            .environment
            .scale_factor;
        let proposal = LayoutProposal::new(constraints, scale_factor);
        let result = self
            .layout
            .negotiate_surface(surface_id, surface, proposal)?;
        self.surfaces.set_viewport_size(surface_id, result.size)?;
        Ok(result)
    }

    /// Destroys `surface_id` and its entire subtree in bottom-up order, automatically cleaning up
    /// gesture capabilities/locks, cached layouts, channel subscriptions, managed windows, and
    /// mounted plugins BEFORE firing user teardown hooks (so even a panicking hook leaves every
    /// subsystem 100% consistent and every subsequent teardown hook still runs).
    pub fn destroy_surface_cascade(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let (torn_down, pending_hooks) = self.surfaces.destroy_surface_parts(surface_id)?;
        #[cfg(any(
            feature = "gesture",
            feature = "layout",
            feature = "channel",
            feature = "window",
            feature = "plugin-host",
        ))]
        for rec in &torn_down {
            #[cfg(feature = "gesture")]
            self.gestures.unregister_surface(rec.surface_id);
            #[cfg(feature = "layout")]
            self.layout.invalidate_surface(rec.surface_id);
            #[cfg(feature = "channel")]
            self.channels.unsubscribe_surface(rec.surface_id);
            #[cfg(feature = "window")]
            if let Some(handle) = self.windows.handle_for_surface(rec.surface_id) {
                let _ = self.windows.close_window(handle);
            }
            #[cfg(feature = "plugin-host")]
            self.plugins.detach_destroyed_surface(rec.surface_id);
        }
        SurfaceOrchestrator::fire_teardown_hooks(&torn_down, pending_hooks);
        Ok(torn_down)
    }
}

#[cfg(test)]
mod runtime_edge_tests;
#[cfg(test)]
mod runtime_tests;
