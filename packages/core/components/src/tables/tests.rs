use std::sync::Arc;

use nve_store::wire::{WireDecoder, WireEncoder};
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Resource, Rgba};

use crate::interaction::{CursorKind, InteractionState, StateStyleRule};
use crate::layout_ext::GridTrack;
use crate::media::{MediaQuery, ResponsiveRule};
use crate::motion::TransitionSpec;
use crate::paint::{BackgroundFill, GradientStop};
use crate::style_patch::StylePatch;

use super::*;

#[test]
fn all_core_tables_fit_32_words_and_register_cleanly() {
    register_core_tables();
    let mut keys = std::collections::HashSet::new();
    for d in CORE_TABLE_DESCS {
        assert!(keys.insert(d.key), "duplicate table key `{}`", d.name);
        assert_eq!(d.key, nve_abi::Key64::of(d.name), "{}", d.name);
        assert!(nve_store::slot_of(d.key).is_some(), "`{}` not registered", d.name);
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
        .set(Typography::SET_FONT_SIZE | Typography::SET_FONT_WEIGHT)
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
            patch: StylePatch::default().opacity(0.5),
        }])));
    tx.edit::<Interaction>(n)
        .event_mask(0b101)
        .cursor(CursorKind::Pointer)
        .tab_index_set(true)
        .tab_index(-1)
        .state_rules(Some(StateRules::new(vec![StateStyleRule {
            state: InteractionState::HOVER,
            patch: StylePatch::default().opacity(0.9),
        }])));
    tx.edit::<Accessibility>(n)
        .role(AriaRole::Button)
        .label("Submit")
        .value_now(42.0)
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
        .virtual_total_height(4000.0);
    tx.edit::<Image>(n)
        .src(Some(ImageSrc::Url("https://cdn.example.com/a.png".into())))
        .fit(ImageFit::Contain)
        .alt("logo");
    tx.edit::<TextInput>(n)
        .value("abc")
        .placeholder("Type...")
        .multiline(true)
        .autofill(OptAutofillHint::some(crate::AutofillHint::Email))
        .max_length(128);

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
    let shadows = dst.resource::<BoxShadows>(v.box_shadows()).unwrap();
    assert_eq!(shadows.0.len(), 1);
    assert_eq!(shadows.0[0].blur_radius, 12.0);

    let tc = dst.try_row::<TextContent>(n).unwrap();
    assert_eq!(tc.text(), "Hello NVE2");

    let ia = dst.try_row::<Interaction>(n).unwrap();
    assert_eq!(ia.cursor(), CursorKind::Pointer);
    assert_eq!(ia.tab_index(), -1);

    let a11y = dst.try_row::<Accessibility>(n).unwrap();
    assert_eq!(a11y.role(), AriaRole::Button);
    assert_eq!(a11y.label(), "Submit");
    assert_eq!(a11y.value_now(), 42.0);

    let ti = dst.try_row::<TextInput>(n).unwrap();
    assert_eq!(ti.value(), "abc");
    assert!(ti.multiline());
    assert_eq!(ti.autofill().get(), Some(crate::AutofillHint::Email));
    assert_eq!(ti.max_length(), 128);

    // Primary slot derived uniformly from RealizationOrder (Image beats TextInput/Scroll/Link/TextContent).
    assert_eq!(dst.primary_slot(n), Image::slot());
}

#[test]
fn resource_decoders_reject_truncated_and_corrupt_inputs() {
    let empty: &[u8] = &[];
    assert!(GridTracks::decode(empty).is_none());
    assert!(BoxShadows::decode(empty).is_none());
    assert!(BackgroundFillRes::decode(empty).is_none());
    assert!(Transitions::decode(empty).is_none());
    assert!(ResponsiveRules::decode(empty).is_none());
    assert!(StateRules::decode(empty).is_none());
    assert!(AriaRelations::decode(empty).is_none());
    assert!(ImageSrc::decode(empty).is_none());

    // Oversized length prefix
    let huge = [0xFFu8, 0xFF, 0xFF, 0x7F];
    assert!(GridTracks::decode(&huge).is_none());
    assert!(BoxShadows::decode(&huge).is_none());

    // EncodedBytes roundtrip
    let img = ImageSrc::EncodedBytes(Arc::from([1u8, 2, 3, 4]));
    let mut buf = Vec::new();
    img.encode(&mut buf);
    assert_eq!(ImageSrc::decode(&buf), Some(img));
}
