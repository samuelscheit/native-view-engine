use nve_abi::{
    ColorRgba, CommandBufferBuilder, ComponentKind, Dimension, EdgeInsets, FlexDirection,
    LayoutProps, RawPropValue, StyleProps, ROOT_NODE_ID,
};
use nve_engine::{RenderBackend, Viewport};

use crate::text::CosmicTextEngine;
use crate::text_grapheme::{
    clamp_grapheme_boundary, grapheme_boundaries, is_grapheme_boundary, next_grapheme_boundary,
    normalize_line_endings, paragraph_bounds_at, prev_grapheme_boundary, word_bounds_at,
};
use crate::{PaintOp, SkiaBackend, FOCUS_SELECTION_COLOR};

fn build_input_backend(value: &str, width: f32, scale_factor: f32) -> SkiaBackend {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 400.0,
            height: 200.0,
            scale_factor,
        },
    );
    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::TextInput, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(width),
            height: Dimension::Points(36.0),
            padding: EdgeInsets::symmetric(0.0, 12.0),
            margin: EdgeInsets {
                left: 16.0,
                top: 20.0,
                ..Default::default()
            },
            ..Default::default()
        },
    );
    b.set_style(
        2,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::rgb(241, 245, 249),
            font_size: 16.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(2, "value", &RawPropValue::String(value.to_owned()));
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();
    skia.set_focused_node(Some(2));
    skia
}

#[test]
fn grapheme_boundaries_never_split_zwj_flags_skin_tones_or_combining_marks() {
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let flag = "🇩🇪";
    let wave = "👋🏽";
    let accented = "e\u{0301}";
    let sample = format!("A{family}{flag}{wave}{accented}Z");

    let expected = vec![
        0,
        1,
        1 + family.len(),
        1 + family.len() + flag.len(),
        1 + family.len() + flag.len() + wave.len(),
        1 + family.len() + flag.len() + wave.len() + accented.len(),
        sample.len(),
    ];
    assert_eq!(grapheme_boundaries(&sample), expected);

    // Every single interior byte clamps strictly to the start of its enclosing cluster.
    for w in expected.windows(2) {
        for b in w[0]..w[1] {
            assert_eq!(clamp_grapheme_boundary(&sample, b), w[0]);
            assert_eq!(is_grapheme_boundary(&sample, b), b == w[0]);
        }
    }
    assert!(is_grapheme_boundary(&sample, sample.len()));

    // Stepping forward and backward are exact inverses across all clusters.
    let mut fwd = vec![0usize];
    let mut cur = 0;
    while cur < sample.len() {
        cur = next_grapheme_boundary(&sample, cur);
        fwd.push(cur);
    }
    assert_eq!(fwd, expected);

    let mut bwd = vec![sample.len()];
    let mut cur = sample.len();
    while cur > 0 {
        cur = prev_grapheme_boundary(&sample, cur);
        bwd.push(cur);
    }
    bwd.reverse();
    assert_eq!(bwd, expected);

    // Word and paragraph selection respect grapheme clusters and line endings.
    let accented_word = format!("caf{accented} {family} world_42!");
    assert_eq!(word_bounds_at(&accented_word, 4), (0, 3 + accented.len()));
    assert_eq!(
        word_bounds_at(&accented_word, 3 + accented.len() + 2),
        (3 + accented.len() + 1, 3 + accented.len() + 1 + family.len())
    );

    let multi = "first line\r\nsecond line\nthird";
    assert_eq!(paragraph_bounds_at(multi, 2), (0, 10));
    assert_eq!(paragraph_bounds_at(multi, 14), (12, 23));
    assert_eq!(paragraph_bounds_at(multi, 26), (24, 29));
    assert_eq!(
        normalize_line_endings("a\r\nb\rc\nd\r\n"),
        "a\nb\nc\nd\n"
    );
}

#[test]
fn hit_testing_never_splits_grapheme_clusters_on_zwj_flags_or_combining_marks() {
    let family = "👨\u{200D}👩\u{200D}👧\u{200D}👦";
    let sample = format!("a{family}🇩🇪👋🏽e\u{0301}z");
    let valid = grapheme_boundaries(&sample);

    let skia = build_input_backend(&sample, 360.0, 1.0);
    let r = skia.layout_of(2).unwrap();

    for step in 0..=180 {
        let x = r.x + (step as f32) * 2.0;
        let hit = skia.cursor_index_at_x(2, x).unwrap();
        assert!(
            valid.contains(&hit),
            "hit_test at x={x} landed at invalid mid-grapheme offset {hit}; valid={valid:?}"
        );
    }

    // Clamping via SkiaBackend::set_selection_range also never splits mid-cluster.
    let mut skia = skia;
    skia.set_selection_range(3, 5 + family.len());
    assert!(is_grapheme_boundary(&sample, skia.selection_anchor_byte_index()));
    assert!(is_grapheme_boundary(&sample, skia.caret_byte_index()));
}

#[test]
fn bidi_mixed_text_produces_disjoint_selection_rects_and_monotonic_run_carets() {
    let engine = CosmicTextEngine::shared();
    let mixed = "Hello مرحبا world";
    let shaped = engine.shape_single_line(mixed, 16.0, 400);
    assert!(shaped.has_rtl(), "expected Arabic segment to have RTL BiDi level");

    // 1. Caret advance is strictly monotonic within LTR run 1 ("Hello "), RTL run ("مرحبا"),
    //    and LTR run 2 (" world").
    let ltr1: Vec<f32> = (0..=6).map(|i| shaped.caret_x_at(mixed, i)).collect();
    for w in ltr1.windows(2) {
        assert!(w[1] > w[0], "LTR run 1 caret must increase: {ltr1:?}");
    }

    let ar_start = "Hello ".len();
    let ar_end = "Hello مرحبا".len();
    let ar_inner: Vec<usize> = grapheme_boundaries(mixed)
        .into_iter()
        .filter(|&b| b > ar_start && b < ar_end)
        .collect();
    let ar_xs: Vec<f32> = ar_inner.iter().map(|&b| shaped.caret_x_at(mixed, b)).collect();
    assert!(ar_xs.len() >= 4, "expected interior Arabic boundaries");
    for w in ar_xs.windows(2) {
        assert!(
            w[1] < w[0],
            "RTL run caret X must move strictly right-to-left: {ar_xs:?}"
        );
    }

    let ltr2_start = ar_end;
    let ltr2: Vec<f32> = (ltr2_start..=mixed.len())
        .map(|i| shaped.caret_x_at(mixed, i))
        .collect();
    for w in ltr2.windows(2) {
        assert!(w[1] > w[0], "LTR run 2 caret must increase: {ltr2:?}");
    }

    // 2. Selecting across LTR + partial RTL ("lo مر", bytes 3..10) yields TWO disjoint visual rects!
    let mut skia = build_input_backend(mixed, 360.0, 1.0);
    let sel_start = 3; // "lo "
    let sel_end = ar_start + "مر".len();
    let rects = skia.selection_rects_for_range(2, sel_start, sel_end);
    assert!(
        rects.len() >= 2,
        "expected >= 2 disjoint visual selection rects across LTR+RTL boundary, got {rects:?}"
    );
    assert!(
        rects[0].x + rects[0].width < rects[1].x,
        "selection rects should be visually disjoint: {rects:?}"
    );

    // Display list emits each disjoint selection rect with FOCUS_SELECTION_COLOR.
    skia.set_selection_range(sel_start, sel_end);
    let painted_sel_count = skia
        .display_list()
        .ops
        .iter()
        .filter(|op| {
            matches!(
                op,
                PaintOp::FillRoundedRect { color, .. } if *color == FOCUS_SELECTION_COLOR
            )
        })
        .count();
    assert_eq!(painted_sel_count, rects.len());

    // 3. Visual hit-testing across the whole mixed line always lands on valid grapheme boundaries.
    for step in 0..=140 {
        let x = (step as f32) * 1.5;
        let hit = shaped.hit_test_x(mixed, x);
        assert!(
            is_grapheme_boundary(mixed, hit),
            "hit_test_x({x}) returned non-grapheme offset {hit}"
        );
    }

    // 4. Visual Left/Right stepping visits every grapheme boundary without panicking or looping.
    let total_stops = grapheme_boundaries(mixed).len();
    let mut pos = 0usize;
    let mut visited_right = vec![pos];
    for _ in 0..total_stops + 5 {
        let next = shaped.move_caret_visual(mixed, pos, 1);
        assert!(is_grapheme_boundary(mixed, next));
        if next == pos {
            break;
        }
        visited_right.push(next);
        pos = next;
    }
    assert_eq!(visited_right.len(), total_stops);

    let mut visited_left = vec![pos];
    for _ in 0..total_stops + 5 {
        let prev = shaped.move_caret_visual(mixed, pos, -1);
        assert!(is_grapheme_boundary(mixed, prev));
        if prev == pos {
            break;
        }
        visited_left.push(prev);
        pos = prev;
    }
    visited_left.reverse();
    assert_eq!(visited_left, visited_right);
}

#[test]
fn bidi_hebrew_numbers_and_arabic_ligatures_map_cleanly() {
    let engine = CosmicTextEngine::shared();

    let hebrew_nums = "שלום 123";
    let h_shaped = engine.shape_single_line(hebrew_nums, 16.0, 400);
    assert!(h_shaped.rtl, "Hebrew-leading paragraph should resolve RTL");
    for step in 0..=80 {
        let hit = h_shaped.hit_test_x(hebrew_nums, step as f32 * 1.5);
        assert!(is_grapheme_boundary(hebrew_nums, hit));
    }
    // Pure Hebrew prefix moves right-to-left (decreasing visual X).
    let x0 = h_shaped.caret_x_at(hebrew_nums, 0);
    let x_mid = h_shaped.caret_x_at(hebrew_nums, "של".len());
    assert!(x_mid < x0, "Hebrew caret should advance right-to-left: x0={x0}, x_mid={x_mid}");

    // Arabic lam-alef ligature ("لا") inside "سلام لا" must stay grapheme-safe and never panic.
    let arabic_lig = "سلام لا";
    let a_shaped = engine.shape_single_line(arabic_lig, 18.0, 400);
    assert!(a_shaped.rtl);
    for b in grapheme_boundaries(arabic_lig) {
        let x = a_shaped.caret_x_at(arabic_lig, b);
        assert!(x.is_finite() && x >= 0.0);
    }
    let spans = a_shaped.selection_spans_for_range(arabic_lig, 0, arabic_lig.len());
    assert!(!spans.is_empty());
}

#[test]
fn ime_cursor_rect_tracks_caret_preedit_scroll_offset_and_hidpi_scale() {
    let mut skia = SkiaBackend::new();
    skia.set_viewport(
        1,
        Viewport {
            width: 320.0,
            height: 120.0,
            scale_factor: 2.0,
        },
    );

    let mut b = CommandBufferBuilder::new(1, 1);
    b.create_node(2, ComponentKind::ScrollView, None);
    b.set_layout(
        2,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(100.0),
            flex_direction: FlexDirection::Column,
            ..Default::default()
        },
    );
    b.create_node(3, ComponentKind::View, None);
    b.set_layout(
        3,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(80.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.create_node(4, ComponentKind::TextInput, None);
    b.set_layout(
        4,
        &LayoutProps {
            width: Dimension::Points(260.0),
            height: Dimension::Points(36.0),
            padding: EdgeInsets::symmetric(0.0, 12.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_style(
        4,
        &StyleProps {
            background: ColorRgba::rgb(15, 23, 42),
            text_color: ColorRgba::WHITE,
            font_size: 16.0,
            ..Default::default()
        },
    );
    b.set_raw_prop(4, "value", &RawPropValue::String("Hello".into()));
    b.create_node(5, ComponentKind::View, None);
    b.set_layout(
        5,
        &LayoutProps {
            width: Dimension::Points(300.0),
            height: Dimension::Points(160.0),
            flex_shrink: 0.0,
            ..Default::default()
        },
    );
    b.set_children(2, &[3, 4, 5]);
    b.set_children(ROOT_NODE_ID, &[2]);
    b.commit(true);
    skia.submit_frame(b.finish()).unwrap();

    assert!(skia.focused_ime_cursor_rect().is_none());
    skia.set_focused_node(Some(4));

    skia.set_caret_byte_index(0);
    let r_start = skia.focused_ime_cursor_rect().expect("caret at 0");
    skia.set_caret_byte_index(5);
    let r_end = skia.focused_ime_cursor_rect().expect("caret at 5");
    assert!(
        r_end.x > r_start.x + 10.0,
        "caret rect must move right across text: start={r_start:?}, end={r_end:?}"
    );

    // Physical rect scales by viewport scale_factor (2.0).
    let r_phys = skia.focused_ime_cursor_physical_rect().unwrap();
    assert!((r_phys.x - r_end.x * 2.0).abs() < 0.01);
    assert!((r_phys.y - r_end.y * 2.0).abs() < 0.01);
    assert!((r_phys.height - r_end.height * 2.0).abs() < 0.01);

    // Active preedit widens the IME cursor area to cover the preedit string.
    skia.set_ime_preedit(Some("nihao".into()), Some((0, 5)));
    let r_preedit = skia.focused_ime_cursor_rect().unwrap();
    assert!(
        r_preedit.width > r_end.width + 10.0,
        "preedit rect should span preedit width: end={r_end:?}, preedit={r_preedit:?}"
    );

    // Scrolling parent ScrollView shifts IME cursor rect upward in viewport coordinates.
    skia.apply_scroll_delta(2, 0.0, 40.0);
    let r_scrolled = skia.focused_ime_cursor_rect().unwrap();
    assert!(
        (r_scrolled.y - (r_preedit.y - 40.0)).abs() < 0.1,
        "expected scrolled y={}, got {}",
        r_preedit.y - 40.0,
        r_scrolled.y
    );
}
