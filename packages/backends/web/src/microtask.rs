//! Re-entrant-safe browser microtask queue (`nve_backend_web::microtask`) backed by
//! `nve_host_services::ReentrantSlotSlab`, keeping reusable scheduling machinery out of `hosts/`.
//!
//! Automatically schedules a coalesced deferred reap microtask whenever a callback unwinds with
//! settled or re-entrantly cancelled closures so the last mounted surface never permanently leaks
//! closures or `Arc`s.

use std::cell::{Cell, RefCell};

use nve_host_services::ReentrantSlotSlab;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

#[wasm_bindgen(inline_js = "
const pendingMicrotasks = new Map();
export function nve_web_queue_microtask(id, cb) {
  pendingMicrotasks.set(id, cb);
  const run = () => {
    const fn = pendingMicrotasks.get(id);
    if (!fn) return;
    pendingMicrotasks.delete(id);
    try { fn(); } catch (_) {}
  };
  if (typeof queueMicrotask === 'function') {
    queueMicrotask(run);
  } else {
    Promise.resolve().then(run);
  }
}
export function nve_web_cancel_microtask(id) {
  pendingMicrotasks.delete(id);
}
export function nve_web_pending_microtask_count() {
  return pendingMicrotasks.size;
}
export function nve_web_schedule_reap(cb) {
  const run = () => { try { cb(); } catch (_) {} };
  if (typeof queueMicrotask === 'function') {
    queueMicrotask(run);
  } else {
    Promise.resolve().then(run);
  }
}
")]
extern "C" {
    fn nve_web_queue_microtask(id: f64, cb: &JsValue);
    fn nve_web_cancel_microtask(id: f64);
    fn nve_web_pending_microtask_count() -> u32;
    fn nve_web_schedule_reap(cb: &JsValue);
}

struct MicrotaskEntry {
    owner_key: usize,
    _cb: Closure<dyn FnMut()>,
}

thread_local! {
    static NEXT_TASK_ID: Cell<u64> = const { Cell::new(1) };
    static PENDING_TASKS: RefCell<ReentrantSlotSlab<u64, MicrotaskEntry>> =
        const { RefCell::new(ReentrantSlotSlab::new()) };
    static REAP_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static REAP_CLOSURE: Closure<dyn FnMut()> = Closure::new(|| {
        REAP_SCHEDULED.with(|f| f.set(false));
        reap_settled_microtasks();
    });
}

fn schedule_deferred_reap() {
    let already = REAP_SCHEDULED.with(|f| f.replace(true));
    if already {
        return;
    }
    REAP_CLOSURE.with(|cb| nve_web_schedule_reap(cb.as_ref()));
}

struct ActiveTaskGuard(u64);

impl Drop for ActiveTaskGuard {
    fn drop(&mut self) {
        let needs_reap = PENDING_TASKS.with(|slab| slab.borrow_mut().leave(self.0, true));
        if needs_reap {
            schedule_deferred_reap();
        }
    }
}

/// Drops any completed or cancelled microtask closures that are no longer executing.
pub fn reap_settled_microtasks() {
    PENDING_TASKS.with(|slab| slab.borrow_mut().reap_deferred());
}

/// Schedules a one-shot browser microtask owned by `owner_key`.
pub fn enqueue_microtask(owner_key: usize, f: impl FnOnce() + 'static) {
    reap_settled_microtasks();
    let id = NEXT_TASK_ID.with(|c| {
        let v = c.get();
        c.set(v.wrapping_add(1).max(1));
        v
    });
    let cb = Closure::once(move || {
        PENDING_TASKS.with(|slab| slab.borrow_mut().enter(id));
        let _guard = ActiveTaskGuard(id);
        f();
    });
    nve_web_queue_microtask(id as f64, cb.as_ref());
    PENDING_TASKS.with(|slab| {
        slab.borrow_mut().insert(
            id,
            MicrotaskEntry {
                owner_key,
                _cb: cb,
            },
        );
    });
}

/// Cancels every pending microtask owned by `owner_key`, deferring drop of any currently executing
/// microtask until after its callback unwinds.
pub fn cancel_owner_microtasks(owner_key: usize) {
    PENDING_TASKS.with(|slab| {
        slab.borrow_mut().remove_where(
            |_, entry| entry.owner_key == owner_key,
            |id| nve_web_cancel_microtask(id as f64),
        );
    });
}

/// Returns raw live `(rust_pending_tasks, js_pending_microtasks)` WITHOUT triggering a hidden reap
/// so leak tests verify automatic deferred reaping directly.
pub fn debug_microtask_counts() -> (usize, usize) {
    (
        PENDING_TASKS.with(|slab| slab.borrow().len()),
        nve_web_pending_microtask_count() as usize,
    )
}
