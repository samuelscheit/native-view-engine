//! Floating diagnostic HUD banner renderer for the pure-Rust software presenter.

use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use crate::text::CosmicTextEngine;

use super::clip::ClipStack;
use super::{fill_rounded_rect, stroke_rounded_rect_prim, PixelBuffer};

/// Renders a compact floating diagnostic pill at the bottom-right corner of `buffer`.
pub fn draw_hud_banner(buffer: &mut PixelBuffer, scale_factor: f32, text: &str) {
    if text.is_empty() || buffer.width() < 64 || buffer.height() < 32 {
        return;
    }
    let scale = if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    };
    let text_engine = CosmicTextEngine::shared();
    let logical_font_size = 11.0f32;
    let pad_x = 10.0f32;
    let pad_y = 5.0f32;
    let margin = 10.0f32;
    let max_logical_w = (buffer.width() as f32 / scale - margin * 2.0 - pad_x * 2.0).max(16.0);
    let (measured_w, measured_h) =
        text_engine.measure_text(text, logical_font_size, Some(max_logical_w));

    let pill_w_logical =
        (measured_w + pad_x * 2.0).min(buffer.width() as f32 / scale - margin * 2.0);
    let pill_h_logical = measured_h + pad_y * 2.0;
    let pill_x_logical = (buffer.width() as f32 / scale - pill_w_logical - margin).max(margin);
    let pill_y_logical = (buffer.height() as f32 / scale - pill_h_logical - margin).max(margin);

    let pill_rect = LayoutRect {
        x: pill_x_logical * scale,
        y: pill_y_logical * scale,
        width: pill_w_logical * scale,
        height: pill_h_logical * scale,
    };
    let clip = ClipStack::new(buffer.width(), buffer.height());
    fill_rounded_rect(
        buffer,
        &clip,
        pill_rect,
        8.0 * scale,
        ColorRgba::rgba(15, 23, 42, 220),
        1.0,
    );
    stroke_rounded_rect_prim(
        buffer,
        &clip,
        pill_rect,
        8.0 * scale,
        1.0 * scale,
        ColorRgba::rgba(56, 189, 248, 180),
        1.0,
    );
    text_engine.rasterize_text(
        buffer,
        LayoutRect {
            x: pill_x_logical + pad_x,
            y: pill_y_logical + pad_y,
            width: (pill_w_logical - pad_x * 2.0).max(1.0),
            height: (pill_h_logical - pad_y * 2.0).max(1.0),
        },
        text,
        logical_font_size,
        scale,
        ColorRgba::rgb(226, 232, 240),
        1.0,
    );
}
