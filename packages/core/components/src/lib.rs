//! Typed cross-platform UI primitives (`View`, `Text`, `ScrollView`, `Pressable`, `SkiaCanvas`,
//! and RN Codegen custom components) plus ergonomic style/layout builders and direct binary FFI vector
//! draw-list IR (`NVE_DRAW1`).

pub mod canvas_ir;
pub mod client_hints;
mod css_shorthand;
mod css_values;
pub mod extended_wire;
mod inline_style;
#[cfg(test)]
mod inline_style_tests;
pub mod input_hints;
pub mod interaction;
pub mod keyboard;
pub mod layout_ext;
pub mod media;
pub mod motion;
pub mod motion_clock;
#[cfg(test)]
mod motion_tests;
pub mod native_view;
pub mod paint;
pub mod reconcile;
pub mod spring;
pub mod style_patch;
pub mod svg;
mod tracker;
pub mod transform;

pub use canvas_ir::{
    CanvasOp, DrawList, DrawListBuilder, DrawListDecoder, DrawListError, DrawListHeader,
    DrawListRecorder, ImageResizeMode, DRAW_LIST_ATTR_PREFIX, MAX_DRAW_OPS, MAX_POLYLINE_POINTS,
    NVE_DRAW_HEADER_SIZE, NVE_DRAW_MAGIC, NVE_DRAW_VERSION,
};
pub use client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
pub use css_values::{parse_color, parse_dim, parse_px};
pub use inline_style::{
    apply_typed_attr, apply_typed_attr_kind, apply_typed_attr_with_ext, canonical_raw_style_key,
    ExtStyleDecl, TypedAttrKind,
};
pub use input_hints::{AutofillHint, InputMode};
pub use interaction::{
    resolve_cursor, resolve_layout_and_style, resolve_patch, resolve_style, CursorKind,
    FocusModality, InteractionState, InteractionTracker, StateStyleRule,
};
pub use keyboard::{
    effective_bottom_inset, keyboard_insets_from_visual_viewport, KeyboardInsets,
    VISUAL_VIEWPORT_KEYBOARD_THRESHOLD_PX,
};
pub use layout_ext::{
    compute_sticky_offset, GridItemPlacement, GridLayout, GridLine, GridSingleTrack, GridTrack,
    GridTrackPlacement, GridTrackUnit, LayoutDirection, LayoutExt, LogicalInlineEdges,
    PositionInsets, PositionMode,
};
pub use media::{
    MediaColorScheme, MediaEnv, MediaPointer, MediaQuery, ResponsiveRule, ResponsiveStylePatch,
    MEDIA_FLAG_HAS_MAX_WIDTH, MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION,
    MEDIA_FLAG_REDUCED_MOTION_VAL,
};
pub use motion::{
    lerp_color_rgba, lerp_f32, lerp_style_property, lerp_style_props, resolve_transition_for,
    EasingCurve, MotionClock, MotionCompletion, MotionGroup, MotionKey, MotionTrack,
    TimedTransition, TransitionProperty, TransitionSpec,
};
pub use paint::{
    lerp_color_premul, normalize_stops, sample_gradient_stops, sample_normalized_stops,
    stops_are_normalized, write_css_color, write_css_gradient, BackgroundFill, GradientStop,
    LinearGradient, RadialGradient,
};
pub use reconcile::{
    reconcile_three_way, ConflictPolicy, DivergenceKind, LogicalClock, ReconcileBatch,
    ReconcileDecision, StateReconciler, ThreeWayEntry,
};
pub use spring::{
    MotionPreference, SpringSpec, SpringState, DEFAULT_REST_DISPLACEMENT, DEFAULT_REST_VELOCITY,
};
pub use style_patch::{Display, FlexWrap, Overflow, StylePatch, TextAlign};
pub use svg::{
    compute_svg_content_hash, parse_svg_path_data, write_path_data_into, FillRule, SvgPicture,
    VectorPathCmd, VectorShape, MAX_SVG_PATH_COMMANDS, MAX_SVG_PATH_INPUT_BYTES,
};
pub use transform::{
    affine_concat, affine_inverse_map_point, affine_map_point, lerp_transform_2d, Transform2D,
};

use nve_abi::ComponentKind;

/// Resolves an HTML/RN tag name (from Dioxus `rsx!` or custom frontends) to a typed `ComponentKind`.
pub fn resolve_tag(tag: &str) -> (ComponentKind, Option<&str>) {
    match tag {
        "root" => (ComponentKind::Root, None),
        "view" | "div" | "section" | "main" | "header" | "footer" | "dialog" | "details"
        | "summary" | "form" => (ComponentKind::View, None),
        "text" | "span" | "p" | "h1" | "h2" | "h3" | "label" => (ComponentKind::Text, None),
        "scrollview" | "scroll" => (ComponentKind::ScrollView, None),
        "image" | "img" => (ComponentKind::Image, None),
        "textinput" | "text_input" | "input" | "textarea" => (ComponentKind::TextInput, None),
        "pressable" | "button" => (ComponentKind::Pressable, None),
        "skia-canvas" | "canvas" => (ComponentKind::SkiaCanvas, None),
        other => (ComponentKind::Custom, Some(other)),
    }
}
