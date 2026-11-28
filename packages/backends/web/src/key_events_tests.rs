//! `key_events` internals: interceptor dispatch re-entrancy, body-key routing, raw node ids.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use nve_store::NodeId;

use crate::key_events::*;

#[test]
fn text_like_input_types_and_raw_node_ids() {
    for t in [
        "text", "search", "EMAIL", "password", "number", "range", "date", "",
    ] {
        assert!(is_text_like_input_type(t), "{t}");
    }
    for t in [
        "checkbox", "Radio", "submit", "button", "file", "color", "hidden",
    ] {
        assert!(!is_text_like_input_type(t), "{t}");
    }
    let id = NodeId::new(0x12_3456, 7);
    assert_eq!(NodeId::from_raw(id.raw()), Some(id));
    assert_eq!(NodeId::from_raw(0), None, "the null node is never a target");
    assert_eq!(parse_dom_node_id(&id.raw().to_string()), Some(id));
    assert_eq!(parse_dom_node_id("1"), Some(NodeId::ROOT));
    for bad in ["0", "", "-3", "x1", "4294967296"] {
        assert_eq!(parse_dom_node_id(bad), None, "{bad}");
    }
}

type Log = Rc<RefCell<Vec<&'static str>>>;

fn logging(log: &Log, name: &'static str, consumes: Option<&'static str>) -> KeyInterceptor {
    let log = Rc::clone(log);
    Box::new(move |info| {
        log.borrow_mut().push(name);
        match consumes {
            Some(k) if k == info.key => WebKeyDisposition::ConsumedPreventDefault,
            _ => WebKeyDisposition::PassThrough,
        }
    })
}

#[test]
fn interceptors_run_in_order_until_one_consumes() {
    let log = Log::default();
    let cell = RefCell::new(KeyInterceptors::default());
    for (name, consumes) in [("a", None), ("b", Some("Enter")), ("c", Some("x"))] {
        cell.borrow_mut().push(logging(&log, name, consumes));
    }
    let enter = WebKeyEventInfo::new("Enter", "Enter", 13);
    assert_eq!(
        KeyInterceptors::try_dispatch(&cell, &enter),
        WebKeyDisposition::ConsumedPreventDefault
    );
    assert_eq!(*log.borrow(), ["a", "b"]);
    log.borrow_mut().clear();
    let q = WebKeyEventInfo::new("q", "KeyQ", 81);
    assert_eq!(
        KeyInterceptors::try_dispatch(&cell, &q),
        WebKeyDisposition::PassThrough
    );
    assert_eq!(*log.borrow(), ["a", "b", "c"]);
    let empty = RefCell::new(KeyInterceptors::default());
    assert_eq!(
        KeyInterceptors::try_dispatch(&empty, &q),
        WebKeyDisposition::PassThrough
    );
}

#[test]
fn interceptors_may_dispatch_add_and_remove_while_running() {
    let log = Log::default();
    let cell = Rc::new(RefCell::new(KeyInterceptors::default()));
    let dropped = Rc::new(Cell::new(false));
    // `a`: nested dispatch, registers `d`, then removes itself.
    let (weak, inner_log, flag) = (Rc::downgrade(&cell), Rc::clone(&log), Rc::clone(&dropped));
    let guard = DropFlag(flag);
    let a_id = Rc::new(Cell::new(None));
    let a_slot = Rc::clone(&a_id);
    let a = cell.borrow_mut().push(Box::new(move |info| {
        let _keep = &guard;
        inner_log.borrow_mut().push("a");
        let Some(cell) = weak.upgrade() else {
            return WebKeyDisposition::PassThrough;
        };
        assert_eq!(
            KeyInterceptors::try_dispatch(&cell, info),
            WebKeyDisposition::PassThrough
        );
        cell.borrow_mut().push(logging(&inner_log, "d", None));
        let removed = a_slot.get().and_then(|id| cell.borrow_mut().remove(id));
        assert!(
            matches!(removed, Some(None)),
            "a running interceptor is removed lazily"
        );
        WebKeyDisposition::PassThrough
    }));
    a_id.set(Some(a));
    cell.borrow_mut().push(logging(&log, "b", None));
    cell.borrow_mut().push(logging(&log, "c", None));
    let key = WebKeyEventInfo::new("ArrowUp", "ArrowUp", 38);
    assert_eq!(
        KeyInterceptors::try_dispatch(&cell, &key),
        WebKeyDisposition::PassThrough
    );
    // Nested dispatch skipped the running `a`; `d` (added meanwhile) ran after `c`.
    assert_eq!(*log.borrow(), ["a", "b", "c", "b", "c", "d"]);
    assert!(
        dropped.get(),
        "self-removed interceptor dropped once it returned"
    );
    log.borrow_mut().clear();
    assert_eq!(
        KeyInterceptors::try_dispatch(&cell, &key),
        WebKeyDisposition::PassThrough
    );
    assert_eq!(*log.borrow(), ["b", "c", "d"]);
    assert!(cell.borrow_mut().remove(a).is_none(), "already gone");
}

struct DropFlag(Rc<Cell<bool>>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.set(true);
    }
}

#[test]
fn body_keys_route_to_the_last_focused_or_first_mounted_live_surface() {
    let attached = |s: &Cell<bool>| s.get();
    let mut route = BodyKeyRoute::new();
    let first = Rc::new(Cell::new(true));
    let second = Rc::new(Cell::new(true));
    assert!(route.accepts(&first, attached), "first mounted claims");
    assert!(!route.accepts(&second, attached));
    assert!(route.accepts(&first, attached));
    route.focus_entered(&second);
    assert!(route.accepts(&second, attached), "focus moves ownership");
    assert!(!route.accepts(&first, attached));
    second.set(false);
    assert!(
        route.accepts(&first, attached),
        "detached owner is replaced"
    );
    drop(first);
    let third = Rc::new(Cell::new(true));
    assert!(
        route.accepts(&third, attached),
        "unmounted owner is replaced"
    );
}
