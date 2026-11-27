//! Cross-platform core word tables (`tables::*`), responsive/state style patches, paint fills,
//! transforms, and shared CSS/color parsers.
//!
//! Zero dependencies on optional capability modules (`motion`, `svg`, `canvas`, `blur`, `webview`,
//! `video`, `camera`, `web-element`, `web-html`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod client_hints;
pub mod css;
pub mod input_hints;
pub mod interaction;
pub mod keyboard;
pub mod layout_ext;
pub mod media;
pub mod paint;
pub mod snapshots;
pub mod style_ext;
pub mod tables;
pub mod transform;

pub use client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
pub use input_hints::{AutofillHint, InputMode};
pub use interaction::{
    resolve_cursor_from_view, resolve_layout_and_style_from_view, resolve_patch_from_view,
    resolve_style_from_view, visit_state_rules_in_cascade, CursorKind, FocusModality,
    InteractionState, StateStyleRule,
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
    ColorScheme, MediaEnv, MediaPointer, MediaQuery, ResponsiveRule, MEDIA_FLAG_HAS_MAX_WIDTH,
    MEDIA_FLAG_HAS_MIN_WIDTH, MEDIA_FLAG_HAS_REDUCED_MOTION, MEDIA_FLAG_REDUCED_MOTION_VAL,
};
pub use nve_store::{RowPatch, RowPatchView};
pub use paint::{
    lerp_color_premul, normalize_stops, sample_gradient_stops, sample_normalized_stops,
    stops_are_normalized, BackgroundFill, GradientStop, LinearGradient, RadialGradient,
};
pub use snapshots::{
    apply_patch_to_layout, apply_patch_to_layout_ext, apply_patch_to_visual,
    apply_patch_view_to_layout, apply_patch_view_to_layout_ext, apply_patch_view_to_visual,
    transform_from_patch, transform_from_patch_view, LayoutSnapshot, VisualSnapshot,
};
pub use style_ext::StylePatchExt;
pub use tables::{Display, FlexWrap, Overflow, TextAlign};
pub use transform::{affine_concat, affine_inverse_map_point, affine_map_point, Transform2D};

pub use css::{
    apply_css_attr_layers_status_to_tx, apply_css_attr_layers_to_tx, apply_css_attr_to_tx,
    apply_inline_style_layers_to_tx, css_attr_layer, css_key_bits, css_key_rank, dim_to_length,
    parse_color, parse_dim, parse_font_weight, parse_px, prop_bits_to_layers, try_parse_dim,
    try_parse_font_weight, CssAttrResult, STYLE_LAYER_ALL, STYLE_LAYER_LAYOUT,
    STYLE_LAYER_TYPOGRAPHY, STYLE_LAYER_VISUAL,
};

#[cfg(test)]
mod tests {
    use super::*;
    use nve_store::{ChangeSet, Length, NodeStore, Rgba};

    #[test]
    fn applies_inline_css_directly_to_nodestore_word_tables() {
        let mut store = NodeStore::new();
        let mut tx = store.tx();
        let n = tx.create();
        assert!(apply_css_attr_to_tx(
            &mut tx,
            n,
            "style",
            "width: 100%; height: 64px; flex-direction: row; gap: 12; padding: 16px; background-color: #0f172a; color: #ffffff; border-radius: 8px; font-weight: bold;"
        ));
        let mut changes = ChangeSet::default();
        tx.commit_into(&mut changes);

        let l = store.try_row::<tables::Layout>(n).unwrap();
        assert_eq!(l.width(), Length::percent(100.0));
        assert_eq!(l.height(), Length::points(64.0));
        assert_eq!(l.flex_direction(), tables::FlexDirection::Row);
        assert_eq!(l.row_gap(), Length::points(12.0));
        assert_eq!(l.column_gap(), Length::points(12.0));
        assert_eq!(l.padding_top(), Length::points(16.0));

        let v = store.try_row::<tables::Visual>(n).unwrap();
        assert_eq!(v.background(), Rgba::rgb(15, 23, 42));
        assert_eq!(v.border_radius(), 8.0);

        let t = store.try_row::<tables::Typography>(n).unwrap();
        assert_eq!(t.color(), Rgba::WHITE);
        assert_eq!(t.font_weight(), 700);

        for (css, expected) in [
            ("display: inline", tables::Display::Inline),
            ("display: inline-block", tables::Display::InlineBlock),
            ("display: inline-flex", tables::Display::InlineFlex),
            ("display: contents", tables::Display::Contents),
            ("display: block", tables::Display::Block),
            ("display: grid", tables::Display::Grid),
            ("display: none", tables::Display::None),
        ] {
            let mut tx = store.tx();
            assert!(apply_css_attr_to_tx(&mut tx, n, "style", css));
            tx.commit_into(&mut changes);
            assert_eq!(store.try_row::<tables::Layout>(n).unwrap().display(), expected);
        }
    }
}
