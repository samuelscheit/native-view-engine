//! Direct zero-serialization in-process typed event bus ([`TypedEventBus`]) and typed async
//! request completion table ([`TypedAsyncTable`]) for `nve-host-services`.
//!
//! Replaces in-process binary byte batch encoding/decoding with direct `TypeId`-indexed
//! `Arc<E>` fan-out, retained snapshots, selective typed queue draining, and typed async
//! resolution with zero `Vec<u8>` serialization.

use std::any::{Any, TypeId};
use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::ServiceError;

/// Handle returned by [`TypedEventBus::subscribe`] for deterministic unsubscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TypedEventSubscription(pub u64);

/// Type-erased in-process host event carrying an `Arc<E>` with zero byte serialization.
#[derive(Clone)]
pub struct TypedHostEvent {
    type_id: TypeId,
    payload: Arc<dyn Any + Send + Sync>,
}

impl std::fmt::Debug for TypedHostEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypedHostEvent")
            .field("type_id", &self.type_id)
            .finish()
    }
}

impl TypedHostEvent {
    /// Wraps an owned event value in an `Arc` without serialization.
    #[inline]
    pub fn new<E: Send + Sync + 'static>(event: E) -> Self {
        Self::from_arc(Arc::new(event))
    }

    /// Wraps an existing `Arc<E>` for zero-copy fan-out.
    #[inline]
    pub fn from_arc<E: Send + Sync + 'static>(event: Arc<E>) -> Self {
        Self {
            type_id: TypeId::of::<E>(),
            payload: event,
        }
    }

    #[inline]
    pub fn type_id(&self) -> TypeId {
        self.type_id
    }

    #[inline]
    pub fn is<E: 'static>(&self) -> bool {
        self.type_id == TypeId::of::<E>()
    }

    #[inline]
    #[expect(
        clippy::disallowed_methods,
        reason = "TypedHostEvent type-erased in-process event bus boundary"
    )]
    pub fn downcast_ref<E: 'static>(&self) -> Option<&E> {
        self.payload.downcast_ref::<E>()
    }

    #[inline]
    pub fn downcast_arc<E: Send + Sync + 'static>(&self) -> Option<Arc<E>> {
        Arc::clone(&self.payload).downcast::<E>().ok()
    }
}

type SubscriberFn = Arc<dyn Fn(&dyn Any) + Send + Sync>;

struct SubscriberEntry {
    id: TypedEventSubscription,
    callback: SubscriberFn,
}

/// Default maximum number of undrained pull-queue events retained by [`TypedEventBus`].
/// Oldest entries are evicted FIFO when full so undrained event types never grow unboundedly.
pub const DEFAULT_TYPED_EVENT_QUEUE_CAP: usize = 1024;

struct TypedEventBusState {
    subscribers: HashMap<TypeId, Vec<SubscriberEntry>>,
    sub_to_type: BTreeMap<TypedEventSubscription, TypeId>,
    retained: HashMap<TypeId, Arc<dyn Any + Send + Sync>>,
    queue: VecDeque<TypedHostEvent>,
    max_queue_len: usize,
    dropped_events: u64,
}

impl TypedEventBusState {
    fn new(max_queue_len: usize) -> Self {
        Self {
            subscribers: HashMap::new(),
            sub_to_type: BTreeMap::new(),
            retained: HashMap::new(),
            queue: VecDeque::new(),
            max_queue_len: max_queue_len.max(1),
            dropped_events: 0,
        }
    }
}

/// Zero-serialization in-process typed event bus supporting:
/// - Synchronous fan-out to typed subscribers (`Arc<E>` shared across all listeners),
/// - Optional per-`TypeId` retained snapshot replay for late subscribers, and
/// - Bounded FIFO pull-queue draining by concrete event type (`drain_typed::<E>`) or all events.
pub struct TypedEventBus {
    next_sub_id: AtomicU64,
    state: Mutex<TypedEventBusState>,
}

impl Default for TypedEventBus {
    fn default() -> Self {
        Self::new()
    }
}

impl TypedEventBus {
    pub fn new() -> Self {
        Self::with_queue_capacity(DEFAULT_TYPED_EVENT_QUEUE_CAP)
    }

    pub fn with_queue_capacity(max_queue_len: usize) -> Self {
        Self {
            next_sub_id: AtomicU64::new(1),
            state: Mutex::new(TypedEventBusState::new(max_queue_len)),
        }
    }

    /// Number of oldest pull-queue events evicted because the queue reached its capacity cap.
    pub fn dropped_count(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .dropped_events
    }

    /// Publishes `event` to the FIFO queue and notifies all subscribers of `E`.
    /// Returns the number of notified subscribers.
    pub fn publish<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.publish_arc(Arc::new(event), false)
    }

    /// Publishes `event`, retains it as the latest snapshot for `E`, and notifies all subscribers.
    /// Returns the number of notified subscribers.
    pub fn publish_retained<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.publish_arc(Arc::new(event), true)
    }

    /// Publishes an already `Arc`-wrapped event with optional retention and zero copies.
    pub fn publish_arc<E: Send + Sync + 'static>(&self, event: Arc<E>, retain: bool) -> usize {
        self.publish_arc_inner(event, retain, true)
    }

    /// Updates the retained snapshot for `E` and notifies active subscribers without appending
    /// to the pull queue (prevents unbounded queue growth when callers already consume the
    /// return value directly, such as `StudioHostServices::ingest_host_batch`).
    pub fn publish_retained_notify<E: Send + Sync + 'static>(&self, event: E) -> usize {
        self.publish_arc_inner(Arc::new(event), true, false)
    }

    fn publish_arc_inner<E: Send + Sync + 'static>(
        &self,
        event: Arc<E>,
        retain: bool,
        enqueue: bool,
    ) -> usize {
        let tid = TypeId::of::<E>();
        let any_arc: Arc<dyn Any + Send + Sync> = event;
        let callbacks: Vec<SubscriberFn> = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if retain {
                st.retained.insert(tid, Arc::clone(&any_arc));
            }
            if enqueue {
                if st.queue.len() >= st.max_queue_len {
                    let _ = st.queue.pop_front();
                    st.dropped_events = st.dropped_events.saturating_add(1);
                }
                st.queue.push_back(TypedHostEvent {
                    type_id: tid,
                    payload: Arc::clone(&any_arc),
                });
            }
            st.subscribers
                .get(&tid)
                .map(|subs| subs.iter().map(|s| Arc::clone(&s.callback)).collect())
                .unwrap_or_default()
        };
        let count = callbacks.len();
        for cb in callbacks {
            cb(any_arc.as_ref());
        }
        count
    }

    /// Registers a typed listener for events of type `E`.
    ///
    /// When `replay_retained` is `true` and a retained snapshot of `E` exists, `handler` is
    /// invoked immediately outside the internal lock.
    pub fn subscribe<E, F>(&self, replay_retained: bool, handler: F) -> TypedEventSubscription
    where
        E: Send + Sync + 'static,
        F: Fn(&E) + Send + Sync + 'static,
    {
        let tid = TypeId::of::<E>();
        let sub_id = TypedEventSubscription(self.next_sub_id.fetch_add(1, Ordering::Relaxed));
        let callback: SubscriberFn = Arc::new(move |any: &dyn Any| {
            #[expect(
                clippy::disallowed_methods,
                reason = "TypedEventBus subscriber dispatch downcasts per TypeId bucket"
            )]
            if let Some(ev) = any.downcast_ref::<E>() {
                handler(ev);
            }
        });
        let retained_snapshot = {
            let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
            st.subscribers
                .entry(tid)
                .or_default()
                .push(SubscriberEntry {
                    id: sub_id,
                    callback: Arc::clone(&callback),
                });
            st.sub_to_type.insert(sub_id, tid);
            if replay_retained {
                st.retained.get(&tid).cloned()
            } else {
                None
            }
        };
        if let Some(snap) = retained_snapshot {
            callback(snap.as_ref());
        }
        sub_id
    }

    /// Removes a previously registered subscription, returning `true` if it was active.
    pub fn unsubscribe(&self, id: TypedEventSubscription) -> bool {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let Some(tid) = st.sub_to_type.remove(&id) else {
            return false;
        };
        if let Some(subs) = st.subscribers.get_mut(&tid) {
            let before = subs.len();
            subs.retain(|s| s.id != id);
            return subs.len() != before;
        }
        false
    }

    /// Number of active subscribers registered for event type `E`.
    pub fn subscriber_count<E: 'static>(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .subscribers
            .get(&TypeId::of::<E>())
            .map_or(0, Vec::len)
    }

    /// Returns the latest retained snapshot for event type `E`, if any.
    pub fn retained<E: Send + Sync + 'static>(&self) -> Option<Arc<E>> {
        let any = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retained
            .get(&TypeId::of::<E>())
            .cloned()?;
        any.downcast::<E>().ok()
    }

    /// Clears any retained snapshot for event type `E`.
    pub fn clear_retained<E: 'static>(&self) -> bool {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .retained
            .remove(&TypeId::of::<E>())
            .is_some()
    }

    /// Drains all queued events of concrete type `E` in FIFO order, leaving other event types
    /// untouched in the queue.
    pub fn drain_typed<E: Send + Sync + 'static>(&self) -> Vec<Arc<E>> {
        self.drain_typed_where(|_| true)
    }

    /// Drains queued events of concrete type `E` matching `predicate` in FIFO order, leaving
    /// non-matching events of type `E` and all other event types untouched.
    pub fn drain_typed_where<E: Send + Sync + 'static, F: FnMut(&E) -> bool>(
        &self,
        mut predicate: F,
    ) -> Vec<Arc<E>> {
        let tid = TypeId::of::<E>();
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut matched = Vec::new();
        let mut remaining = VecDeque::with_capacity(st.queue.len());
        while let Some(ev) = st.queue.pop_front() {
            if ev.type_id == tid {
                match ev.payload.downcast::<E>() {
                    Ok(typed) if predicate(typed.as_ref()) => matched.push(typed),
                    Ok(typed) => remaining.push_back(TypedHostEvent {
                        type_id: ev.type_id,
                        payload: typed,
                    }),
                    Err(payload) => remaining.push_back(TypedHostEvent {
                        type_id: ev.type_id,
                        payload,
                    }),
                }
            } else {
                remaining.push_back(ev);
            }
        }
        st.queue = remaining;
        matched
    }

    /// Drains queued [`TypedHostEvent`]s matching `predicate` in FIFO order, leaving other events
    /// untouched.
    pub fn drain_matching<F: FnMut(&TypedHostEvent) -> bool>(
        &self,
        mut predicate: F,
    ) -> Vec<TypedHostEvent> {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut matched = Vec::new();
        let mut remaining = VecDeque::with_capacity(st.queue.len());
        while let Some(ev) = st.queue.pop_front() {
            if predicate(&ev) {
                matched.push(ev);
            } else {
                remaining.push_back(ev);
            }
        }
        st.queue = remaining;
        matched
    }

    /// Drains every queued [`TypedHostEvent`] in FIFO order.
    pub fn poll_all(&self) -> Vec<TypedHostEvent> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .drain(..)
            .collect()
    }

    /// Number of events currently queued across all event types.
    pub fn queued_len(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .queue
            .len()
    }
}

type BoxedTypedAsyncCallback =
    Box<dyn FnOnce(Result<Arc<dyn Any + Send + Sync>, ServiceError>) + Send + 'static>;

enum TypedAsyncSlot {
    Pending {
        callback: Option<BoxedTypedAsyncCallback>,
    },
    Completed(Result<Arc<dyn Any + Send + Sync>, ServiceError>),
}

/// Zero-serialization in-process table tracking typed async host request completions.
#[derive(Default)]
pub struct TypedAsyncTable {
    slots: BTreeMap<u64, TypedAsyncSlot>,
}

impl TypedAsyncTable {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_pending(&mut self, request_id: u64) {
        self.slots
            .insert(request_id, TypedAsyncSlot::Pending { callback: None });
    }

    pub fn register_callback<R, F>(&mut self, request_id: u64, on_complete: F)
    where
        R: Send + Sync + 'static,
        F: FnOnce(Result<Arc<R>, ServiceError>) + Send + 'static,
    {
        let cb: BoxedTypedAsyncCallback = Box::new(move |res| match res {
            Ok(any) => match any.downcast::<R>() {
                Ok(typed) => on_complete(Ok(typed)),
                Err(_) => on_complete(Err(ServiceError::InternalError(
                    "typed async completion type mismatch".into(),
                ))),
            },
            Err(err) => on_complete(Err(err)),
        });
        self.slots
            .insert(request_id, TypedAsyncSlot::Pending { callback: Some(cb) });
    }

    /// Completes `request_id` with a typed Rust result (zero serialization).
    ///
    /// Returns a thunk to invoke outside any mutex lock when a callback was registered.
    pub fn complete<R: Send + Sync + 'static>(
        &mut self,
        request_id: u64,
        result: Result<R, ServiceError>,
    ) -> Option<Box<dyn FnOnce() + Send + 'static>> {
        let erased: Result<Arc<dyn Any + Send + Sync>, ServiceError> =
            result.map(|v| Arc::new(v) as Arc<dyn Any + Send + Sync>);
        match self.slots.remove(&request_id) {
            Some(TypedAsyncSlot::Pending { callback: Some(cb) }) => {
                Some(Box::new(move || cb(erased)))
            }
            Some(TypedAsyncSlot::Pending { callback: None })
            | Some(TypedAsyncSlot::Completed(_))
            | None => {
                self.slots
                    .insert(request_id, TypedAsyncSlot::Completed(erased));
                None
            }
        }
    }

    /// Consumes a completed typed async response if present and matching `R`.
    pub fn take<R: Send + Sync + 'static>(
        &mut self,
        request_id: u64,
    ) -> Option<Result<Arc<R>, ServiceError>> {
        if !matches!(
            self.slots.get(&request_id),
            Some(TypedAsyncSlot::Completed(_))
        ) {
            return None;
        }
        let Some(TypedAsyncSlot::Completed(res)) = self.slots.remove(&request_id) else {
            return None;
        };
        Some(match res {
            Ok(any) => any.downcast::<R>().map_err(|_| {
                ServiceError::InternalError("typed async completion type mismatch".into())
            }),
            Err(err) => Err(err),
        })
    }

    pub fn pending_count(&self) -> usize {
        self.slots
            .values()
            .filter(|s| matches!(s, TypedAsyncSlot::Pending { .. }))
            .count()
    }
}
