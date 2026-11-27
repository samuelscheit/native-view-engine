//! End-to-end tests for `DioxusSurface` directly over `NodeStore`, `TypedStyle`, `TableAttr`
//! extensibility, surgical `HotPatchMsg`, direct event dispatch, and `NVE2` wire round-trip.

use nve_abi::{ColorRgba, EventPayload, NativeEvent, NativeEventKind};
use nve_components::tables::{
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
    let mut surface = DioxusSurface::new(1, CounterApp);
    let mut wire_bytes = Vec::new();
    let (_rec_us, _enc_us) = surface.render_nve2(1, &mut wire_bytes);
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
        AriaRole::Button
    );
    assert_ne!(
        surface.store().try_row::<Interaction>(btn).unwrap().event_mask(),
        0
    );
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
    let handled = surface.dispatch_events(&[NativeEvent {
        surface_id: 1,
        node_id: btn.raw(),
        timestamp_us: 100,
        kind: NativeEventKind::Press,
        payload: EventPayload::Point { x: 10.0, y: 10.0 },
    }]);
    assert!(handled);

    surface.render_nve2(2, &mut wire_bytes);
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
    let mut surface = DioxusSurface::new(1, MultiCardApp);
    surface.reconcile(1);

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
    surface.reconcile(2);

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
    let mut surface = DioxusSurface::new(1, CascadeApp);
    surface.reconcile(1);
    let noop = core::task::Waker::noop();
    let mut pumps = 0;
    while surface.poll_work(noop) {
        pumps += 1;
        assert!(pumps <= 4, "sync cascade must settle well inside MAX_PUMPS (8)");
        surface.reconcile(1 + pumps);
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
    surface.reconcile(99);
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
    let mut surface = DioxusSurface::new(1, SlotRoleApp);
    surface.reconcile(1);
    let role = |s: &DioxusSurface| {
        let slot = find_by_test_id(s.store(), "slot");
        let b = s.store().try_row::<StreamBoundary>(slot).expect("boundary row");
        (b.stream_slot().to_owned(), b.slot_role())
    };
    assert_eq!(role(&surface), ("draft".to_owned(), StreamSlotRole::Input));

    let press = |s: &mut DioxusSurface, frame| {
        let btn = find_by_test_id(s.store(), "next");
        assert!(s.dispatch_events(&[NativeEvent {
            surface_id: 1,
            node_id: btn.raw(),
            timestamp_us: 0,
            kind: NativeEventKind::Press,
            payload: EventPayload::Point { x: 1.0, y: 1.0 },
        }]));
        s.reconcile(frame);
    };
    press(&mut surface, 2);
    assert_eq!(role(&surface), ("draft".to_owned(), StreamSlotRole::Output));

    press(&mut surface, 3);
    let mut stack = vec![NodeId::ROOT];
    while let Some(id) = stack.pop() {
        assert!(!surface.store().has::<StreamBoundary>(id), "slot removed: no boundary left");
        stack.extend_from_slice(surface.store().children(id));
    }
    assert_eq!(StreamSlotRole::from_attr("bogus"), StreamSlotRole::None);
    assert_eq!(StreamSlotRole::None.as_attr(), None);
}
