//! Dioxus (`dioxus-core 0.7`) frontend adapter for `native-view-engine`.
//!
//! Implements `dioxus_core::WriteMutations` to emit a single flat, typed binary `CommandBuffer`
//! per frame — feeding Fabric (iOS/Android), Skia (Desktop/Mobile), Web (WASM), and Headless
//! backends identically.

pub mod canvas;
pub mod dioxus_elements;
pub use dioxus_elements as elements;
pub mod error_boundary;
mod events;
pub mod hot_patch;
pub mod native_view;
pub mod signal_archive;
mod surface;
#[cfg(test)]
mod tests;
pub mod traits;
pub mod typed_style;
mod writer;

pub use canvas::CanvasDrawList;
pub use elements::{
    FocusEvent, FocusEventData, KeyEvent, KeyEventData, ScrollEvent, ScrollEventData, SubmitEvent,
    SubmitEventData,
};
pub use error_boundary::{
    throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundary, ErrorBoundaryProps,
    ErrorBoundaryReset, ErrorContext, ErrorFallback, UiError, UiErrorReport,
};
pub use nve_components::canvas_ir::{CanvasOp, DrawList, DrawListRecorder};
pub use nve_components::input_hints::{AutofillHint, InputMode};
pub use native_view::{
    use_native_view_handle, use_native_view_registry, use_provide_native_view_registry,
    BlurMaterial, BlurSurface, BlurSurfaceProps, DecodeNativeCommand, DecodeNativeProps,
    DecodedWebElementProps, EncodeNativeCommand, EncodeNativeProps, NativeCommand,
    NativeCommandDecoder, NativeCommandEncoder, NativeProp, NativePropDecoder, NativePropEncoder,
    NativePropValue, NativeProps, NativeValueRef, NativeView, NativeViewCodecError,
    NativeViewHandle, NativeViewProvider, NativeViewRegistry, NativeViewSpec, NoCommand,
    QueuedNativeCommand, TypedNativeViewCommand, TypedNativeViewProps, VideoPlayer,
    VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand,
    WebViewProps, NATIVE_CMD_MAGIC, NATIVE_PROPS_MAGIC, NATIVE_VIEW_KIND_PROP_KEY,
    NATIVE_VIEW_PROPS_PROP_KEY,
};
pub use nve_components::interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use nve_components::layout_ext::{
    compute_sticky_offset, GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack,
    GridTrackPlacement, GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges,
    PositionInsets, PositionMode,
};
pub use nve_components::media::{
    Display, FlexWrap, MediaColorScheme, MediaEnv, MediaPointer, MediaQuery, Overflow,
    ResponsiveRule, ResponsiveStylePatch, StylePatch, TextAlign, MEDIA_FLAG_HAS_MAX_WIDTH,
    MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION, MEDIA_FLAG_REDUCED_MOTION_VAL,
};
pub use nve_components::motion::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, resolve_transition_for,
    EasingCurve, MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack,
    TimedTransition, TransitionProperty, TransitionSpec,
};
pub use nve_components::spring::{MotionPreference, SpringSpec, SpringState};
pub use nve_components::transform::{
    affine_concat, affine_inverse_map_point, affine_map_point, lerp_transform_2d, Transform2D,
};
pub use signal_archive::{
    reconcile_signal_entry, signal_slot_id, BinValueSlot, HydrationReport, SignalArchive,
    SignalArchiveError, SignalCodec, SignalDelta, SignalDeltaOp, SignalRegistry,
    SignalTableSnapshot, SignalValue, SignalValueTag, NVE_SIGNAL_ARCHIVE_MAGIC,
    NVE_SIGNAL_ARCHIVE_VERSION, NVE_SIGNAL_DELTA_MAGIC,
};
pub use surface::DioxusSurface;
pub use traits::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, use_environment,
    use_provide_existing_trait_store, use_provide_trait_store, use_trait, use_trait_store,
    BuiltinTraitSlot, BuiltinTraits, CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard,
    HighContrast, HostToolkit, KeyboardInsets, Locale, Orientation, PlatformOnly, PlatformOs,
    PointerKind, ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass,
    TextScale, TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitResolver,
    TraitScope, TraitStore, TraitSwitch, TraitValueScope, UiDensity, VerticalSizeClass,
    ViewportChangeMask, VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
pub use typed_style::{
    parse_flex_align, parse_flex_direction, parse_flex_justify, ShowOn, TypedStyle,
    NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR,
};
pub use writer::NveMutationWriter;

use dioxus_core::Event;

/// Alias matching `elements::events::onblur` (`FocusEvent`) so blur listeners share one canonical payload type.
pub type BlurEventData = FocusEventData;
pub type BlurEvent = FocusEvent;

/// Payload for generic native `Custom` events dispatched into Dioxus custom event listeners.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomEventData {
    pub name: String,
    pub data: Vec<u8>,
}

pub type CustomEvent = Event<CustomEventData>;

pub mod prelude {
    pub use crate::canvas::CanvasDrawList;
    pub use crate::elements::{
        self as dioxus_elements, events::*, KeyEvent, KeyEventData, PressEvent, SubmitEvent,
        SubmitEventData, TextEvent,
    };
    pub use crate::native_view::{
        use_native_view_handle, use_native_view_registry, use_provide_native_view_registry,
        BlurMaterial, BlurSurface, BlurSurfaceProps, NativeCommand, NativeProps, NativeView,
        NativeViewCodecError, NativeViewHandle, NativeViewProvider, NativeViewRegistry,
        NativeViewSpec, TypedNativeViewCommand, TypedNativeViewProps, VideoPlayer,
        VideoPlayerCommand, VideoPlayerProps, WebElement, WebElementProps, WebView, WebViewCommand,
        WebViewProps,
    };
    pub use nve_components::canvas_ir::{CanvasOp, DrawList, DrawListRecorder};
    pub use crate::error_boundary::{
        throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundary, ErrorBoundaryProps,
        ErrorBoundaryReset, ErrorContext, ErrorFallback, UiError, UiErrorReport,
    };
    pub use crate::traits::{
        effective_bottom_inset, keyboard_insets_from_visual_viewport, use_environment,
        use_provide_existing_trait_store, use_provide_trait_store, use_trait, use_trait_store,
        CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard, HighContrast, HostToolkit,
        KeyboardInsets, Locale, Orientation, PlatformOnly, PlatformOs, PointerKind,
        ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass, TextScale,
        TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitScope, TraitStore,
        TraitSwitch, TraitValueScope, UiDensity, VerticalSizeClass,
    };
    pub use nve_components::input_hints::{AutofillHint, InputMode};
    pub use crate::typed_style::{ShowOn, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR};
    pub use crate::use_signal;
    pub use crate::{
        BlurEvent, BlurEventData, CustomEvent, CustomEventData, FocusEvent, FocusEventData,
        ScrollEvent, ScrollEventData,
    };
    pub use nve_components::interaction::{
        CursorKind, FocusModality, InteractionState, InteractionTracker, StateStyleRule,
    };
    pub use nve_components::layout_ext::{
        GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack, GridTrackPlacement,
        GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges, PositionInsets,
        PositionMode,
    };
    pub use nve_components::media::{
        Display, FlexWrap, MediaColorScheme, MediaEnv, MediaPointer, MediaQuery, Overflow,
        ResponsiveRule, ResponsiveStylePatch, StylePatch, TextAlign,
    };
    pub use nve_components::motion::{
        EasingCurve, MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack,
        TimedTransition, TransitionProperty, TransitionSpec,
    };
    pub use nve_components::spring::{MotionPreference, SpringSpec, SpringState};
    pub use nve_components::transform::Transform2D;
    pub use dioxus_core;
    pub use dioxus_core::{Element, VirtualDom};
    pub use dioxus_core_macro::{component, rsx, Props};
    pub use dioxus_signals;
    pub use dioxus_signals::{ReadableExt, Signal, WritableExt};
}

/// Lightweight `use_signal` hook backed directly by `dioxus_core::use_hook` + `dioxus_signals::Signal`.
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> dioxus_signals::Signal<T> {
    dioxus_core::use_hook(|| dioxus_signals::Signal::new(init()))
}
