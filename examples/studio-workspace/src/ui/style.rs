//! Per-view styling context: the resolved Studio [`Theme`], density/radius [`LayoutMetrics`],
//! and the X-Ray inspector overlay. X-Ray is isolated here so views never branch on it for
//! colors; they ask for a tier's [`Outline`].

use std::fmt;

use dioxus_signals::ReadableExt;
use nve_abi::ColorRgba;
use nve_ui::{ColorHex, Hex, Theme, use_theme};

use crate::theme::{LayoutMetrics, STRETCH_COL, XRayPalette};
use crate::ui::store::use_studio;

/// Border paint of one structural tier, rendered as `border-width: ..; border-color: ..;`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Outline {
    pub width: u32,
    pub color: ColorRgba,
}

impl fmt::Display for Outline {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "border-width: {}px; border-color: {};",
            self.width,
            self.color.hex()
        )
    }
}

/// Everything a Studio view needs to paint itself.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StudioStyle {
    pub t: Theme,
    pub m: LayoutMetrics,
    xray: Option<XRayPalette>,
}

impl StudioStyle {
    pub fn new(t: Theme, m: LayoutMetrics, xray: bool) -> Self {
        Self {
            t,
            m,
            xray: xray.then(|| XRayPalette::for_theme(&t)),
        }
    }

    pub const fn xray(&self) -> bool {
        self.xray.is_some()
    }

    /// Top-level panes (sidebar, workspace, docked Studio).
    pub fn panel(&self) -> Outline {
        match self.xray {
            Some(x) => Outline {
                width: 2,
                color: x.panel,
            },
            None => Outline {
                width: 1,
                color: self.t.border.subtle,
            },
        }
    }

    /// Card surfaces.
    pub fn card(&self) -> Outline {
        match self.xray {
            Some(x) => Outline {
                width: 2,
                color: x.card,
            },
            None => Outline {
                width: 1,
                color: self.t.border.default,
            },
        }
    }

    /// Inner rows and header strips (invisible unless inspecting).
    pub fn row(&self) -> Outline {
        match self.xray {
            Some(x) => Outline {
                width: 1,
                color: x.row,
            },
            None => Outline {
                width: 0,
                color: ColorRgba::TRANSPARENT,
            },
        }
    }

    /// Interactive controls; `active` ones carry the accent indicator (≥3:1 non-text).
    pub fn control(&self, active: bool) -> Outline {
        match self.xray {
            Some(x) => Outline {
                width: 1,
                color: x.row,
            },
            None if active => Outline {
                width: 1,
                color: self.t.status.accent.text,
            },
            None => Outline {
                width: 1,
                color: self.t.border.default,
            },
        }
    }

    /// Fill of a selectable control (nav tab, toggle).
    pub fn control_bg(&self, active: bool) -> Hex {
        if active {
            self.t.status.accent.subtle.hex()
        } else {
            self.t.surface.subtle.hex()
        }
    }

    /// Inline style of a compact `nve-ui` control: `dims` (size declarations), horizontal
    /// padding, corner radius, and an optional selectable paint (`Some(active)`). Fixed-size
    /// controls (`dims` not starting with `flex:`) never shrink.
    pub fn compact_control(
        &self,
        dims: &str,
        pad_x: u32,
        radius: u32,
        paint: Option<bool>,
    ) -> String {
        let paint = paint.map_or_else(String::new, |active| {
            format!(
                "background-color: {}; {}",
                self.control_bg(active),
                self.control(active)
            )
        });
        let shrink = if dims.starts_with("flex:") {
            ""
        } else {
            "flex-shrink: 0; "
        };
        format!(
            "{dims} {shrink}{STRETCH_COL} padding-horizontal: {pad_x}px; padding-vertical: 0px; border-radius: {radius}px; {paint}"
        )
    }

    /// Zebra fill of list row `idx`.
    pub const fn stripe(&self, idx: usize) -> ColorRgba {
        if idx.is_multiple_of(2) {
            self.t.surface.raised
        } else {
            self.t.surface.sunken
        }
    }

    /// Hairline of a list row (the X-Ray row hue while inspecting).
    pub fn list_row_border(&self) -> ColorRgba {
        self.xray.map_or(self.t.border.subtle, |x| x.row)
    }

    /// Label of a selectable control.
    pub fn control_fg(&self, active: bool) -> Hex {
        if active {
            self.t.text.primary.hex()
        } else {
            self.t.text.secondary.hex()
        }
    }
}

/// `data-*` attributes (`(name, value)` pairs) of a node, for web hosts and test probes.
pub(crate) fn data_attrs<const N: usize>(
    pairs: [(&'static str, String); N],
) -> nve_dioxus::TableAttr {
    let pairs: Vec<(String, String)> = pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
    nve_dioxus::table_attr_fn(move |tx, node| {
        tx.edit::<nve_web_html::HtmlDataAttrs>(node)
            .entries(Some(nve_web_html::DataAttrList::from_pairs(pairs.clone())));
    })
}

/// The current [`StudioStyle`]; subscribes the calling view to the theme and the `style` domain
/// (never to the panel toggle or content).
pub(crate) fn use_studio_style() -> StudioStyle {
    let t = use_theme();
    let config = use_studio().style.read().config;
    StudioStyle::new(t, LayoutMetrics::from_studio(&config), config.xray)
}
