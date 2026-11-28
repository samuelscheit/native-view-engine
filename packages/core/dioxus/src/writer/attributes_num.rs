//! Numeric (`f64`/`i64`) attribute writers for [`super::StoreMutationTx`] (zero string parsing).

use nve_store::{Length, NodeId, Transaction};
use nve_tables::tables::{Accessibility, Layout, Scroll, Typography, Visual};

/// Applies a numeric attribute directly to the store. Returns `true` iff `name` is a style
/// attribute that should be preserved across `style`/`nve_style` clears/updates.
#[inline]
pub(crate) fn apply_numeric_attr_untracked(
    tx: &mut Transaction<'_>,
    node: NodeId,
    name: &str,
    v: f64,
) -> bool {
    let max = nve_abi::num::MAX_STYLE_SCALAR;
    let f = if v.is_finite() { (v as f32).clamp(-max, max) } else { 0.0 };
    match name {
        "width" => {
            tx.edit::<Layout>(node).width(Length::points(f.max(0.0)));
        }
        "height" => {
            tx.edit::<Layout>(node).height(Length::points(f.max(0.0)));
        }
        "min-width" | "minWidth" => {
            tx.edit::<Layout>(node).min_width(Length::points(f.max(0.0)));
        }
        "min-height" | "minHeight" => {
            tx.edit::<Layout>(node).min_height(Length::points(f.max(0.0)));
        }
        "max-width" | "maxWidth" => {
            tx.edit::<Layout>(node).max_width(Length::points(f.max(0.0)));
        }
        "max-height" | "maxHeight" => {
            tx.edit::<Layout>(node).max_height(Length::points(f.max(0.0)));
        }
        "flex" | "flex-grow" | "flexGrow" => {
            tx.edit::<Layout>(node).flex_grow(f.max(0.0));
        }
        "flex-shrink" | "flexShrink" => {
            tx.edit::<Layout>(node).flex_shrink(f.max(0.0));
        }
        "flex-basis" | "flexBasis" => {
            tx.edit::<Layout>(node).flex_basis(Length::points(f.max(0.0)));
        }
        "gap" => {
            let l = Length::points(f.max(0.0));
            tx.edit::<Layout>(node).row_gap(l).column_gap(l);
        }
        "row-gap" | "rowGap" => {
            tx.edit::<Layout>(node).row_gap(Length::points(f.max(0.0)));
        }
        "column-gap" | "columnGap" => {
            tx.edit::<Layout>(node).column_gap(Length::points(f.max(0.0)));
        }
        "padding" => {
            let l = Length::points(f.max(0.0));
            tx.edit::<Layout>(node)
                .padding_top(l)
                .padding_right(l)
                .padding_bottom(l)
                .padding_left(l);
        }
        "padding-horizontal" | "paddingHorizontal" => {
            let l = Length::points(f.max(0.0));
            tx.edit::<Layout>(node).padding_left(l).padding_right(l);
        }
        "padding-vertical" | "paddingVertical" => {
            let l = Length::points(f.max(0.0));
            tx.edit::<Layout>(node).padding_top(l).padding_bottom(l);
        }
        "margin" => {
            let l = Length::points(f);
            tx.edit::<Layout>(node)
                .margin_top(l)
                .margin_right(l)
                .margin_bottom(l)
                .margin_left(l);
        }
        "border-width" | "borderWidth" => {
            tx.edit::<Visual>(node).border_width(f.max(0.0));
        }
        "border-radius" | "borderRadius" => {
            tx.edit::<Visual>(node).border_radius(f.max(0.0));
        }
        "opacity" => {
            tx.edit::<Visual>(node).opacity(f.clamp(0.0, 1.0));
        }
        "font-size" | "fontSize" => {
            tx.edit::<Typography>(node).font_size(f.max(1.0));
        }
        "z-index" | "zIndex" => {
            tx.edit::<Visual>(node).z_index(f as i32);
        }
        "requestedScrollX" | "requested-scroll-x" | "requested_scroll_x" => {
            let val = v.is_finite().then_some(v as f32);
            tx.edit::<Scroll>(node).requested_scroll_x(val);
            return false;
        }
        "requestedScrollY" | "requested-scroll-y" | "requested_scroll_y" => {
            let val = v.is_finite().then_some(v as f32);
            tx.edit::<Scroll>(node).requested_scroll_y(val);
            return false;
        }
        "requestedScrollSeq" | "requested-scroll-seq" | "requested_scroll_seq" => {
            let seq = if v.is_finite() && v >= 0.0 {
                (v as u64).min(u64::from(u32::MAX)) as u32
            } else {
                0
            };
            tx.edit::<Scroll>(node).requested_scroll_seq(seq);
            return false;
        }
        "ariaValueNow" | "aria-valuenow" | "aria_valuenow" => {
            tx.edit::<Accessibility>(node).value_now(Some(f));
            return false;
        }
        "ariaValueMin" | "aria-valuemin" | "aria_valuemin" => {
            tx.edit::<Accessibility>(node).value_min(Some(f));
            return false;
        }
        "ariaValueMax" | "aria-valuemax" | "aria_valuemax" => {
            tx.edit::<Accessibility>(node).value_max(Some(f));
            return false;
        }
        _ => return false,
    }
    true
}
