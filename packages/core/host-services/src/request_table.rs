//! Inline multi-slot + overflow [`RequestTable`] enforcing exactly-once resolution for
//! asynchronous `NVE_HOST_PENDING` requests on [`crate::FfiHostServices`].

use std::collections::BTreeMap;

use crate::error::ServiceError;

pub(crate) type AsyncCallback = Box<dyn FnOnce(Result<Vec<u8>, ServiceError>) + Send + 'static>;

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
    pub const fn new() -> Self {
        Self {
            inline: [const { None }; INLINE_REQUEST_SLOTS],
            overflow: BTreeMap::new(),
        }
    }

    /// Registers a `Pending` slot for `request_id`.
    ///
    /// Returns `false` without overwriting if `request_id` already exists (either `Pending` or
    /// `Completed`).
    pub fn register_pending(&mut self, request_id: u64, callback: Option<AsyncCallback>) -> bool {
        let mut free_idx: Option<usize> = None;
        for (idx, slot) in self.inline.iter().enumerate() {
            match slot {
                Some((id, _)) if *id == request_id => return false,
                None if free_idx.is_none() => free_idx = Some(idx),
                _ => {}
            }
        }
        if !self.overflow.is_empty() && self.overflow.contains_key(&request_id) {
            return false;
        }
        let next = RequestSlot::Pending { callback };
        if let Some(idx) = free_idx {
            self.inline[idx] = Some((request_id, next));
        } else {
            self.overflow.insert(request_id, next);
        }
        true
    }

    /// Unconditionally removes `request_id` after a synchronous completion or error, even if a
    /// racing `cancel_all_pending` transitioned or removed it mid-call (prevents leaked inline
    /// slots from ever breaking the 0-alloc sync fast path).
    pub fn remove_sync_slot(&mut self, request_id: u64) -> Option<AsyncCallback> {
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
    pub fn resolve(
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

    /// Clears every slot (both `Pending` and uncollected `Completed` entries) so unbind /
    /// reload / drop never leaks zombie entries or permanently spills inline slots into
    /// `overflow`, returning `(pending_cancelled_count, callbacks)` for lock-free
    /// `ServiceError::Cancelled` invocation.
    pub fn cancel_all_pending(&mut self) -> (usize, Vec<AsyncCallback>) {
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

    pub fn total_slot_count(&self) -> usize {
        self.inline.iter().filter(|s| s.is_some()).count() + self.overflow.len()
    }
}
