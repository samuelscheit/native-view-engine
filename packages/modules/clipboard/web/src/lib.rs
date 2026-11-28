//! Browser `wasm32` leaf provider for `SERVICE_CLIPBOARD` (`0x0010`).
//!
//! Uses a synchronous shadow buffer + DOM `copy`/`paste` listeners alongside an asynchronous
//! `HandlerReply::Pending` path for `navigator.clipboard.readText()` managed through
//! `nve_host_services::ReentrantSlotSlab` keyed by a globally unique atomic token mapped to
//! `(instance_id, request_id)` (zero `unsafe impl Send`, zero cross-surface ID collisions,
//! zero re-entrant closure UAF, zero leaked closures).

#![cfg_attr(doctest, doc = include_str!("../README.md"))]

use std::any::{Any, TypeId};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::SERVICE_CLIPBOARD;
pub use nve_clipboard::decode_read_text_response;
use nve_clipboard::{
    decode_write_text_request, encode_read_text_response, ClipboardBackend, METHOD_CLEAR,
    METHOD_HAS_TEXT, METHOD_READ_TEXT, METHOD_WRITE_TEXT,
};
use nve_host_services::{
    pack_typed, CompletionSink, HandlerReply, HostServiceHandler, ResponseBuf, ServiceError,
};

static NEXT_INSTANCE_ID: AtomicU64 = AtomicU64::new(1);
#[cfg(any(target_arch = "wasm32", test))]
static NEXT_READ_TOKEN: AtomicU64 = AtomicU64::new(1);

/// Snapshot of live clipboard closure/listener slab counts (for leak regression guards).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ClipboardSlabCounts {
    pub pending_reads: usize,
    pub dom_listeners: usize,
    pub js_pending_reads: usize,
    pub js_dom_listeners: usize,
}

/// Browser clipboard [`HostServiceHandler`] for [`SERVICE_CLIPBOARD`].
///
/// All `!Send` JS closures live in a `thread_local!` [`nve_host_services::ReentrantSlotSlab`]
/// indexed by `instance_id` / global read `token`, so `WebClipboard` is safely `Send + Sync`
/// without any `unsafe impl`.
pub struct WebClipboard {
    instance_id: u64,
    shadow: Arc<Mutex<Option<String>>>,
    #[cfg(test)]
    force_async_read: std::sync::atomic::AtomicBool,
}

impl Default for WebClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl WebClipboard {
    /// Creates a browser clipboard handler and (on `wasm32`) attaches DOM `copy`/`paste` listeners
    /// keeping the shadow buffer synchronized.
    pub fn new() -> Self {
        let instance_id = NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed);
        let shadow = Arc::new(Mutex::new(None));
        drop(shadow.lock());
        #[cfg(target_arch = "wasm32")]
        wasm_impl::attach_dom_listeners(instance_id, Arc::clone(&shadow));
        Self {
            instance_id,
            shadow,
            #[cfg(test)]
            force_async_read: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Creates a deterministic instance without attaching global DOM listeners (useful for tests).
    pub fn headless() -> Self {
        let shadow = Arc::new(Mutex::new(None));
        drop(shadow.lock());
        Self {
            instance_id: NEXT_INSTANCE_ID.fetch_add(1, Ordering::Relaxed),
            shadow,
            #[cfg(test)]
            force_async_read: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Returns this instance's globally unique identifier.
    #[inline]
    pub fn instance_id(&self) -> u64 {
        self.instance_id
    }

    /// Returns a snapshot of the synchronous shadow clipboard buffer.
    pub fn shadow_text(&self) -> Option<String> {
        self.shadow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// Updates the synchronous shadow clipboard buffer directly (e.g. from DOM `paste` events).
    pub fn set_shadow_text(&self, text: Option<String>) {
        *self.shadow.lock().unwrap_or_else(|e| e.into_inner()) = text;
    }

    /// Returns whether the shadow buffer currently holds non-empty text (zero heap allocation).
    #[inline]
    pub fn has_shadow_text(&self) -> bool {
        self.shadow
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_deref()
            .is_some_and(|s| !s.is_empty())
    }

    /// Cancels all in-flight browser clipboard `readText()` Promises owned by this instance,
    /// resolves their `CompletionSink` entries as `Cancelled`, and drops their `Closure` entries.
    pub fn cancel_pending(&self) {
        #[cfg(target_arch = "wasm32")]
        wasm_impl::cancel_for_instance(self.instance_id);
        #[cfg(all(test, not(target_arch = "wasm32")))]
        native_test_slab::cancel_for_instance(self.instance_id);
    }

    /// Cancels ONLY in-flight browser clipboard `readText()` Promises belonging to `owner_tag`
    /// on this instance, leaving sibling owners' pending Promises intact.
    pub fn cancel_pending_for_owner(&self, owner_tag: u64) {
        #[cfg(target_arch = "wasm32")]
        wasm_impl::cancel_for_owner(self.instance_id, owner_tag);
        #[cfg(all(test, not(target_arch = "wasm32")))]
        native_test_slab::cancel_for_owner(self.instance_id, owner_tag);
        #[cfg(not(any(target_arch = "wasm32", test)))]
        let _ = owner_tag;
    }

    /// Drops `wasm-bindgen` closures for Promises that have already settled outside JS callbacks.
    pub fn reap_settled_closures() {
        #[cfg(target_arch = "wasm32")]
        wasm_impl::reap_settled();
        #[cfg(all(test, not(target_arch = "wasm32")))]
        native_test_slab::reap_settled();
    }

    /// Returns raw live Rust and JS closure/listener slab counts WITHOUT triggering a hidden reap
    /// so leak tests verify automatic deferred reaping directly.
    pub fn debug_slab_counts() -> ClipboardSlabCounts {
        #[cfg(target_arch = "wasm32")]
        {
            wasm_impl::debug_slab_counts()
        }
        #[cfg(all(test, not(target_arch = "wasm32")))]
        {
            native_test_slab::debug_slab_counts()
        }
        #[cfg(not(any(target_arch = "wasm32", test)))]
        {
            ClipboardSlabCounts::default()
        }
    }
}

impl Drop for WebClipboard {
    fn drop(&mut self) {
        self.cancel_pending();
        #[cfg(target_arch = "wasm32")]
        wasm_impl::detach_dom_listeners(self.instance_id);
    }
}

impl ClipboardBackend for WebClipboard {
    fn read_text(&self) -> Result<Option<String>, ServiceError> {
        Ok(self.shadow_text())
    }

    fn write_text(&self, text: &str) -> Result<(), ServiceError> {
        *self.shadow.lock().unwrap_or_else(|e| e.into_inner()) = Some(text.to_owned());
        #[cfg(target_arch = "wasm32")]
        wasm_impl::write_text(text);
        Ok(())
    }

    fn has_text(&self) -> Result<bool, ServiceError> {
        Ok(self.has_shadow_text())
    }

    fn clear(&self) -> Result<(), ServiceError> {
        *self.shadow.lock().unwrap_or_else(|e| e.into_inner()) = None;
        #[cfg(target_arch = "wasm32")]
        wasm_impl::write_text("");
        Ok(())
    }
}

impl HostServiceHandler for WebClipboard {
    fn service_id(&self) -> u32 {
        SERVICE_CLIPBOARD
    }

    fn handle(
        &self,
        method_id: u32,
        #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(unused_variables))]
        request_id: u64,
        request: &[u8],
        #[cfg_attr(not(any(target_arch = "wasm32", test)), allow(unused_variables))]
        done: &CompletionSink,
    ) -> Result<HandlerReply, ServiceError> {
        match method_id {
            METHOD_WRITE_TEXT => {
                let text = decode_write_text_request(request)?;
                self.write_text(&text)?;
                Ok(HandlerReply::empty())
            }
            METHOD_HAS_TEXT => {
                let flag = [u8::from(self.has_shadow_text())];
                Ok(HandlerReply::Ready(ResponseBuf::from_slice_or_vec(&flag)))
            }
            METHOD_CLEAR => {
                self.clear()?;
                Ok(HandlerReply::empty())
            }
            METHOD_READ_TEXT => {
                #[cfg(target_arch = "wasm32")]
                {
                    let token = NEXT_READ_TOKEN.fetch_add(1, Ordering::Relaxed);
                    let owner_tag = done.owner_tag_for_in_flight(request_id).unwrap_or(0);
                    if wasm_impl::start_async_read(
                        token,
                        self.instance_id,
                        owner_tag,
                        request_id,
                        Arc::clone(&self.shadow),
                        done.clone(),
                    ) {
                        return Ok(HandlerReply::Pending);
                    }
                }
                #[cfg(all(test, not(target_arch = "wasm32")))]
                {
                    if self.force_async_read.load(Ordering::Acquire) {
                        let token = NEXT_READ_TOKEN.fetch_add(1, Ordering::Relaxed);
                        let owner_tag = done.owner_tag_for_in_flight(request_id).unwrap_or(0);
                        native_test_slab::insert(
                            token,
                            self.instance_id,
                            owner_tag,
                            request_id,
                            Arc::clone(&self.shadow),
                            done.clone(),
                        );
                        return Ok(HandlerReply::Pending);
                    }
                }
                let text = self.shadow_text();
                let bytes = encode_read_text_response(text.as_deref())?;
                Ok(HandlerReply::ready(bytes))
            }
            _ => Err(ServiceError::Unsupported),
        }
    }

    fn typed_backend(self: Arc<Self>) -> Option<(TypeId, Arc<dyn Any + Send + Sync>)> {
        Some(pack_typed::<dyn ClipboardBackend>(self))
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
use wasm as wasm_impl;

#[cfg(all(test, not(target_arch = "wasm32")))]
mod native_test_slab {
    use std::sync::atomic::Ordering;
    use std::sync::{Arc, Mutex};

    use nve_clipboard::{decode_read_text_response, encode_read_text_response, METHOD_READ_TEXT};
    use nve_host_services::{
        CompletionSink, HandlerReply, HostServiceHandler, ReentrantSlotSlab, ServiceError,
    };

    use super::{ClipboardSlabCounts, WebClipboard};

    #[derive(Clone)]
    struct PendingReadEntry {
        instance_id: u64,
        owner_tag: u64,
        request_id: u64,
        shadow: Arc<Mutex<Option<String>>>,
        done: CompletionSink,
    }

    static SLAB: Mutex<ReentrantSlotSlab<u64, PendingReadEntry>> =
        Mutex::new(ReentrantSlotSlab::new());

    pub(super) fn insert(
        token: u64,
        instance_id: u64,
        owner_tag: u64,
        request_id: u64,
        shadow: Arc<Mutex<Option<String>>>,
        done: CompletionSink,
    ) {
        let evicted = SLAB.lock().unwrap_or_else(|e| e.into_inner()).insert(
            token,
            PendingReadEntry {
                instance_id,
                owner_tag,
                request_id,
                shadow,
                done,
            },
        );
        drop(evicted);
    }

    fn cancel_matching(mut pred: impl FnMut(&PendingReadEntry) -> bool) {
        let mut to_cancel: Vec<(u64, CompletionSink)> = Vec::new();
        let (_keys, evicted) = SLAB
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove_where(
                |_, e| pred(e),
                |_, e| to_cancel.push((e.request_id, e.done.clone())),
            );
        drop(evicted);
        for (req_id, done) in to_cancel {
            let _ = done.complete_err(req_id, &ServiceError::Cancelled);
        }
    }

    pub(super) fn cancel_for_instance(instance_id: u64) {
        cancel_matching(|e| e.instance_id == instance_id);
    }

    pub(super) fn cancel_for_owner(instance_id: u64, owner_tag: u64) {
        cancel_matching(|e| e.instance_id == instance_id && e.owner_tag == owner_tag);
    }

    pub(super) fn reap_settled() {
        let evicted = SLAB
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .reap_deferred();
        drop(evicted);
    }

    pub(super) fn debug_slab_counts() -> ClipboardSlabCounts {
        ClipboardSlabCounts {
            pending_reads: SLAB.lock().unwrap_or_else(|e| e.into_inner()).len(),
            dom_listeners: 0,
            js_pending_reads: 0,
            js_dom_listeners: 0,
        }
    }

    fn tokens_for(instance_id: u64) -> Vec<u64> {
        SLAB.lock()
            .unwrap_or_else(|e| e.into_inner())
            .iter()
            .filter_map(|(tok, e)| (e.instance_id == instance_id).then_some(*tok))
            .collect()
    }

    fn resolve_token(token: u64, text: Result<&str, ServiceError>) -> bool {
        let entry = {
            let mut st = SLAB.lock().unwrap_or_else(|e| e.into_inner());
            let Some((_, e)) = st.iter().find(|(k, _)| **k == token) else {
                return false;
            };
            let cloned = e.clone();
            st.enter(token);
            cloned
        };
        struct LeaveGuard(u64);
        impl Drop for LeaveGuard {
            fn drop(&mut self) {
                let evicted = {
                    let mut st = SLAB.lock().unwrap_or_else(|e| e.into_inner());
                    if st.leave(self.0, true) {
                        st.reap_deferred()
                    } else {
                        Vec::new()
                    }
                };
                drop(evicted);
            }
        }
        let _guard = LeaveGuard(token);
        match text {
            Ok(txt) => {
                *entry.shadow.lock().unwrap_or_else(|e| e.into_inner()) = Some(txt.to_owned());
                match encode_read_text_response(Some(txt)) {
                    Ok(payload) => {
                        let _ = entry.done.complete_ok(entry.request_id, payload);
                    }
                    Err(err) => {
                        let _ = entry.done.complete_err(entry.request_id, &err);
                    }
                }
            }
            Err(err) => {
                let _ = entry.done.complete_err(entry.request_id, &err);
            }
        }
        true
    }

    #[test]
    fn multi_surface_same_request_id_and_nested_reentrant_cancel_with_panic_recovery() {
        let clip_a = Arc::new(WebClipboard::headless());
        let clip_b = Arc::new(WebClipboard::headless());
        clip_a.force_async_read.store(true, Ordering::Release);
        clip_b.force_async_read.store(true, Ordering::Release);

        let sink_a = CompletionSink::new();
        let sink_b = CompletionSink::new();

        assert!(sink_a.register_in_flight(1, 0x0010, METHOD_READ_TEXT));
        assert!(sink_b.register_in_flight(1, 0x0010, METHOD_READ_TEXT));
        assert_eq!(
            clip_a.handle(METHOD_READ_TEXT, 1, &[], &sink_a).unwrap(),
            HandlerReply::Pending
        );
        assert_eq!(
            clip_b.handle(METHOD_READ_TEXT, 1, &[], &sink_b).unwrap(),
            HandlerReply::Pending
        );

        let toks_a = tokens_for(clip_a.instance_id());
        let toks_b = tokens_for(clip_b.instance_id());
        assert_eq!(toks_a.len(), 1);
        assert_eq!(toks_b.len(), 1);
        assert_ne!(toks_a[0], toks_b[0]);

        // Re-entrant cancel + panicking on_ready recovery: LeaveGuard unwinds and reaps cleanly.
        let clip_a_reentrant = Arc::clone(&clip_a);
        sink_a.set_on_ready(Some(Arc::new(move || {
            clip_a_reentrant.cancel_pending();
            panic!("simulated on_ready panic");
        })));
        let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            resolve_token(toks_a[0], Ok("from-surface-A"))
        }));
        assert!(res.is_err());
        assert!(resolve_token(toks_b[0], Ok("from-surface-B")));

        let res_a = sink_a.take_completed_response(1).unwrap().unwrap();
        let res_b = sink_b.take_completed_response(1).unwrap().unwrap();
        assert_eq!(
            decode_read_text_response(&res_a).unwrap().as_deref(),
            Some("from-surface-A")
        );
        assert_eq!(
            decode_read_text_response(&res_b).unwrap().as_deref(),
            Some("from-surface-B")
        );
        assert!(tokens_for(clip_a.instance_id()).is_empty());
        assert!(tokens_for(clip_b.instance_id()).is_empty());
        assert_eq!(WebClipboard::debug_slab_counts().pending_reads, 0);
    }

    #[test]
    fn cancel_pending_for_owner_preserves_sibling_and_cancel_resolves_sink() {
        let clip = Arc::new(WebClipboard::headless());
        clip.force_async_read.store(true, Ordering::Release);
        let sink = CompletionSink::new();

        let h1 = sink
            .register_in_flight_for_owner(10, 1, 0x0010, METHOD_READ_TEXT)
            .unwrap();
        let h2 = sink
            .register_in_flight_for_owner(20, 1, 0x0010, METHOD_READ_TEXT)
            .unwrap();
        assert_eq!(
            clip.handle(METHOD_READ_TEXT, h1, &[], &sink).unwrap(),
            HandlerReply::Pending
        );
        assert_eq!(
            clip.handle(METHOD_READ_TEXT, h2, &[], &sink).unwrap(),
            HandlerReply::Pending
        );

        // Cancel ONLY owner 10: owner 10's request resolves as Cancelled; owner 20 stays pending!
        clip.cancel_pending_for_owner(10);
        let toks = tokens_for(clip.instance_id());
        assert_eq!(toks.len(), 1, "sibling owner 20's pending read must remain intact");
        let mut b10 = nve_abi::NveHostEventBatchBuilder::new();
        assert_eq!(sink.drain_batch_for_owner_into(10, &mut b10).unwrap(), 1);
        let parsed10 = nve_abi::NveHostEventBatchRef::parse(b10.as_bytes()).unwrap();
        let rec10 = parsed10.iter().next().unwrap();
        assert_eq!(rec10.status, nve_abi::NveHostStatus::Cancelled);

        // Owner 20 still completes normally!
        assert!(resolve_token(toks[0], Ok("sibling-intact")));
        let mut b20 = nve_abi::NveHostEventBatchBuilder::new();
        assert_eq!(sink.drain_batch_for_owner_into(20, &mut b20).unwrap(), 1);
        let parsed20 = nve_abi::NveHostEventBatchRef::parse(b20.as_bytes()).unwrap();
        let rec20 = parsed20.iter().next().unwrap();
        assert_eq!(rec20.status, nve_abi::NveHostStatus::Ok);
        assert_eq!(
            decode_read_text_response(rec20.payload).unwrap().as_deref(),
            Some("sibling-intact")
        );

        // Dropping WebClipboard with a live pending read resolves CompletionSink as Cancelled.
        let h3 = sink
            .register_in_flight_for_owner(30, 7, 0x0010, METHOD_READ_TEXT)
            .unwrap();
        assert_eq!(
            clip.handle(METHOD_READ_TEXT, h3, &[], &sink).unwrap(),
            HandlerReply::Pending
        );
        drop(clip);
        let mut b30 = nve_abi::NveHostEventBatchBuilder::new();
        assert_eq!(sink.drain_batch_for_owner_into(30, &mut b30).unwrap(), 1);
        let parsed30 = nve_abi::NveHostEventBatchRef::parse(b30.as_bytes()).unwrap();
        assert_eq!(
            parsed30.iter().next().unwrap().status,
            nve_abi::NveHostStatus::Cancelled
        );
    }
}
