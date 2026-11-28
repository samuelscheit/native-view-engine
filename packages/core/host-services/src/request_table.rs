//! Inline multi-slot + overflow [`RequestTable`] enforcing exactly-once resolution for
//! asynchronous `NVE_HOST_PENDING` requests on [`crate::FfiHostServices`].

use std::collections::BTreeMap;

use crate::error::ServiceError;
use crate::provider::ResponseBuf;

/// Type-erased async completion callback avoiding double-boxing and `ResponseBuf::Inline` heap
/// copies when delivering replies through [`nve_codec::ByteTransport::call_async`].
pub(crate) enum AsyncCallback {
    Owned(Box<dyn FnOnce(Result<Vec<u8>, ServiceError>) + Send + 'static>),
    Transport(nve_codec::TransportDoneFn),
}

impl AsyncCallback {
    #[inline]
    pub(crate) fn invoke_vec(self, res: Result<Vec<u8>, ServiceError>) {
        match self {
            Self::Owned(cb) => cb(res),
            Self::Transport(cb) => match res {
                Ok(bytes) => cb(Ok(&bytes)),
                Err(err) => cb(Err(err.into())),
            },
        }
    }

    #[inline]
    pub(crate) fn invoke_buf(self, res: Result<ResponseBuf, ServiceError>) {
        match self {
            Self::Owned(cb) => cb(res.map(ResponseBuf::into_vec)),
            Self::Transport(cb) => match res {
                Ok(buf) => cb(Ok(buf.as_slice())),
                Err(err) => cb(Err(err.into())),
            },
        }
    }

    #[inline]
    pub(crate) fn invoke_err(self, err: ServiceError) {
        match self {
            Self::Owned(cb) => cb(Err(err)),
            Self::Transport(cb) => cb(Err(err.into())),
        }
    }
}

const INLINE_REQUEST_SLOTS: usize = 8;

enum RequestSlot {
    Pending { callback: Option<AsyncCallback> },
    Completed(Result<Vec<u8>, ServiceError>),
}

/// Tracks in-flight and resolved async `NVE_HOST_PENDING` requests.
///
/// Uses an 8-entry inline slot array so synchronous invocations incur zero heap allocation even
/// while several long-lived `Pending` requests remain outstanding, and enforces **exactly-once**
/// resolution for every pending `request_id`.
pub struct RequestTable {
    inline: [Option<(u64, RequestSlot)>; INLINE_REQUEST_SLOTS],
    overflow: BTreeMap<u64, RequestSlot>,
}

impl Default for RequestTable {
    fn default() -> Self {
        Self::new()
    }
}

impl RequestTable {
    #[must_use]
    pub const fn new() -> Self {
        Self {
            inline: [const { None }; INLINE_REQUEST_SLOTS],
            overflow: BTreeMap::new(),
        }
    }

    /// Registers a `Pending` slot for `request_id`.
    ///
    /// Returns `Err(callback)` without overwriting if `request_id` already exists (either
    /// `Pending` or `Completed`), returning ownership of `callback` to the caller without extra
    /// lookup passes.
    pub(crate) fn register_pending_callback(
        &mut self,
        request_id: u64,
        callback: Option<AsyncCallback>,
    ) -> Result<(), Option<AsyncCallback>> {
        let mut free_idx: Option<usize> = None;
        for (idx, slot) in self.inline.iter().enumerate() {
            match slot {
                Some((id, _)) if *id == request_id => return Err(callback),
                None if free_idx.is_none() => free_idx = Some(idx),
                _ => {}
            }
        }
        if !self.overflow.is_empty() && self.overflow.contains_key(&request_id) {
            return Err(callback);
        }
        let next = RequestSlot::Pending { callback };
        if let Some(idx) = free_idx {
            self.inline[idx] = Some((request_id, next));
        } else {
            self.overflow.insert(request_id, next);
        }
        Ok(())
    }

    /// Registers a polled `Pending` slot for `request_id` (without a callback).
    ///
    /// Returns `false` without overwriting if `request_id` already exists.
    pub fn register_pending(&mut self, request_id: u64) -> bool {
        self.register_pending_callback(request_id, None).is_ok()
    }

    /// Unconditionally removes `request_id` after a synchronous completion or error, even if a
    /// racing `cancel_all_pending` transitioned or removed it mid-call (prevents leaked inline
    /// slots from ever breaking the 0-alloc sync fast path).
    pub(crate) fn remove_sync_slot(&mut self, request_id: u64) -> Option<AsyncCallback> {
        for slot in &mut self.inline {
            if let Some((id, _)) = slot
                && *id == request_id
            {
                return match slot.take() {
                    Some((_, RequestSlot::Pending { callback })) => callback,
                    _ => None,
                };
            }
        }
        if self.overflow.is_empty() {
            return None;
        }
        match self.overflow.remove(&request_id) {
            Some(RequestSlot::Pending { callback }) => callback,
            _ => None,
        }
    }

    /// Resolves `request_id` from a completed async host response **exactly once**.
    /// Returns any registered callback so the caller can invoke it outside the mutex lock.
    /// Duplicate or unknown `request_id` completions are ignored cleanly.
    pub(crate) fn resolve_callback(
        &mut self,
        request_id: u64,
        result: Result<Vec<u8>, ServiceError>,
    ) -> Option<(AsyncCallback, Result<Vec<u8>, ServiceError>)> {
        for slot in &mut self.inline {
            if let Some((id, _)) = slot
                && *id == request_id
            {
                return match slot.take() {
                    Some((_, RequestSlot::Pending { callback: Some(cb) })) => Some((cb, result)),
                    Some((id, RequestSlot::Pending { callback: None })) => {
                        *slot = Some((id, RequestSlot::Completed(result)));
                        None
                    }
                    Some((id, done @ RequestSlot::Completed(_))) => {
                        *slot = Some((id, done));
                        None
                    }
                    None => None,
                };
            }
        }
        if self.overflow.is_empty() {
            return None;
        }
        match self.overflow.remove(&request_id) {
            Some(RequestSlot::Pending { callback: Some(cb) }) => Some((cb, result)),
            Some(RequestSlot::Pending { callback: None }) => {
                self.overflow
                    .insert(request_id, RequestSlot::Completed(result));
                None
            }
            Some(done @ RequestSlot::Completed(_)) => {
                self.overflow.insert(request_id, done);
                None
            }
            None => None,
        }
    }

    /// Resolves a polled `Pending` slot `request_id` **exactly once**.
    pub fn resolve(&mut self, request_id: u64, result: Result<Vec<u8>, ServiceError>) {
        if let Some((cb, res)) = self.resolve_callback(request_id, result) {
            cb.invoke_vec(res);
        }
    }

    /// Clears every slot (both `Pending` and uncollected `Completed` entries) so unbind /
    /// reload / drop never leaks zombie entries or permanently spills inline slots into
    /// `overflow`, returning `(pending_cancelled_count, callbacks)` for lock-free
    /// `ServiceError::Cancelled` invocation.
    pub(crate) fn cancel_all_pending_callbacks(&mut self) -> (usize, Vec<AsyncCallback>) {
        let mut total = 0usize;
        let mut callbacks = Vec::new();
        for slot in &mut self.inline {
            if let Some((_, RequestSlot::Pending { callback })) = slot.take() {
                total += 1;
                if let Some(cb) = callback {
                    callbacks.push(cb);
                }
            }
        }
        if !self.overflow.is_empty() {
            let overflow = std::mem::take(&mut self.overflow);
            for (_, entry) in overflow {
                if let RequestSlot::Pending { callback } = entry {
                    total += 1;
                    if let Some(cb) = callback {
                        callbacks.push(cb);
                    }
                }
            }
        }
        (total, callbacks)
    }

    /// Clears every slot and returns the count of pending requests cancelled.
    pub fn cancel_all_pending(&mut self) -> usize {
        let (total, cbs) = self.cancel_all_pending_callbacks();
        for cb in cbs {
            cb.invoke_err(ServiceError::Cancelled);
        }
        total
    }

    pub fn take_completed(&mut self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        for slot in &mut self.inline {
            if let Some((id, RequestSlot::Completed(_))) = slot
                && *id == request_id
                && let Some((_, RequestSlot::Completed(res))) = slot.take()
            {
                return Some(res);
            }
        }
        if matches!(
            self.overflow.get(&request_id),
            Some(RequestSlot::Completed(_))
        ) && let Some(RequestSlot::Completed(res)) = self.overflow.remove(&request_id)
        {
            return Some(res);
        }
        None
    }

    #[must_use]
    pub fn pending_count(&self) -> usize {
        let inline_pending = self
            .inline
            .iter()
            .filter(|s| matches!(s, Some((_, RequestSlot::Pending { .. }))))
            .count();
        inline_pending
            + self
                .overflow
                .values()
                .filter(|s| matches!(s, RequestSlot::Pending { .. }))
                .count()
    }

    #[must_use]
    pub fn total_slot_count(&self) -> usize {
        self.inline.iter().filter(|s| s.is_some()).count() + self.overflow.len()
    }
}
