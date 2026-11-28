//! Deterministic in-memory mock [`HostServiceProvider`] (`MemoryHostServices`) with fault
//! injection, direct typed async resolution, zero-serialization [`TypedEventBus`], and
//! `NveChannelBus` retained replay.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use nve_abi::NveHostStatus;
use nve_interop::NveChannelBus;

use crate::completion::CompletionSink;
use crate::error::ServiceError;
use crate::events::{TypedAsyncTable, TypedEventBus, TypedEventSubscription, TypedHostEvent};
use crate::provider::{
    next_host_request_id, HostEvent, HostServiceHandler, HostServiceProvider, InvokeResult,
    ResponseBuf,
};
use crate::registry::HostServiceRegistry;

#[derive(Debug, Clone)]
enum InjectedFault {
    Status(NveHostStatus),
    Pending,
}

#[derive(Default)]
struct MemoryFaultState {
    faults: BTreeMap<(u32, u32), VecDeque<InjectedFault>>,
    pending_async: BTreeMap<u64, Option<Result<Vec<u8>, ServiceError>>>,
    typed_async: TypedAsyncTable,
}

/// Deterministic in-memory mock [`HostServiceProvider`] composing any opt-in
/// [`HostServiceHandler`]s with fault injection, direct typed async resolution,
/// zero-serialization [`TypedEventBus`], and `NveChannelBus` retained replay.
pub struct MemoryHostServices {
    registry: HostServiceRegistry,
    fault_state: Mutex<MemoryFaultState>,
}

impl Default for MemoryHostServices {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryHostServices {
    pub fn new() -> Self {
        Self {
            registry: HostServiceRegistry::new(),
            fault_state: Mutex::new(MemoryFaultState::default()),
        }
    }

    pub fn with_channel_bus(channel_bus: Arc<Mutex<NveChannelBus>>) -> Self {
        Self {
            registry: HostServiceRegistry::with_channel_bus(channel_bus),
            fault_state: Mutex::new(MemoryFaultState::default()),
        }
    }

    pub fn registry(&self) -> &HostServiceRegistry {
        &self.registry
    }

    pub fn register<H: HostServiceHandler + 'static>(&self, handler: H) -> &Self {
        self.registry.register(handler);
        self
    }

    pub fn register_arc(&self, handler: Arc<dyn HostServiceHandler>) -> &Self {
        self.registry.register_arc(handler);
        self
    }

    pub fn with_handler<H: HostServiceHandler + 'static>(self, handler: H) -> Self {
        self.registry.register(handler);
        self
    }

    pub fn with_handler_arc(self, handler: Arc<dyn HostServiceHandler>) -> Self {
        self.registry.register_arc(handler);
        self
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        self.registry.channel_bus()
    }

    pub fn typed_events(&self) -> &Arc<TypedEventBus> {
        self.registry.typed_events()
    }

    pub fn emit_typed<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.registry.emit_typed(event)
    }

    pub fn emit_typed_retained<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.registry.emit_typed_retained(event)
    }

    pub fn subscribe_typed<E, F>(&self, replay_retained: bool, handler: F) -> TypedEventSubscription
    where
        E: Send + Sync + 'static,
        F: Fn(&E) + Send + Sync + 'static,
    {
        self.registry.subscribe_typed(replay_retained, handler)
    }

    pub fn drain_typed_events<E: Send + Sync + 'static>(&self) -> Vec<Arc<E>> {
        self.registry.drain_typed_events::<E>()
    }

    pub fn poll_typed_events(&self) -> Vec<TypedHostEvent> {
        self.registry.poll_typed_events()
    }

    /// Allocates and registers a pending typed async request with an optional callback.
    pub fn register_typed_async<R, F>(&self, on_complete: Option<F>) -> u64
    where
        R: Send + Sync + 'static,
        F: FnOnce(Result<Arc<R>, ServiceError>) + Send + 'static,
    {
        let req_id = next_host_request_id();
        let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
        match on_complete {
            Some(cb) => st.typed_async.register_callback::<R, F>(req_id, cb),
            None => st.typed_async.register_pending(req_id),
        }
        req_id
    }

    /// Resolves a typed async request directly with zero byte serialization.
    pub fn complete_typed_async<R: Send + Sync + 'static>(
        &self,
        request_id: u64,
        result: Result<R, ServiceError>,
    ) {
        let thunk = {
            let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
            st.typed_async.complete(request_id, result)
        };
        if let Some(run) = thunk {
            run();
        }
    }

    /// Consumes a completed typed async response if present.
    pub fn take_typed_async<R: Send + Sync + 'static>(
        &self,
        request_id: u64,
    ) -> Option<Result<Arc<R>, ServiceError>> {
        self.fault_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .typed_async
            .take::<R>(request_id)
    }

    /// Injects a one-shot [`NveHostStatus`] failure (e.g., `PermissionDenied`, `Cancelled`,
    /// `BufferTooSmall`, `Unsupported`) for the next invocation of `(service_id, method_id)`.
    pub fn inject_status(&self, service_id: u32, method_id: u32, status: NveHostStatus) {
        self.fault_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .faults
            .entry((service_id, method_id))
            .or_default()
            .push_back(InjectedFault::Status(status));
    }

    /// Injects a one-shot async `Pending` response for the next invocation of `(service_id, method_id)`.
    pub fn inject_async_pending(&self, service_id: u32, method_id: u32) {
        self.fault_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .faults
            .entry((service_id, method_id))
            .or_default()
            .push_back(InjectedFault::Pending);
    }

    /// Enqueues an unsolicited host push event (e.g. `UrlReceived`).
    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        self.registry.push_host_event(service_id, event_id, payload);
    }

    /// Completes an injected async pending request directly.
    pub fn complete_async(&self, request_id: u64, status: NveHostStatus, payload: Vec<u8>) {
        let outcome = if status.is_ok() {
            Ok(payload)
        } else {
            Err(ServiceError::from_status(status))
        };
        self.fault_state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .pending_async
            .insert(request_id, Some(outcome));
    }
}

impl HostServiceProvider for MemoryHostServices {
    fn invoke_raw_with_id(
        &self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        self.invoke_raw_with_id_for_owner(0, service_id, method_id, request_id, request)
    }

    fn invoke_raw_with_id_for_owner(
        &self,
        owner_tag: u64,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        let injected = {
            let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
            st.faults
                .get_mut(&(service_id, method_id))
                .and_then(VecDeque::pop_front)
        };
        if let Some(fault) = injected {
            return match fault {
                InjectedFault::Status(status) => {
                    if status.is_ok() {
                        Ok(InvokeResult::Ready(ResponseBuf::empty()))
                    } else {
                        Err(ServiceError::from_status(status))
                    }
                }
                InjectedFault::Pending => {
                    self.fault_state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pending_async
                        .insert(request_id, None);
                    Ok(InvokeResult::Pending { request_id })
                }
            };
        }
        self.registry
            .invoke_raw_with_id_for_owner(owner_tag, service_id, method_id, request_id, request)
    }

    fn typed_service(
        &self,
        service_id: u32,
        method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let has_fault = {
            let st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
            st.faults
                .get(&(service_id, method_id))
                .is_some_and(|q| !q.is_empty())
        };
        if has_fault {
            return None;
        }
        self.registry.typed_service(service_id, method_id, type_id)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        self.registry.channel_publish(channel_id, retain, payload)
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.registry.poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        self.registry.typed_event_bus()
    }

    fn host_completion_sink(&self) -> Option<&CompletionSink> {
        Some(self.registry.completion_sink())
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
        let ready = matches!(st.pending_async.get(&request_id), Some(Some(_)));
        if ready {
            st.pending_async.remove(&request_id).flatten()
        } else {
            drop(st);
            self.registry.take_async_response(request_id)
        }
    }

    fn cancel_pending(&self) -> usize {
        let mut cancelled = 0usize;
        {
            let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
            for slot in st.pending_async.values_mut() {
                if slot.is_none() {
                    *slot = Some(Err(ServiceError::Cancelled));
                    cancelled += 1;
                }
            }
        }
        cancelled + self.registry.cancel_pending()
    }

    fn discard_pending(&self) -> usize {
        let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
        let discarded = st.pending_async.len();
        st.pending_async.clear();
        drop(st);
        discarded + self.registry.discard_pending()
    }

    fn discard_pending_for_owner(&self, owner_tag: u64) -> usize {
        self.registry.discard_pending_for_owner(owner_tag)
    }
}
