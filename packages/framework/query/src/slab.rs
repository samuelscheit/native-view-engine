//! Hot/cold dense slot storage (`QueryMeta` 64B cache-line aligned + `QueryPayload`) and flat
//! doubly-linked `TagEdge` pool for O(k) tag invalidation and O(own tags) GC unlinking.

use std::collections::HashMap;

use bytes::Bytes;
use nve_rpc::{ProcId, RpcError, RpcStatus, Tag64, TransportError, TransportErrorKind, STATUS_TABLE};

use crate::optimistic::OptimisticLayer;
use crate::types::{IdentityBuildHasher, QueryOptions, SharedAny};

pub(crate) const NONE_IDX: u32 = u32::MAX;
pub(crate) const MAX_PROTOCOL_ERR_MSG_BYTES: usize = 256;

/// Maps a raw `NVEQ` v2 `RpcStatus` discriminant byte back to [`RpcStatus`], rejecting `0` (`Ok`)
/// and out-of-range values so error wire entries fail closed.
#[inline]
pub(crate) fn rpc_status_from_u8(raw: u8) -> Option<RpcStatus> {
    let status = STATUS_TABLE.get(raw as usize)?.0;
    (status != RpcStatus::Ok).then_some(status)
}

/// Truncates `s` to at most `max_bytes` UTF-8 bytes on a valid char boundary.
#[inline]
pub(crate) fn truncate_utf8_bytes(s: &str, max_bytes: usize) -> &str {
    if s.len() <= max_bytes {
        return s;
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    &s[..end]
}

/// Raw wire representation of a settled query error stored in [`QueryPayload`] for zero-reencode
/// `NVEQ` v2 dehydration and lazy client decoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum RawErrorWire {
    Domain { status: u8, body: Bytes },
    Protocol { status: u8, message: String },
}

impl RawErrorWire {
    /// Captures a sanitized protocol error wire entry: `5xx`/internal statuses are clamped strictly
    /// to `status.code_str()` (never leaking internal details), and `4xx` messages are capped at
    /// `256` UTF-8 bytes.
    pub(crate) fn from_protocol(status: RpcStatus, message: &str) -> Self {
        let status = if status == RpcStatus::Ok {
            RpcStatus::Internal
        } else {
            status
        };
        let sanitized = if status.http_status() >= 500 {
            status.code_str().to_owned()
        } else {
            truncate_utf8_bytes(message, MAX_PROTOCOL_ERR_MSG_BYTES).to_owned()
        };
        Self::Protocol {
            status: status as u8,
            message: sanitized,
        }
    }

    /// Captures a sanitized protocol error wire entry for non-cancelled transport failures,
    /// clamping the message strictly to `status.code_str()` so internal hosts/sockets never leak.
    pub(crate) fn from_transport(err: &TransportError) -> Option<Self> {
        let status = match err.kind {
            TransportErrorKind::Cancelled => return None,
            TransportErrorKind::Timeout => RpcStatus::RequestTimeout,
            TransportErrorKind::Network | TransportErrorKind::Offline => RpcStatus::Unavailable,
            TransportErrorKind::ResponseTooLarge => RpcStatus::PayloadTooLarge,
            TransportErrorKind::Config
            | TransportErrorKind::Redirect
            | TransportErrorKind::Other => RpcStatus::Internal,
        };
        Some(Self::Protocol {
            status: status as u8,
            message: status.code_str().to_owned(),
        })
    }

    /// Captures a sanitized wire error from an [`RpcError`] when raw domain bytes are not
    /// separately available (e.g. sans-I/O `complete_fetch` with protocol/transport errors).
    pub(crate) fn from_non_domain_error<E>(err: &RpcError<E>) -> Option<Self> {
        match err {
            RpcError::Domain { .. } => None,
            RpcError::Protocol { status, message } => Some(Self::from_protocol(*status, message)),
            RpcError::Transport(terr) => Self::from_transport(terr),
            RpcError::Codec(_) => Some(Self::Protocol {
                status: RpcStatus::Internal as u8,
                message: RpcStatus::Internal.code_str().to_owned(),
            }),
        }
    }
}

#[inline]
pub(crate) const fn pack_invalidation_version(wildcard_epoch: u32, fetch_gen: u32) -> u64 {
    ((wildcard_epoch as u64) << 32) | (fetch_gen as u64)
}

/// Dense index into the query cache's hot (`QueryMeta`) and cold (`QueryPayload`) slabs.
#[repr(transparent)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuerySlotId(pub(crate) u32);

impl QuerySlotId {
    #[inline]
    pub const fn index(self) -> usize {
        self.0 as usize
    }

    #[inline]
    pub const fn raw(self) -> u32 {
        self.0
    }
}

pub(crate) const FLAG_FREE: u16 = 1 << 0;
pub(crate) const FLAG_HAS_DATA: u16 = 1 << 1;
pub(crate) const FLAG_HAS_ERROR: u16 = 1 << 2;
pub(crate) const FLAG_IS_FETCHING: u16 = 1 << 3;
pub(crate) const FLAG_IS_INVALIDATED: u16 = 1 << 4;
pub(crate) const FLAG_IN_DIRTY_QUEUE: u16 = 1 << 5;
pub(crate) const FLAG_REFETCH_ON_FOCUS: u16 = 1 << 6;
pub(crate) const FLAG_REFETCH_ON_RECONNECT: u16 = 1 << 7;
pub(crate) const FLAG_SUBSCRIBED_ALL: u16 = 1 << 8;
pub(crate) const FLAG_HYDRATED: u16 = 1 << 9;
pub(crate) const FLAG_STREAM_PENDING: u16 = 1 << 10;

/// Hot per-query metadata packed into exactly one 64-byte cache line with zero heap pointers.
#[repr(C, align(64))]
#[derive(Debug, Clone, Copy)]
pub struct QueryMeta {
    pub proc_id: ProcId,
    pub args_hash: u64,
    pub updated_at_ms: i64,
    pub invalidated_epoch: u32,
    pub stale_time_ms: u32,
    pub gc_time_ms: u32,
    pub fetch_gen: u32,
    pub data_version: u32,
    pub slot_gen: u32,
    pub hash_next: u32,
    pub first_tag_edge: u32,
    pub subscriber_count: u32,
    pub flags: u16,
    pub optimistic_rev: u16,
}

const _: () = {
    assert!(core::mem::size_of::<QueryMeta>() == 64);
    assert!(core::mem::align_of::<QueryMeta>() == 64);
};

impl QueryMeta {
    #[inline]
    pub(crate) fn free(slot_gen: u32) -> Self {
        Self {
            proc_id: ProcId::from_raw(0),
            args_hash: 0,
            updated_at_ms: 0,
            invalidated_epoch: 0,
            stale_time_ms: 0,
            gc_time_ms: 0,
            fetch_gen: 0,
            data_version: 0,
            slot_gen,
            hash_next: NONE_IDX,
            first_tag_edge: NONE_IDX,
            subscriber_count: 0,
            flags: FLAG_FREE,
            optimistic_rev: 0,
        }
    }

    #[inline]
    pub fn is_free(&self) -> bool {
        (self.flags & FLAG_FREE) != 0
    }

    #[inline]
    pub fn has_data(&self) -> bool {
        (self.flags & FLAG_HAS_DATA) != 0
    }

    #[inline]
    pub fn has_error(&self) -> bool {
        (self.flags & FLAG_HAS_ERROR) != 0
    }

    #[inline]
    pub fn is_fetching(&self) -> bool {
        (self.flags & FLAG_IS_FETCHING) != 0
    }

    #[inline]
    pub fn is_stream_pending(&self) -> bool {
        (self.flags & FLAG_STREAM_PENDING) != 0
    }

    #[inline]
    pub fn is_invalidated(&self, wildcard_epoch: u32) -> bool {
        (self.flags & FLAG_IS_INVALIDATED) != 0 || self.invalidated_epoch != wildcard_epoch
    }

    #[inline]
    pub fn is_stale(&self, now_ms: u64, wildcard_epoch: u32, hydration_grace_ms: u32) -> bool {
        if self.is_invalidated(wildcard_epoch) {
            return true;
        }
        let hydrated_settled =
            (self.flags & FLAG_HYDRATED) != 0 && (self.has_data() || self.has_error());
        if !self.has_data() && !hydrated_settled {
            return true;
        }
        let eff_stale = if hydrated_settled {
            i64::from(self.stale_time_ms.max(hydration_grace_ms))
        } else {
            i64::from(self.stale_time_ms)
        };
        let now_i64 = i64::try_from(now_ms).unwrap_or(i64::MAX);
        now_i64.saturating_sub(self.updated_at_ms) >= eff_stale
    }
}

/// Cold per-slot storage holding encoded args, raw response bytes, decoded values, and optimistic
/// layers. Kept out of [`QueryMeta`] so hot sweeps never touch heap pointers.
pub(crate) struct QueryPayload {
    pub proc_name: &'static str,
    pub encoded_args: Bytes,
    pub raw_response: Option<Bytes>,
    pub raw_body_hash: u64,
    pub raw_error: Option<RawErrorWire>,
    pub committed: Option<SharedAny>,
    pub committed_error: Option<SharedAny>,
    pub effective_optimistic: Option<SharedAny>,
    pub optimistic_layers: Vec<OptimisticLayer>,
    pub last_inactive_ms: u64,
    pub active_pos: u32,
    pub completed_seq: u32,
    pub completed_invalidation_version: u64,
    pub in_flight_waiters: Vec<core::task::Waker>,
    pub options: QueryOptions,
}

impl QueryPayload {
    pub(crate) fn empty() -> Self {
        Self {
            proc_name: "",
            encoded_args: Bytes::new(),
            raw_response: None,
            raw_body_hash: 0,
            raw_error: None,
            committed: None,
            committed_error: None,
            effective_optimistic: None,
            optimistic_layers: Vec::new(),
            last_inactive_ms: 0,
            active_pos: NONE_IDX,
            completed_seq: 0,
            completed_invalidation_version: 0,
            in_flight_waiters: Vec::new(),
            options: QueryOptions::default(),
        }
    }

    #[inline]
    pub(crate) fn record_commit(&mut self, inv_ver: u64) {
        self.completed_seq = self.completed_seq.wrapping_add(1).max(1);
        self.completed_invalidation_version = inv_ver;
    }

    /// Resets cold payload storage WITHOUT waking waiters under the cache lock; callers must wake
    /// returned waiters after releasing the lock.
    pub(crate) fn reset(&mut self) -> Vec<core::task::Waker> {
        self.proc_name = "";
        self.encoded_args = Bytes::new();
        self.raw_response = None;
        self.raw_body_hash = 0;
        self.raw_error = None;
        self.committed = None;
        self.committed_error = None;
        self.effective_optimistic = None;
        self.optimistic_layers.clear();
        self.last_inactive_ms = 0;
        self.active_pos = NONE_IDX;
        self.completed_seq = 0;
        self.completed_invalidation_version = 0;
        self.options = QueryOptions::default();
        core::mem::take(&mut self.in_flight_waiters)
    }

    /// Returns the currently visible value (optimistic overlay if active, otherwise committed).
    #[inline]
    pub(crate) fn visible_data(&self) -> Option<&SharedAny> {
        self.effective_optimistic
            .as_ref()
            .or(self.committed.as_ref())
    }
}

/// 24-byte edge inside the flat tag-index pool linking one `(Tag64, QuerySlotId)` pair.
#[derive(Debug, Clone, Copy)]
struct TagEdge {
    slot: QuerySlotId,
    tag: Tag64,
    next_for_tag: u32,
    prev_for_tag: u32,
    next_for_slot: u32,
}

const _: () = {
    assert!(core::mem::size_of::<TagEdge>() == 24);
};

/// Flat inverted index mapping `Tag64 -> [QuerySlotId]` with O(k) tag invalidation and O(own tags)
/// slot eviction, using zero per-tag `Vec` allocations.
#[derive(Default)]
pub(crate) struct TagIndex {
    heads: HashMap<Tag64, u32, IdentityBuildHasher>,
    edges: Vec<TagEdge>,
    free_edges: Vec<u32>,
}

impl TagIndex {
    pub(crate) fn attach_tags(
        &mut self,
        slot: QuerySlotId,
        first_edge: &mut u32,
        tags: &[Tag64],
    ) {
        for &tag in tags {
            // Skip duplicate tag already attached to this slot.
            let mut cur = *first_edge;
            let mut dup = false;
            while cur != NONE_IDX {
                let edge = self.edges[cur as usize];
                if edge.tag == tag {
                    dup = true;
                    break;
                }
                cur = edge.next_for_slot;
            }
            if dup {
                continue;
            }

            let old_tag_head = self.heads.get(&tag).copied().unwrap_or(NONE_IDX);
            let edge_idx = if let Some(reused) = self.free_edges.pop() {
                self.edges[reused as usize] = TagEdge {
                    slot,
                    tag,
                    next_for_tag: old_tag_head,
                    prev_for_tag: NONE_IDX,
                    next_for_slot: *first_edge,
                };
                reused
            } else {
                let idx = u32::try_from(self.edges.len()).expect("TagEdge pool exceeded u32::MAX");
                self.edges.push(TagEdge {
                    slot,
                    tag,
                    next_for_tag: old_tag_head,
                    prev_for_tag: NONE_IDX,
                    next_for_slot: *first_edge,
                });
                idx
            };

            if old_tag_head != NONE_IDX {
                self.edges[old_tag_head as usize].prev_for_tag = edge_idx;
            }
            self.heads.insert(tag, edge_idx);
            *first_edge = edge_idx;
        }
    }

    pub(crate) fn unlink_slot(&mut self, first_edge: &mut u32) {
        let mut cur = *first_edge;
        *first_edge = NONE_IDX;
        while cur != NONE_IDX {
            let edge = self.edges[cur as usize];
            if edge.prev_for_tag != NONE_IDX {
                self.edges[edge.prev_for_tag as usize].next_for_tag = edge.next_for_tag;
            } else if edge.next_for_tag != NONE_IDX {
                self.heads.insert(edge.tag, edge.next_for_tag);
            } else {
                self.heads.remove(&edge.tag);
            }
            if edge.next_for_tag != NONE_IDX {
                self.edges[edge.next_for_tag as usize].prev_for_tag = edge.prev_for_tag;
            }
            self.free_edges.push(cur);
            cur = edge.next_for_slot;
        }
    }

    pub(crate) fn for_each_slot(&self, tag: Tag64, mut f: impl FnMut(QuerySlotId)) {
        let mut cur = self.heads.get(&tag).copied().unwrap_or(NONE_IDX);
        while cur != NONE_IDX {
            let edge = self.edges[cur as usize];
            f(edge.slot);
            cur = edge.next_for_tag;
        }
    }

    pub(crate) fn collect_tags_for_slot(&self, first_edge: u32, out: &mut Vec<Tag64>) {
        out.clear();
        let mut cur = first_edge;
        while cur != NONE_IDX {
            let edge = self.edges[cur as usize];
            out.push(edge.tag);
            cur = edge.next_for_slot;
        }
    }

    #[inline]
    pub(crate) fn live_edge_count(&self) -> usize {
        self.edges.len().saturating_sub(self.free_edges.len())
    }
}
