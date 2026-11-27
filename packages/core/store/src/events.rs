//! Zero-serialization in-process typed `TableEvent` bus + FFI `NveDirectEvent` bridge.
//!
//! Modules emit typed return events (e.g. `WebView` load/message, `VideoPlayer` progress,
//! `Camera` frame, 3P `MapView` region-change) without allocating or serializing in-process.
//! When crossing a C ABI / wire boundary, each event encodes cleanly into
//! `NativeEventKind::TableEvent` (`NveDirectEvent::table_event`).

use std::any::Any;
use std::sync::Arc;

use nve_abi::{EventPayload, Key64, NativeEvent, NativeEventKind, NveDirectEvent, SurfaceId};

use crate::id::NodeId;
use crate::table::{NveTable, TableSlot};

/// One typed table event delivered in-process with zero serialization.
#[derive(Debug, Clone)]
pub struct TypedTableEvent {
    pub surface_id: SurfaceId,
    pub node: NodeId,
    pub timestamp_us: u64,
    pub table: TableSlot,
    pub table_key: Key64,
    pub event_id: u32,
    /// In-process typed payload (`None` for wire-decoded events that only carry raw bytes).
    typed: Option<Arc<dyn Any + Send + Sync>>,
    /// Wire-encoded payload bytes (populated lazily or when decoded from FFI/EventRing).
    wire_bytes: Vec<u8>,
}

impl TypedTableEvent {
    /// Creates an in-process typed event (zero serialization).
    pub fn typed<T: NveTable, P: Send + Sync + 'static>(
        surface_id: SurfaceId,
        node: NodeId,
        timestamp_us: u64,
        event_id: u32,
        payload: P,
    ) -> Self {
        Self {
            surface_id,
            node,
            timestamp_us,
            table: T::slot(),
            table_key: T::DESC.key,
            event_id,
            typed: Some(Arc::new(payload)),
            wire_bytes: Vec::new(),
        }
    }

    /// Creates a table event carrying raw wire payload bytes (from FFI / EventRing).
    pub fn from_wire(
        surface_id: SurfaceId,
        node: NodeId,
        timestamp_us: u64,
        table: TableSlot,
        table_key: Key64,
        event_id: u32,
        payload: Vec<u8>,
    ) -> Self {
        Self {
            surface_id,
            node,
            timestamp_us,
            table,
            table_key,
            event_id,
            typed: None,
            wire_bytes: payload,
        }
    }

    /// Zero-cost typed downcast for in-process consumers.
    #[inline]
    pub fn payload<P: 'static>(&self) -> Option<&P> {
        self.typed.as_deref()?.downcast_ref::<P>()
    }

    /// Raw wire bytes (if present).
    #[inline]
    pub fn wire_bytes(&self) -> &[u8] {
        &self.wire_bytes
    }

    /// Converts into a cross-boundary [`NativeEvent`] (encodes via `encode_payload` if needed).
    pub fn to_native_event(&self, encode_payload: impl FnOnce(&Self) -> Vec<u8>) -> NativeEvent {
        let payload = if !self.wire_bytes.is_empty() {
            self.wire_bytes.clone()
        } else {
            encode_payload(self)
        };
        NativeEvent {
            surface_id: self.surface_id,
            node_id: self.node.0,
            timestamp_us: self.timestamp_us,
            kind: NativeEventKind::TableEvent,
            payload: EventPayload::TableEvent {
                table_key: self.table_key.0,
                event_id: self.event_id,
                payload,
            },
        }
    }

    /// Borrows this event's raw wire bytes as a zero-copy [`NveDirectEvent`].
    #[inline]
    pub fn as_direct_event(&self) -> NveDirectEvent {
        NveDirectEvent::table_event(
            self.surface_id,
            self.node.0,
            self.timestamp_us,
            self.table_key,
            self.event_id,
            &self.wire_bytes,
        )
    }
}

/// Reusable per-surface queue of typed table return events.
#[derive(Debug, Default)]
pub struct TableEventBus {
    queue: Vec<TypedTableEvent>,
}

impl TableEventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// Enqueues a zero-serialization typed event for table `T`.
    pub fn emit<T: NveTable, P: Send + Sync + 'static>(
        &mut self,
        surface_id: SurfaceId,
        node: NodeId,
        timestamp_us: u64,
        event_id: u32,
        payload: P,
    ) {
        self.queue
            .push(TypedTableEvent::typed::<T, P>(surface_id, node, timestamp_us, event_id, payload));
    }

    /// Enqueues a raw wire event decoded from FFI / EventRing.
    pub fn emit_raw(&mut self, event: TypedTableEvent) {
        self.queue.push(event);
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Drains all pending table events.
    pub fn drain(&mut self) -> std::vec::Drain<'_, TypedTableEvent> {
        self.queue.drain(..)
    }

    pub fn clear(&mut self) {
        self.queue.clear();
    }
}
