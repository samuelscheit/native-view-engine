//! Raw CSS style props (`font-weight`, `text-align`, `box-shadow`, `overflow`, `flex-wrap`,
//! object fit): one slot table routes and stores them, and values are validated at ingestion so a
//! raw prop can only ever become one CSS declaration value.

use nve_abi::num::{write_f64, write_i64};
use nve_abi::RawPropValue;
use nve_components::canvas_ir::ImageResizeMode;

/// Maps `resizeMode` / `resize_mode` / `objectFit` / `object-fit` raw prop values to CSS
/// `object-fit` keywords.
pub fn raw_prop_to_object_fit(value: &RawPropValue) -> Option<&'static str> {
    match value {
        RawPropValue::String(raw) => {
            let trimmed = raw.trim();
            if let Some(mode) = ImageResizeMode::parse(trimmed) {
                return Some(match mode {
                    ImageResizeMode::Cover => "cover",
                    ImageResizeMode::Contain => "contain",
                    ImageResizeMode::Stretch => "fill",
                    ImageResizeMode::Center => "none",
                });
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "fill" => Some("fill"),
                "none" => Some("none"),
                "scale-down" | "scaledown" => Some("scale-down"),
                _ => None,
            }
        }
        RawPropValue::I64(n) => {
            let mode = u8::try_from(*n).ok().and_then(ImageResizeMode::from_u8)?;
            Some(match mode {
                ImageResizeMode::Cover => "cover",
                ImageResizeMode::Contain => "contain",
                ImageResizeMode::Stretch => "fill",
                ImageResizeMode::Center => "none",
            })
        }
        _ => None,
    }
}

/// Returns `true` if `key` names an image fit prop (`resizeMode`, `resize_mode`, `objectFit`,
/// or `object-fit`).
#[inline]
pub fn is_image_fit_prop(key: &str) -> bool {
    matches!(key, "resizeMode" | "resize_mode" | "objectFit" | "object-fit")
}

/// The CSS declaration slot a raw prop key fills. One table drives both routing
/// ([`is_raw_style_prop`]) and storage ([`RawStyleProps::apply_prop`]), so a key can never be
/// routed as a style without being stored, or stored without being routed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RawStyleSlot {
    Display,
    FontWeight,
    FontFamily,
    FontStyle,
    TextAlign,
    TextDecoration,
    LineHeight,
    BoxShadow,
    Overflow,
    OverflowX,
    OverflowWrap,
    WordBreak,
    WhiteSpace,
    VerticalAlign,
    BorderCollapse,
    FlexWrap,
    ObjectFit,
}

fn raw_style_slot(key: &str) -> Option<RawStyleSlot> {
    Some(match key {
        "display" => RawStyleSlot::Display,
        "font-weight" | "fontWeight" | "font_weight" => RawStyleSlot::FontWeight,
        "font-family" | "fontFamily" | "font_family" => RawStyleSlot::FontFamily,
        "font-style" | "fontStyle" | "font_style" => RawStyleSlot::FontStyle,
        "text-align" | "textAlign" | "text_align" => RawStyleSlot::TextAlign,
        "text-decoration" | "textDecoration" | "text_decoration" => RawStyleSlot::TextDecoration,
        "line-height" | "lineHeight" | "line_height" => RawStyleSlot::LineHeight,
        "box-shadow" | "boxShadow" | "box_shadow" => RawStyleSlot::BoxShadow,
        "overflow" => RawStyleSlot::Overflow,
        "overflow-x" | "overflowX" | "overflow_x" => RawStyleSlot::OverflowX,
        "overflow-wrap" | "overflowWrap" | "overflow_wrap" | "word-wrap" | "wordWrap" => {
            RawStyleSlot::OverflowWrap
        }
        "word-break" | "wordBreak" | "word_break" => RawStyleSlot::WordBreak,
        "white-space" | "whiteSpace" | "white_space" => RawStyleSlot::WhiteSpace,
        "vertical-align" | "verticalAlign" | "vertical_align" => RawStyleSlot::VerticalAlign,
        "border-collapse" | "borderCollapse" | "border_collapse" => RawStyleSlot::BorderCollapse,
        "flex-wrap" | "flexWrap" | "flex_wrap" => RawStyleSlot::FlexWrap,
        _ if is_image_fit_prop(key) => RawStyleSlot::ObjectFit,
        _ => return None,
    })
}

/// Validates and canonicalizes CSS `display` keywords for [`RawStyleProps`].
fn raw_prop_to_display(value: &RawPropValue) -> Option<&'static str> {
    let RawPropValue::String(raw) = value else {
        return None;
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "flex" => Some("flex"),
        "inline-flex" => Some("inline-flex"),
        "block" => Some("block"),
        "inline" => Some("inline"),
        "inline-block" => Some("inline-block"),
        "grid" => Some("grid"),
        "inline-grid" => Some("inline-grid"),
        "none" => Some("none"),
        "contents" => Some("contents"),
        "table" => Some("table"),
        "table-header-group" => Some("table-header-group"),
        "table-row-group" => Some("table-row-group"),
        "table-row" => Some("table-row"),
        "table-cell" => Some("table-cell"),
        _ => None,
    }
}

/// Returns `true` if `key` names a CSS style raw prop captured by [`RawStyleProps`] rather than
/// an HTML attribute.
#[inline]
pub fn is_raw_style_prop(key: &str) -> bool {
    raw_style_slot(key).is_some()
}

/// Compact structured CSS properties routed from `Command::SetRawProp` instead of being written
/// as invalid HTML attributes.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RawStyleProps {
    pub display: Option<&'static str>,
    pub font_weight: Option<String>,
    pub font_family: Option<String>,
    pub font_style: Option<String>,
    pub text_align: Option<String>,
    pub text_decoration: Option<String>,
    pub line_height: Option<String>,
    pub box_shadow: Option<String>,
    pub overflow: Option<String>,
    pub overflow_x: Option<String>,
    pub overflow_wrap: Option<String>,
    pub word_break: Option<String>,
    pub white_space: Option<String>,
    pub vertical_align: Option<String>,
    pub border_collapse: Option<String>,
    pub flex_wrap: Option<String>,
    pub object_fit: Option<&'static str>,
}

/// Returns `true` if `value` can only ever be one CSS declaration value. The allow-list leaves
/// out `; { } < > " ' \ * : !` and control characters, so a value cannot end the declaration or
/// rule, open a comment, string or escape, or close the `<style>` element in SSR. The structural
/// rules land in a shared stylesheet, so one hostile value would otherwise style or hide any node.
pub fn is_safe_css_value(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || matches!(b, b' ' | b'#' | b'.' | b',' | b'(' | b')' | b'%' | b'+' | b'-' | b'/'))
}

fn raw_prop_css_value(value: &RawPropValue) -> Option<String> {
    let mut out = String::new();
    match value {
        RawPropValue::String(s) => out.push_str(s.trim()),
        RawPropValue::I64(n) => write_i64(&mut out, *n),
        RawPropValue::F64(n) if n.is_finite() => write_f64(&mut out, *n),
        _ => return None,
    }
    (!out.is_empty() && is_safe_css_value(&out)).then_some(out)
}

impl RawStyleProps {
    /// Applies a raw prop if `key` is a recognized CSS style prop. Returns `true` when handled.
    /// `Null`, empty and unsafe values (see [`is_safe_css_value`]) clear the slot.
    pub fn apply_prop(&mut self, key: &str, value: &RawPropValue) -> bool {
        let Some(slot) = raw_style_slot(key) else {
            return false;
        };
        let slot_value = match slot {
            RawStyleSlot::Display => {
                self.display = raw_prop_to_display(value);
                return true;
            }
            RawStyleSlot::FontWeight => &mut self.font_weight,
            RawStyleSlot::FontFamily => &mut self.font_family,
            RawStyleSlot::FontStyle => &mut self.font_style,
            RawStyleSlot::TextAlign => &mut self.text_align,
            RawStyleSlot::TextDecoration => &mut self.text_decoration,
            RawStyleSlot::LineHeight => &mut self.line_height,
            RawStyleSlot::BoxShadow => &mut self.box_shadow,
            RawStyleSlot::Overflow => &mut self.overflow,
            RawStyleSlot::OverflowX => &mut self.overflow_x,
            RawStyleSlot::OverflowWrap => &mut self.overflow_wrap,
            RawStyleSlot::WordBreak => &mut self.word_break,
            RawStyleSlot::WhiteSpace => &mut self.white_space,
            RawStyleSlot::VerticalAlign => &mut self.vertical_align,
            RawStyleSlot::BorderCollapse => &mut self.border_collapse,
            RawStyleSlot::FlexWrap => &mut self.flex_wrap,
            RawStyleSlot::ObjectFit => {
                self.object_fit = raw_prop_to_object_fit(value);
                return true;
            }
        };
        *slot_value = raw_prop_css_value(value);
        true
    }

    #[inline]
    pub fn effective_display(&self) -> Option<&'static str> {
        self.display.filter(|&v| v != "flex")
    }

    #[inline]
    pub fn effective_font_weight(&self) -> Option<&str> {
        self.font_weight
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal") && *v != "400")
    }

    #[inline]
    pub fn effective_font_family(&self) -> Option<&str> {
        self.font_family.as_deref()
    }

    #[inline]
    pub fn effective_font_style(&self) -> Option<&str> {
        self.font_style
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal"))
    }

    #[inline]
    pub fn effective_text_align(&self) -> Option<&str> {
        self.text_align
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("left") && !v.eq_ignore_ascii_case("start"))
    }

    #[inline]
    pub fn effective_text_decoration(&self) -> Option<&str> {
        self.text_decoration
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("none"))
    }

    #[inline]
    pub fn effective_line_height(&self) -> Option<&str> {
        self.line_height
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal"))
    }

    #[inline]
    pub fn effective_box_shadow(&self) -> Option<&str> {
        self.box_shadow
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("none"))
    }

    #[inline]
    pub fn effective_overflow(&self) -> Option<&str> {
        self.overflow
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("visible"))
    }

    #[inline]
    pub fn effective_overflow_x(&self) -> Option<&str> {
        self.overflow_x
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("visible"))
    }

    #[inline]
    pub fn effective_overflow_wrap(&self) -> Option<&str> {
        self.overflow_wrap
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal"))
    }

    #[inline]
    pub fn effective_word_break(&self) -> Option<&str> {
        self.word_break
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal"))
    }

    #[inline]
    pub fn effective_white_space(&self) -> Option<&str> {
        self.white_space
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal"))
    }

    #[inline]
    pub fn effective_vertical_align(&self) -> Option<&str> {
        self.vertical_align
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("baseline"))
    }

    #[inline]
    pub fn effective_border_collapse(&self) -> Option<&str> {
        self.border_collapse
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("separate"))
    }

    #[inline]
    pub fn effective_flex_wrap(&self) -> Option<&str> {
        self.flex_wrap
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("nowrap"))
    }

    /// Appends non-default raw style declarations into `out`.
    pub fn write_compact_into(&self, skip_typography: bool, out: &mut String) {
        if let Some(d) = self.effective_display() {
            out.push_str("display:");
            out.push_str(d);
            out.push(';');
        }
        if !skip_typography {
            for (prop, val) in [
                ("font-weight:", self.effective_font_weight()),
                ("font-family:", self.effective_font_family()),
                ("font-style:", self.effective_font_style()),
                ("text-align:", self.effective_text_align()),
                ("text-decoration:", self.effective_text_decoration()),
                ("line-height:", self.effective_line_height()),
            ] {
                if let Some(v) = val {
                    out.push_str(prop);
                    out.push_str(v);
                    out.push(';');
                }
            }
        }
        for (prop, val) in [
            ("box-shadow:", self.effective_box_shadow()),
            ("overflow:", self.effective_overflow()),
            ("overflow-x:", self.effective_overflow_x()),
            ("overflow-wrap:", self.effective_overflow_wrap()),
            ("word-break:", self.effective_word_break()),
            ("white-space:", self.effective_white_space()),
            ("vertical-align:", self.effective_vertical_align()),
            ("border-collapse:", self.effective_border_collapse()),
            ("flex-wrap:", self.effective_flex_wrap()),
            ("object-fit:", self.object_fit),
        ] {
            if let Some(v) = val {
                out.push_str(prop);
                out.push_str(v);
                out.push(';');
            }
        }
    }
}
