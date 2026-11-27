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
#[derive(Default)]
pub struct InteropRuntime {
    pub surfaces: SurfaceOrchestrator,
    #[cfg(feature = "layout")]
    pub layout: LayoutNegotiator,
    #[cfg(feature = "gesture")]
    pub gestures: GestureArbitrator,
    #[cfg(feature = "channel")]
    pub channels: NveChannelBus,
    #[cfg(feature = "window")]
    pub windows: WindowManager,
    #[cfg(feature = "plugin-host")]
    pub plugins: NvePluginHost,
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
        match self.destroy_surface_cascade(meta.surface_id) {
            Ok(_) | Err(InteropError::SurfaceNotFound(_)) => Ok(meta),
            Err(err) => Err(PluginError::SurfaceAllocationFailed(err.to_string())),
        }
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
