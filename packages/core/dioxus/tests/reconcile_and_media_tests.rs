//! Integration tests for `signal_archive` 3-way hot-reload reconciliation and zero-serialization
//! direct `TypedStyle` / `nve_show_on` storage on `NodeRecord` / `DioxusSurface`.

use std::cell::RefCell;
use std::rc::Rc;

use dioxus_core::{AttributeValue, ElementId, IntoAttributeValue, WriteMutations};
use nve_abi::{Command, CommandDecoder, RawPropValue};
use nve_components::media::{
    FlexWrap, MediaColorScheme, MediaQuery, Overflow, ResponsiveStylePatch, TextAlign,
};
use nve_components::reconcile::DivergenceKind;
use nve_dioxus::prelude::*;
use nve_dioxus::{
    reconcile_signal_entry, DioxusSurface, NveMutationWriter, SignalRegistry, SignalTableSnapshot,
    SignalValue,
};

fn decode_cmds(bytes: &[u8]) -> Vec<Command> {
    CommandDecoder::new(bytes)
        .expect("valid command buffer header")
        .decode_all()
        .expect("valid command buffer")
        .1
}

#[test]
fn signal_archive_three_way_hot_reload_reconciliation() {
    let clean = reconcile_signal_entry(SignalValue::I64(10), SignalValue::I64(10), SignalValue::I64(10));
    assert_eq!(clean.kind, DivergenceKind::Clean);
    assert_eq!(clean.resolved, SignalValue::I64(10));

    let untouched_code_updated =
        reconcile_signal_entry(SignalValue::I64(10), SignalValue::I64(10), SignalValue::I64(25));
    assert_eq!(untouched_code_updated.kind, DivergenceKind::IncomingOnly);
    assert_eq!(untouched_code_updated.resolved, SignalValue::I64(25));
    assert!(untouched_code_updated.apply_local);

    let user_mutated =
        reconcile_signal_entry(SignalValue::I64(10), SignalValue::I64(99), SignalValue::I64(10));
    assert_eq!(user_mutated.kind, DivergenceKind::LocalOnlyDirty);
    assert_eq!(user_mutated.resolved, SignalValue::I64(99));
    assert!(user_mutated.emit_upstream);

    let user_mutated_vs_new_default =
        reconcile_signal_entry(SignalValue::I64(10), SignalValue::I64(99), SignalValue::I64(25));
    assert_eq!(user_mutated_vs_new_default.kind, DivergenceKind::Conflict);
    assert_eq!(user_mutated_vs_new_default.resolved, SignalValue::I64(99));
    assert!(user_mutated_vs_new_default.emit_upstream);

    let converged =
        reconcile_signal_entry(SignalValue::I64(10), SignalValue::I64(25), SignalValue::I64(25));
    assert_eq!(converged.kind, DivergenceKind::Converged);
    assert_eq!(converged.resolved, SignalValue::I64(25));

    // End-to-end SignalRegistry::hydrate_hot_reload:
    let mut old_reg = SignalRegistry::new(1);
    let old_count = Rc::new(RefCell::new(0i64));
    let old_theme = Rc::new(RefCell::new("light".to_string()));
    let old_title = Rc::new(RefCell::new("v1".to_string()));
    {
        let c = Rc::clone(&old_count);
        let c2 = Rc::clone(&old_count);
        old_reg.register_slot("app", "count", move || *c.borrow(), move |v| *c2.borrow_mut() = v);
        let t = Rc::clone(&old_theme);
        let t2 = Rc::clone(&old_theme);
        old_reg.register_slot("app", "theme", move || t.borrow().clone(), move |v| *t2.borrow_mut() = v);
        let ti = Rc::clone(&old_title);
        let ti2 = Rc::clone(&old_title);
        old_reg.register_slot("app", "title", move || ti.borrow().clone(), move |v| *ti2.borrow_mut() = v);
    }
    let base_defaults: SignalTableSnapshot = old_reg.capture();

    *old_count.borrow_mut() = 42;
    *old_title.borrow_mut() = "custom".to_string();
    let archived_local: SignalTableSnapshot = old_reg.capture();

    let mut new_reg = SignalRegistry::new(2);
    let new_count = Rc::new(RefCell::new(5i64));
    let new_theme = Rc::new(RefCell::new("dark".to_string()));
    let new_title = Rc::new(RefCell::new("v2".to_string()));
    let new_zoom = Rc::new(RefCell::new(100i64));
    {
        let c = Rc::clone(&new_count);
        let c2 = Rc::clone(&new_count);
        new_reg.register_slot("app", "count", move || *c.borrow(), move |v| *c2.borrow_mut() = v);
        let t = Rc::clone(&new_theme);
        let t2 = Rc::clone(&new_theme);
        new_reg.register_slot("app", "theme", move || t.borrow().clone(), move |v| *t2.borrow_mut() = v);
        let ti = Rc::clone(&new_title);
        let ti2 = Rc::clone(&new_title);
        new_reg.register_slot("app", "title", move || ti.borrow().clone(), move |v| *ti2.borrow_mut() = v);
        let z = Rc::clone(&new_zoom);
        let z2 = Rc::clone(&new_zoom);
        new_reg.register_slot("app", "zoom", move || *z.borrow(), move |v| *z2.borrow_mut() = v);
    }

    let report = new_reg.hydrate_hot_reload(&base_defaults, &archived_local);
    assert_eq!(report.hydrated_slots, 3);
    assert_eq!(report.untouched_new_slots, 1);
    assert_eq!(report.skipped_unknown_slots, 0);
    assert_eq!(report.skipped_type_mismatches, 0);

    assert_eq!(*new_count.borrow(), 42);
    assert_eq!(*new_title.borrow(), "custom");
    assert_eq!(*new_theme.borrow(), "dark");
    assert_eq!(*new_zoom.borrow(), 100);
}

#[test]
fn dioxus_surface_stores_typed_show_on_and_responsive_rules_with_zero_serialization() {
    fn app() -> Element {
        let sidebar_style = TypedStyle::new()
            .width_px(260.0)
            .flex_col()
            .show_on(MediaQuery::min_width(960))
            .on_compact(ResponsiveStylePatch::new().width_pct(100.0).gap(8.0))
            .at(
                MediaQuery::min_width(1200).and_color_scheme(MediaColorScheme::Dark),
                ResponsiveStylePatch::new()
                    .width_px(300.0)
                    .flex_wrap(FlexWrap::Wrap)
                    .overflow(Overflow::Auto)
                    .text_align(TextAlign::Left)
                    .font_weight(600),
            )
            .on_expanded(ResponsiveStylePatch::new().padding(24.0));

        rsx! {
            view {
                nve_style: sidebar_style,
                text { "Sidebar" }
            }
            view {
                nve_show_on: ShowOn(MediaQuery::max_width(959)),
                text { "Mobile Drawer Trigger" }
            }
        }
    }

    let mut surface = DioxusSurface::new(1, app);
    let (_, _, frame_bytes) = surface.render_frame(1);
    let cmds = decode_cmds(&frame_bytes);

    // Zero byte serialization in CommandBuffer for in-process responsive props!
    assert!(!cmds.iter().any(|c| matches!(
        c,
        Command::SetRawProp {
            key,
            value: RawPropValue::Bytes(_),
            ..
        } if key == "nve-show-on" || key == "nve-media"
    )));

    // Direct zero-copy typed access via DioxusSurface getters!
    let mut show_ons: Vec<_> = surface.iter_show_on().collect();
    show_ons.sort_by_key(|(id, _)| *id);
    assert_eq!(show_ons.len(), 2);
    assert_eq!(show_ons[0].1, MediaQuery::min_width(960));
    assert_eq!(show_ons[1].1, MediaQuery::max_width(959));
    assert_eq!(surface.node_show_on(show_ons[0].0), Some(MediaQuery::min_width(960)));

    let rules_entries: Vec<_> = surface.iter_responsive_rules().collect();
    assert_eq!(rules_entries.len(), 1);
    let (sidebar_id, rules) = rules_entries[0];
    assert_eq!(sidebar_id, show_ons[0].0);
    assert_eq!(rules.len(), 3);
    assert_eq!(rules[0].query, MediaQuery::compact());
    assert_eq!(
        rules[1].query,
        MediaQuery::min_width(1200).and_color_scheme(MediaColorScheme::Dark)
    );
    assert_eq!(rules[1].patch.font_weight, Some(600));
    assert_eq!(rules[2].query, MediaQuery::expanded());
    assert_eq!(surface.node_responsive_rules(sidebar_id), rules);
}

#[test]
fn mutation_writer_handles_direct_media_query_composition_and_clearing() {
    let mut writer = NveMutationWriter::new(1);
    writer.begin_frame(1, 1);

    writer.create_placeholder(ElementId(5));
    writer.set_attribute(
        "nve-show-on",
        None,
        &AttributeValue::Any(Rc::new(MediaQuery::dark().and_min_width(768))),
        ElementId(5),
    );
    let _ = writer.finish_frame(true);

    let (node_id, q) = writer.iter_show_on().next().expect("direct show_on stored");
    assert_eq!(q, MediaQuery::dark().and_min_width(768));
    assert_eq!(writer.node_show_on(node_id), Some(q));

    // Applying a TypedStyle with its own show_on + responsive rules preserves explicit nve_show_on.
    writer.begin_frame(1, 2);
    let styled = TypedStyle::new()
        .show_on(MediaQuery::expanded())
        .on_compact(ResponsiveStylePatch::new().gap(4.0));
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &styled.into_value(), ElementId(5));
    let _ = writer.finish_frame(true);
    assert_eq!(writer.node_show_on(node_id), Some(MediaQuery::dark().and_min_width(768)));
    assert_eq!(writer.node_responsive_rules(node_id).len(), 1);

    // Clearing explicit nve_show_on falls back cleanly to TypedStyle::show_on.
    writer.begin_frame(1, 3);
    writer.set_attribute("nve-show-on", None, &AttributeValue::None, ElementId(5));
    let _ = writer.finish_frame(true);
    assert_eq!(writer.node_show_on(node_id), Some(MediaQuery::expanded()));

    // Clearing TypedStyle clears both show_on and responsive_rules.
    writer.begin_frame(1, 4);
    writer.set_attribute(NVE_TYPED_STYLE_ATTR, None, &AttributeValue::None, ElementId(5));
    let _ = writer.finish_frame(true);
    assert_eq!(writer.node_show_on(node_id), None);
    assert!(writer.node_responsive_rules(node_id).is_empty());
}
