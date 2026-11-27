//! `<QuerySuspense>` slot IDs (disjoint auto `_sN` / explicit namespaces, identical server and
//! client sequences), distinct `SsrBoundaryRegistry` entries for duplicate explicit IDs, and
//! reference-counted `BoundaryQuerySet` attribution across hook key switches and unmounts.

use core::task::Waker;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query::{QueryHandle, QueryOptions};
use nve_query_dioxus::prelude::*;

use crate::{pump_settled, render_once, GetItem, NoopFlagWaker, RegTransport};

#[test]
fn auto_and_explicit_slot_ids_never_collide_in_either_order() {
    // Explicit "s0" first, then auto: auto IDs live in the reserved `_` namespace.
    let reg = SsrBoundaryRegistry::new();
    assert_eq!(reg.allocate_slot_id(Some("s0")), "s0");
    assert_eq!(reg.allocate_slot_id(None), "_s0");
    assert_eq!(reg.allocate_slot_id(Some(" s!1 ")), "s1", "sanitized explicit id");
    assert_eq!(reg.allocate_slot_id(None), "_s1");

    // Auto first, then explicit IDs spelling an auto ID: the leading `_` is stripped.
    let reg = SsrBoundaryRegistry::new();
    assert_eq!(reg.allocate_slot_id(None), "_s0");
    assert_eq!(reg.allocate_slot_id(Some("_s0")), "s0");
    assert_eq!(reg.allocate_slot_id(Some("__s1")), "s1");
    assert_eq!(reg.allocate_slot_id(Some("_-x_")), "-x_", "only leading `_` is reserved");
    assert_eq!(reg.allocate_slot_id(Some("___")), "_s1", "all-`_` id falls back to auto");
    assert_eq!(reg.allocate_slot_id(Some("!!")), "_s2", "empty id falls back to auto");
}

#[test]
fn duplicate_explicit_ids_register_distinct_boundaries_without_merging_handles() {
    let reg = SsrBoundaryRegistry::new();
    let (k1, first) = reg.register(reg.allocate_slot_id(Some("stats")));
    let (k2, second) = reg.register(reg.allocate_slot_id(Some("stats")));
    assert_ne!(k1, k2);
    assert!(!first.ptr_eq(&second), "duplicate ids keep separate collectors");
    assert_eq!(reg.boundaries().len(), 2);

    let client = erased_ssr_query_client(RegTransport::new("D"));
    let h = client.subscribe::<GetItem>(&7, QueryOptions::default()).unwrap();
    second.record(h);
    assert!(first.handles().is_empty(), "no cross-boundary leakage");
    assert_eq!(second.handles(), vec![h]);
    // Neither suspended: lookups resolve to the first registered boundary.
    assert!(reg.entry("stats").unwrap().handles.ptr_eq(&first));
    assert!(reg.pending_handles().is_empty(), "unsuspended boundaries stream nothing");
}

/// `data-nve-suspense` values in document order.
fn suspense_ids(html: &str) -> Vec<String> {
    html.split("data-nve-suspense=\"")
        .skip(1)
        .map(|rest| rest[..rest.find('"').unwrap()].to_owned())
        .collect()
}

#[component]
fn MixedIdsApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    let ids: [Option<&str>; 8] = [
        None,
        Some("s0"),
        Some("_s0"),
        None,
        Some("stats"),
        Some("_s1"),
        None,
        Some(""),
    ];
    rsx! {
        QueryClientProvider {
            client: client,
            for id in ids {
                QuerySuspense {
                    id: id.map(str::to_owned),
                    fallback: rsx! { text { "LOADING" } },
                    text { "READY" }
                }
            }
        }
    }
}

#[test]
fn server_and_client_assign_identical_disjoint_suspense_ids() {
    let render = |client: ErasedQueryClient, registry: Option<SsrBoundaryRegistry>| {
        let mut surface = DioxusSurface::new(1, MixedIdsApp).with_context(client);
        if let Some(reg) = registry {
            surface = surface.with_context(reg);
        }
        let mut web = WebBackend::new();
        let mut frame = 0;
        let _ = render_once(&mut surface, &mut web, &mut frame);
        suspense_ids(&web.render_html())
    };
    let registry = SsrBoundaryRegistry::new();
    let server = render(erased_ssr_query_client(RegTransport::new("S")), Some(registry.clone()));
    let client = render(erased_query_client(RegTransport::new("C")), None);

    let expected = ["_s0", "s0", "s0", "_s1", "stats", "s1", "_s2", "_s3"];
    assert_eq!(server, expected);
    assert_eq!(client, server, "client and server ids match 1-to-1");
    let auto: Vec<_> = server.iter().filter(|id| id.starts_with('_')).collect();
    assert!(server
        .iter()
        .filter(|id| !id.starts_with('_'))
        .all(|explicit| !auto.contains(&explicit)));
    assert_eq!(registry.boundaries().len(), expected.len(), "every boundary is distinct");
}

#[derive(Clone, Default)]
struct HookCtl {
    a_key: Rc<RefCell<Option<Signal<u64>>>>,
    b_key: Rc<RefCell<Option<Signal<u64>>>>,
    show_a: Rc<RefCell<Option<Signal<bool>>>>,
}

#[component]
fn Reader(key_slot: Rc<RefCell<Option<Signal<u64>>>>) -> Element {
    let key = use_signal(|| 1u64);
    use_hook(move || *key_slot.borrow_mut() = Some(key));
    let q = use_query::<GetItem>(key());
    let item = q.suspend()?;
    rsx! { text { "ITEM:{item.id}" } }
}

#[component]
fn SharedBoundaryApp() -> Element {
    let client = dioxus_core::consume_context::<ErasedQueryClient>();
    let ctl = dioxus_core::consume_context::<HookCtl>();
    let show_a = use_signal(|| true);
    let show_slot = ctl.show_a.clone();
    use_hook(move || *show_slot.borrow_mut() = Some(show_a));
    rsx! {
        QueryClientProvider {
            client: client,
            QuerySuspense {
                id: Some("k".into()),
                fallback: rsx! { text { "LOADING" } },
                if show_a() {
                    Reader { key_slot: ctl.a_key.clone() }
                }
                Reader { key_slot: ctl.b_key.clone() }
            }
        }
    }
}

#[test]
fn boundary_attribution_is_reference_counted_across_switches_and_unmounts() {
    let ctl = HookCtl::default();
    let registry = SsrBoundaryRegistry::new();
    let client = erased_ssr_query_client(RegTransport::new("Item"));
    let mut surface = DioxusSurface::new(1, SharedBoundaryApp)
        .with_context(client.clone())
        .with_context(registry.clone())
        .with_context(ctl.clone());
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));
    let handle_of = |id: u64| {
        let h = client.subscribe::<GetItem>(&id, QueryOptions::default()).unwrap();
        client.unsubscribe(h);
        h
    };
    let mut settle = |surface: &mut DioxusSurface| {
        let _ = render_once(surface, &mut web, &mut frame);
        let _ = pump_settled(surface, &mut web, &waker, &mut frame);
    };
    let attributed = || {
        let mut hs = registry.entry("k").unwrap().query_handles();
        hs.sort_by_key(|h| (h.slot.index(), h.generation));
        hs
    };
    let set_of = |ids: &[u64]| {
        let mut hs: Vec<QueryHandle> = ids.iter().map(|&id| handle_of(id)).collect();
        hs.sort_by_key(|h| (h.slot.index(), h.generation));
        hs
    };

    settle(&mut surface);
    assert_eq!(attributed(), set_of(&[1]), "both hooks share H1");

    // Hook A switches to H2: H1 stays attributed while hook B still reads it.
    let mut a_key = ctl.a_key.borrow().unwrap();
    surface.in_root_scope(|| a_key.set(2));
    settle(&mut surface);
    assert_eq!(attributed(), set_of(&[1, 2]));

    // Hook A unmounts: its H2 reference is released, B's H1 stays.
    let mut show_a = ctl.show_a.borrow().unwrap();
    surface.in_root_scope(|| show_a.set(false));
    settle(&mut surface);
    assert_eq!(attributed(), set_of(&[1]));

    // Hook B switches: the last H1 reference is released.
    let mut b_key = ctl.b_key.borrow().unwrap();
    surface.in_root_scope(|| b_key.set(3));
    settle(&mut surface);
    assert_eq!(attributed(), set_of(&[3]));

    // Remounting A records H1 again (fresh hook, fresh reference).
    surface.in_root_scope(|| show_a.set(true));
    settle(&mut surface);
    assert_eq!(attributed(), set_of(&[1, 3]));
}
