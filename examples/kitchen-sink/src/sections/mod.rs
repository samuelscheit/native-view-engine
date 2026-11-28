//! Modular Kitchen Sink gallery sections.

pub mod forms;
pub mod layout;
pub mod motion;
pub mod overview;
pub mod overlays;
pub mod selection;

use nve_abi::FlexAlign;
use nve_dioxus::prelude::*;
use nve_ui::{use_theme, Badge, BadgeVariant};

/// Reusable section heading banner shared across all 6 gallery sections.
#[component]
pub fn SectionHeader(title: String, subtitle: String, tag: String) -> Element {
    let theme = use_theme();

    rsx! {
        view {
            nve_style: TypedStyle::new().width_pct(100.0).flex_col().gap(6.0),
            view {
                nve_style: TypedStyle::new()
                    .flex_row()
                    .flex_wrap(FlexWrap::Wrap)
                    .align_items(FlexAlign::Center)
                    .gap(10.0),
                text {
                    nve_style: TypedStyle::new().font_size(22.0).font_weight(700).color(theme.text.primary),
                    "{title}"
                }
                Badge {
                    label: tag,
                    variant: BadgeVariant::Accent,
                }
            }
            text {
                nve_style: TypedStyle::new().font_size(13.0).color(theme.text.tertiary),
                "{subtitle}"
            }
        }
    }
}

const ONE_COLUMN: MediaQuery = MediaQuery::max_width(767);

/// Reusable 2-column responsive grid that collapses to 1 column on viewports `< 768px`.
/// Its children use [`responsive_two_col_cell`].
pub fn responsive_two_col() -> TypedStyle {
    TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(14.0)
        .at(ONE_COLUMN, RowPatch::new().flex_col())
}

/// Equal-width cell of [`responsive_two_col`]: `flex: 1 1 0%` shares the row, and the stacked
/// column restores `flex: 0 1 auto` (a zero basis there would be a zero *height*).
pub fn responsive_two_col_cell() -> TypedStyle {
    TypedStyle::new()
        .flex_grow(1.0)
        .flex_shrink(1.0)
        .flex_basis_pct(0.0)
        .min_width_px(0.0)
        .at(
            ONE_COLUMN,
            RowPatch::new().flex_grow(0.0).flex_basis(Dimension::Auto),
        )
}
