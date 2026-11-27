//! Dioxus (`dioxus-core 0.7`) frontend adapter for `native-view-engine`.
//!
//! Bridges `dioxus_core::WriteMutations` directly into a single-owner [`nve_store::NodeStore`]
//! via [`nve_store::Transaction`] — feeding Headless, Skia, Web (WASM/SSR), and cross-boundary
//! `NVE2` native hosts identically with zero mirror trees, zero `ComponentKind`, and zero
//! optional module dependencies.

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod dioxus_elements;
pub use dioxus_elements as elements;
pub mod error_boundary;
mod events;
pub mod hot_patch;
pub mod signal_archive;
pub mod stream_slot;
mod surface;
pub mod table_attr;
#[cfg(test)]
mod tests;
pub mod traits;
pub mod typed_style;
mod writer;

pub use elements::{
    CompositionEvent, CompositionEventData, FocusEvent, FocusEventData, KeyEvent, KeyEventData,
    PressEvent, PressEventData, ScrollEvent, ScrollEventData, SubmitEvent, SubmitEventData,
    TableEvent, TableEventData, TextEvent, TextEventData,
};
pub use error_boundary::{
    throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundary, ErrorBoundaryProps,
    ErrorBoundaryReset, ErrorContext, ErrorFallback, UiError, UiErrorReport,
};
pub use nve_components::input_hints::{AutofillHint, InputMode};
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
    MediaEnv, MediaPointer, MediaQuery, ResponsiveRule, MEDIA_FLAG_HAS_MAX_WIDTH,
    MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION, MEDIA_FLAG_REDUCED_MOTION_VAL,
};
pub use nve_components::tables::{Display, FlexWrap, Overflow, TextAlign};
pub use nve_components::{LayoutSnapshot, StylePatchExt, VisualSnapshot};
pub use nve_store::RowPatch;
pub use nve_components::motion::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, resolve_transition_for,
    EasingCurve, MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack,
    TimedTransition, TransitionProperty, TransitionSpec,
};
pub use nve_components::paint::{
    lerp_color_premul, sample_gradient_stops, write_css_gradient, BackgroundFill, GradientStop,
    LinearGradient, RadialGradient,
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
pub use stream_slot::{
    NveStreamSlot, NveStreamSlotProps, StreamSlotRole, SuspenseSlot, SuspenseSlotProps,
    NVE_STREAM_SLOT_ATTR, NVE_STREAM_SLOT_NAME_PREFIX, NVE_STREAM_SLOT_ROLE_ATTR,
};
pub use surface::DioxusSurface;
pub use table_attr::{table_attr_fn, table_attr_fn_with_clear, TableAttr, TableWriter, NVE_TABLE_ATTR};
pub use traits::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, use_environment,
    use_provide_existing_trait_store, use_provide_trait_store, use_trait, use_trait_store,
    BuiltinTraitSlot, BuiltinTraits, CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard,
    HighContrast, HostToolkit, KeyboardInsets, Locale, Orientation, PlatformOnly, PlatformOs,
    PointerKind, ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass,
    TextScale, TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitResolver,
    TraitScope, TraitStore, TraitSwitch, UiDensity, VerticalSizeClass, ViewportChangeMask,
    BUILTIN_TRAIT_COUNT, VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
pub use typed_style::{
    parse_flex_align, parse_flex_direction, parse_flex_justify, PendingResponsiveRule,
    PendingStateRule, PendingStylePatch, ShowOn, TypedStyle, NVE_SHOW_ON_ATTR,
    NVE_TYPED_STYLE_ATTR,
};
pub use writer::{NveMutationWriter, StoreMutationTx};

/// Alias matching `elements::events::onblur` (`FocusEvent`) so blur listeners share one canonical payload type.
pub type BlurEventData = FocusEventData;
pub type BlurEvent = FocusEvent;

/// Lightweight `use_signal` hook backed directly by `dioxus_core::use_hook` + `dioxus_signals::Signal`.
pub fn use_signal<T: 'static>(init: impl FnOnce() -> T) -> dioxus_signals::Signal<T> {
    dioxus_core::use_hook(|| dioxus_signals::Signal::new(init()))
}

/// Payload delivered to Dioxus handlers registered for custom host events.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CustomEventData {
    pub name: String,
    pub data: Vec<u8>,
}

pub type CustomEvent = dioxus_core::Event<CustomEventData>;

/// Prelude imported by Dioxus NVE apps (`use nve_dioxus::prelude::*;`).
pub mod prelude {
    pub use crate::dioxus_elements;
    pub use crate::elements::{
        events::*, CompositionEvent, CompositionEventData, FocusEvent, FocusEventData, KeyEvent,
        KeyEventData, PressEvent, PressEventData, ScrollEvent, ScrollEventData, SubmitEvent,
        SubmitEventData, TableEvent, TableEventData, TextEvent, TextEventData,
    };
    pub use crate::error_boundary::{
        throw_error, throw_ui_error, ui_error, CapturedError, ErrorBoundary, ErrorBoundaryProps,
        ErrorBoundaryReset, ErrorContext, ErrorFallback, UiError, UiErrorReport,
    };
    pub use crate::stream_slot::{NveStreamSlot, StreamSlotRole, SuspenseSlot};
    pub use crate::table_attr::{
        table_attr_fn, table_attr_fn_with_clear, TableAttr, TableWriter, NVE_TABLE_ATTR,
    };
    pub use crate::traits::{
        use_environment, use_provide_existing_trait_store, use_provide_trait_store, use_trait,
        use_trait_store, CanHover, ColorScheme, ContainerSize, HasHardwareKeyboard, HighContrast,
        HostToolkit, KeyboardInsets, Locale, Orientation, PlatformOnly, PlatformOs, PointerKind,
        ReactiveTraitStore, ReducedMotion, SafeAreaInsets, ScaleFactor, SizeClass, TextScale,
        TraitEnvironment, TraitKey, TraitOverrides, TraitProvider, TraitScope, TraitStore,
        TraitSwitch, UiDensity, VerticalSizeClass,
    };
    pub use crate::typed_style::{
        PendingStylePatch, ShowOn, TypedStyle, NVE_SHOW_ON_ATTR, NVE_TYPED_STYLE_ATTR,
    };
    pub use crate::{BlurEvent, BlurEventData, CustomEvent, CustomEventData, DioxusSurface};
    pub use dioxus_core;
    pub use dioxus_core::{
        consume_context, current_scope_id, provide_context, spawn, try_consume_context, use_drop,
        use_hook, Element, IntoDynNode, VNode, VirtualDom,
    };
    pub use dioxus_core_macro::{component, rsx, Props};
    pub use crate::use_signal;
    pub use dioxus_signals;
    pub use dioxus_signals::{ReadableExt, Signal, WritableExt};
    pub use nve_abi::{ColorRgba, Dimension, EdgeInsets, FlexAlign, FlexDirection, FlexJustify};
    pub use nve_components::interaction::{CursorKind, InteractionState, StateStyleRule};
    pub use nve_components::layout_ext::{
        GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit, LayoutDirection,
        PositionMode,
    };
    pub use nve_components::media::{MediaQuery, ResponsiveRule};
    pub use nve_components::tables::Display;
    pub use nve_components::StylePatchExt;
    pub use nve_store::RowPatch;
    pub use nve_components::input_hints::{AutofillHint, InputMode};
    pub use nve_components::motion::{EasingCurve, TransitionProperty, TransitionSpec};
    pub use nve_components::paint::{BackgroundFill, GradientStop, LinearGradient, RadialGradient};
    pub use nve_components::spring::{MotionPreference, SpringSpec, SpringState};
    pub use nve_components::transform::Transform2D;
}
