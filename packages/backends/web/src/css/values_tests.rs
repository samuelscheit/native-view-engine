//! Canonical CSS formatting of core value types (moved out of `nve-tables`).

use nve_abi::ColorRgba;
use nve_tables::interaction::CursorKind;
use nve_tables::layout_ext::{GridLine, GridSingleTrack, GridTrack, GridTrackPlacement, GridTrackUnit};
use nve_tables::paint::{BackgroundFill, GradientStop};
use nve_tables::tables::BoxShadow;
use nve_tables::transform::Transform2D;
use nve_store::Rgba;

use super::{write_box_shadows, write_grid_tracks, write_transform_decls, Keyword, WriteCss};

fn css(v: &impl WriteCss) -> String {
    let mut s = String::new();
    v.write_css(&mut s);
    s
}

#[test]
fn cursor_keywords_roundtrip_through_core_parser() {
    for raw in 0..=12u32 {
        let kind = CursorKind::try_from_word(raw).expect("variant");
        assert_eq!(CursorKind::parse_css(kind.keyword()), Some(kind), "{kind:?}");
    }
}

#[test]
fn transform_decls_handle_identity_and_custom_origin() {
    let mut base = String::new();
    write_transform_decls(&Transform2D::IDENTITY, false, &mut base);
    assert!(base.is_empty());

    let mut patch = String::new();
    write_transform_decls(&Transform2D::IDENTITY, true, &mut patch);
    assert_eq!(patch, "transform:none;");

    let mut full = String::new();
    let tf = Transform2D::translate(12.0, -4.0).with_scale(1.05).with_rotate(15.0).with_origin(0.0, 1.0);
    write_transform_decls(&tf, false, &mut full);
    assert_eq!(full, "transform:translate(12px,-4px) scale(1.05) rotate(15deg);transform-origin:0% 100%;");
}

#[test]
fn grid_tracks_and_placements_format_canonical_css() {
    let tracks = [
        GridTrack::px(240.0),
        GridTrack::percent(25.0),
        GridTrack::minmax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(1.0)),
        GridTrack::repeat(3, GridSingleTrack::minmax_zero_fr(1.0)),
        GridTrack::auto(),
    ];
    let mut list = String::new();
    write_grid_tracks(tracks, &mut list);
    assert_eq!(list, "240px 25% minmax(0,1fr) repeat(3,minmax(0,1fr)) auto");
    assert_eq!(css(&GridTrackPlacement::span(2)), "span 2");
    assert_eq!(css(&GridTrackPlacement::range(1, 3)), "1/3");
    assert_eq!(css(&GridTrackPlacement { start: GridLine::Line(2), end: GridLine::Span(3) }), "2/span 3");
    assert_eq!(css(&GridTrackPlacement::AUTO), "auto");
}

#[test]
fn gradients_and_colors_are_canonical_and_injection_free() {
    let lin = BackgroundFill::linear(
        90.0,
        [GradientStop::new(0.0, ColorRgba::rgb(255, 0, 0)), GradientStop::new(1.0, ColorRgba::rgb(0, 0, 255))],
    );
    assert_eq!(css(&lin), "linear-gradient(90deg,#ff0000 0%,#0000ff 100%)");
    let rad = BackgroundFill::radial(
        0.5,
        0.25,
        1.0,
        [GradientStop::new(0.0, ColorRgba::WHITE), GradientStop::new(1.0, ColorRgba::TRANSPARENT)],
    );
    assert_eq!(css(&rad), "radial-gradient(50% 50% at 50% 25%,#ffffff 0%,transparent 100%)");
    // Unsorted stops are normalized; a single stop spans the whole gradient.
    let unsorted = BackgroundFill::linear(
        f32::NAN,
        [GradientStop::new(1.0, ColorRgba::WHITE), GradientStop::new(0.0, ColorRgba::BLACK)],
    );
    assert_eq!(css(&unsorted), "linear-gradient(180deg,#000000 0%,#ffffff 100%)");
    let single = BackgroundFill::linear(0.0, [GradientStop::new(0.3, ColorRgba::rgba(1, 2, 3, 128))]);
    assert!(css(&single).ends_with(",rgba(1,2,3,0.502) 0%,rgba(1,2,3,0.502) 100%)"), "{}", css(&single));
}

#[test]
fn box_shadows_format_lists_and_none() {
    let mut out = String::new();
    write_box_shadows([], &mut out);
    assert_eq!(out, "none");
    out.clear();
    let drop = BoxShadow::new(0.0, 4.0, 12.0, Rgba::new(0, 0, 0, 64));
    let inner = BoxShadow::new(1.0, 0.0, 0.0, Rgba::new(255, 255, 255, 255)).spread(-2.0).inset();
    write_box_shadows([drop, inner], &mut out);
    assert_eq!(out, "0 4px 12px rgba(0,0,0,0.251), inset 1px 0 0 -2px #ffffff");
}
