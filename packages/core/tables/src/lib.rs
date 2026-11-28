//! Cross-platform core word tables (`tables::*`), responsive/state style patches, paint fills,
//! and 2D transforms (zero runtime CSS string parsing).
//!
//! Zero dependencies on optional capability modules (`motion`, `svg`, `canvas`, `blur`, `webview`,
//! `video`, `camera`, `web-element`, `web-html`).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

pub mod client_hints;
pub mod input_hints;
pub mod interaction;
pub mod keyboard;
pub mod layout_ext;
pub mod media;
pub mod paint;
pub mod snapshots;
pub mod style_ext;
pub mod style_layers;
pub mod tables;
pub mod transform;

pub use client_hints::{
    ClientHintsSnapshot, SSR_ACCEPT_CH_HEADER, SSR_CRITICAL_CH_HEADER, SSR_VARY_HEADER,
};
pub use style_layers::{
    dim_to_length, effective_text_color, STYLE_LAYER_ALL, STYLE_LAYER_LAYOUT,
    STYLE_LAYER_TYPOGRAPHY, STYLE_LAYER_VISUAL,
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

#[cfg(test)]
mod tests {
    use super::*;
    use nve_abi::ColorRgba;
    use nve_store::{ChangeSet, FlatResource, Length, NodeStore, Rgba};

    #[test]
    fn applies_typed_row_patch_directly_to_nodestore_word_tables() {
        tables::register_core_tables();
        let mut store = NodeStore::new();
        let mut changes = ChangeSet::default();
        let patch = RowPatch::new()
            .width_pct(100.0)
            .height_px(64.0)
            .flex_row()
            .gap(12.0)
            .padding(16.0)
            .bg(ColorRgba::rgb(15, 23, 42))
            .color(ColorRgba::WHITE)
            .radius(8.0)
            .font_weight(700);
        let mut words = Vec::new();
        assert!(patch.write_words(&mut words));
        let view = RowPatchView::from_words(&words).expect("valid patch view");

        let n = {
            let mut tx = store.tx();
            let n = tx.create();
            tx.apply_row_patch(n, view);
            tx.commit_into(&mut changes);
            n
        };

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

        for expected in [
            tables::Display::Inline,
            tables::Display::InlineBlock,
            tables::Display::InlineFlex,
            tables::Display::Contents,
            tables::Display::Block,
            tables::Display::Grid,
            tables::Display::None,
        ] {
            let dp = RowPatch::new().display(expected);
            words.clear();
            assert!(dp.write_words(&mut words));
            let dp_view = RowPatchView::from_words(&words).expect("valid display patch view");
            let mut tx = store.tx();
            tx.apply_row_patch(n, dp_view);
            tx.commit_into(&mut changes);
            assert_eq!(store.try_row::<tables::Layout>(n).unwrap().display(), expected);
        }
    }
}
