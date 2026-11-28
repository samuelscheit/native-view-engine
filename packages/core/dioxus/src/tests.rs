//! End-to-end tests for `DioxusSurface` directly over `NodeStore`, `TypedStyle`, `TableAttr`
//! extensibility, surgical `HotPatchMsg`, direct event dispatch, and `NVE2` wire round-trip.

use nve_abi::{
    ColorRgba, EventBatchBuf, FrameId, InteractionEvent, SurfaceId, TimestampUs,
};
use nve_tables::tables::{
    Accessibility, AriaRole, DebugId, Interaction, Layout, StreamBoundary, TextContent, TextInput,
    Typography, Visual,
};
use nve_engine::HotPatchMsg;
use nve_store::wire::WireDecoder;
use nve_store::{ChangeSet, Length, NodeId, NodeStore, NveTable, Rgba};

use crate::prelude::*;

#[allow(dead_code)]
#[derive(NveTable)]
#[nve_table(name = "acme.badge", storage = "sparse", flags = "publish")]
struct CustomBadge {
    #[effect(paint)]
    pub count: u32,
}

#[component]
fn CounterApp() -> Element {
    let mut count = use_signal(|| 0i32);
    let badge_attr = table_attr_fn(move |tx, node| {
        tx.edit::<CustomBadge>(node).count(count().max(0) as u32);
    });
    let card_style = TypedStyle::new()
        .width_px(320.0)
        .padding(16.0)
        .bg(ColorRgba::rgb(15, 23, 42))
        .radius(12.0)
        .color(ColorRgba::WHITE)
        .font_size(18.0)
        .font_weight("bold")
        .cursor(CursorKind::Pointer);

    rsx! {
        view {
            test_id: "card-root",
            nve_style: card_style,
            nve_table: badge_attr,
            pressable {
                test_id: "inc-btn",
                onpress: move |_| count += 1,
                text { "Count: {count}" }
            }
            textinput {
                test_id: "name-input",
                placeholder: "Your name",
                value: "Ada",
            }
        }
    }
}

fn find_by_test_id(store: &NodeStore, tid: &str) -> NodeId {
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if id != NodeId::ROOT
            && let Some(dbg) = store.try_row::<DebugId>(id)
            && dbg.test_id() == tid
        {
            return id;
        }
        stack.extend_from_slice(store.children(id));
    }
    panic!("missing node with test_id={tid}");
}

#[test]
fn dioxus_surface_nodestore_typed_style_table_attr_events_and_nve2_roundtrip() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, CounterApp);
    let mut wire_bytes = Vec::new();
    let (_rec_us, _enc_us) = surface.render_nve2(FrameId(1), &mut wire_bytes);
    assert!(!wire_bytes.is_empty());

    let card = find_by_test_id(surface.store(), "card-root");
    let btn = find_by_test_id(surface.store(), "inc-btn");
    let input = find_by_test_id(surface.store(), "name-input");

    assert_eq!(
        surface.store().try_row::<Layout>(card).unwrap().width(),
        Length::points(320.0)
    );
    assert_eq!(
        surface.store().try_row::<Visual>(card).unwrap().background(),
        Rgba::rgb(15, 23, 42)
    );
    assert_eq!(
        surface.store().try_row::<Typography>(card).unwrap().font_weight(),
        700
    );
    assert_eq!(
        surface.store().try_row::<CustomBadge>(card).unwrap().count(),
        0
    );
    assert_eq!(
        surface.store().try_row::<Accessibility>(btn).unwrap().role(),
        Some(AriaRole::Button)
    );
    assert!(
        surface
            .store()
            .try_row::<Interaction>(btn)
            .unwrap()
            .listeners()
            .contains(InteractionEvent::Press)
    );
    let btn_view = ComponentView::<Interaction>::from_store(surface.store(), btn).unwrap();
    assert!(btn_view.is_listening(InteractionEvent::Press));
    let ti = surface.store().try_row::<TextInput>(input).unwrap();
    assert_eq!(ti.value(), "Ada");
    assert_eq!(ti.placeholder(), "Your name");

    // Replicate frame 1 into an independent NodeStore via NVE2 WireDecoder.
    let mut replica = NodeStore::replica();
    let mut decoder = WireDecoder::default();
    let mut replica_changes = ChangeSet::default();
    {
        let mut tx = replica.tx();
        decoder.apply(&mut tx, &wire_bytes).unwrap();
        tx.commit_into(&mut replica_changes);
    }
    let replica_card = find_by_test_id(&replica, "card-root");
    assert_eq!(
        replica.try_row::<CustomBadge>(replica_card).unwrap().count(),
        0
    );

    // Dispatch a Press event on the button and verify reactive update + NVE2 delta.
    let mut ev_buf = EventBatchBuf::new();
    ev_buf.push_press(SurfaceId::ROOT, btn, TimestampUs(100), 10.0, 10.0);
    let handled = surface.dispatch_events(ev_buf.as_validated());
    assert!(handled);

    surface.render_nve2(FrameId(2), &mut wire_bytes);
    assert_eq!(
        surface.store().try_row::<CustomBadge>(card).unwrap().count(),
        1
    );
    {
        let mut tx = replica.tx();
        decoder.apply(&mut tx, &wire_bytes).unwrap();
        tx.commit_into(&mut replica_changes);
    }
    assert_eq!(
        replica.try_row::<CustomBadge>(replica_card).unwrap().count(),
        1
    );
}

#[component]
fn MultiCardApp() -> Element {
    rsx! {
        view {
            view {
                test_id: "card-alpha",
                style: "padding: 8px; background-color: #111111; border-radius: 4px;",
                text { "# engineering" }
            }
            view {
                test_id: "card-beta",
                style: "padding: 8px; background-color: #222222; border-radius: 4px;",
                text { "# design-sync" }
            }
        }
    }
}

#[test]
fn targeted_hot_patch_updates_only_addressed_node_and_text() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, MultiCardApp);
    surface.reconcile(FrameId(1));

    let alpha = find_by_test_id(surface.store(), "card-alpha");
    let beta = find_by_test_id(surface.store(), "card-beta");
    let beta_bg = surface.store().try_row::<Visual>(beta).unwrap().background();

    assert!(surface.apply_hot_patch(&HotPatchMsg::TemplatePatch {
        generation: 2,
        template_id: "ui.rs".into(),
        attr_updates: vec![(
            "style".into(),
            "background-color: #2563eb; border-radius: 16px;".into(),
        )],
        text_updates: vec![],
        target_test_id: Some("card-alpha".into()),
        node_path: vec![0, 0],
        text_replacements: vec![("# engineering".into(), "# runtime".into())],
    }));
    surface.reconcile(FrameId(2));

    let alpha_vis = surface.store().try_row::<Visual>(alpha).unwrap();
    assert_eq!(alpha_vis.background(), Rgba::rgb(37, 99, 235));
    assert_eq!(alpha_vis.border_radius(), 16.0);
    assert_eq!(
        surface.store().try_row::<Visual>(beta).unwrap().background(),
        beta_bg
    );

    // Verify exact text replacement updated only "# engineering" -> "# runtime".
    let mut texts = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if let Some(tc) = surface.store().try_row::<TextContent>(id) {
            texts.push(tc.text().to_owned());
        }
        stack.extend_from_slice(surface.store().children(id));
    }
    assert!(texts.iter().any(|t| t == "# runtime"));
    assert!(texts.iter().any(|t| t == "# design-sync"));
    assert!(!texts.iter().any(|t| t == "# engineering"));
}

/// Collects every `TextContent` under `NodeId::ROOT` in DFS order.
fn all_text(store: &NodeStore) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        if let Some(t) = store.try_row::<TextContent>(id) {
            out.push(t.text().to_owned());
        }
        stack.extend(store.children(id).iter().rev());
    }
    out
}

#[component]
fn ThrowingLeaf() -> Element {
    Err(crate::error_boundary::ui_error("leaf failed"))?;
    rsx! { text { "unreachable" } }
}

#[component]
fn StatusBanner(status: String) -> Element {
    if !status.is_empty() {
        Err(crate::error_boundary::ui_error(format!("banner rejected {status}")))?;
    }
    rsx! { text { "status ok" } }
}

/// Three synchronous scheduler hops: the leaf throws -> (1) the inner boundary renders its
/// fallback and its `on_error` sets the app's status signal -> (2) the app re-renders
/// `StatusBanner`, which throws (zero edits: the last good nodes stay) -> (3) the outer boundary
/// renders its fallback.
#[component]
fn CascadeApp() -> Element {
    let mut status = use_signal(String::new);
    rsx! {
        view {
            text { "sibling stays" }
            ErrorBoundary {
                fallback: |r: UiErrorReport, _| rsx! { text { "OUTER:{r.message}" } },
                StatusBanner { status: status() }
            }
            ErrorBoundary {
                fallback: |r: UiErrorReport, _| rsx! { text { "INNER:{r.message}" } },
                on_error: move |r: UiErrorReport| status.set(r.message),
                ThrowingLeaf {}
            }
        }
    }
}

/// The host drive-loop contract (`WebAppSession::pump_async_work_with`, `nve-ssr`
/// `flush_ssr_until_quiescent`): `reconcile`, then `poll_work` -> `reconcile` until idle, settles
/// a multi-hop sync cascade within a few pumps, without any async wake-up.
#[test]
fn drive_loop_settles_multi_hop_sync_error_boundary_cascade_without_async_wakeups() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, CascadeApp);
    surface.reconcile(FrameId(1));
    let noop = core::task::Waker::noop();
    let mut pumps = 0;
    while surface.poll_work(noop) {
        pumps += 1;
        assert!(pumps <= 4, "sync cascade must settle well inside MAX_PUMPS (8)");
        surface.reconcile(FrameId(1 + pumps));
    }
    assert!(pumps >= 1, "a 3-hop cascade needs the drive loop beyond one 2-pass reconcile");
    let texts = all_text(surface.store());
    assert!(texts.iter().any(|t| t == "sibling stays"), "{texts:?}");
    assert!(texts.iter().any(|t| t == "INNER:leaf failed"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t == "OUTER:banner rejected leaf failed"),
        "outer fallback must settle once the drive loop finishes the cascade: {texts:?}"
    );
    assert!(!texts.iter().any(|t| t == "unreachable" || t == "status ok"), "{texts:?}");

    // Settled: a further reconcile emits no mutations.
    surface.reconcile(FrameId(99));
    assert!(surface.last_changes().is_empty(), "{:?}", surface.last_changes());
}

#[component]
fn SlotRoleApp() -> Element {
    let mut phase = use_signal(|| 0u8);
    let p = phase();
    rsx! {
        view {
            pressable { test_id: "next", onpress: move |_| phase += 1, text { "next" } }
            if p == 0 {
                NveStreamSlot { slot_id: "draft", role: StreamSlotRole::Input, test_id: "slot",
                    text { "form" }
                }
            } else if p == 1 {
                NveStreamSlot { slot_id: "draft", test_id: "slot", text { "out" } }
            }
        }
    }
}

#[test]
fn stream_slot_role_is_a_typed_boundary_column_that_updates_and_clears() {
    let mut surface = DioxusSurface::new(SurfaceId::ROOT, SlotRoleApp);
    surface.reconcile(FrameId(1));
    let role = |s: &DioxusSurface| {
        let slot = find_by_test_id(s.store(), "slot");
        let b = s.store().try_row::<StreamBoundary>(slot).expect("boundary row");
        (b.stream_slot().to_owned(), b.slot_role())
    };
    assert_eq!(role(&surface), ("draft".to_owned(), Some(StreamSlotRole::Input)));

    let press = |s: &mut DioxusSurface, frame| {
        let btn = find_by_test_id(s.store(), "next");
        let mut evs = EventBatchBuf::new();
        evs.push_press(SurfaceId::ROOT, btn, TimestampUs::ZERO, 1.0, 1.0);
        assert!(s.dispatch_events(evs.as_validated()));
        s.reconcile(FrameId(frame));
    };
    press(&mut surface, 2);
    assert_eq!(role(&surface), ("draft".to_owned(), Some(StreamSlotRole::Output)));

    press(&mut surface, 3);
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        assert!(!surface.store().has::<StreamBoundary>(id), "slot removed: no boundary left");
        stack.extend_from_slice(surface.store().children(id));
    }
    assert_eq!(StreamSlotRole::from_attr("bogus"), None);
}

#[test]
fn retained_event_clone_is_isolated_and_layout_and_table_events_dispatch() {
    use std::cell::RefCell;
    use std::rc::Rc;

    use dioxus_core::Event;
    use nve_abi::{EventFlags, EventId, Key64};

    type Shared<T> = Rc<RefCell<Vec<T>>>;

    #[derive(Clone)]
    struct Log {
        retained_presses: Shared<Event<PressEventData>>,
        layouts: Shared<(f32, f32, f32, f32)>,
        table_events: Shared<(nve_abi::table::TableKey, EventId, Vec<u8>)>,
    }

    impl PartialEq for Log {
        fn eq(&self, other: &Self) -> bool {
            Rc::ptr_eq(&self.retained_presses, &other.retained_presses)
                && Rc::ptr_eq(&self.layouts, &other.layouts)
                && Rc::ptr_eq(&self.table_events, &other.table_events)
        }
    }

    let log = Log {
        retained_presses: Rc::new(RefCell::new(Vec::new())),
        layouts: Rc::new(RefCell::new(Vec::new())),
        table_events: Rc::new(RefCell::new(Vec::new())),
    };

    #[component]
    fn EventLogApp(log: Log) -> Element {
        let rp = Rc::clone(&log.retained_presses);
        let lay = Rc::clone(&log.layouts);
        let tev = Rc::clone(&log.table_events);
        rsx! {
            view {
                test_id: "event-target",
                onpress: move |ev| {
                    rp.borrow_mut().push(ev);
                },
                onlayout: move |ev| {
                    lay.borrow_mut().push((ev.raw.payload_f32(0), ev.raw.payload_f32(1), ev.raw.payload_f32(2), ev.raw.payload_f32(3)));
                },
                ontableevent: move |ev| {
                    tev.borrow_mut().push((ev.table_key, ev.event_id, ev.payload.clone()));
                },
            }
        }
    }

    let log_for_app = log.clone();
    let mut surface = DioxusSurface::new_with_props(SurfaceId::ROOT, EventLogApp, EventLogAppProps { log: log_for_app });
    surface.reconcile(FrameId(1));

    let target = find_by_test_id(surface.store(), "event-target");
    let mod_key = Key64::of("acme.chart");

    let mut evs = EventBatchBuf::new();
    evs.push_press(SurfaceId::ROOT, target, TimestampUs(10), 11.0, 22.0);
    evs.push_press(SurfaceId::ROOT, target, TimestampUs(20), 99.0, 88.0);
    evs.push_layout(SurfaceId::ROOT, target, TimestampUs(30), 4.0, 8.0, 320.0, 180.0);
    evs.push_table_event(
        mod_key,
        SurfaceId::ROOT,
        target,
        TimestampUs(40),
        EventId(5),
        EventFlags::EMPTY,
        [0; 6],
        &[0xDE, 0xAD, 0xBE, 0xEF],
    );
    assert!(surface.dispatch_events(evs.as_validated()));

    // Retained Event<PressEventData> clones must NOT alias each other across EventSlots::reuse.
    let presses = log.retained_presses.borrow();
    assert_eq!(presses.len(), 2);
    assert_eq!((presses[0].x, presses[0].y), (11.0, 22.0));
    assert_eq!((presses[1].x, presses[1].y), (99.0, 88.0));

    let layouts = log.layouts.borrow();
    assert_eq!(layouts.as_slice(), &[(4.0, 8.0, 320.0, 180.0)]);

    let tevs = log.table_events.borrow();
    assert_eq!(
        tevs.as_slice(),
        &[(mod_key, EventId(5), vec![0xDE, 0xAD, 0xBE, 0xEF])]
    );
}

#[test]
fn value_table_attr_compares_by_value_and_detaches_on_clear() {
    use nve_tables::tables::Transform;

    fn write(tx: &mut nve_store::Transaction<'_>, node: NodeId, x: &f32) {
        tx.edit::<Transform>(node).translate_x(*x);
    }
    let a = crate::table_attr::<Transform, _>(4.0f32, write);
    assert!(a == crate::table_attr::<Transform, _>(4.0f32, write), "equal values skip rewrites");
    assert!(a != crate::table_attr::<Transform, _>(5.0f32, write));
    assert!(a != crate::table_attr_fn(|_, _| {}), "different writer kinds never compare equal");

    let mut store = NodeStore::new();
    let mut tx = store.tx();
    let n = tx.create();
    tx.set_children(NodeId::ROOT, &[n]);
    a.write_to(&mut tx, n);
    assert_eq!(tx.store().row::<Transform>(n).translate_x(), 4.0);
    a.clear_from(&mut tx, n);
    assert!(!tx.store().has::<Transform>(n), "clearing the attribute detaches its table");
}
