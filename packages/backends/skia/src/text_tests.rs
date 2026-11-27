use nve_abi::ColorRgba;
use nve_engine::LayoutRect;

use crate::font::BitmapFontTextMeasurer;
use crate::raster::PixelBuffer;
use crate::text::{parse_font_weight, CosmicTextEngine, TextAlign};

#[test]
fn cosmic_engine_discovers_system_fonts_and_measures_proportionally() {
    let engine = CosmicTextEngine::shared();
    assert!(
        engine.has_system_fonts(),
        "expected native system fonts to be discovered on host"
    );
    assert!(engine.loaded_font_count() > 0);

    assert_eq!(engine.measure_text("", 16.0, None), (0.0, 0.0));

    let (w_narrow, h_narrow) = engine.measure_text("iiii", 20.0, None);
    let (w_wide, h_wide) = engine.measure_text("WWWW", 20.0, None);
    assert!(
        w_narrow > 0.0 && w_wide > w_narrow * 1.8,
        "expected proportional shaping ('iiii'={w_narrow} vs 'WWWW'={w_wide})"
    );
    assert_eq!(h_narrow, h_wide);

    let (w_small, h_small) = engine.measure_text("Native View Engine", 12.0, None);
    let (w_large, h_large) = engine.measure_text("Native View Engine", 28.0, None);
    assert!(h_large > h_small * 1.8, "h_small={h_small}, h_large={h_large}");
    assert!(w_large > w_small * 1.8, "w_small={w_small}, w_large={w_large}");
}

#[test]
fn cosmic_engine_wraps_under_max_width_handles_newlines_and_unicode() {
    let engine = CosmicTextEngine::shared();

    let sample = "Native View Engine renders real HarfBuzz shaped proportional typography";
    let (unwrapped_w, unwrapped_h) = engine.measure_text(sample, 16.0, None);
    let (wrapped_w, wrapped_h) = engine.measure_text(sample, 16.0, Some(120.0));
    assert!(unwrapped_w > 200.0, "expected long line width > 200, got {unwrapped_w}");
    assert!(wrapped_w <= 120.0, "wrapped width must respect max_width=120, got {wrapped_w}");
    assert!(wrapped_h >= unwrapped_h * 2.0);

    let (_, h1) = engine.measure_text("Line 1", 16.0, None);
    let (_, h3) = engine.measure_text("Line 1\nLine 2\nLine 3", 16.0, None);
    assert!(h3 >= h1 * 2.5);

    let (mw0_w, mw0_h) = engine.measure_text("Probe\nLine", 16.0, Some(0.0));
    assert_eq!(mw0_w, 0.0);
    assert!(mw0_h.is_finite() && mw0_h > 0.0);
    let (nan_w, nan_h) = engine.measure_text("Fallback", f32::NAN, Some(f32::INFINITY));
    assert!(nan_w.is_finite() && nan_h.is_finite() && nan_h > 0.0);

    let (uni_w, uni_h) = engine.measure_text("Café — naïve • 日本語 🚀", 18.0, None);
    assert!(uni_w > 20.0 && uni_h > 18.0);
}

#[test]
fn cosmic_engine_rasterizes_antialiased_subpixel_edges_and_clips_strictly() {
    let engine = CosmicTextEngine::shared();
    let bg = ColorRgba::rgb(15, 23, 42);
    let fg = ColorRgba::rgb(56, 189, 248);

    let mut buf = PixelBuffer::new(320, 120);
    buf.clear(bg);

    let rect = LayoutRect {
        x: 20.0,
        y: 20.0,
        width: 240.0,
        height: 48.0,
    };
    engine.rasterize_text(&mut buf, rect, "Antialiased Orbit", 24.0, 1.0, fg, 1.0);

    let mut exact_fg_count = 0usize;
    let mut antialiased_edge_count = 0usize;
    for y in 20..68u32 {
        for x in 20..260u32 {
            let rgb = buf.get_rgb(x, y).unwrap();
            if rgb == (fg.r, fg.g, fg.b) {
                exact_fg_count += 1;
            } else if rgb != (bg.r, bg.g, bg.b) {
                assert!(rgb.0 >= bg.r && rgb.0 <= fg.r);
                assert!(rgb.1 >= bg.g && rgb.1 <= fg.g);
                assert!(rgb.2 >= bg.b && rgb.2 <= fg.b);
                antialiased_edge_count += 1;
            }
        }
    }
    assert!(exact_fg_count > 30);
    assert!(antialiased_edge_count > 30);

    let mut clip_buf = PixelBuffer::new(200, 80);
    clip_buf.clear(bg);
    let clip_rect = LayoutRect {
        x: 30.0,
        y: 20.0,
        width: 40.0,
        height: 24.0,
    };
    engine.rasterize_text(
        &mut clip_buf,
        clip_rect,
        "WWWWWWWWWWWWWWWWWWWW",
        22.0,
        1.0,
        fg,
        1.0,
    );
    for y in 0..80u32 {
        for x in 0..200u32 {
            let inside = (30..70).contains(&x) && (20..44).contains(&y);
            if !inside {
                assert_eq!(clip_buf.get_rgb(x, y), Some((bg.r, bg.g, bg.b)));
            }
        }
    }
}

#[test]
fn cosmic_engine_scales_cleanly_at_2x_hidpi_and_caches_consistently() {
    let engine = CosmicTextEngine::shared();
    let bg = ColorRgba::BLACK;
    let fg = ColorRgba::WHITE;
    let rect = LayoutRect {
        x: 10.0,
        y: 10.0,
        width: 180.0,
        height: 40.0,
    };

    let mut buf_1x = PixelBuffer::new(200, 60);
    buf_1x.clear(bg);
    engine.rasterize_text(&mut buf_1x, rect, "Retina 2x", 18.0, 1.0, fg, 1.0);

    let mut buf_2x_a = PixelBuffer::new(400, 120);
    buf_2x_a.clear(bg);
    engine.rasterize_text(&mut buf_2x_a, rect, "Retina 2x", 18.0, 2.0, fg, 1.0);

    let mut buf_2x_b = PixelBuffer::new(400, 120);
    buf_2x_b.clear(bg);
    engine.rasterize_text(&mut buf_2x_b, rect, "Retina 2x", 18.0, 2.0, fg, 1.0);
    assert_eq!(buf_2x_a.pixels(), buf_2x_b.pixels());

    let lit_1x = buf_1x.pixels().iter().filter(|&&p| p != 0).count();
    let lit_2x = buf_2x_a.pixels().iter().filter(|&&p| p != 0).count();
    assert!(lit_2x > lit_1x * 2);
}

#[test]
fn cosmic_engine_falls_back_cleanly_to_bitmap_font_when_no_system_fonts_exist() {
    let fallback = CosmicTextEngine::new_without_system_fonts();
    assert!(!fallback.has_system_fonts());
    assert_eq!(fallback.loaded_font_count(), 0);

    let expected = BitmapFontTextMeasurer.measure_text("Fallback OK", 16.0, None);
    assert_eq!(fallback.measure_text("Fallback OK", 16.0, None), expected);

    let mut buf = PixelBuffer::new(200, 60);
    buf.clear(ColorRgba::BLACK);
    fallback.rasterize_text(
        &mut buf,
        LayoutRect {
            x: 8.0,
            y: 8.0,
            width: 180.0,
            height: 32.0,
        },
        "Fallback OK",
        16.0,
        1.0,
        ColorRgba::WHITE,
        1.0,
    );
    let lit = buf.pixels().iter().filter(|&&p| p != 0).count();
    assert!(lit > 20);
}

#[test]
fn parses_font_weights_and_text_align_keywords() {
    assert_eq!(parse_font_weight("bold"), Some(700));
    assert_eq!(parse_font_weight("semibold"), Some(600));
    assert_eq!(parse_font_weight("medium"), Some(500));
    assert_eq!(parse_font_weight("normal"), Some(400));
    assert_eq!(parse_font_weight("100"), Some(100));
    assert_eq!(parse_font_weight("900"), Some(900));
    assert_eq!(parse_font_weight("1200"), Some(900));
    assert_eq!(parse_font_weight("50"), Some(100));
    assert_eq!(parse_font_weight("bogus"), None);

    assert_eq!(TextAlign::parse("left"), Some(TextAlign::Left));
    assert_eq!(TextAlign::parse("center"), Some(TextAlign::Center));
    assert_eq!(TextAlign::parse("RIGHT"), Some(TextAlign::Right));
    assert_eq!(TextAlign::parse("justify"), None);
}
