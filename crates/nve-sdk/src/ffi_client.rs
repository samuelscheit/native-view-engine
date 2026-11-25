//! Zero-JSON C-ABI client (`FfiHostServices`) backing `NVE_IFACE_HOST = 5`.
//!
//! Uses a 4 KiB stack buffer fast-path on every `NveHostCallbacks::invoke` call, automatically
//! retries once with an exact-sized heap buffer on `NVE_HOST_BUFFER_TOO_SMALL`, and resolves
//! `NVE_HOST_PENDING` requests via an async `RequestTable` fed by `NVH1` batches.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostCallbacks, NveHostEventBatch, NveHostEventRecordRef, NveHostStatus,
    NVE_CHANNEL_FLAG_RETAIN,
};
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::error::SdkError;
use crate::provider::{HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult};

/// 4 KiB stack-allocated response buffer avoiding heap allocation on the fast path.
pub const FFI_STACK_BUF_SIZE: usize = 4096;

/// Defensive cap (64 MiB) against a buggy or hostile host requesting unbounded retry allocation.
pub const FFI_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

type AsyncCallback = Box<dyn FnOnce(Result<Vec<u8>, SdkError>) + Send + 'static>;

enum RequestSlot {
    Pending { callback: Option<AsyncCallback> },
    Completed(Result<Vec<u8>, SdkError>),
}

/// Tracks in-flight and resolved async `NVE_HOST_PENDING` requests.
#[derive(Default)]
pub struct RequestTable {
    slots: BTreeMap<u64, RequestSlot>,
}

impl RequestTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_pending(&mut self, request_id: u64, callback: Option<AsyncCallback>) {
        self.slots
            .insert(request_id, RequestSlot::Pending { callback });
    }

    /// Resolves `request_id` from an inbound `NVH1` `AsyncResponse`.
    /// Returns any registered callback so the caller can invoke it outside the mutex lock.
    pub fn resolve(
        &mut self,
        request_id: u64,
        result: Result<Vec<u8>, SdkError>,
    ) -> Option<(AsyncCallback, Result<Vec<u8>, SdkError>)> {
        match self.slots.remove(&request_id) {
            Some(RequestSlot::Pending { callback: Some(cb) }) => Some((cb, result)),
            Some(RequestSlot::Pending { callback: None }) | None => {
                self.slots
                    .insert(request_id, RequestSlot::Completed(result));
                None
            }
            Some(RequestSlot::Completed(_)) => {
                self.slots
                    .insert(request_id, RequestSlot::Completed(result));
                None
            }
        }
    }

    pub fn take_completed(&mut self, request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        if matches!(self.slots.get(&request_id), Some(RequestSlot::Completed(_)))
            && let Some(RequestSlot::Completed(res)) = self.slots.remove(&request_id)
        {
            return Some(res);
        }
        None
    }

    pub fn pending_count(&self) -> usize {
        self.slots
            .values()
            .filter(|s| matches!(s, RequestSlot::Pending { .. }))
            .count()
    }
}

struct FfiClientState {
    callbacks: Option<NveHostCallbacks>,
    requests: RequestTable,
    events: VecDeque<HostEvent>,
    channel_bus: Option<Arc<Mutex<NveChannelBus>>>,
}

/// C-ABI host capability client wrapping [`NveHostCallbacks`] and decoding `NVH1` batches.
pub struct FfiHostServices {
    next_request_id: AtomicU64,
    state: Mutex<FfiClientState>,
}

impl Default for FfiHostServices {
    fn default() -> Self {
        Self::new()
    }
}

impl FfiHostServices {
    pub fn new() -> Self {
        Self {
            next_request_id: AtomicU64::new(1),
            state: Mutex::new(FfiClientState {
                callbacks: None,
                requests: RequestTable::new(),
                events: VecDeque::new(),
                channel_bus: None,
            }),
        }
    }

    pub fn with_callbacks(callbacks: NveHostCallbacks) -> Self {
        let client = Self::new();
        client.bind(Some(callbacks));
        client
    }

    /// Binds or unbinds the host callback table supplied via `NveHostApi::bind_host`.
    pub fn bind(&self, callbacks: Option<NveHostCallbacks>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.callbacks = callbacks;
    }

    /// Returns `true` if a host callback table is currently bound.
    pub fn is_bound(&self) -> bool {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.callbacks.is_some()
    }

    /// Attaches an [`NveChannelBus`] that automatically receives inbound `NVH1` `ChannelMessage`
    /// records during [`Self::dispatch_host_events`].
    pub fn attach_channel_bus(&self, bus: Arc<Mutex<NveChannelBus>>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.channel_bus = Some(bus);
    }

    /// Number of currently unresolved `NVE_HOST_PENDING` requests in the request table.
    pub fn pending_request_count(&self) -> usize {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.pending_count()
    }

    /// Invokes a host method and registers `on_complete` for async resolution.
    /// If the host completes synchronously, `on_complete` is invoked immediately.
    pub fn invoke_with_callback<F>(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
        on_complete: F,
    ) -> Result<Option<u64>, SdkError>
    where
        F: FnOnce(Result<Vec<u8>, SdkError>) + Send + 'static,
    {
        let (cb, request_id) = self.prepare_invoke()?;
        let Some(invoke_fn) = cb.invoke else {
            return Err(SdkError::Unsupported);
        };

        // Pre-register callback before invoking so even re-entrant completion cannot race.
        match self.exec_invoke_with_retry(cb.host_ctx, invoke_fn, service_id, method_id, request_id, request) {
            Ok(InvokeResult::Ready(bytes)) => {
                on_complete(Ok(bytes));
                Ok(None)
            }
            Ok(InvokeResult::Pending { request_id }) => {
                let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                guard
                    .requests
                    .register_pending(request_id, Some(Box::new(on_complete)));
                Ok(Some(request_id))
            }
            Err(err) => {
                on_complete(Err(err.clone()));
                Err(err)
            }
        }
    }

    /// Decodes an inbound binary `NVH1` batch (from `NveHostApi::dispatch_host_events`),
    /// resolves pending async requests, queues push/channel events, and forwards channel
    /// messages to any attached [`NveChannelBus`].
    ///
    /// Returns the total number of decoded records.
    pub fn dispatch_host_events(&self, batch_bytes: &[u8]) -> Result<usize, SdkError> {
        let records = NveHostEventBatch::decode_refs(batch_bytes)?;
        let count = records.len();
        let mut callbacks_to_run: Vec<(AsyncCallback, Result<Vec<u8>, SdkError>)> = Vec::new();
        let mut channel_forwards: Vec<(u32, bool, Vec<u8>)> = Vec::new();
        let attached_bus: Option<Arc<Mutex<NveChannelBus>>>;

        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            attached_bus = guard.channel_bus.clone();
            for rec in records {
                match rec {
                    NveHostEventRecordRef::AsyncResponse {
                        request_id,
                        status,
                        payload,
                    } => {
                        let outcome = if status.is_ok() {
                            Ok(payload.to_vec())
                        } else {
                            Err(SdkError::from_status(status))
                        };
                        if let Some(cb_pair) = guard.requests.resolve(request_id, outcome) {
                            callbacks_to_run.push(cb_pair);
                        }
                    }
                    NveHostEventRecordRef::HostPushEvent {
                        service_id,
                        event_id,
                        payload,
                    } => {
                        guard.events.push_back(HostEvent::Push(HostPushEvent {
                            service_id,
                            event_id,
                            payload: payload.to_vec(),
                        }));
                    }
                    NveHostEventRecordRef::ChannelMessage {
                        channel_id,
                        flags,
                        payload,
                    } => {
                        let retain = (flags & NVE_CHANNEL_FLAG_RETAIN) != 0;
                        let owned = payload.to_vec();
                        if attached_bus.is_some() {
                            channel_forwards.push((channel_id, retain, owned.clone()));
                        }
                        guard.events.push_back(HostEvent::Channel(HostChannelEvent {
                            channel_id,
                            retain,
                            payload: owned,
                        }));
                    }
                }
            }
        }

        // Invoke completion callbacks and NveChannelBus fan-out outside `self.state` lock to avoid deadlock.
        if let Some(bus_arc) = attached_bus
            && !channel_forwards.is_empty()
        {
            let mut bus = bus_arc.lock().unwrap_or_else(|e| e.into_inner());
            for (channel_id, retain, payload) in channel_forwards {
                let _ = bus.publish_by_id(
                    channel_id,
                    None,
                    PayloadEncoding::Binary,
                    ChannelPayload::from_vec(payload),
                    retain,
                );
            }
        }

        for (cb, res) in callbacks_to_run {
            cb(res);
        }

        Ok(count)
    }

    fn prepare_invoke(&self) -> Result<(NveHostCallbacks, u64), SdkError> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cb = guard.callbacks.ok_or(SdkError::Unsupported)?;
        let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
        Ok((cb, req_id))
    }

    fn exec_invoke_with_retry(
        &self,
        host_ctx: *mut std::ffi::c_void,
        invoke_fn: nve_abi::NveHostInvokeFn,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        let req_slice = NveBufferSlice::from_slice(request);

        // Fast path: 4 KiB stack buffer (zero heap allocation until copying the exact result bytes).
        let mut stack_buf = [0u8; FFI_STACK_BUF_SIZE];
        let mut out_len: usize = 0;

        let raw_status = unsafe {
            invoke_fn(
                host_ctx,
                service_id,
                method_id,
                request_id,
                req_slice,
                stack_buf.as_mut_ptr(),
                stack_buf.len(),
                &mut out_len,
            )
        };

        match NveHostStatus::from_i32(raw_status) {
            NveHostStatus::Ok => {
                if out_len > stack_buf.len() {
                    return Err(SdkError::InternalError(format!(
                        "host reported out_len {out_len} > stack cap {}",
                        stack_buf.len()
                    )));
                }
                Ok(InvokeResult::Ready(stack_buf[..out_len].to_vec()))
            }
            NveHostStatus::Pending => Ok(InvokeResult::Pending { request_id }),
            NveHostStatus::BufferTooSmall => {
                let required = out_len;
                if required == 0 || required > FFI_MAX_RESPONSE_BYTES {
                    return Err(SdkError::BufferTooSmall { required });
                }
                let mut heap_buf = vec![0u8; required];
                let mut retry_len: usize = 0;
                let retry_status = unsafe {
                    invoke_fn(
                        host_ctx,
                        service_id,
                        method_id,
                        request_id,
                        req_slice,
                        heap_buf.as_mut_ptr(),
                        heap_buf.len(),
                        &mut retry_len,
                    )
                };
                match NveHostStatus::from_i32(retry_status) {
                    NveHostStatus::Ok => {
                        if retry_len > heap_buf.len() {
                            return Err(SdkError::InternalError(format!(
                                "host reported retry out_len {retry_len} > heap cap {}",
                                heap_buf.len()
                            )));
                        }
                        heap_buf.truncate(retry_len);
                        Ok(InvokeResult::Ready(heap_buf))
                    }
                    NveHostStatus::Pending => Ok(InvokeResult::Pending { request_id }),
                    NveHostStatus::BufferTooSmall => Err(SdkError::BufferTooSmall {
                        required: retry_len,
                    }),
                    other => Err(SdkError::from_status(other)),
                }
            }
            other => Err(SdkError::from_status(other)),
        }
    }
}

impl HostServiceProvider for FfiHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, SdkError> {
        let (cb, request_id) = self.prepare_invoke()?;
        let Some(invoke_fn) = cb.invoke else {
            return Err(SdkError::Unsupported);
        };
        let res = self.exec_invoke_with_retry(
            cb.host_ctx,
            invoke_fn,
            service_id,
            method_id,
            request_id,
            request,
        )?;
        if let InvokeResult::Pending { request_id } = res {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.register_pending(request_id, None);
        }
        Ok(res)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), SdkError> {
        let cb = {
            let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.callbacks.ok_or(SdkError::Unsupported)?
        };
        let Some(publish_fn) = cb.channel_publish else {
            return Err(SdkError::Unsupported);
        };
        let flags = if retain { NVE_CHANNEL_FLAG_RETAIN } else { 0 };
        let status = unsafe {
            publish_fn(
                cb.host_ctx,
                channel_id,
                flags,
                NveBufferSlice::from_slice(payload),
            )
        };
        let parsed = NveHostStatus::from_i32(status);
        if parsed.is_ok() {
            Ok(())
        } else {
            Err(SdkError::from_status(parsed))
        }
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.events.drain(..).collect()
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, SdkError>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.take_completed(request_id)
    }
}
