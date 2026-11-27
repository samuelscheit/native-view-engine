//! Shared-fetch sibling unmount (P1 #5), row-unmount optimistic survival & pre-poll provider
//! unmount rollback (P1 #6), zero-alloc `flush_dirty_slots`, and clean wake-callback teardown.

use core::task::Waker;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{ManualClock, QueryClientConfig, QueryOptions, Shared};
use nve_query_dioxus::prelude::*;
use nve_query_dioxus::DioxusQueryBridge;
use nve_rpc::Tag64;
use nve_ui::prelude::*;

use crate::{
    count_allocs, pump_settled, render_once, GetItem, Item, NoopFlagWaker, RegTransport,
    RenameItem, RenameItemInput,
};

#[test]
fn sibling_unmount_mid_fetch_never_strands_survivor_and_optimistic_survives_row_unmount() {
    let transport = RegTransport::new("Initial");
    transport.inner.lock().unwrap().defer_queries = true;
    let client = erased_query_client_with_clock(
        transport.clone(),
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );

    type FlagsSlot = Rc<RefCell<Option<(Signal<bool>, Signal<bool>, Signal<bool>)>>>;
    let flags_slot: FlagsSlot = Rc::new(RefCell::new(None));

    #[component]
    fn SiblingA() -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let txt = q.data().map(|d| d.name.clone()).unwrap_or_else(|| "LOAD_A".into());
        rsx! { text { "A:{txt}" } }
    }

    #[component]
    fn SiblingB() -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let txt = q.data().map(|d| d.name.clone()).unwrap_or_else(|| "LOAD_B".into());
        rsx! { text { "B:{txt}" } }
    }

    #[component]
    fn RowAction(mut show_row: Signal<bool>) -> Element {
        let act = use_action::<RenameItem>();
        rsx! {
            Button {
                test_id: "row-opt-btn".to_owned(),
                on_press: move |_| {
                    act.run_optimistic(
                        RenameItemInput {
                            id: 1,
                            name: "RenamedOnServer".into(),
                        },
                        |tx| {
                            let _ = tx.update_query::<GetItem>(&1, |old| Item {
                                id: old.id,
                                name: "OptimisticName".into(),
                            });
                        },
                    );
                    show_row.set(false);
                },
                "RenameAndUnmount"
            }
        }
    }

    #[component]
    fn SharedRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let slot = dioxus_core::consume_context::<FlagsSlot>();
        let show_a = use_signal(|| true);
        let show_row = use_signal(|| true);
        let show_provider = use_signal(|| true);
        use_hook(move || {
            *slot.borrow_mut() = Some((show_a, show_row, show_provider));
        });
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                if show_provider() {
                    QueryClientProvider {
                        client: client,
                        Col {
                            if show_a() {
                                SiblingA {}
                            }
                            SiblingB {}
                            if show_row() {
                                RowAction { show_row: show_row }
                            }
                        }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(1, SharedRoot)
        .with_context(client.clone())
        .with_context(Rc::clone(&flags_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("A:LOAD_A"));
    assert!(web.render_html().contains("B:LOAD_B"));

    // Unmount SiblingA (starter of shared fetch) while request is pending.
    let (mut show_a, _, mut show_provider) = flags_slot.borrow().unwrap();
    surface.in_root_scope(|| show_a.set(false));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);

    // Resolve server query -> SiblingB MUST receive "Initial" (provider-scoped fetch survives!).
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html().contains("B:Initial"),
        "surviving sibling must not be stranded when starter unmounts: {}",
        web.render_html()
    );

    // Defer action, click RowAction (which applies optimistic patch AND unmounts itself!).
    transport.inner.lock().unwrap().defer_actions = true;
    let btn = web
        .iter_nodes()
        .find_map(|(id, el)| {
            matches!(
                el.raw_props.get("testID"),
                Some(nve_abi::RawPropValue::String(s)) if s == "row-opt-btn"
            )
            .then_some(id)
        })
        .unwrap();
    assert!(surface.dispatch_events(&[nve_abi::NativeEvent {
        surface_id: 1,
        node_id: btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(
        web.render_html().contains("B:OptimisticName"),
        "optimistic patch must remain visible after triggering row unmounts: {}",
        web.render_html()
    );

    // Release server action -> completes and settles server value even though RowAction unmounted!
    transport.release_all();
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert_eq!(transport.inner.lock().unwrap().action_calls, 1);
    assert!(
        web.render_html().contains("B:RenamedOnServer"),
        "action must complete after triggering row unmounts: {}",
        web.render_html()
    );

    // Unmount provider cleanly -> active slots drop to 0 and detached wake callback never panics.
    surface.in_root_scope(|| show_provider.set(false));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(client.active_slot_count(), 0);
    let _ = client.invalidate_all();
}

#[test]
fn provider_unmount_before_first_poll_rolls_back_optimistic_and_flush_is_zero_alloc() {
    let transport = RegTransport::new("Base");
    let client = erased_query_client_with_clock(
        transport,
        Shared::new(ManualClock::new(1_000)),
        QueryClientConfig::default(),
    );
    client
        .set_query_data::<GetItem>(
            &1,
            Item {
                id: 1,
                name: "Base".into(),
            },
        )
        .unwrap();

    type BridgeSlot = Rc<RefCell<Option<(DioxusQueryBridge, Signal<bool>)>>>;
    let slot: BridgeSlot = Rc::new(RefCell::new(None));

    #[component]
    fn Inner(mut show_provider: Signal<bool>) -> Element {
        let q = use_query_with::<GetItem>(1, QueryOptions::default().stale_time_ms(60_000));
        let act = use_action::<RenameItem>();
        let bridge = dioxus_core::consume_context::<DioxusQueryBridge>();
        let slot = dioxus_core::consume_context::<BridgeSlot>();
        use_hook(move || {
            *slot.borrow_mut() = Some((bridge, show_provider));
        });
        let name = q.data().map(|d| d.name.clone()).unwrap_or_default();
        rsx! {
            Col {
                text { "VAL:{name}" }
                Button {
                    test_id: "unmount-provider-btn".to_owned(),
                    on_press: move |_| {
                        act.run_optimistic(
                            RenameItemInput {
                                id: 1,
                                name: "Ghost".into(),
                            },
                            |tx| {
                                let _ = tx.update_query::<GetItem>(&1, |old| Item {
                                    id: old.id,
                                    name: "GhostOpt".into(),
                                });
                            },
                        );
                        show_provider.set(false);
                    },
                    "KillProvider"
                }
            }
        }
    }

    #[component]
    fn WrapRoot() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        let show_provider = use_signal(|| true);
        rsx! {
            ThemeProvider {
                theme: UiTheme::light(),
                if show_provider() {
                    QueryClientProvider {
                        client: client,
                        Inner { show_provider: show_provider }
                    }
                }
            }
        }
    }

    let mut surface = DioxusSurface::new(1, WrapRoot)
        .with_context(client.clone())
        .with_context(Rc::clone(&slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("VAL:Base"));

    let (bridge, _) = slot.borrow().clone().unwrap();
    assert_eq!(bridge.live_signal_count(), 1);

    // Warm one flush cycle, then verify:
    // 1. Quiet/no-op `flush_dirty_slots` performs 0 total heap allocations.
    // 2. Dirty `flush_dirty_slots` adds 0 bridge allocations on top of raw `dioxus_signals::Signal::set`.
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 1)]);
    surface.in_root_scope(|| bridge.flush_dirty_slots());
    let (_, quiet_allocs) = count_allocs(|| {
        surface.in_root_scope(|| bridge.flush_dirty_slots());
    });
    assert_eq!(quiet_allocs, 0, "quiet flush_dirty_slots must allocate 0 times");

    let mut raw_sig = surface.in_root_scope(|| Signal::new(0u32));
    let _ = surface.in_root_scope(|| *raw_sig.read());
    let (_, raw_sig_allocs) = count_allocs(|| {
        surface.in_root_scope(|| raw_sig.set(1));
    });
    let _ = client.invalidate_tags(&[Tag64::keyed("item", 1)]);
    let (_, flush_allocs) = count_allocs(|| {
        surface.in_root_scope(|| bridge.flush_dirty_slots());
    });
    assert!(
        flush_allocs <= raw_sig_allocs + 1,
        "flush_dirty_slots added unexpected bridge allocations (flush={flush_allocs}, raw_signal_set={raw_sig_allocs})"
    );

    // Click KillProvider: applies optimistic patch, spawns task, and IMMEDIATELY unmounts provider
    // before the spawned future's first poll -> pre-spawn OptimisticDropGuard MUST roll back!
    let btn = web
        .iter_nodes()
        .find_map(|(id, el)| {
            matches!(
                el.raw_props.get("testID"),
                Some(nve_abi::RawPropValue::String(s)) if s == "unmount-provider-btn"
            )
            .then_some(id)
        })
        .unwrap();
    assert!(surface.dispatch_events(&[nve_abi::NativeEvent {
        surface_id: 1,
        node_id: btn,
        timestamp_us: 0,
        kind: nve_abi::NativeEventKind::Press,
        payload: nve_abi::EventPayload::Point { x: 1.0, y: 1.0 },
    }]));
    let _ = render_once(&mut surface, &mut web, &mut frame);
    assert_eq!(bridge.live_signal_count(), 0);

    // Re-subscribe and verify the optimistic patch was rolled back cleanly to "Base"!
    let h = client.subscribe::<GetItem>(&1, QueryOptions::default()).unwrap();
    assert_eq!(client.read::<GetItem>(h).data.unwrap().name, "Base");
}
