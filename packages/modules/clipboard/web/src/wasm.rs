//! Browser `wasm32` `wasm-bindgen` glue & re-entrant-safe [`ReentrantSlotSlab`] for
//! `nve-clipboard-web`, with automatic deferred reaping on callback unwind.

use std::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex};

use nve_clipboard::encode_read_text_response;
use nve_host_services::{CompletionSink, ReentrantSlotSlab, ServiceError};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::*;

use crate::ClipboardSlabCounts;

#[wasm_bindgen(inline_js = "
const pendingReads = new Map();
const clipListeners = new Map();

export function nve_web_clip_write(text) {
  try {
    if (typeof navigator !== 'undefined' && navigator.clipboard && typeof navigator.clipboard.writeText === 'function') {
      const p = navigator.clipboard.writeText(text);
      if (p && typeof p.catch === 'function') p.catch(() => {});
    }
  } catch (_) {}
}

export function nve_web_clip_read(token, onOk, onErr) {
  try {
    if (typeof navigator === 'undefined' || !navigator.clipboard || typeof navigator.clipboard.readText !== 'function') {
      return false;
    }
    pendingReads.set(token, { onOk, onErr });
    Promise.resolve(navigator.clipboard.readText()).then(
      (txt) => {
        const entry = pendingReads.get(token);
        if (entry) {
          pendingReads.delete(token);
          entry.onOk(String(txt ?? ''));
        }
      },
      (err) => {
        const entry = pendingReads.get(token);
        if (entry) {
          pendingReads.delete(token);
          const name = err && err.name ? String(err.name) : '';
          entry.onErr(name);
        }
      }
    );
    return true;
  } catch (_) {
    pendingReads.delete(token);
    return false;
  }
}

export function nve_web_clip_cancel(token) {
  pendingReads.delete(token);
}

export function nve_web_clip_attach(instId, onText) {
  if (typeof window === 'undefined' || typeof window.addEventListener !== 'function') return;
  const onCopy = (e) => {
    try {
      const data = e && e.clipboardData ? e.clipboardData.getData('text/plain') : '';
      if (data) { onText(data); return; }
      const sel = typeof window.getSelection === 'function' ? String(window.getSelection() || '') : '';
      if (sel) onText(sel);
    } catch (_) {}
  };
  const onPaste = (e) => {
    try {
      const data = e && e.clipboardData ? e.clipboardData.getData('text/plain') : '';
      if (data) onText(data);
    } catch (_) {}
  };
  window.addEventListener('copy', onCopy, true);
  window.addEventListener('paste', onPaste, true);
  clipListeners.set(instId, { onCopy, onPaste });
}

export function nve_web_clip_detach(instId) {
  const entry = clipListeners.get(instId);
  if (!entry) return;
  clipListeners.delete(instId);
  if (typeof window !== 'undefined' && typeof window.removeEventListener === 'function') {
    window.removeEventListener('copy', entry.onCopy, true);
    window.removeEventListener('paste', entry.onPaste, true);
  }
}

export function nve_web_clip_pending_count() {
  return pendingReads.size;
}

export function nve_web_clip_listener_count() {
  return clipListeners.size;
}

export function nve_web_clip_schedule_reap(cb) {
  const run = () => { try { cb(); } catch (_) {} };
  if (typeof queueMicrotask === 'function') {
    queueMicrotask(run);
  } else {
    Promise.resolve().then(run);
  }
}
")]
extern "C" {
    fn nve_web_clip_write(text: &str);
    fn nve_web_clip_read(token: f64, on_ok: &JsValue, on_err: &JsValue) -> bool;
    fn nve_web_clip_cancel(token: f64);
    fn nve_web_clip_attach(inst_id: f64, on_text: &JsValue);
    fn nve_web_clip_detach(inst_id: f64);
    fn nve_web_clip_pending_count() -> u32;
    fn nve_web_clip_listener_count() -> u32;
    fn nve_web_clip_schedule_reap(cb: &JsValue);
}

type TextClosure = Closure<dyn FnMut(String)>;

struct PendingReadSlot {
    instance_id: u64,
    owner_tag: u64,
    request_id: u64,
    done: CompletionSink,
    _on_ok: TextClosure,
    _on_err: TextClosure,
}

thread_local! {
    static PENDING_READS: RefCell<ReentrantSlotSlab<u64, PendingReadSlot>> =
        const { RefCell::new(ReentrantSlotSlab::new()) };
    static DOM_LISTENERS: RefCell<ReentrantSlotSlab<u64, TextClosure>> =
        const { RefCell::new(ReentrantSlotSlab::new()) };
    static REAP_SCHEDULED: Cell<bool> = const { Cell::new(false) };
    static REAP_CLOSURE: Closure<dyn FnMut()> = Closure::new(|| {
        REAP_SCHEDULED.with(|f| f.set(false));
        reap_settled();
    });
}

fn schedule_deferred_reap() {
    if REAP_SCHEDULED.with(|f| f.replace(true)) {
        return;
    }
    REAP_CLOSURE.with(|cb| nve_web_clip_schedule_reap(cb.as_ref()));
}

struct ActiveReadGuard(u64);

impl Drop for ActiveReadGuard {
    fn drop(&mut self) {
        let needs_reap = PENDING_READS.with(|slab| slab.borrow_mut().leave(self.0, true));
        if needs_reap {
            schedule_deferred_reap();
        }
    }
}

struct ActiveListenerGuard(u64);

impl Drop for ActiveListenerGuard {
    fn drop(&mut self) {
        let needs_reap = DOM_LISTENERS.with(|slab| slab.borrow_mut().leave(self.0, false));
        if needs_reap {
            schedule_deferred_reap();
        }
    }
}

pub(crate) fn write_text(text: &str) {
    nve_web_clip_write(text);
}

pub(crate) fn attach_dom_listeners(instance_id: u64, shadow: Arc<Mutex<Option<String>>>) {
    reap_settled();
    let cb = Closure::<dyn FnMut(String)>::new(move |txt: String| {
        DOM_LISTENERS.with(|slab| slab.borrow_mut().enter(instance_id));
        let _guard = ActiveListenerGuard(instance_id);
        *shadow.lock().unwrap_or_else(|e| e.into_inner()) = Some(txt);
    });
    nve_web_clip_attach(instance_id as f64, cb.as_ref());
    let evicted = DOM_LISTENERS.with(|slab| slab.borrow_mut().insert(instance_id, cb));
    drop(evicted);
}

pub(crate) fn detach_dom_listeners(instance_id: u64) {
    nve_web_clip_detach(instance_id as f64);
    let evicted = DOM_LISTENERS.with(|slab| slab.borrow_mut().remove_or_defer(instance_id));
    drop(evicted);
}

pub(crate) fn reap_settled() {
    let evicted_reads = PENDING_READS.with(|slab| slab.borrow_mut().reap_deferred());
    drop(evicted_reads);
    let evicted_listeners = DOM_LISTENERS.with(|slab| slab.borrow_mut().reap_deferred());
    drop(evicted_listeners);
}

fn cancel_matching(mut pred: impl FnMut(&PendingReadSlot) -> bool) {
    let mut to_cancel: Vec<(u64, CompletionSink)> = Vec::new();
    let (_keys, evicted) = PENDING_READS.with(|slab| {
        slab.borrow_mut().remove_where(
            |_, slot| pred(slot),
            |token, slot| {
                nve_web_clip_cancel(token as f64);
                to_cancel.push((slot.request_id, slot.done.clone()));
            },
        )
    });
    drop(evicted);
    for (req_id, done) in to_cancel {
        let _ = done.complete_err(req_id, &ServiceError::Cancelled);
    }
}

pub(crate) fn cancel_for_instance(instance_id: u64) {
    cancel_matching(|slot| slot.instance_id == instance_id);
}

pub(crate) fn cancel_for_owner(instance_id: u64, owner_tag: u64) {
    cancel_matching(|slot| slot.instance_id == instance_id && slot.owner_tag == owner_tag);
}

pub(crate) fn start_async_read(
    token: u64,
    instance_id: u64,
    owner_tag: u64,
    request_id: u64,
    shadow: Arc<Mutex<Option<String>>>,
    done: CompletionSink,
) -> bool {
    reap_settled();
    let shadow_ok = Arc::clone(&shadow);
    let done_ok = done.clone();
    let on_ok = Closure::<dyn FnMut(String)>::new(move |text: String| {
        PENDING_READS.with(|slab| slab.borrow_mut().enter(token));
        let _guard = ActiveReadGuard(token);
        *shadow_ok.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.clone());
        match encode_read_text_response(Some(&text)) {
            Ok(payload) => {
                let _ = done_ok.complete_ok(request_id, payload);
            }
            Err(err) => {
                let _ = done_ok.complete_err(request_id, &err);
            }
        }
    });

    let done_err = done.clone();
    let on_err = Closure::<dyn FnMut(String)>::new(move |name: String| {
        PENDING_READS.with(|slab| slab.borrow_mut().enter(token));
        let _guard = ActiveReadGuard(token);
        if name.eq_ignore_ascii_case("NotAllowedError")
            || name.eq_ignore_ascii_case("SecurityError")
        {
            let _ = done_err.complete_err(request_id, &ServiceError::PermissionDenied);
        } else if let Some(cached) = shadow.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            match encode_read_text_response(Some(&cached)) {
                Ok(payload) => {
                    let _ = done_err.complete_ok(request_id, payload);
                }
                Err(err) => {
                    let _ = done_err.complete_err(request_id, &err);
                }
            }
        } else {
            let _ = done_err.complete_err(request_id, &ServiceError::Unsupported);
        }
    });

    let started = nve_web_clip_read(token as f64, on_ok.as_ref(), on_err.as_ref());
    if started {
        let evicted = PENDING_READS.with(|slab| {
            slab.borrow_mut().insert(
                token,
                PendingReadSlot {
                    instance_id,
                    owner_tag,
                    request_id,
                    done,
                    _on_ok: on_ok,
                    _on_err: on_err,
                },
            )
        });
        drop(evicted);
    }
    started
}

pub(crate) fn debug_slab_counts() -> ClipboardSlabCounts {
    ClipboardSlabCounts {
        pending_reads: PENDING_READS.with(|slab| slab.borrow().len()),
        dom_listeners: DOM_LISTENERS.with(|slab| slab.borrow().len()),
        js_pending_reads: nve_web_clip_pending_count() as usize,
        js_dom_listeners: nve_web_clip_listener_count() as usize,
    }
}
