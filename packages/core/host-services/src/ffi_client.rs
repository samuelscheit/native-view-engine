//! Binary C-ABI client (`FfiHostServices`) backing `NVE_IFACE_HOST = 5`.
//!
//! Uses a 4 KiB stack buffer fast-path on every `NveHostCallbacks::invoke` call, automatically
//! retries once with an exact-sized heap buffer on `NVE_HOST_BUFFER_TOO_SMALL`, and resolves
//! async requests and host events directly via [`RequestTable`] and [`TypedEventBus`].

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use nve_abi::{NveBufferSlice, NveHostCallbacks, NveHostStatus, NVE_CHANNEL_FLAG_RETAIN};
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::error::ServiceError;
use crate::events::TypedEventBus;
use crate::provider::{
    HostChannelEvent, HostEvent, HostPushEvent, HostServiceProvider, InvokeResult, ResponseBuf,
};
pub use crate::provider::FFI_STACK_BUF_SIZE;

/// Defensive cap (64 MiB) against a buggy or hostile host requesting unbounded retry allocation.
pub const FFI_MAX_RESPONSE_BYTES: usize = 64 * 1024 * 1024;

type AsyncCallback = Box<dyn FnOnce(Result<Vec<u8>, ServiceError>) + Send + 'static>;

enum RequestSlot {
    Pending { callback: Option<AsyncCallback> },
    Completed(Result<Vec<u8>, ServiceError>),
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
        if matches!(self.slots.get(&request_id), Some(RequestSlot::Completed(_))) {
            return;
        }
        self.slots
            .insert(request_id, RequestSlot::Pending { callback });
    }

    /// Removes a pre-registered pending slot after synchronous completion or error, returning any
    /// uninvoked callback so the caller can fire it outside the lock.
    pub fn cancel_pending(&mut self, request_id: u64) -> Option<AsyncCallback> {
        match self.slots.remove(&request_id) {
            Some(RequestSlot::Pending { callback }) => callback,
            Some(RequestSlot::Completed(_)) | None => None,
        }
    }

    /// Resolves `request_id` from a completed async host response.
    /// Returns any registered callback so the caller can invoke it outside the mutex lock.
    pub fn resolve(
        &mut self,
        request_id: u64,
        result: Result<Vec<u8>, ServiceError>,
    ) -> Option<(AsyncCallback, Result<Vec<u8>, ServiceError>)> {
        match self.slots.remove(&request_id) {
            Some(RequestSlot::Pending { callback: Some(cb) }) => Some((cb, result)),
            Some(RequestSlot::Pending { callback: None })
            | Some(RequestSlot::Completed(_))
            | None => {
                self.slots
                    .insert(request_id, RequestSlot::Completed(result));
                None
            }
        }
    }

    pub fn take_completed(&mut self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
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

/// C-ABI host capability client wrapping [`NveHostCallbacks`] with an optional in-process
/// [`TypedEventBus`] and `NveHostApi::dispatch_host_events` bridge.
pub struct FfiHostServices {
    next_request_id: AtomicU64,
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
            next_request_id: AtomicU64::new(1),
            typed_events: Arc::new(TypedEventBus::new()),
            state: Mutex::new(FfiClientState {
                callbacks: None,
                requests: RequestTable::new(),
                events: VecDeque::new(),
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
    pub fn bind(&self, callbacks: Option<NveHostCallbacks>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.callbacks = callbacks;
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
        let (cb, request_id) = self.prepare_invoke()?;
        let Some(invoke_fn) = cb.invoke else {
            return Err(ServiceError::Unsupported);
        };

        // Register pending BEFORE calling across FFI so a fast cross-thread or re-entrant
        // `resolve_async_response` can never race ahead and be overwritten.
        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard
                .requests
                .register_pending(request_id, Some(Box::new(on_complete)));
        }

        match self.exec_invoke_with_retry(
            cb.host_ctx,
            invoke_fn,
            service_id,
            method_id,
            request_id,
            request,
        ) {
            Ok(InvokeResult::Ready(bytes)) => {
                let callback = {
                    let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.requests.cancel_pending(request_id)
                };
                if let Some(cb) = callback {
                    cb(Ok(bytes.into_vec()));
                }
                Ok(None)
            }
            Ok(InvokeResult::Pending { request_id }) => Ok(Some(request_id)),
            Err(err) => {
                let callback = {
                    let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    guard.requests.cancel_pending(request_id)
                };
                if let Some(cb) = callback {
                    cb(Err(err.clone()));
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
            guard.requests.resolve(request_id, outcome)
        };
        if let Some((cb, res)) = cb_pair {
            cb(res);
        }
    }

    /// Enqueues a host push event directly without intermediate batch encoding.
    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.events.push_back(HostEvent::Push(HostPushEvent {
            service_id,
            event_id,
            payload,
        }));
    }

    /// Enqueues a channel message and forwards it to any attached [`NveChannelBus`].
    pub fn push_channel_message(&self, channel_id: u32, retain: bool, payload: Vec<u8>) {
        let attached_bus = {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.events.push_back(HostEvent::Channel(HostChannelEvent {
                channel_id,
                retain,
                payload: payload.clone(),
            }));
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
        }
    }

    fn prepare_invoke(&self) -> Result<(NveHostCallbacks, u64), ServiceError> {
        let guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let cb = guard.callbacks.ok_or(ServiceError::Unsupported)?;
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
    ) -> Result<InvokeResult, ServiceError> {
        let req_slice = NveBufferSlice::from_slice(request);

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

impl HostServiceProvider for FfiHostServices {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        let (cb, request_id) = self.prepare_invoke()?;
        let Some(invoke_fn) = cb.invoke else {
            return Err(ServiceError::Unsupported);
        };
        {
            let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
            guard.requests.register_pending(request_id, None);
        }
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
            let _ = guard.requests.cancel_pending(request_id);
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
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.events.drain(..).collect()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        Some(&self.typed_events)
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        let mut guard = self.state.lock().unwrap_or_else(|e| e.into_inner());
        guard.requests.take_completed(request_id)
    }
}
