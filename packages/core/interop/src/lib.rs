//! Universal Framework Compatibility Orchestration Layer for Native View Engine (NVE).
//!
//! Enables embedding NVE components natively inside any UI framework (React Native Fabric,
//! Flutter, SwiftUI/UIKit, Jetpack Compose, Web DOM) with unified surface lifecycle,
//! layout negotiation, gesture arbitration, typed native views, and shared binary channels.

#[cfg(feature = "channel")]
pub mod channel;
#[cfg(feature = "crdt")]
pub mod crdt;
pub mod error;
#[cfg(feature = "gesture")]
pub mod gesture;
#[cfg(feature = "layout")]
pub mod layout;
#[cfg(feature = "native-view")]
pub mod native_view;
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
    DEFAULT_PLUGIN_SURFACE_BASE, NvePluginHost, NvePluginSlot, PluginCapabilities, PluginError,
    PluginId, PluginMetadata, PluginRequirement, PluginSourceKind,
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
    ColorScheme, EnvironmentOverrides, FrameworkKind, LifecycleState, SafeAreaInsets,
    SurfaceDescriptor, SurfaceEnvironment, SurfaceEvent, SurfaceOrchestrator, SurfaceRecord,
    TeardownRecord, ThemeContext,
};
#[cfg(feature = "window")]
pub use window::{
    ManagedWindow, WindowCommand, WindowDescriptor, WindowError, WindowFlags, WindowHandleId,
    WindowLifecycleEvent, WindowLifecycleState, WindowManager, WindowRect, WindowState,
};
#[cfg(feature = "native-view")]
pub use native_view::{
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, MAX_NATIVE_CMD_ARGS,
    MAX_NATIVE_PROP_FIELDS, NATIVE_CMD_HEADER_SIZE, NATIVE_CMD_MAGIC, NATIVE_CMD_VERSION,
    NATIVE_PROPS_HEADER_SIZE, NATIVE_PROPS_MAGIC, NATIVE_PROPS_VERSION, NATIVE_VIEW_KIND_PROP_KEY,
    NATIVE_VIEW_PROPS_PROP_KEY, NativeCommand, NativeCommandBuilder, NativeCommandDecoder,
    NativeCommandEncoder, NativeCommandSink, NativeCommandSource, NativeProp, NativePropDecoder,
    NativePropEncoder, NativePropSink, NativePropSource, NativePropValue, NativeProps,
    NativePropsBuilder, NativeValueRef, NativeViewCodecError,
    NativeViewHandle as TypedNativeViewHandle, NativeViewHandleBinder, NativeViewRegistry,
    NativeViewSpec, NoCommand, QueuedNativeCommand, TypedNativeViewCommand, TypedNativeViewProps,
    VideoPlayer, VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView,
    WebViewCommand, WebViewProps,
};
#[cfg(feature = "traits")]
pub use traits::{
    BUILTIN_TRAIT_COUNT, BuiltinTraitSlot, BuiltinTraits, CanHover,
    ColorScheme as TraitColorScheme, ContainerSize, DeviceFormFactor, HasHardwareKeyboard,
    HighContrast, HostToolkit, KeyboardInsets, Locale, Orientation, PlatformOs, PointerKind,
    ReducedMotion, ScaleFactor, SizeClass, TextScale, TraitEnvironment, TraitKey, TraitOverrides,
    TraitResolver, TraitStore, UiDensity, VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX, VerticalSizeClass,
    ViewportChangeMask, effective_bottom_inset, keyboard_insets_from_visual_viewport,
};

use nve_abi::SurfaceId;

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
        let chain = self.surfaces.ancestor_chain(innermost_surface)?;
        Ok(self.gestures.arbitrate(&chain, sample))
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
    /// gesture capabilities/locks, cached layouts, and channel subscriptions for every torn-down surface.
    pub fn destroy_surface_cascade(
        &mut self,
        surface_id: SurfaceId,
    ) -> Result<Vec<TeardownRecord>, InteropError> {
        let torn_down = self.surfaces.destroy_surface(surface_id)?;
        #[cfg(any(
            feature = "gesture",
            feature = "layout",
            feature = "channel",
            feature = "window",
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
        }
        Ok(torn_down)
    }
}

#[cfg(test)]
mod runtime_edge_tests;
#[cfg(test)]
mod runtime_tests;
