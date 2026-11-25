//! Modular Kitchen Sink gallery sections.

pub mod forms;
pub mod layout;
pub mod motion;
pub mod overview;
pub mod overlays;
pub mod selection;

use nve_dioxus::prelude::*;
use nve_ui::{use_theme, Badge, BadgeVariant};

/// Reusable section heading banner shared across all 6 gallery sections.
#[component]
pub fn SectionHeader(title: String, subtitle: String, tag: String) -> Element {
    let theme = use_theme();
    let text_primary = theme.palette.text_primary.as_ref();
    let text_muted = theme.palette.text_muted.as_ref();

    rsx! {
        view {
            style: "width: 100%; flex-direction: column; gap: 6px;",
            view {
                style: "flex-direction: row; flex-wrap: wrap; align-items: center; gap: 10px;",
                text {
                    style: "font-size: 22px; font-weight: 700; color: {text_primary};",
                    "{title}"
                }
                Badge {
                    label: tag,
                    variant: BadgeVariant::Accent,
                }
            }
            text {
                style: "font-size: 13px; color: {text_muted};",
                "{subtitle}"
            }
        }
    }
}

/// Reusable 2-column responsive grid that collapses to 1 column on viewports `< 768px`.
pub fn responsive_two_col() -> TypedStyle {
    TypedStyle::new()
        .width_pct(100.0)
        .flex_row()
        .gap(14.0)
        .at(
            MediaQuery::max_width(767),
            ResponsiveStylePatch::new().flex_col(),
        )
}
