use std::sync::Arc;

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{
    ChangeSet, FlatResource, Length, NodeId, NodeStore, NveTable, Rgba, RowPatch, RowPatchView,
    ValidatedWords,
};

use crate::interaction::{CursorKind, InteractionState, StateStyleRule};
use crate::layout_ext::GridTrack;
use crate::media::{MediaQuery, ResponsiveRule};
use crate::paint::{BackgroundFill, GradientStop};
use crate::style_ext::StylePatchExt;

use super::*;

#[test]
fn all_core_tables_fit_32_words_and_register_cleanly() {
    register_core_tables();
    let mut keys = std::collections::BTreeSet::new();
    for d in CORE_TABLE_DESCS {
        assert!(
            d.words() <= nve_abi::table::MAX_TABLE_WORDS,
            "table `{}` has {} words (> 32)",
            d.name,
            d.words()
        );
        assert_eq!(d.word_effects.len(), d.defaults.len());
        assert!(
            nve_store::slot_of(d.key).is_some(),
            "`{}` is not registered by register_core_tables",
            d.name
        );
        assert!(keys.insert(d.key), "`{}` is listed twice in CORE_TABLE_DESCS", d.name);
    }
    // Visual visibility must be paint-only (never triggers layout).
    let vis_field = Visual::DESC
        .fields
        .iter()
        .find(|f| f.name == "visibility")
        .unwrap();
    assert_eq!(vis_field.effects, nve_abi::table::effect::PAINT);
    assert_eq!(
        Visual::DESC.word_effects[vis_field.word as usize],
        nve_abi::table::effect::PAINT
    );
}

#[test]
fn core_tables_roundtrip_through_nve2_wire() {
    register_core_tables();
    let mut src = NodeStore::new();
    src.set_realization_order(core_realization_order());
    let mut tx = src.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);

    tx.edit::<Layout>(n)
        .width(Length::points(320.0))
        .height(Length::percent(50.0))
        .flex_direction(FlexDirection::Row)
        .align_self(AlignSelf::Center)
        .padding_inline_logical(true);
    tx.edit::<GridLayout>(n)
        .template_columns(Some(GridTracks::new(vec![
            GridTrack::Px(100.0),
            GridTrack::Fr(1.0),
        ])));
    tx.edit::<GridItem>(n)
        .column_start(GridLine::Line(1))
        .column_end(GridLine::Span(2));
    tx.edit::<Visual>(n)
        .background(Rgba::rgb(20, 30, 40))
        .opacity(0.75)
        .visibility(Visibility::Hidden)
        .box_shadows(Some(BoxShadows::new(vec![BoxShadow::new(
            0.0,
            4.0,
            12.0,
            Rgba::new(0, 0, 0, 64),
        )])))
        .background_fill(Some(BackgroundFillRes(BackgroundFill::linear(
            90.0,
            vec![
                GradientStop::new(0.0, nve_abi::ColorRgba::rgb(255, 0, 0)),
                GradientStop::new(1.0, nve_abi::ColorRgba::rgb(0, 0, 255)),
            ],
        ))));
    tx.edit::<Typography>(n)
        .font_size(20.0)
        .font_weight(700)
        .font_family("Inter");
    tx.edit::<TextContent>(n).text("Hello NVE2");
    tx.edit::<Transform>(n).translate_x(12.0).rotate_deg(15.0);
    tx.edit::<Responsive>(n)
        .rules(Some(ResponsiveRules::new(vec![ResponsiveRule {
            query: MediaQuery::min_width(768),
            patch: RowPatch::new().opacity(0.5),
        }])));
    tx.edit::<Interaction>(n)
        .listen(InteractionEvent::Press)
        .listen(InteractionEvent::PointerDown)
        .cursor(CursorKind::Pointer)
        .tab_index(Some(-1))
        .state_rules(Some(StateRules::new(vec![StateStyleRule {
            state: InteractionState::HOVER,
            patch: RowPatch::new().opacity(0.9),
        }])));
    tx.edit::<Accessibility>(n)
        .role(Some(AriaRole::Button))
        .label("Submit")
        .value_now(Some(42.0))
        .relations(Some(AriaRelations {
            controls: "panel-1".into(),
            ..Default::default()
        }));
    tx.edit::<DebugId>(n).test_id("submit-btn").id("btn-1");
    tx.edit::<Link>(n)
        .href("https://example.com")
        .target(LinkTarget::Blank);
    tx.edit::<Scroll>(n)
        .axes(ScrollAxes::Vertical)
        .virtual_total_height(Some(4000.0));
    tx.edit::<Image>(n)
        .src(Some(ImageSrc::Url("https://cdn.example.com/a.png".into())))
        .fit(ImageFit::Contain)
        .alt("logo");
    tx.edit::<TextInput>(n)
        .value("abc")
        .placeholder("Type...")
        .multiline(true)
        .autofill(Some(crate::AutofillHint::Email))
        .max_length(Some(128));

    let mut changes = ChangeSet::default();
    tx.commit_into(&mut changes);

    let mut enc = WireEncoder::default();
    let mut wire = Vec::new();
    enc.encode(&src, &changes, &mut wire);

    let mut dst = NodeStore::replica();
    dst.set_realization_order(core_realization_order());
    let mut dec = WireDecoder::default();
    let mut dst_tx = dst.tx();
    dec.apply(&mut dst_tx, &wire).expect("NVE2 decode");
    let mut dst_changes = ChangeSet::default();
    dst_tx.commit_into(&mut dst_changes);

    let l = dst.try_row::<Layout>(n).unwrap();
    assert_eq!(l.width(), Length::points(320.0));
    assert_eq!(l.height(), Length::percent(50.0));
    assert_eq!(l.align_self(), AlignSelf::Center);
    assert!(l.padding_inline_logical());

    let v = dst.try_row::<Visual>(n).unwrap();
    assert_eq!(v.background(), Rgba::rgb(20, 30, 40));
    assert_eq!(v.opacity(), 0.75);
    assert_eq!(v.visibility(), Visibility::Hidden);
    // 1. BoxShadows exact field equality
    let shadows = dst.resource::<BoxShadows>(v.box_shadows()).unwrap();
    assert_eq!(shadows.len(), 1);
    let sh0 = shadows.get(0).unwrap();
    assert_eq!(sh0.offset_x, 0.0);
    assert_eq!(sh0.offset_y, 4.0);
    assert_eq!(sh0.blur_radius, 12.0);
    assert_eq!(sh0.spread_radius, 0.0);
    assert_eq!(sh0.color, Rgba::new(0, 0, 0, 64), "rgba(0,0,0,0.25) -> alpha 64");
    assert!(!sh0.inset);

    // 2. GridTracks exact field equality
    let gl = dst.try_row::<GridLayout>(n).unwrap();
    let cols = dst.resource::<GridTracks>(gl.template_columns()).unwrap();
    assert_eq!(
        cols.iter().collect::<Vec<_>>(),
        vec![GridTrack::Px(100.0), GridTrack::Fr(1.0)]
    );

    // 3. BackgroundFillRes exact field equality
    let bg_fill = dst
        .resource::<BackgroundFillRes>(v.background_fill())
        .unwrap();
    assert_eq!(
        bg_fill.to_owned(),
        BackgroundFill::linear(
            90.0,
            vec![
                GradientStop::new(0.0, nve_abi::ColorRgba::rgb(255, 0, 0)),
                GradientStop::new(1.0, nve_abi::ColorRgba::rgb(0, 0, 255)),
            ],
        )
    );

    // 4. ResponsiveRules exact field equality
    let resp = dst.try_row::<Responsive>(n).unwrap();
    let rrules: Vec<_> = dst
        .resource::<ResponsiveRules>(resp.rules())
        .unwrap()
        .iter()
        .map(|(q, p)| (q, p.get_prop(Visual::OPACITY)))
        .collect();
    assert_eq!(rrules, vec![(MediaQuery::min_width(768), Some(0.5))]);

    let typo = dst.try_row::<Typography>(n).unwrap();
    assert!(typo.is_font_size_explicit());
    assert!(typo.is_font_weight_explicit());
    assert!(typo.is_font_family_explicit());
    assert!(!typo.is_color_explicit());
    assert_eq!(typo.font_size(), 20.0);
    assert_eq!(typo.font_weight(), 700);
    assert_eq!(typo.font_family(), "Inter");

    let tc = dst.try_row::<TextContent>(n).unwrap();
    assert_eq!(tc.text(), "Hello NVE2");

    // 6. StateRules exact field equality
    let ia = dst.try_row::<Interaction>(n).unwrap();
    assert_eq!(ia.cursor(), CursorKind::Pointer);
    assert_eq!(ia.tab_index(), Some(-1));
    let srules: Vec<_> = dst
        .resource::<StateRules>(ia.state_rules())
        .unwrap()
        .iter()
        .map(|(st, p)| (st, p.get_prop(Visual::OPACITY)))
        .collect();
    assert_eq!(srules, vec![(InteractionState::HOVER, Some(0.9))]);

    // 7. AriaRelations exact field equality
    let a11y = dst.try_row::<Accessibility>(n).unwrap();
    assert_eq!(a11y.role(), Some(AriaRole::Button));
    assert_eq!(a11y.label(), "Submit");
    assert_eq!(a11y.value_now(), Some(42.0));
    let rel = dst.resource::<AriaRelations>(a11y.relations()).unwrap();
    assert_eq!(
        rel.to_owned(),
        AriaRelations {
            controls: "panel-1".into(),
            ..Default::default()
        }
    );

    // 8. ImageSrc exact field equality
    let img = dst.try_row::<Image>(n).unwrap();
    assert_eq!(img.fit(), ImageFit::Contain);
    assert_eq!(img.alt(), "logo");
    let img_src = dst.resource::<ImageSrc>(img.src()).unwrap();
    assert_eq!(
        img_src.to_owned(),
        ImageSrc::Url("https://cdn.example.com/a.png".into())
    );

    let ti = dst.try_row::<TextInput>(n).unwrap();
    assert_eq!(ti.value(), "abc");
    assert!(ti.multiline());
    assert_eq!(ti.autofill(), Some(crate::AutofillHint::Email));
    assert_eq!(ti.max_length(), Some(128));

    // Primary slot derived uniformly from RealizationOrder (Image beats TextInput/Scroll/Link/TextContent).
    assert_eq!(dst.primary_slot(n), Image::slot());
}

#[test]
fn resource_validators_reject_truncated_and_corrupt_inputs() {
    let empty: &[u32] = &[];
    assert!(!GridTracks::validate(empty));
    assert!(!BoxShadows::validate(empty));
    assert!(!BackgroundFillRes::validate(empty));
    assert!(!ResponsiveRules::validate(empty));
    assert!(!StateRules::validate(empty));
    assert!(!AriaRelations::validate(empty));
    assert!(!ImageSrc::validate(empty));

    // Oversized length prefix & bad discriminants
    let huge = [0x7FFF_FFFFu32];
    assert!(!GridTracks::validate(&huge));
    assert!(!BoxShadows::validate(&huge));
    assert!(!ResponsiveRules::validate(&huge));
    assert!(!StateRules::validate(&huge));
    assert!(!BackgroundFillRes::validate(&[99, 0]));
    assert!(!ImageSrc::validate(&[2, 0]));
    // Non-zero padding byte in UTF-8 string word
    assert!(!ImageSrc::validate(&[0, 1, 0x00FF_0061]));
    // Invalid UTF-8 in AriaRelations
    assert!(!AriaRelations::validate(&[1, 0, 0, 0, 0x0000_00FF]));

    // EncodedBytes roundtrip
    let img = ImageSrc::EncodedBytes(Arc::from([1u8, 2, 3, 4]));
    let mut buf = Vec::new();
    assert!(img.write_words(&mut buf));
    let vw = ValidatedWords::<ImageSrc>::try_new(&buf).unwrap();
    assert_eq!(ImageSrc::view(vw), ImageSrcView::EncodedBytes(&[1, 2, 3, 4]));

    // Hostile valid StateRules with two 20-table HOVER rules (40 distinct synthetic TableKeys)
    // must validate AND merge via `resolve_patch_from_view` without panicking!
    let make_20_patch = |base_key: u64| -> Vec<u32> {
        let mut w = vec![20u32];
        for i in 0..20u64 {
            let k = nve_abi::Key64(base_key + i);
            w.extend_from_slice(&[k.lo(), k.hi(), 1, 0, u32::MAX, i as u32]);
        }
        w
    };
    let p1 = make_20_patch(0x1000);
    let p2 = make_20_patch(0x2000);
    let mut sr_words = vec![2u32];
    sr_words.push(u32::from(InteractionState::HOVER.bits()));
    sr_words.push(p1.len() as u32);
    sr_words.extend_from_slice(&p1);
    sr_words.push(u32::from(InteractionState::HOVER.bits()));
    sr_words.push(p2.len() as u32);
    sr_words.extend_from_slice(&p2);
    let sr_vw = ValidatedWords::<StateRules>::try_new(&sr_words).expect("valid 2x20 StateRules");
    let merged = crate::interaction::resolve_patch_from_view(
        StateRules::view(sr_vw),
        InteractionState::HOVER,
    );
    assert_eq!(merged.table_count(), 40);
    let _ = RowPatchView::from_words(&p1).unwrap();
}

#[test]
fn background_fill_and_grid_tracks_reject_non_finite_and_negative_domain_floats() {
    use crate::layout_ext::{GridSingleTrack, GridTrackUnit};
    use crate::paint::{LinearGradient, RadialGradient};

    let red = nve_abi::ColorRgba::rgb(255, 0, 0);
    let blue = nve_abi::ColorRgba::rgb(0, 0, 255);
    let ok_stops = Arc::from([GradientStop::new(0.0, red), GradientStop::new(1.0, blue)]);

    // 1. BackgroundFillRes: NaN, +inf, -inf in angle / center / radius / stop offset, and negative radius.
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        // Wire validation rejects non-finite linear angle and stop offset.
        assert!(!BackgroundFillRes::validate(&[1, bad.to_bits(), 0]));
        assert!(!BackgroundFillRes::validate(&[
            1,
            90.0f32.to_bits(),
            1,
            bad.to_bits(),
            red.to_u32(),
        ]));
        // Wire validation rejects non-finite radial center_x, center_y, radius, and stop offset.
        assert!(!BackgroundFillRes::validate(&[
            2,
            bad.to_bits(),
            0.5f32.to_bits(),
            1.0f32.to_bits(),
            0,
        ]));
        assert!(!BackgroundFillRes::validate(&[
            2,
            0.5f32.to_bits(),
            bad.to_bits(),
            1.0f32.to_bits(),
            0,
        ]));
        assert!(!BackgroundFillRes::validate(&[
            2,
            0.5f32.to_bits(),
            0.5f32.to_bits(),
            bad.to_bits(),
            0,
        ]));
        assert!(!BackgroundFillRes::validate(&[
            2,
            0.5f32.to_bits(),
            0.5f32.to_bits(),
            1.0f32.to_bits(),
            1,
            bad.to_bits(),
            red.to_u32(),
        ]));

        // Producer `write_words` rejects directly-constructed invalid fills and leaves `out` untouched.
        let mut out = vec![0xDEAD_BEEFu32];
        let bad_lin = BackgroundFillRes(BackgroundFill::Linear(LinearGradient {
            angle_deg: bad,
            stops: Arc::clone(&ok_stops),
        }));
        assert!(!bad_lin.write_words(&mut out));
        assert_eq!(out, vec![0xDEAD_BEEF]);

        let bad_stop = BackgroundFillRes(BackgroundFill::Linear(LinearGradient {
            angle_deg: 45.0,
            stops: Arc::from([GradientStop { offset: bad, color: red }]),
        }));
        assert!(!bad_stop.write_words(&mut out));
        assert_eq!(out, vec![0xDEAD_BEEF]);

        let bad_rad = BackgroundFillRes(BackgroundFill::Radial(RadialGradient {
            center_x: 0.5,
            center_y: 0.5,
            radius: bad,
            stops: Arc::clone(&ok_stops),
        }));
        assert!(!bad_rad.write_words(&mut out));
        assert_eq!(out, vec![0xDEAD_BEEF]);
    }

    // Negative radial radius rejected on both wire validate and producer write_words
    // (without reserving capacity on an empty output buffer).
    assert!(!BackgroundFillRes::validate(&[
        2,
        0.5f32.to_bits(),
        0.5f32.to_bits(),
        (-0.01f32).to_bits(),
        0,
    ]));
    let mut empty_out = Vec::new();
    let neg_rad = BackgroundFillRes(BackgroundFill::Radial(RadialGradient {
        center_x: 0.5,
        center_y: 0.5,
        radius: -1.0,
        stops: ok_stops,
    }));
    assert!(!neg_rad.write_words(&mut empty_out));
    assert_eq!(empty_out.capacity(), 0, "pre-validation must not reserve on invalid fill");
    let mut out = vec![7u32];
    assert!(!neg_rad.write_words(&mut out));
    assert_eq!(out, vec![7]);

    // Zero radial radius (degenerate point gradient) stays valid.
    assert!(BackgroundFillRes::validate(&[
        2,
        0.5f32.to_bits(),
        0.5f32.to_bits(),
        0.0f32.to_bits(),
        0,
    ]));

    // 2. GridTracks: NaN, +inf, -inf in Px/Percent/Fr and negative Fr (top-level, MinMax, Repeat).
    for bad in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        // Raw wire words: tag 1 (Px), tag 2 (Percent), tag 3 (Fr), tag 4 (MinMax), tag 5 (Repeat).
        for tag in 1u32..=3 {
            assert!(!GridTracks::validate(&[1, tag, bad.to_bits(), 0]));
            assert!(!GridTracks::validate(&[
                1,
                5 | (tag << 4) | (2u32 << 16),
                bad.to_bits(),
                0,
            ]));
            assert!(!GridTracks::validate(&[
                1,
                4 | (tag << 7) | (1u32 << 10),
                bad.to_bits(),
                10.0f32.to_bits(),
            ]));
            assert!(!GridTracks::validate(&[
                1,
                4 | (1u32 << 7) | (tag << 10),
                10.0f32.to_bits(),
                bad.to_bits(),
            ]));
        }

        // Producer `write_words` rejects non-finite tracks and truncates back cleanly.
        for track in [
            GridTrack::Px(bad),
            GridTrack::Percent(bad),
            GridTrack::Fr(bad),
            GridTrack::MinMax(GridTrackUnit::Px(bad), GridTrackUnit::Fr(1.0)),
            GridTrack::MinMax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(bad)),
            GridTrack::Repeat(2, GridSingleTrack::Px(bad)),
            GridTrack::Repeat(2, GridSingleTrack::Fr(bad)),
            GridTrack::Repeat(
                2,
                GridSingleTrack::MinMax(GridTrackUnit::Percent(bad), GridTrackUnit::Auto),
            ),
        ] {
            let mut buf = vec![42u32];
            assert!(!GridTracks::new(vec![track]).write_words(&mut buf));
            assert_eq!(buf, vec![42]);
        }
    }

    // Negative Px/Percent/Fr breadths and Repeat(0, ...) rejected everywhere (wire & write_words).
    for tag in 1u32..=3 {
        assert!(!GridTracks::validate(&[1, tag, (-50.0f32).to_bits(), 0]));
        assert!(!GridTracks::validate(&[
            1,
            5 | (tag << 4) | (3u32 << 16),
            (-10.0f32).to_bits(),
            0,
        ]));
        assert!(!GridTracks::validate(&[
            1,
            4 | (tag << 7) | (1u32 << 10),
            (-1.0f32).to_bits(),
            10.0f32.to_bits(),
        ]));
        assert!(!GridTracks::validate(&[
            1,
            4 | (1u32 << 7) | (tag << 10),
            0.0f32.to_bits(),
            (-2.0f32).to_bits(),
        ]));
    }
    // Repeat(0, ...) rejected on wire for every single-track variant.
    assert!(!GridTracks::validate(&[1, 5, 0, 0])); // Repeat(0, Auto)
    assert!(!GridTracks::validate(&[1, 5 | (1 << 4), 100.0f32.to_bits(), 0])); // Repeat(0, Px)
    assert!(!GridTracks::validate(&[1, 5 | (3 << 4), 1.0f32.to_bits(), 0])); // Repeat(0, Fr)
    assert!(!GridTracks::validate(&[
        1,
        5 | (4 << 4) | (1 << 7) | (3 << 10),
        10.0f32.to_bits(),
        1.0f32.to_bits(),
    ])); // Repeat(0, MinMax)

    for invalid_track in [
        GridTrack::Px(-50.0),
        GridTrack::Percent(-10.0),
        GridTrack::Fr(-1.0),
        GridTrack::MinMax(GridTrackUnit::Px(-50.0), GridTrackUnit::Fr(1.0)),
        GridTrack::MinMax(GridTrackUnit::Percent(-10.0), GridTrackUnit::Auto),
        GridTrack::MinMax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(-0.25)),
        GridTrack::Repeat(2, GridSingleTrack::Px(-50.0)),
        GridTrack::Repeat(2, GridSingleTrack::Percent(-10.0)),
        GridTrack::Repeat(3, GridSingleTrack::Fr(-1.0)),
        GridTrack::Repeat(
            3,
            GridSingleTrack::MinMax(GridTrackUnit::Fr(-0.1), GridTrackUnit::Auto),
        ),
        GridTrack::Repeat(0, GridSingleTrack::Auto),
        GridTrack::Repeat(0, GridSingleTrack::Px(100.0)),
        GridTrack::Repeat(0, GridSingleTrack::Fr(1.0)),
    ] {
        let mut empty_buf = Vec::new();
        assert!(!GridTracks::new(vec![invalid_track]).write_words(&mut empty_buf));
        assert_eq!(
            empty_buf.capacity(),
            0,
            "pre-validation must not reserve on invalid track {invalid_track:?}"
        );
        let mut buf = vec![99u32];
        assert!(!GridTracks::new(vec![invalid_track]).write_words(&mut buf));
        assert_eq!(buf, vec![99]);
    }

    // Valid finite tracks including 0px / 0% / 0fr / Repeat(1, ...) pass both write_words and validate.
    let valid_tracks = GridTracks::new(vec![
        GridTrack::Px(0.0),
        GridTrack::Px(100.0),
        GridTrack::Percent(0.0),
        GridTrack::Percent(25.0),
        GridTrack::Fr(0.0),
        GridTrack::Fr(1.5),
        GridTrack::MinMax(GridTrackUnit::Px(0.0), GridTrackUnit::Fr(1.0)),
        GridTrack::Repeat(1, GridSingleTrack::Auto),
        GridTrack::Repeat(3, GridSingleTrack::Fr(1.0)),
    ]);
    let mut valid_buf = Vec::new();
    assert!(valid_tracks.write_words(&mut valid_buf));
    assert!(GridTracks::validate(&valid_buf));
}
