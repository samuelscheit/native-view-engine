//! Zero-overhead composable [`HostServiceRegistry`] and [`MemoryHostServices`] dispatching
//! `service_id` calls to opt-in [`HostServiceHandler`]s.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};

use nve_abi::NveHostStatus;
use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::error::ServiceError;
use crate::events::{TypedAsyncTable, TypedEventBus, TypedEventSubscription, TypedHostEvent};
use crate::provider::{
    HostEvent, HostPushEvent, HostServiceHandler, HostServiceProvider, InvokeResult,
};

struct RegisteredService {
    handler: Arc<dyn HostServiceHandler>,
    typed: Option<(TypeId, Arc<dyn Any + Send + Sync>)>,
}

/// Dynamic per-`service_id` dispatcher composing any set of opt-in [`HostServiceHandler`]
/// modules with zero-serialization in-process typed dispatch, a shared [`NveChannelBus`],
/// a zero-serialization [`TypedEventBus`], and an inbound [`HostEvent`] queue.
pub struct HostServiceRegistry {
    handlers: RwLock<BTreeMap<u32, RegisteredService>>,
    channel_bus: Arc<Mutex<NveChannelBus>>,
    typed_events: Arc<TypedEventBus>,
    events: Mutex<VecDeque<HostEvent>>,
}

impl Default for HostServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HostServiceRegistry {
    pub fn new() -> Self {
        Self::with_channel_bus(Arc::new(Mutex::new(NveChannelBus::new())))
    }

    pub fn with_channel_bus(channel_bus: Arc<Mutex<NveChannelBus>>) -> Self {
        Self {
            handlers: RwLock::new(BTreeMap::new()),
            channel_bus,
            typed_events: Arc::new(TypedEventBus::new()),
            events: Mutex::new(VecDeque::new()),
        }
    }

    /// Registers a capability handler under its [`HostServiceHandler::service_id`].
    pub fn register<H: HostServiceHandler + 'static>(&self, handler: H) -> &Self {
        self.register_arc(Arc::new(handler))
    }

    /// Registers an already `Arc`-wrapped capability handler (and caches its typed in-process
    /// backend once so in-process calls bypass binary wire serialization completely).
    pub fn register_arc(&self, handler: Arc<dyn HostServiceHandler>) -> &Self {
        let id = handler.service_id();
        let typed = Arc::clone(&handler).typed_backend();
        self.handlers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .insert(id, RegisteredService { handler, typed });
        self
    }

    /// Builder-style variant of [`Self::register`].
    pub fn with_handler<H: HostServiceHandler + 'static>(self, handler: H) -> Self {
        self.register(handler);
        self
    }

    /// Builder-style variant of [`Self::register_arc`].
    pub fn with_handler_arc(self, handler: Arc<dyn HostServiceHandler>) -> Self {
        self.register_arc(handler);
        self
    }

    /// Removes the handler registered for `service_id`, returning `true` if one existed.
    pub fn unregister(&self, service_id: u32) -> bool {
        self.handlers
            .write()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&service_id)
            .is_some()
    }

    /// Returns `true` if a handler is registered for `service_id`.
    pub fn has_service(&self, service_id: u32) -> bool {
        self.handlers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(&service_id)
    }

    /// Returns the sorted list of registered service IDs.
    pub fn registered_services(&self) -> Vec<u32> {
        self.handlers
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .copied()
            .collect()
    }

    pub fn channel_bus(&self) -> Arc<Mutex<NveChannelBus>> {
        Arc::clone(&self.channel_bus)
    }

    pub fn typed_events(&self) -> &Arc<TypedEventBus> {
        &self.typed_events
    }

    pub fn emit_typed<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.typed_events.publish(event)
    }

    pub fn emit_typed_retained<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.typed_events.publish_retained(event)
    }

    pub fn subscribe_typed<E, F>(&self, replay_retained: bool, handler: F) -> TypedEventSubscription
    where
        E: Send + Sync + 'static,
        F: Fn(&E) + Send + Sync + 'static,
    {
        self.typed_events.subscribe(replay_retained, handler)
    }

    pub fn drain_typed_events<E: Send + Sync + 'static>(&self) -> Vec<Arc<E>> {
        self.typed_events.drain_typed::<E>()
    }

    pub fn poll_typed_events(&self) -> Vec<TypedHostEvent> {
        self.typed_events.poll_all()
    }

    pub fn push_event(&self, event: HostEvent) {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push_back(event);
    }

    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        self.push_event(HostEvent::Push(HostPushEvent {
            service_id,
            event_id,
            payload,
        }));
    }

    /// Dispatches `(service_id, method_id, request)` to the registered [`HostServiceHandler`].
    pub fn dispatch(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<Vec<u8>, ServiceError> {
        let handler = {
            let guard = self.handlers.read().unwrap_or_else(|e| e.into_inner());
            guard.get(&service_id).map(|e| Arc::clone(&e.handler))
        };
        match handler {
            Some(h) => h.handle(method_id, request),
            None => Err(ServiceError::Unsupported),
        }
    }

    /// Zero-serialization lookup for a typed backend registered under `service_id`.
    pub fn typed_backend_for(
        &self,
        service_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        let guard = self.handlers.read().unwrap_or_else(|e| e.into_inner());
        let entry = guard.get(&service_id)?;
        let (stored_tid, backend) = entry.typed.as_ref()?;
        if *stored_tid == type_id {
            Some(Arc::clone(backend))
        } else {
            None
        }
    }
}

impl HostServiceProvider for HostServiceRegistry {
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<InvokeResult, ServiceError> {
        self.dispatch(service_id, method_id, request)
            .map(|bytes| InvokeResult::Ready(bytes.into()))
    }

    fn typed_service(
        &self,
        service_id: u32,
        _method_id: u32,
        type_id: TypeId,
    ) -> Option<Arc<dyn Any + Send + Sync>> {
        self.typed_backend_for(service_id, type_id)
    }

    fn channel_publish(
        &self,
        channel_id: u32,
        retain: bool,
        payload: &[u8],
    ) -> Result<(), ServiceError> {
        let mut bus = self.channel_bus.lock().unwrap_or_else(|e| e.into_inner());
        bus.publish_by_id(
            channel_id,
            None,
            PayloadEncoding::Binary,
            ChannelPayload::from_slice(payload),
            retain,
        )
        .map(|_| ())
        .map_err(|e| ServiceError::InternalError(e.to_string()))
    }

    fn poll_host_events(&self) -> Vec<HostEvent> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .drain(..)
            .collect()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        Some(&self.typed_events)
    }
}

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
    next_request_id: AtomicU64,
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
            next_request_id: AtomicU64::new(1),
            fault_state: Mutex::new(MemoryFaultState::default()),
        }
    }

    pub fn with_channel_bus(channel_bus: Arc<Mutex<NveChannelBus>>) -> Self {
        Self {
            registry: HostServiceRegistry::with_channel_bus(channel_bus),
            next_request_id: AtomicU64::new(1),
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
        let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
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
    fn invoke_raw(
        &self,
        service_id: u32,
        method_id: u32,
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
                        Ok(InvokeResult::Ready(crate::ResponseBuf::empty()))
                    } else {
                        Err(ServiceError::from_status(status))
                    }
                }
                InjectedFault::Pending => {
                    let req_id = self.next_request_id.fetch_add(1, Ordering::Relaxed);
                    self.fault_state
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .pending_async
                        .insert(req_id, None);
                    Ok(InvokeResult::Pending { request_id: req_id })
                }
            };
        }
        self.registry.invoke_raw(service_id, method_id, request)
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

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        let mut st = self.fault_state.lock().unwrap_or_else(|e| e.into_inner());
        let ready = matches!(st.pending_async.get(&request_id), Some(Some(_)));
        if ready {
            st.pending_async.remove(&request_id).flatten()
        } else {
            None
        }
    }
}
