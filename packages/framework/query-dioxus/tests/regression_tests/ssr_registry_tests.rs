//! `SsrBoundaryRegistry` slot-ID allocation / duplicate-ID merging and bounded
//! `BoundaryQuerySet` attribution across hook key switches.

use core::task::Waker;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use nve_backend_web::WebBackend;
use nve_dioxus::prelude::*;
use nve_dioxus::DioxusSurface;
use nve_query_dioxus::prelude::*;

use crate::{pump_settled, render_once, GetItem, NoopFlagWaker, RegTransport};

#[test]
fn auto_slot_ids_skip_explicit_ids_and_duplicate_ids_share_one_collector() {
    // Explicit "s0" registered first: the auto sequence must skip it.
    let reg = SsrBoundaryRegistry::new();
    let explicit = reg.allocate_slot_id(Some("s0"));
    assert_eq!(explicit, "s0");
    let first = reg.register(explicit);
    assert_eq!(reg.allocate_slot_id(None), "s1");
    assert_eq!(reg.allocate_slot_id(Some(" s!0 ")), "s0", "sanitized explicit id");

    // A later boundary reusing an existing id shares the original collector (no clobbering).
    let dup = reg.register("s0".into());
    assert!(Rc::ptr_eq(&first, &dup));
    assert_eq!(reg.boundaries().len(), 1);
    assert!(Rc::ptr_eq(&reg.entry("s0").unwrap().handles, &first));

    // Auto "s0" registered first, then an explicit "s0" collides with it.
    let reg = SsrBoundaryRegistry::new();
    let auto = reg.allocate_slot_id(None);
    assert_eq!(auto, "s0");
    let auto_set = reg.register(auto);
    let explicit_set = reg.register(reg.allocate_slot_id(Some("s0")));
    assert!(Rc::ptr_eq(&auto_set, &explicit_set));
    assert_eq!(reg.allocate_slot_id(None), "s1");
    assert_eq!(reg.boundaries().len(), 1);
}

#[test]
fn hook_key_switch_inside_ssr_boundary_keeps_attribution_bounded() {
    type KeySlot = Rc<RefCell<Option<Signal<u64>>>>;
    let key_slot: KeySlot = Rc::new(RefCell::new(None));

    #[component]
    fn Switching() -> Element {
        let slot = dioxus_core::consume_context::<KeySlot>();
        let key = use_signal(|| 1u64);
        use_hook(move || *slot.borrow_mut() = Some(key));
        let q = use_query::<GetItem>(key());
        let item = q.suspend()?;
        rsx! { text { "ITEM:{item.id}" } }
    }

    #[component]
    fn Root() -> Element {
        let client = dioxus_core::consume_context::<ErasedQueryClient>();
        rsx! {
            QueryClientProvider {
                client: client,
                QuerySuspense {
                    id: Some("k".into()),
                    fallback: rsx! { text { "LOADING" } },
                    Switching {}
                }
            }
        }
    }

    let registry = SsrBoundaryRegistry::new();
    let client = erased_ssr_query_client(RegTransport::new("Item"));
    let mut surface = DioxusSurface::new(1, Root)
        .with_context(client)
        .with_context(registry.clone())
        .with_context(Rc::clone(&key_slot));
    let mut web = WebBackend::new();
    let mut frame = 0;
    let waker = Waker::from(Arc::new(NoopFlagWaker(AtomicBool::new(false))));

    let _ = render_once(&mut surface, &mut web, &mut frame);
    let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
    assert!(web.render_html().contains("ITEM:1"));
    assert_eq!(registry.entry("k").unwrap().query_handles().len(), 1);

    for next in 2..=5u64 {
        let mut key = key_slot.borrow().unwrap();
        surface.in_root_scope(|| key.set(next));
        let _ = render_once(&mut surface, &mut web, &mut frame);
        let _ = pump_settled(&mut surface, &mut web, &waker, &mut frame);
        let html = web.render_html();
        assert!(html.contains(&format!("ITEM:{next}")), "{html}");
        assert_eq!(
            registry.entry("k").unwrap().query_handles().len(),
            1,
            "only the live key stays attributed after switching to {next}"
        );
    }
}
