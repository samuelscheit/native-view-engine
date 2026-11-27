use std::sync::Arc;

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{
    ChangeSet, FlatResource, Length, NodeId, NodeStore, NveTable, Rgba, RowPatch, RowPatchView,
    ValidatedWords,
};

use crate::interaction::{CursorKind, InteractionState, StateStyleRule};
use crate::layout_ext::GridTrack;
use crate::media::{MediaQuery, ResponsiveRule};
use crate::motion::TransitionSpec;
use crate::paint::{BackgroundFill, GradientStop};
use crate::style_ext::StylePatchExt;

use super::*;

#[test]
fn all_core_tables_fit_32_words_and_register_cleanly() {
    register_core_tables();
    let descs = [
        Layout::DESC,
        GridLayout::DESC,
        GridItem::DESC,
        Visual::DESC,
        Typography::DESC,
        TextContent::DESC,
        Transform::DESC,
        Motion::DESC,
        Responsive::DESC,
        Interaction::DESC,
        Accessibility::DESC,
        DebugId::DESC,
        Link::DESC,
        Scroll::DESC,
        Image::DESC,
        TextInput::DESC,
    ];
    for d in descs {
        assert!(
            d.words() <= nve_abi::table::MAX_TABLE_WORDS,
            "table `{}` has {} words (> 32)",
            d.name,
            d.words()
        );
        assert_eq!(d.word_effects.len(), d.defaults.len());
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
        .box_shadows(Some(BoxShadows::new(parse_box_shadow(
            "0 4px 12px rgba(0, 0, 0, 0.25)",
        ))))
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
    tx.edit::<Motion>(n)
        .transitions(Some(Transitions::new(vec![TransitionSpec::all(200)])));
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

    // 4. Transitions exact field equality
    let mot = dst.try_row::<Motion>(n).unwrap();
    let trs = dst.resource::<Transitions>(mot.transitions()).unwrap();
    assert_eq!(trs.iter().collect::<Vec<_>>(), vec![TransitionSpec::all(200)]);

    // 5. ResponsiveRules exact field equality
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
    assert!(!Transitions::validate(empty));
    assert!(!ResponsiveRules::validate(empty));
    assert!(!StateRules::validate(empty));
    assert!(!AriaRelations::validate(empty));
    assert!(!ImageSrc::validate(empty));

    // Oversized length prefix & bad discriminants
    let huge = [0x7FFF_FFFFu32];
    assert!(!GridTracks::validate(&huge));
    assert!(!BoxShadows::validate(&huge));
    assert!(!Transitions::validate(&huge));
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
