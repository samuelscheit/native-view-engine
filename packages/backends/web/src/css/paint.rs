//! Visual, scroll, image, typography, and compositor CSS emission for `NodeStore` word tables.

use crate::css::Keyword;
use nve_abi::num::{write_i64, write_u64};
use nve_abi::ColorRgba;
use crate::css::WriteCss;
use nve_tables::tables::{
    BackgroundFillRes, BoxShadows, Image, Overflow, OverflowWrap, Scroll, ScrollAxes, Scrollbars,
    TextAlign, TextContent, TextDecoration, Transform, Typography, Visibility, Visual, WhiteSpace,
    WordBreak,
};
use nve_store::{NodeId, NodeStore, Rgba};

use super::write_compact_num;

pub(super) fn write_visual_and_typography_css(
    store: &NodeStore,
    id: NodeId,
    include_compositor: bool,
    out: &mut String,
) {
    if let Some(v) = store.try_row::<Visual>(id) {
        if !v.background_fill().is_none()
            && let Some(fill) = store.resource::<BackgroundFillRes>(v.background_fill())
        {
            out.push_str("background:");
            fill.write_css(out);
            out.push(';');
        } else if v.background() != Rgba::TRANSPARENT {
            out.push_str("background:");
            ColorRgba::from_u32(v.background().0).write_css(out);
            out.push(';');
        }
        if v.border_width() > 0.0 && v.border_color() != Rgba::TRANSPARENT {
            out.push_str("border:");
            crate::css::write_px(v.border_width(), out);
            out.push_str(" solid ");
            ColorRgba::from_u32(v.border_color().0).write_css(out);
            out.push(';');
        }
        if v.border_radius() > 0.0 {
            out.push_str("border-radius:");
            crate::css::write_px(v.border_radius(), out);
            out.push(';');
        }
        match v.visibility() {
            Visibility::Inherit => {}
            Visibility::Visible => out.push_str("visibility:visible;"),
            Visibility::Hidden => out.push_str("visibility:hidden;"),
        }
        if v.overflow_x() != Overflow::Visible {
            out.push_str("overflow-x:");
            out.push_str(match v.overflow_x() {
                Overflow::Visible => "visible",
                Overflow::Hidden => "hidden",
                Overflow::Scroll => "scroll",
                Overflow::Auto => "auto",
            });
            out.push(';');
        }
        if v.overflow_y() != Overflow::Visible {
            out.push_str("overflow-y:");
            out.push_str(match v.overflow_y() {
                Overflow::Visible => "visible",
                Overflow::Hidden => "hidden",
                Overflow::Scroll => "scroll",
                Overflow::Auto => "auto",
            });
            out.push(';');
        }
        if !v.box_shadows().is_none()
            && let Some(shadows) = store.resource::<BoxShadows>(v.box_shadows())
            && !shadows.is_empty()
        {
            out.push_str("box-shadow:");
            crate::css::write_box_shadows(shadows.iter(), out);
            out.push(';');
        }
        if include_compositor {
            if (v.opacity() - 1.0).abs() > 1e-4 {
                out.push_str("opacity:");
                write_compact_num(v.opacity().clamp(0.0, 1.0), out);
                out.push(';');
            }
            if v.z_index() != 0 {
                out.push_str("z-index:");
                write_i64(out, i64::from(v.z_index()));
                out.push(';');
            }
        }
    }

    if let Some(sc) = store.try_row::<Scroll>(id) {
        match sc.axes() {
            ScrollAxes::Vertical => out.push_str("overflow-y:auto;overflow-x:hidden;"),
            ScrollAxes::Horizontal => out.push_str("overflow-x:auto;overflow-y:hidden;"),
            ScrollAxes::Both => out.push_str("overflow:auto;"),
            ScrollAxes::None => out.push_str("overflow:hidden;"),
        }
        if sc.scrollbars() == Scrollbars::Never {
            out.push_str("scrollbar-width:none;");
        }
    }

    if let Some(img) = store.try_row::<Image>(id) {
        out.push_str("object-fit:");
        out.push_str(img.fit().keyword());
        out.push(';');
    }

    if let Some(t) = store.try_row::<Typography>(id) {
        if t.is_color_explicit() {
            out.push_str("color:");
            ColorRgba::from_u32(t.color().0).write_css(out);
            out.push(';');
        }
        if t.is_font_size_explicit() {
            out.push_str("font-size:");
            crate::css::write_px(t.font_size(), out);
            out.push(';');
        }
        if t.is_font_weight_explicit() {
            out.push_str("font-weight:");
            write_u64(out, u64::from(t.font_weight()));
            out.push(';');
        }
        if t.is_line_height_explicit() {
            out.push_str("line-height:");
            if t.line_height().is_auto() {
                out.push_str("normal");
            } else if t.line_height().is_points() {
                crate::css::write_px(t.line_height().value(), out);
            } else {
                write_compact_num(t.line_height().value() / 100.0, out);
            }
            out.push(';');
        }
        if t.is_letter_spacing_explicit() {
            out.push_str("letter-spacing:");
            crate::css::write_px(t.letter_spacing(), out);
            out.push(';');
        }
        if t.is_overflow_wrap_explicit() {
            match t.overflow_wrap() {
                OverflowWrap::Normal => out.push_str("overflow-wrap:normal;"),
                OverflowWrap::BreakWord => out.push_str("overflow-wrap:break-word;"),
                OverflowWrap::Anywhere => out.push_str("overflow-wrap:anywhere;"),
            }
        }
        if t.is_word_break_explicit() {
            match t.word_break() {
                WordBreak::Normal => out.push_str("word-break:normal;"),
                WordBreak::BreakAll => out.push_str("word-break:break-all;"),
                WordBreak::KeepAll => out.push_str("word-break:keep-all;"),
                WordBreak::BreakWord => out.push_str("word-break:break-word;"),
            }
        }
        if t.is_white_space_explicit() {
            match t.white_space() {
                WhiteSpace::Normal => out.push_str("white-space:normal;"),
                WhiteSpace::NoWrap => out.push_str("white-space:nowrap;"),
                WhiteSpace::Pre => out.push_str("white-space:pre;"),
                WhiteSpace::PreWrap => out.push_str("white-space:pre-wrap;"),
                WhiteSpace::PreLine => out.push_str("white-space:pre-line;"),
                WhiteSpace::BreakSpaces => out.push_str("white-space:break-spaces;"),
            }
        }
        if t.is_font_family_explicit() {
            let family = t.font_family();
            if family.trim().is_empty() {
                out.push_str("font-family:initial;");
            } else if is_safe_font_family(family) {
                out.push_str("font-family:");
                out.push_str(family.trim());
                out.push(';');
            }
        }
        if t.is_text_align_explicit() {
            match t.text_align() {
                TextAlign::Start => out.push_str("text-align:start;"),
                TextAlign::End => out.push_str("text-align:end;"),
                TextAlign::Left => out.push_str("text-align:left;"),
                TextAlign::Right => out.push_str("text-align:right;"),
                TextAlign::Center => out.push_str("text-align:center;"),
                TextAlign::Justify => out.push_str("text-align:justify;"),
            }
        }
        if t.is_text_decoration_explicit() {
            let deco = t.text_decoration();
            if deco.is_empty() {
                out.push_str("text-decoration-line:none;");
            } else {
                out.push_str("text-decoration-line:");
                let mut first = true;
                for (flag, kw) in [
                    (TextDecoration::UNDERLINE, "underline"),
                    (TextDecoration::OVERLINE, "overline"),
                    (TextDecoration::LINE_THROUGH, "line-through"),
                ] {
                    if deco.contains(flag) {
                        if !first {
                            out.push(' ');
                        }
                        out.push_str(kw);
                        first = false;
                    }
                }
                out.push(';');
            }
        }
        // `Typography::max_lines` is `Option<NonZeroU16>`, guaranteeing `>= 1` by type.
        if t.is_max_lines_explicit()
            && let Some(max_lines) = t.max_lines()
        {
            out.push_str("display:-webkit-box;-webkit-box-orient:vertical;overflow:hidden;-webkit-line-clamp:");
            write_u64(out, u64::from(max_lines.get()));
            out.push(';');
        }
    }

    if include_compositor
        && let Some(tf) = store.try_row::<Transform>(id)
    {
        write_transform_css(tf, out);
    }

    let _ = TextContent::KEY;
}

/// Emits compositor-only inline CSS (`opacity`, `z-index`, `transform`) for the 120 Hz fast lane.
pub fn write_compositor_css(store: &NodeStore, id: NodeId, out: &mut String) {
    if let Some(v) = store.try_row::<Visual>(id) {
        if (v.opacity() - 1.0).abs() > 1e-4 {
            out.push_str("opacity:");
            write_compact_num(v.opacity().clamp(0.0, 1.0), out);
            out.push(';');
        }
        if v.z_index() != 0 {
            out.push_str("z-index:");
            write_i64(out, i64::from(v.z_index()));
            out.push(';');
        }
    }
    if let Some(tf) = store.try_row::<Transform>(id) {
        write_transform_css(tf, out);
    }
}

pub fn write_transform_css(tf: nve_tables::tables::TransformRef<'_>, out: &mut String) {
    let tf = nve_tables::transform::Transform2D {
        translate_x: tf.translate_x(),
        translate_y: tf.translate_y(),
        scale_x: tf.scale_x(),
        scale_y: tf.scale_y(),
        rotate_deg: tf.rotate_deg(),
        origin_x: tf.origin_x(),
        origin_y: tf.origin_y(),
    };
    crate::css::write_transform_decls(&tf, false, out);
}

/// Validates a CSS `font-family` list against CSS injection, comments, unbalanced quotes, and
/// `<style>` breakout characters.
#[inline]
pub(crate) fn is_safe_font_family(raw: &str) -> bool {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return false;
    }
    if trimmed.bytes().any(|b| {
        b < 0x20
            || b == 0x7F
            || matches!(
                b,
                b';' | b'{' | b'}' | b'<' | b'>' | b'\\' | b'@' | b'(' | b')' | b'/' | b'*'
            )
    }) {
        return false;
    }
    for part in trimmed.split(',') {
        let p = part.trim();
        if p.is_empty() {
            return false;
        }
        let bytes = p.as_bytes();
        if bytes[0] == b'"' || bytes[0] == b'\'' {
            let q = bytes[0];
            if bytes.len() < 3 || bytes[bytes.len() - 1] != q {
                return false;
            }
            let inner = &p[1..p.len() - 1];
            if inner.trim().is_empty() || inner.contains('"') || inner.contains('\'') {
                return false;
            }
        } else {
            if p.contains('"') || p.contains('\'') {
                return false;
            }
            for tok in p.split_ascii_whitespace() {
                if !tok
                    .chars()
                    .all(|c| c.is_alphanumeric() || matches!(c, '-' | '_'))
                {
                    return false;
                }
            }
        }
    }
    true
}
