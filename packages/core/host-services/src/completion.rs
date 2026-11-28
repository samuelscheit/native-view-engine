//! Asynchronous host-service handler completion sink (`CompletionSink`) and [`HandlerReply`].
//!
//! Enforces the **exactly-once** resolution invariant for every `HandlerReply::Pending` call,
//! maintains an 8-slot inline fast table so synchronous calls stay zero-allocation even while
//! multiple async `Pending` requests remain in flight, bounds outbound queue depth, and enforces
//! [`NVE_HOST_BATCH_MAX_BYTES`] per drained batch.

use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use nve_abi::{
    host_batch_padded_record_len, NveHostBatchError, NveHostEventBatchBuilder, NveHostEventKind,
    NveHostStatus, NVE_HOST_BATCH_MAX_BYTES, NVE_HOST_BATCH_MAX_PAYLOAD,
    NVE_HOST_RECORD_FLAG_RETAIN, SERVICE_CHANNEL,
};

use crate::error::ServiceError;
use crate::provider::{
    next_host_request_id, HostChannelEvent, HostEvent, HostPushEvent, ResponseBuf,
};

/// Number of inline in-flight slots kept off-heap so synchronous invocations stay 0-alloc even
/// while several long-lived `Pending` requests are outstanding.
const INLINE_IN_FLIGHT_SLOTS: usize = 8;

/// Default maximum number of queued outbound records retained by [`CompletionSink`].
pub const DEFAULT_COMPLETION_QUEUE_CAP: usize = 4096;

/// Synchronous or asynchronous reply returned by [`crate::HostServiceHandler::handle`].
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HandlerReply {
    /// Handler completed synchronously with a binary response payload.
    Ready(ResponseBuf),
    /// Handler accepted the request asynchronously and will complete `request_id`
    /// exactly once through the supplied [`CompletionSink`].
    Pending,
}

impl HandlerReply {
    #[inline]
    pub fn ready(bytes: impl Into<ResponseBuf>) -> Self {
        Self::Ready(bytes.into())
    }

    #[inline]
    pub const fn empty() -> Self {
        Self::Ready(ResponseBuf::empty())
    }
}

#[derive(Debug, Clone, Copy)]
struct PendingMeta {
    owner_tag: u64,
    caller_request_id: u64,
    service_id: u32,
    method_id: u32,
}

#[derive(Debug, Clone)]
struct QueuedRecord {
    owner_tag: u64,
    kind: NveHostEventKind,
    flags: u8,
    status: NveHostStatus,
    service_id: u32,
    method_or_event_or_channel: u32,
    request_id: u64,
    payload: Vec<u8>,
}

type ReadyWakeFn = Arc<dyn Fn() + Send + Sync>;

struct CompletionSinkState {
    inline_in_flight: [Option<(u64, PendingMeta)>; INLINE_IN_FLIGHT_SLOTS],
    overflow_in_flight: BTreeMap<u64, PendingMeta>,
    records: VecDeque<QueuedRecord>,
    max_queued_records: usize,
    on_ready: Option<ReadyWakeFn>,
    dropped_oversized_records: u64,
    dropped_overflow_records: u64,
}

impl CompletionSinkState {
    fn new(max_queued_records: usize) -> Self {
        Self {
            inline_in_flight: [None; INLINE_IN_FLIGHT_SLOTS],
            overflow_in_flight: BTreeMap::new(),
            records: VecDeque::new(),
            max_queued_records: max_queued_records.max(1),
            on_ready: None,
            dropped_oversized_records: 0,
            dropped_overflow_records: 0,
        }
    }

    #[inline]
    fn insert_in_flight(&mut self, host_id: u64, meta: PendingMeta) -> bool {
        let mut free_slot: Option<usize> = None;
        for (idx, slot) in self.inline_in_flight.iter().enumerate() {
            match slot {
                Some((id, existing))
                    if *id == host_id
                        || (existing.owner_tag == meta.owner_tag
                            && existing.caller_request_id == meta.caller_request_id) =>
                {
                    return false;
                }
                None if free_slot.is_none() => free_slot = Some(idx),
                _ => {}
            }
        }
        if !self.overflow_in_flight.is_empty()
            && (self.overflow_in_flight.contains_key(&host_id)
                || self.overflow_in_flight.values().any(|existing| {
                    existing.owner_tag == meta.owner_tag
                        && existing.caller_request_id == meta.caller_request_id
                }))
        {
            return false;
        }
        if let Some(idx) = free_slot {
            self.inline_in_flight[idx] = Some((host_id, meta));
        } else {
            self.overflow_in_flight.insert(host_id, meta);
        }
        true
    }

    #[inline]
    fn remove_in_flight(&mut self, request_id: u64) -> Option<PendingMeta> {
        for slot in &mut self.inline_in_flight {
            if let Some((id, _)) = slot
                && *id == request_id
            {
                return slot.take().map(|(_, m)| m);
            }
        }
        if self.overflow_in_flight.is_empty() {
            return None;
        }
        self.overflow_in_flight.remove(&request_id)
    }

    #[inline]
    fn in_flight_len(&self) -> usize {
        self.inline_in_flight
            .iter()
            .filter(|s| s.is_some())
            .count()
            + self.overflow_in_flight.len()
    }

    fn push_record_bounded(&mut self, record: QueuedRecord) {
        if self.records.len() >= self.max_queued_records {
            // Prefer dropping the oldest unsolicited Push/Channel record over an async Response
            // so pending callers still resolve whenever possible.
            if let Some(pos) = self
                .records
                .iter()
                .position(|r| r.kind != NveHostEventKind::Response)
            {
                let _ = self.records.remove(pos);
            } else {
                let _ = self.records.pop_front();
            }
            self.dropped_overflow_records = self.dropped_overflow_records.saturating_add(1);
        }
        self.records.push_back(record);
    }
}

/// Thread-safe completion sink collecting async [`HandlerReply::Pending`] responses, host push
/// events, and channel messages to drain into a canonical `NveHostEventBatch`.
#[derive(Clone)]
pub struct CompletionSink {
    inner: Arc<Mutex<CompletionSinkState>>,
}

impl Default for CompletionSink {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for CompletionSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        f.debug_struct("CompletionSink")
            .field("in_flight", &st.in_flight_len())
            .field("queued_records", &st.records.len())
            .field("dropped_oversized_records", &st.dropped_oversized_records)
            .field("dropped_overflow_records", &st.dropped_overflow_records)
            .finish()
    }
}

impl CompletionSink {
    pub fn new() -> Self {
        Self::with_max_queued_records(DEFAULT_COMPLETION_QUEUE_CAP)
    }

    pub fn with_max_queued_records(max_queued_records: usize) -> Self {
        let inner = Arc::new(Mutex::new(CompletionSinkState::new(max_queued_records)));
        // Eagerly prime OS mutex storage (e.g. Darwin lazy allocated `os_unfair_lock`)
        // during construction so even the first synchronous hot-path call allocates 0 bytes.
        drop(inner.lock());
        Self { inner }
    }

    pub fn set_on_ready(&self, callback: Option<ReadyWakeFn>) {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.on_ready = callback;
    }

    /// Registers an in-flight request before invoking a [`crate::HostServiceHandler`].
    /// Returns `false` without overwriting when `request_id` is already in flight.
    pub fn register_in_flight(&self, request_id: u64, service_id: u32, method_id: u32) -> bool {
        self.register_in_flight_for_owner(0, request_id, service_id, method_id)
            .is_some()
    }

    /// Registers an in-flight request tagged with `owner_tag` (e.g., per-window/per-generation
    /// `HotAppSlot` ID) so sibling windows' requests are isolated across close and hot reload.
    ///
    /// Returns `Some(handler_request_id)` to pass into [`crate::HostServiceHandler::handle`]
    /// (identical to `caller_request_id` when `owner_tag == 0`, or a collision-free host token
    /// when `owner_tag != 0` so two independent cdylib generations or windows can both issue
    /// `caller_request_id == 1` concurrently without colliding). Returns `None` on duplicate
    /// `(owner_tag, caller_request_id)`.
    pub fn register_in_flight_for_owner(
        &self,
        owner_tag: u64,
        caller_request_id: u64,
        service_id: u32,
        method_id: u32,
    ) -> Option<u64> {
        let host_id = if owner_tag == 0 {
            caller_request_id
        } else {
            next_host_request_id()
        };
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        st.insert_in_flight(
            host_id,
            PendingMeta {
                owner_tag,
                caller_request_id,
                service_id,
                method_id,
            },
        )
        .then_some(host_id)
    }

    /// Cleans up a pre-registered slot when the handler returned synchronously (`Ready` or `Err`).
    pub fn finish_sync(&self, host_request_id: u64) {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if st.remove_in_flight(host_request_id).is_none() {
            st.records.retain(|r| {
                !(r.kind == NveHostEventKind::Response && r.request_id == host_request_id)
            });
        }
    }

    /// Resolves an in-flight `request_id` with `(status, payload)` **at most once**.
    pub fn complete(&self, request_id: u64, status: NveHostStatus, payload: Vec<u8>) -> bool {
        let wake = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let Some(meta) = st.remove_in_flight(request_id) else {
                return false;
            };
            let (final_status, final_payload) = if payload.len() > NVE_HOST_BATCH_MAX_PAYLOAD {
                st.dropped_oversized_records = st.dropped_oversized_records.saturating_add(1);
                (NveHostStatus::InternalError, Vec::new())
            } else {
                (status, payload)
            };
            st.push_record_bounded(QueuedRecord {
                owner_tag: meta.owner_tag,
                kind: NveHostEventKind::Response,
                flags: 0,
                status: final_status,
                service_id: meta.service_id,
                method_or_event_or_channel: meta.method_id,
                request_id: meta.caller_request_id,
                payload: final_payload,
            });
            st.on_ready.clone()
        };
        if let Some(cb) = wake {
            cb();
        }
        true
    }

    #[inline]
    pub fn complete_ok(&self, request_id: u64, payload: Vec<u8>) -> bool {
        self.complete(request_id, NveHostStatus::Ok, payload)
    }

    #[inline]
    pub fn complete_err(&self, request_id: u64, err: &ServiceError) -> bool {
        self.complete(request_id, err.to_status(), Vec::new())
    }

    /// Enqueues an unsolicited host-to-app `Push` record into the next drained batch.
    pub fn push_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        let wake = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if payload.len() > NVE_HOST_BATCH_MAX_PAYLOAD {
                st.dropped_oversized_records = st.dropped_oversized_records.saturating_add(1);
                return;
            }
            st.push_record_bounded(QueuedRecord {
                owner_tag: 0,
                kind: NveHostEventKind::Push,
                flags: 0,
                status: NveHostStatus::Ok,
                service_id,
                method_or_event_or_channel: event_id,
                request_id: 0,
                payload,
            });
            st.on_ready.clone()
        };
        if let Some(cb) = wake {
            cb();
        }
    }

    /// Enqueues an inbound `Channel` record into the next drained batch.
    pub fn push_channel(&self, channel_id: u32, retain: bool, payload: Vec<u8>) {
        let flags = if retain {
            NVE_HOST_RECORD_FLAG_RETAIN
        } else {
            0
        };
        let wake = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if payload.len() > NVE_HOST_BATCH_MAX_PAYLOAD {
                st.dropped_oversized_records = st.dropped_oversized_records.saturating_add(1);
                return;
            }
            st.push_record_bounded(QueuedRecord {
                owner_tag: 0,
                kind: NveHostEventKind::Channel,
                flags,
                status: NveHostStatus::Ok,
                service_id: SERVICE_CHANNEL,
                method_or_event_or_channel: channel_id,
                request_id: 0,
                payload,
            });
            st.on_ready.clone()
        };
        if let Some(cb) = wake {
            cb();
        }
    }

    /// Drains all queued `Push` and `Channel` records as [`HostEvent`]s, leaving `Response`
    /// records intact in the sink.
    pub fn poll_host_events(&self) -> Vec<HostEvent> {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut events = Vec::new();
        let mut remaining = VecDeque::with_capacity(st.records.len());
        while let Some(rec) = st.records.pop_front() {
            match rec.kind {
                NveHostEventKind::Push => events.push(HostEvent::Push(HostPushEvent {
                    service_id: rec.service_id,
                    event_id: rec.method_or_event_or_channel,
                    payload: rec.payload,
                })),
                NveHostEventKind::Channel => events.push(HostEvent::Channel(HostChannelEvent {
                    channel_id: rec.method_or_event_or_channel,
                    retain: (rec.flags & NVE_HOST_RECORD_FLAG_RETAIN) != 0,
                    payload: rec.payload,
                })),
                NveHostEventKind::Response => remaining.push_back(rec),
            }
        }
        st.records = remaining;
        events
    }

    pub fn dropped_oversized_count(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dropped_oversized_records
    }

    pub fn dropped_overflow_count(&self) -> u64 {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dropped_overflow_records
    }

    /// Flushes every currently in-flight `Pending` request as a [`NveHostStatus::Cancelled`]
    /// response record so callers still alive never hang.
    pub fn cancel_all_pending(&self) -> usize {
        let (cancelled, wake) = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let mut count = 0usize;
            for idx in 0..INLINE_IN_FLIGHT_SLOTS {
                if let Some((_, meta)) = st.inline_in_flight[idx].take() {
                    st.push_record_bounded(QueuedRecord {
                        owner_tag: meta.owner_tag,
                        kind: NveHostEventKind::Response,
                        flags: 0,
                        status: NveHostStatus::Cancelled,
                        service_id: meta.service_id,
                        method_or_event_or_channel: meta.method_id,
                        request_id: meta.caller_request_id,
                        payload: Vec::new(),
                    });
                    count += 1;
                }
            }
            if !st.overflow_in_flight.is_empty() {
                let overflow = std::mem::take(&mut st.overflow_in_flight);
                for (_, meta) in overflow {
                    st.push_record_bounded(QueuedRecord {
                        owner_tag: meta.owner_tag,
                        kind: NveHostEventKind::Response,
                        flags: 0,
                        status: NveHostStatus::Cancelled,
                        service_id: meta.service_id,
                        method_or_event_or_channel: meta.method_id,
                        request_id: meta.caller_request_id,
                        payload: Vec::new(),
                    });
                    count += 1;
                }
            }
            (count, (count > 0).then(|| st.on_ready.clone()).flatten())
        };
        if let Some(cb) = wake {
            cb();
        }
        cancelled
    }

    /// Drops every currently in-flight `Pending` request and strips any queued `Response` records
    /// WITHOUT queuing `Cancelled` records (used across Tier-2 dylib reloads so generation N+1
    /// never receives stale generation N records).
    pub fn discard_all_pending(&self) -> usize {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut discarded = 0usize;
        for slot in &mut st.inline_in_flight {
            if slot.take().is_some() {
                discarded += 1;
            }
        }
        discarded += st.overflow_in_flight.len();
        st.overflow_in_flight.clear();
        st.records.retain(|r| r.kind != NveHostEventKind::Response);
        discarded
    }

    /// Drops ONLY in-flight requests and queued `Response` records belonging to `owner_tag`,
    /// leaving all sibling owners untouched.
    pub fn discard_pending_for_owner(&self, owner_tag: u64) -> usize {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let mut discarded = 0usize;
        for slot in &mut st.inline_in_flight {
            if let Some((_, meta)) = slot
                && meta.owner_tag == owner_tag
            {
                slot.take();
                discarded += 1;
            }
        }
        let before = st.overflow_in_flight.len();
        st.overflow_in_flight
            .retain(|_, meta| meta.owner_tag != owner_tag);
        discarded += before - st.overflow_in_flight.len();
        st.records
            .retain(|r| !(r.kind == NveHostEventKind::Response && r.owner_tag == owner_tag));
        discarded
    }

    /// Drains queued records matching `owner_tag` (plus unscoped `Push`/`Channel` records) into
    /// `builder`, leaving other owners' `Response` records intact.
    pub fn drain_batch_for_owner_into(
        &self,
        owner_tag: u64,
        builder: &mut NveHostEventBatchBuilder,
    ) -> Result<usize, NveHostBatchError> {
        let (written, remaining_wake) = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let mut written = 0usize;
            let mut deferred = VecDeque::new();
            let mut stopped_on_cap = false;
            while let Some(rec) = st.records.pop_front() {
                let belongs = rec.kind != NveHostEventKind::Response
                    || owner_tag == 0
                    || rec.owner_tag == owner_tag;
                if !belongs {
                    deferred.push_back(rec);
                    continue;
                }
                if stopped_on_cap {
                    deferred.push_back(rec);
                    continue;
                }
                let rec_bytes = host_batch_padded_record_len(rec.payload.len());
                if written > 0
                    && builder.as_bytes().len().saturating_add(rec_bytes)
                        > NVE_HOST_BATCH_MAX_BYTES
                {
                    deferred.push_back(rec);
                    stopped_on_cap = true;
                    continue;
                }
                match builder.push_record(
                    rec.kind,
                    rec.flags,
                    rec.status,
                    rec.service_id,
                    rec.method_or_event_or_channel,
                    rec.request_id,
                    &rec.payload,
                ) {
                    Ok(()) => written += 1,
                    Err(NveHostBatchError::PayloadTooLarge(_)) => {
                        st.dropped_oversized_records =
                            st.dropped_oversized_records.saturating_add(1);
                    }
                    Err(_) => {
                        deferred.push_back(rec);
                        stopped_on_cap = true;
                    }
                }
            }
            st.records = deferred;
            let wake = (stopped_on_cap && !st.records.is_empty())
                .then(|| st.on_ready.clone())
                .flatten();
            (written, wake)
        };
        if let Some(cb) = remaining_wake {
            cb();
        }
        Ok(written)
    }

    pub fn in_flight_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .in_flight_len()
    }

    pub fn queued_count(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .records
            .len()
    }

    pub fn take_completed_response(
        &self,
        request_id: u64,
    ) -> Option<Result<Vec<u8>, ServiceError>> {
        let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let pos = st
            .records
            .iter()
            .position(|r| r.kind == NveHostEventKind::Response && r.request_id == request_id)?;
        let rec = st.records.remove(pos)?;
        if rec.status.is_ok() {
            Some(Ok(rec.payload))
        } else {
            Some(Err(ServiceError::from_status(rec.status)))
        }
    }

    /// Drains queued records into `builder` up to `u16::MAX` records and [`NVE_HOST_BATCH_MAX_BYTES`]
    /// total bytes, re-firing `on_ready` outside the lock when records remain.
    pub fn drain_batch_into(
        &self,
        builder: &mut NveHostEventBatchBuilder,
    ) -> Result<usize, NveHostBatchError> {
        self.drain_batch_into_with_budget(builder, NVE_HOST_BATCH_MAX_BYTES)
    }

    /// Drains queued records into `builder` respecting both `u16::MAX` record count and `max_bytes`.
    pub fn drain_batch_into_with_budget(
        &self,
        builder: &mut NveHostEventBatchBuilder,
        max_bytes: usize,
    ) -> Result<usize, NveHostBatchError> {
        let (written, remaining_wake) = {
            let mut st = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            let mut written = 0usize;
            while let Some(front) = st.records.front() {
                let rec_bytes = host_batch_padded_record_len(front.payload.len());
                if written > 0 && builder.as_bytes().len().saturating_add(rec_bytes) > max_bytes {
                    break;
                }
                match builder.push_record(
                    front.kind,
                    front.flags,
                    front.status,
                    front.service_id,
                    front.method_or_event_or_channel,
                    front.request_id,
                    &front.payload,
                ) {
                    Ok(()) => {
                        let _ = st.records.pop_front();
                        written += 1;
                    }
                    Err(NveHostBatchError::PayloadTooLarge(_)) => {
                        let _ = st.records.pop_front();
                        st.dropped_oversized_records =
                            st.dropped_oversized_records.saturating_add(1);
                    }
                    Err(NveHostBatchError::CountOverflow) => break,
                    Err(_) => break,
                }
            }
            let wake = (!st.records.is_empty()).then(|| st.on_ready.clone()).flatten();
            (written, wake)
        };
        if let Some(cb) = remaining_wake {
            cb();
        }
        Ok(written)
    }

    /// Drains queued records into a canonical `NveHostEventBatch` byte buffer (`None` when empty).
    pub fn drain_batch(&self) -> Option<Vec<u8>> {
        if self.queued_count() == 0 {
            return None;
        }
        let mut builder = NveHostEventBatchBuilder::new();
        let _ = self.drain_batch_into(&mut builder);
        (!builder.is_empty()).then(|| builder.into_bytes())
    }
}
