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
    FontWeight,
    TextAlign,
    BoxShadow,
    Overflow,
    FlexWrap,
    ObjectFit,
}

fn raw_style_slot(key: &str) -> Option<RawStyleSlot> {
    Some(match key {
        "font-weight" | "fontWeight" | "font_weight" => RawStyleSlot::FontWeight,
        "text-align" | "textAlign" | "text_align" => RawStyleSlot::TextAlign,
        "box-shadow" | "boxShadow" | "box_shadow" => RawStyleSlot::BoxShadow,
        "overflow" => RawStyleSlot::Overflow,
        "flex-wrap" | "flexWrap" | "flex_wrap" => RawStyleSlot::FlexWrap,
        _ if is_image_fit_prop(key) => RawStyleSlot::ObjectFit,
        _ => return None,
    })
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
    pub font_weight: Option<String>,
    pub text_align: Option<String>,
    pub box_shadow: Option<String>,
    pub overflow: Option<String>,
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
            RawStyleSlot::FontWeight => &mut self.font_weight,
            RawStyleSlot::TextAlign => &mut self.text_align,
            RawStyleSlot::BoxShadow => &mut self.box_shadow,
            RawStyleSlot::Overflow => &mut self.overflow,
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
    pub fn effective_font_weight(&self) -> Option<&str> {
        self.font_weight
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("normal") && *v != "400")
    }

    #[inline]
    pub fn effective_text_align(&self) -> Option<&str> {
        self.text_align
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("left") && !v.eq_ignore_ascii_case("start"))
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
    pub fn effective_flex_wrap(&self) -> Option<&str> {
        self.flex_wrap
            .as_deref()
            .filter(|v| !v.eq_ignore_ascii_case("nowrap"))
    }

    /// Appends non-default raw style declarations into `out`.
    pub fn write_compact_into(&self, skip_typography: bool, out: &mut String) {
        if !skip_typography {
            if let Some(fw) = self.effective_font_weight() {
                out.push_str("font-weight:");
                out.push_str(fw);
                out.push(';');
            }
            if let Some(ta) = self.effective_text_align() {
                out.push_str("text-align:");
                out.push_str(ta);
                out.push(';');
            }
        }
        if let Some(bs) = self.effective_box_shadow() {
            out.push_str("box-shadow:");
            out.push_str(bs);
            out.push(';');
        }
        if let Some(ov) = self.effective_overflow() {
            out.push_str("overflow:");
            out.push_str(ov);
            out.push(';');
        }
        if let Some(fw) = self.effective_flex_wrap() {
            out.push_str("flex-wrap:");
            out.push_str(fw);
            out.push(';');
        }
        if let Some(fit) = self.object_fit {
            out.push_str("object-fit:");
            out.push_str(fit);
            out.push(';');
        }
    }
}
