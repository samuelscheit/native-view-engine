//! Per-view styling context: the resolved Studio [`Theme`], density/radius [`LayoutMetrics`],
//! and the X-Ray inspector overlay. X-Ray is isolated here so views never branch on it for
//! colors; they ask for a tier's [`Outline`].

use dioxus_signals::ReadableExt;
use nve_abi::{ColorRgba, FlexAlign, FlexJustify};
use nve_dioxus::TypedStyle;
use nve_ui::{Theme, use_theme};

use crate::theme::{LayoutMetrics, XRayPalette};
use crate::ui::store::use_studio;

/// Border paint of one structural tier.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Outline {
    pub width: u32,
    pub color: ColorRgba,
}

/// Studio-specific fluent builders layered onto [`TypedStyle`].
pub(crate) trait StudioStyleExt: Sized {
    fn outline(self, outline: Outline) -> Self;
    fn stretch_col(self) -> Self;
}

impl StudioStyleExt for TypedStyle {
    #[inline]
    fn outline(self, outline: Outline) -> Self {
        self.border(outline.width as f32, outline.color)
    }

    #[inline]
    fn stretch_col(self) -> Self {
        self.flex_col()
            .justify_content(FlexJustify::Center)
            .align_items(FlexAlign::Stretch)
    }
}

/// Fixed width + fixed height (non-shrinking) dimensions for compact controls.
#[inline]
pub(crate) fn fixed_dims(w: f32, h: f32) -> TypedStyle {
    TypedStyle::new()
        .width_px(w)
        .height_px(h)
        .min_height_px(h)
        .flex_shrink(0.0)
}

/// Minimum width + fixed height (non-shrinking) dimensions for compact pill buttons.
#[inline]
pub(crate) fn min_w_dims(min_w: f32, h: f32) -> TypedStyle {
    TypedStyle::new()
        .min_width_px(min_w)
        .height_px(h)
        .min_height_px(h)
        .flex_shrink(0.0)
}

/// Minimum width + fixed height (non-shrinking) dimensions for compact badges.
#[inline]
pub(crate) fn badge_dims(min_w: f32, h: f32) -> TypedStyle {
    TypedStyle::new()
        .min_width_px(min_w)
        .height_px(h)
        .flex_shrink(0.0)
}

/// Flex-1 + fixed height dimensions for segmented/equal-width controls.
#[inline]
pub(crate) fn flex_dims(h: f32) -> TypedStyle {
    TypedStyle::new().flex_1().height_px(h).min_height_px(h)
}

/// Text color + font-size shorthand.
#[inline]
pub(crate) fn text_style(color: ColorRgba, px: f32) -> TypedStyle {
    TypedStyle::new().color(color).font_size(px)
}

/// Full-width non-shrinking horizontal space-between row.
#[inline]
pub(crate) fn row_between() -> TypedStyle {
    TypedStyle::new()
        .width_pct(100.0)
        .flex_shrink(0.0)
        .flex_row()
        .justify_content(FlexJustify::SpaceBetween)
        .align_items(FlexAlign::Center)
}

/// Horizontal centered row with uniform gap.
#[inline]
pub(crate) fn row_center(gap: f32) -> TypedStyle {
    TypedStyle::new()
        .flex_row()
        .align_items(FlexAlign::Center)
        .gap(gap)
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
    pub const fn control_bg(&self, active: bool) -> ColorRgba {
        if active {
            self.t.status.accent.subtle
        } else {
            self.t.surface.subtle
        }
    }

    /// Typed style of a compact `nve-ui` control: base sizing, horizontal padding, corner radius,
    /// and an optional selectable paint (`Some(active)`).
    pub fn compact_control(
        &self,
        base: TypedStyle,
        pad_x: u32,
        radius: u32,
        paint: Option<bool>,
    ) -> TypedStyle {
        let mut style = base
            .stretch_col()
            .padding_xy(pad_x as f32, 0.0)
            .radius(radius as f32);
        if let Some(active) = paint {
            style = style
                .bg(self.control_bg(active))
                .outline(self.control(active));
        }
        style
    }

    /// Full-width non-shrinking card surface with uniform padding, gap, background, radius, and
    /// card outline.
    pub fn card_surface(&self, pad: u32, gap: u32, bg: ColorRgba, radius: u32) -> TypedStyle {
        TypedStyle::new()
            .width_pct(100.0)
            .flex_shrink(0.0)
            .padding(pad as f32)
            .gap(gap as f32)
            .bg(bg)
            .radius(radius as f32)
            .outline(self.card())
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
    pub const fn control_fg(&self, active: bool) -> ColorRgba {
        if active {
            self.t.text.primary
        } else {
            self.t.text.secondary
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
