//! Zero-overhead composable [`HostServiceRegistry`] dispatching `service_id` calls to opt-in
//! [`HostServiceHandler`]s.

use std::any::{Any, TypeId};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, RwLock};

use nve_interop::{ChannelPayload, NveChannelBus, PayloadEncoding};

use crate::completion::{CompletionSink, HandlerReply};
use crate::error::ServiceError;
use crate::events::{TypedEventBus, TypedEventSubscription, TypedHostEvent};
use crate::provider::{
    next_host_request_id, HostEvent, HostServiceHandler, HostServiceProvider, InvokeResult,
};

struct RegisteredService {
    handler: Arc<dyn HostServiceHandler>,
    typed: Option<(TypeId, Arc<dyn Any + Send + Sync>)>,
}

struct InFlightGuard<'a> {
    sink: &'a CompletionSink,
    request_id: u64,
    armed: bool,
}

impl Drop for InFlightGuard<'_> {
    fn drop(&mut self) {
        if self.armed {
            self.sink.finish_sync(self.request_id);
        }
    }
}

/// Dynamic per-`service_id` dispatcher composing any set of opt-in [`HostServiceHandler`]
/// modules with zero-serialization in-process typed dispatch, unified [`CompletionSink`] event &
/// async batching, a shared [`NveChannelBus`], and a zero-serialization [`TypedEventBus`].
pub struct HostServiceRegistry {
    handlers: RwLock<BTreeMap<u32, RegisteredService>>,
    channel_bus: Arc<Mutex<NveChannelBus>>,
    typed_events: Arc<TypedEventBus>,
    completion_sink: CompletionSink,
}

impl Default for HostServiceRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HostServiceRegistry {
    fn drop(&mut self) {
        let _ = self.completion_sink.cancel_all_pending();
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
            completion_sink: CompletionSink::new(),
        }
    }

    /// Returns the shared [`CompletionSink`] collecting async responses and outbound events.
    pub fn completion_sink(&self) -> &CompletionSink {
        &self.completion_sink
    }

    /// Cancels all in-flight [`HandlerReply::Pending`] requests, queuing `Cancelled` records.
    pub fn cancel_pending(&self) -> usize {
        self.completion_sink.cancel_all_pending()
    }

    /// Drains all queued async responses, push events, and channel messages into a canonical
    /// `NveHostEventBatch` byte buffer ready for `NveHostApi::dispatch_host_events`.
    pub fn drain_host_event_batch(&self) -> Option<Vec<u8>> {
        self.completion_sink.drain_batch()
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
        match event {
            HostEvent::Push(p) => {
                self.completion_sink
                    .push_event(p.service_id, p.event_id, p.payload);
            }
            HostEvent::Channel(c) => {
                self.completion_sink
                    .push_channel(c.channel_id, c.retain, c.payload);
            }
        }
    }

    pub fn push_host_event(&self, service_id: u32, event_id: u32, payload: Vec<u8>) {
        self.completion_sink
            .push_event(service_id, event_id, payload);
    }

    /// Dispatches `(service_id, method_id, request_id, request)` to the registered
    /// [`HostServiceHandler`], tracking `Pending` replies on [`Self::completion_sink`].
    ///
    /// Rejects duplicate in-flight `request_id`s with [`ServiceError::InvalidArg`] without
    /// invoking the handler, and cleans up the in-flight slot even if the handler panics.
    pub fn dispatch_with_id(
        &self,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<HandlerReply, ServiceError> {
        self.dispatch_with_id_for_owner(0, service_id, method_id, request_id, request)
    }

    /// Owner-scoped variant of [`Self::dispatch_with_id`] tagging in-flight `Pending` entries
    /// with `owner_tag` so sibling windows/generations can discard pending requests independently.
    pub fn dispatch_with_id_for_owner(
        &self,
        owner_tag: u64,
        service_id: u32,
        method_id: u32,
        request_id: u64,
        request: &[u8],
    ) -> Result<HandlerReply, ServiceError> {
        let handler = {
            let guard = self.handlers.read().unwrap_or_else(|e| e.into_inner());
            guard.get(&service_id).map(|e| Arc::clone(&e.handler))
        };
        let Some(h) = handler else {
            return Err(ServiceError::Unsupported);
        };
        let Some(host_req_id) = self.completion_sink.register_in_flight_for_owner(
            owner_tag,
            request_id,
            service_id,
            method_id,
        ) else {
            return Err(ServiceError::InvalidArg(format!(
                "duplicate in-flight request_id {request_id}"
            )));
        };
        let mut guard = InFlightGuard {
            sink: &self.completion_sink,
            request_id: host_req_id,
            armed: true,
        };
        let res = h.handle(method_id, host_req_id, request, &self.completion_sink);
        if matches!(res, Ok(HandlerReply::Pending)) {
            guard.armed = false;
        }
        res
    }

    /// Dispatches `(service_id, method_id, request)` with a freshly allocated `request_id`.
    pub fn dispatch(
        &self,
        service_id: u32,
        method_id: u32,
        request: &[u8],
    ) -> Result<HandlerReply, ServiceError> {
        self.dispatch_with_id(service_id, method_id, next_host_request_id(), request)
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
        match self.dispatch_with_id_for_owner(
            owner_tag,
            service_id,
            method_id,
            request_id,
            request,
        )? {
            HandlerReply::Ready(buf) => Ok(InvokeResult::Ready(buf)),
            HandlerReply::Pending => Ok(InvokeResult::Pending { request_id }),
        }
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
        self.completion_sink.poll_host_events()
    }

    fn typed_event_bus(&self) -> Option<&TypedEventBus> {
        Some(&self.typed_events)
    }

    fn host_completion_sink(&self) -> Option<&CompletionSink> {
        Some(&self.completion_sink)
    }

    fn take_async_response(&self, request_id: u64) -> Option<Result<Vec<u8>, ServiceError>> {
        self.completion_sink.take_completed_response(request_id)
    }

    fn cancel_pending(&self) -> usize {
        self.completion_sink.cancel_all_pending()
    }

    fn discard_pending(&self) -> usize {
        self.completion_sink.discard_all_pending()
    }

    fn discard_pending_for_owner(&self, owner_tag: u64) -> usize {
        self.completion_sink.discard_pending_for_owner(owner_tag)
    }
}
