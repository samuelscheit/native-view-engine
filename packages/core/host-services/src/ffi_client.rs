//! Binary C-ABI client (`FfiHostServices`) backing `NVE_IFACE_HOST = 5`.
//!
//! Uses a 4 KiB stack buffer fast-path on every `NveHostCallbacks::invoke` call, automatically
//! retries once with an exact-sized heap buffer on `NVE_HOST_BUFFER_TOO_SMALL`, and resolves
//! async requests and host events directly via [`RequestTable`] and [`TypedEventBus`].

use std::sync::{Arc, Mutex};

use nve_abi::{
    NveBufferSlice, NveHostBatchError, NveHostCallbacks, NveHostEventBatchRef, NveHostEventKind,
    NveHostStatus, NVE_CHANNEL_FLAG_RETAIN,
};
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::error::ServiceError;
use crate::events::TypedEventBus;
use crate::provider::{
    next_host_request_id, HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider,
    InvokeResult, ResponseBuf,
};
pub use crate::provider::FFI_STACK_BUF_SIZE;
use crate::request_table::AsyncCallback;
pub use crate::request_table::RequestTable;

/// Defensive cap (64 MiB) against a buggy or hostile host requesting unbounded retry allocation.
pub const FFI_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

/// Summary returned by [`FfiHostServices::ingest_batch`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BatchIngestStats {
    pub responses: usize,
    pub cancelled_responses: usize,
    pub pushes: usize,
    pub channels: usize,
}

impl BatchIngestStats {
    #[inline]
    pub const fn total(&self) -> usize {
        self.responses + self.pushes + self.channels
    }

    #[inline]
    pub const fn non_cancelled_responses(&self) -> usize {
        self.responses.saturating_sub(self.cancelled_responses)
    }
}

struct FfiClientState {
    callbacks: Option<NveHostCallbacks>,
    requests: RequestTable,
    channel_bus: Option<Arc<Mutex<NveChannelBus>>>,
}

/// C-ABI host capability client wrapping [`NveHostCallbacks`] with an optional in-process
/// [`TypedEventBus`] and `NveHostApi::dispatch_host_events` bridge.
pub struct FfiHostServices {
    typed_events: Arc<TypedEventBus>,
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
            typed_events: Arc::new(TypedEventBus::new()),
            state: Mutex::new(FfiClientState {
                callbacks: None,
                requests: RequestTable::new(),
                channel_bus: None,
            }),
        }
    }

    pub fn typed_events(&self) -> &Arc<TypedEventBus> {
        &self.typed_events
    }

    pub fn with_callbacks(callbacks: NveHostCallbacks) -> Self {
        let client = Self::new();
        client.bind(Some(callbacks));
        client
    }

    /// Binds or unbinds the host callback table supplied via `NveHostApi::bind_host`.
    ///
    /// Flushes any in-flight pending async requests as [`ServiceError::Cancelled`] whenever
    /// unbinding or replacing an active host binding (e.g., on Tier-2 hot reload or teardown).
    pub fn bind(&self, callbacks: Option<NveHostCallbacks>) {
        let cancelled = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let should_cancel = callbacks.is_none() || guard.callbacks.is_some();
            guard.callbacks = callbacks;
            if should_cancel {
                guard.requests.cancel_all_pending_callbacks().1
            } else {
                Vec::new()
            }
        };
        for cb in cancelled {
            cb.invoke_err(ServiceError::Cancelled);
        }
    }

    /// Cancels all in-flight pending requests (both callback-bearing and polled) as
    /// [`ServiceError::Cancelled`], returning the total count cancelled.
    pub fn cancel_all_pending(&self) -> usize {
        let (total, cancelled) = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.cancel_all_pending_callbacks()
        };
        for cb in cancelled {
            cb.invoke_err(ServiceError::Cancelled);
        }
        total
    }

    /// Validates and routes a canonical `NveHostEventBatch` (`NVHB` v1) byte slice over a single
    /// unified delivery path:
    /// - `Response` → [`RequestTable::resolve`] (invoking callbacks outside the lock)
    /// - `Push` → [`TypedEventBus`] (`HostPushEvent`)
    /// - `Channel` → attached [`NveChannelBus`] directly, or [`TypedEventBus`] (`HostChannelEvent`)
    ///   when no channel bus is attached
    pub fn ingest_batch(&self, batch: &[u8]) -> Result<BatchIngestStats, NveHostBatchError> {
        let parsed = NveHostEventBatchRef::parse(batch)?;
        let mut stats = BatchIngestStats::default();
        for rec in parsed.iter() {
            match rec.kind {
                NveHostEventKind::Response => {
                    self.resolve_async_response(
                        rec.request_id.raw(),
                        rec.status,
                        rec.payload.to_vec(),
                    );
                    stats.responses += 1;
                    if rec.status == NveHostStatus::Cancelled {
                        stats.cancelled_responses += 1;
                    }
                }
                NveHostEventKind::Push => {
                    self.push_host_event(
                        rec.service_id.raw(),
                        rec.method_or_event_or_channel,
                        rec.payload.to_vec(),
                    );
                    stats.pushes += 1;
                }
                NveHostEventKind::Channel => {
                    self.push_channel_message(
                        rec.method_or_event_or_channel,
                        rec.is_retained(),
                        rec.payload.to_vec(),
                    );
                    stats.channels += 1;
                }
            }
        }
        Ok(stats)
    }

    /// Returns `true` if a host callback table is currently bound.
    pub fn is_bound(&self) -> bool {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.callbacks.is_some()
    }

    /// Attaches an [`NveChannelBus`] that receives channel messages forwarded via
    /// [`Self::push_channel_message`].
    pub fn attach_channel_bus(&self, bus: Arc<Mutex<NveChannelBus>>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.channel_bus = Some(bus);
    }

    /// Number of currently unresolved `NVE_HOST_PENDING` requests in the request table.
    pub fn pending_request_count(&self) -> usize {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.pending_count()
    }

    /// Total number of slots currently occupied in [`RequestTable`] (pending + uncollected completed).
    pub fn total_request_slot_count(&self) -> usize {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.total_slot_count()
    }

    /// Invokes a host method and registers `on_complete` for async resolution.
    /// If the host completes synchronously, `on_complete` is invoked immediately.
    pub fn invoke_with_callback<F>(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
        on_complete: F,
    ) -> Result<Option<u64>, ServiceError>
    where
        F: FnOnce(Result<Vec<u8>, ServiceError>) + Send + 'static,
    {
        self.invoke_with_async_callback(
            service_id,
            method_id,
            request,
            AsyncCallback::Owned(Box::new(on_complete)),
            false,
        )
    }

    fn invoke_with_async_callback(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
        callback: AsyncCallback,
        notify_on_pre_err: bool,
    ) -> Result<Option<u64>, ServiceError> {
        // Fetch callbacks and register pending under ONE mutex lock so a concurrent `bind(None)`
        // cannot race between reading `host_ctx` and registering the pending slot.
        let (cb, invoke_fn, request_id) = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(cb) = guard.callbacks else {
                drop(guard);
                if notify_on_pre_err {
                    callback.invoke_err(ServiceError::Unsupported);
                }
                return Err(ServiceError::Unsupported);
            };
            let Some(invoke_fn) = cb.invoke else {
                drop(guard);
                if notify_on_pre_err {
                    callback.invoke_err(ServiceError::Unsupported);
                }
                return Err(ServiceError::Unsupported);
            };
            let request_id = next_host_request_id();
            if let Err(returned) = guard
                .requests
                .register_pending_callback(request_id, Some(callback))
            {
                drop(guard);
                let err =
                    ServiceError::InvalidArg(format!("duplicate pending request_id {request_id}"));
                if notify_on_pre_err && let Some(cb_fn) = returned {
                    cb_fn.invoke_err(err.clone());
                }
                return Err(err);
            }
            (cb, invoke_fn, request_id)
        };

        match self.exec_invoke_with_retry(
            cb.host_ctx,
            invoke_fn,
            service_id,
            method_id,
            request_id,
            request,
        ) {
            Ok(InvokeResult::Ready(bytes)) => {
                let cb_opt = {
                    let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.requests.remove_sync_slot(request_id)
                };
                if let Some(cb_fn) = cb_opt {
                    cb_fn.invoke_buf(Ok(bytes));
                }
                Ok(None)
            }
            Ok(InvokeResult::Pending { request_id }) => Ok(Some(request_id)),
            Err(err) => {
                let cb_opt = {
                    let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.requests.remove_sync_slot(request_id)
                };
                if let Some(cb_fn) = cb_opt {
                    cb_fn.invoke_err(err.clone());
                }
                Err(err)
            }
        }
    }

    /// Resolves a pending async request directly without intermediate batch encoding.
    pub fn resolve_async_response(
        &self,
        request_id: u64,
        status: NveHostStatus,
        payload: Vec<u8>,
    ) {
        let outcome = if status.is_ok() {
            Ok(payload)
        } else {
            Err(ServiceError::from_status(status))
        };
        let cb_pair = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.resolve_callback(request_id, outcome)
        };
        if let Some((cb, res)) = cb_pair {
            cb.invoke_vec(res);
        }
    }

    /// Publishes a host push event onto [`TypedEventBus`] (single unified delivery path).
    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        let _ = self.typed_events.publish(HostPushEvent {
            service_id,
            event_id,
            payload,
        });
    }

    /// Routes an inbound channel message directly to the attached [`NveChannelBus`] with zero
    /// extra clones, or publishes [`HostChannelEvent`] onto [`TypedEventBus`] when no channel bus
    /// is attached.
    pub fn push_channel_message(&self, channel_id: u32, retain: bool, payload: Vec<u8>) {
        let attached_bus = {
            let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.channel_bus.clone()
        };
        if let Some(bus_arc) = attached_bus {
            let mut bus = bus_arc.lock().unwrap_or_else(|e| e.into_inner());
            let _ = bus.publish_by_id(
                channel_id,
                None,
                PayloadEncoding::Binary,
                ChannelPayload::from_vec(payload),
                retain,
            );
        } else {
            let _ = self.typed_events.publish(HostChannelEvent {
                channel_id,
                retain,
                payload,
            });
        }
    }

    fn exec_invoke_with_retry(
        &self,
        host_ctx: *mut std::ffi::c_void,
        invoke_fn: nve_abi::NveHostInvokeFn,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        let req_slice = NveBufferSlice::from_slice(request);

        let mut stack_buf = [0u8; FFI_STACK_BUF_SIZE];
        let mut out_len: usize = 0;

        // SAFETY: `invoke_fn` and `host_ctx` were supplied via `bind_host`; `req_slice`, `stack_buf`,
        // and `out_len` remain valid for the synchronous FFI invocation.
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
                    return Err(ServiceError::InternalError(format!(
                        "host reported out_len {out_len} > stack cap {}",
                        stack_buf.len()
                    )));
                }
                Ok(InvokeResult::Ready(ResponseBuf::from_inline(
                    stack_buf, out_len,
                )))
            }
            NveHostStatus::Pending => Ok(InvokeResult::Pending { request_id }),
            NveHostStatus::BufferTooSmall => {
                let required = out_len;
                if required == 0 || required > FFI_MAX_RESPONSE_BYTES {
                    return Err(ServiceError::BufferTooSmall { required });
                }
                let mut heap_buf = vec![0u8; required];
                let mut retry_len: usize = 0;
                // SAFETY: `invoke_fn` and `host_ctx` were supplied via `bind_host`; `req_slice`,
                // `heap_buf`, and `retry_len` remain valid for the synchronous retry invocation.
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
                            return Err(ServiceError::InternalError(format!(
                                "host reported retry out_len {retry_len} > heap cap {}",
                                heap_buf.len()
                            )));
                        }
                        heap_buf.truncate(retry_len);
                        Ok(InvokeResult::Ready(ResponseBuf::Heap(heap_buf)))
                    }
                    NveHostStatus::Pending => Ok(InvokeResult::Pending { request_id }),
                    NveHostStatus::BufferTooSmall => Err(ServiceError::BufferTooSmall {
                        required: retry_len,
                    }),
                    other => Err(ServiceError::from_status(other)),
                }
            }
            other => Err(ServiceError::from_status(other)),
        }
    }
}

impl Drop for FfiHostServices {
    fn drop(&mut self) {
        let (_, cancelled) = self
            .state
            .get_mut()
            .unwrap_or_else(|e| e.into_inner())
            .requests
            .cancel_all_pending_callbacks();
        for cb in cancelled {
            cb.invoke_err(ServiceError::Cancelled);
        }
    }
}

impl HostServiceProvider for FfiHostServices {
    fn invoke_raw_with_id(
        &self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        let (cb, invoke_fn) = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let cb = guard.callbacks.ok_or(ServiceError::Unsupported)?;
            let invoke_fn = cb.invoke.ok_or(ServiceError::Unsupported)?;
            if guard
                .requests
                .register_pending_callback(request_id, None)
                .is_err()
            {
                return Err(ServiceError::InvalidArg(format!(
                    "duplicate pending request_id {request_id}"
                )));
            }
            (cb, invoke_fn)
        };
        let res = self.exec_invoke_with_retry(
            cb.host_ctx,
            invoke_fn,
            service_id,
            method_id,
            request_id,
            request,
        );
        if !matches!(res, Ok(InvokeResult::Pending { .. })) {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            let _ = guard.requests.remove_sync_slot(request_id);
        }
        res
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        let cb = {
            let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.callbacks.ok_or(ServiceError::Unsupported)?
        };
        let Some(publish_fn) = cb.channel_publish else {
            return Err(ServiceError::Unsupported);
        };
        let flags = if retain { NVE_CHANNEL_FLAG_RETAIN } else { 0 };
        // SAFETY: `publish_fn` and `cb.host_ctx` were supplied via `bind_host`; `payload` stays valid for the call.
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
            Err(ServiceError::from_status(parsed))
        }
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.typed_events
            .drain_matching(|ev| ev.is::<HostPushEvent>() || ev.is::<HostChannelEvent>())
            .into_iter()
            .filter_map(|ev| {
                if let Some(push) = ev.downcast_arc::<HostPushEvent>() {
                    Some(HostEvent::Push((*push).clone()))
                } else {
                    ev.downcast_arc::<HostChannelEvent>()
                        .map(|ch| HostEvent::Channel((*ch).clone()))
                }
            })
            .collect()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        Some(&self.typed_events)
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.take_completed(request_id)
    }

    fn cancel_pending(&self) -> usize {
        self.cancel_all_pending()
    }
}

impl nve_codec::ByteTransport for FfiHostServices {
    fn call_sync(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_reply: &mut dyn FnMut(&[u8]),
    ) -> Result<(), nve_codec::TransportError> {
        match self.invoke_sync(service_id, method_id, req) {
            Ok(resp) => {
                on_reply(resp.as_slice());
                Ok(())
            }
            Err(ServiceError::Pending { request_id }) => {
                let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                let _ = guard.requests.remove_sync_slot(request_id);
                Err(ServiceError::Pending { request_id }.into())
            }
            Err(err) => Err(err.into()),
        }
    }

    fn call_async(
        &self,
        service_id: u32,
        method_id: u32,
        req: &[u8],
        on_done: nve_codec::TransportDoneFn,
    ) {
        let _ = self.invoke_with_async_callback(
            service_id,
            method_id,
            req,
            AsyncCallback::Transport(on_done),
            true,
        );
    }
}
