//! Compact zero-`serde_json` `NVEQ` v2 dehydration, per-boundary delta streaming cursors,
//! monotonic fill-only hydration, and truncated-stream recovery for [`crate::QueryClient`].

mod codec;

use std::collections::HashMap;

use nve_hash::xxh64;
use nve_rpc::{MaybeSend, MaybeSync, Tag64, Transport};

pub use self::codec::{nveq_entry_count, DehydrateError, NVEQ_VERSION};
use self::codec::{
    decode_hydration_entries, encode_hydration_entries, DehydrateEntryRef, DehydratePayloadRef,
    HydratedPayload,
};
use crate::engine::{query_key_hash, run_side_effects, QueryClient, BODY_HASH_SEED};
use crate::slab::{
    pack_invalidation_version, QueryMeta, QueryPayload, QuerySlotId, RawErrorWire, FLAG_HAS_DATA,
    FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_FETCHING, FLAG_IS_INVALIDATED,
    FLAG_SUBSCRIBED_ALL,
};
use crate::state::{CacheState, WakeCallback};
use crate::types::{IdentityBuildHasher, QueryHandle};

fn apply_hydrated_error(
    s: &mut CacheState,
    slot: QuerySlotId,
    updated_at: i64,
    raw_error: RawErrorWire,
    waiters: &mut Vec<core::task::Waker>,
    wake_cb: &mut Option<WakeCallback>,
) {
    let idx = slot.index();
    s.clear_stream_pending(idx, FLAG_HAS_DATA | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING);
    let meta = &mut s.metas[idx];
    meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
    meta.optimistic_rev = meta.optimistic_rev.wrapping_add(1);
    meta.updated_at_ms = updated_at;
    meta.invalidated_epoch = s.wildcard_epoch;
    meta.flags |= FLAG_HAS_ERROR | FLAG_HYDRATED;

    let payload = &mut s.payloads[idx];
    waiters.append(&mut payload.in_flight_waiters);
    payload.raw_response = None;
    payload.raw_body_hash = 0;
    payload.raw_error = Some(raw_error);
    payload.committed = None;
    payload.committed_error = None;
    payload.effective_optimistic = None;
    payload.record_commit(pack_invalidation_version(s.wildcard_epoch, meta.fetch_gen));
    *wake_cb = s.mark_dirty(slot);
}

/// Tracks `(slot_gen, completed_seq)` per [`QuerySlotId`] already emitted during a streaming SSR
/// response so shared handles across sibling boundaries are dehydrated at most once per revision.
#[derive(Debug, Default)]
pub struct DehydrateCursor {
    emitted: HashMap<QuerySlotId, (u32, u32), IdentityBuildHasher>,
}

impl DehydrateCursor {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.emitted.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.emitted.is_empty()
    }

    #[inline]
    pub fn clear(&mut self) {
        self.emitted.clear();
    }
}

/// Wire payload of a settled slot: committed data wins over a coexisting error (stale data +
/// failed refetch renders data, matching `UseQuery::suspend`), errors only when no data exists.
#[inline]
fn settled_payload_ref<'a>(
    meta: QueryMeta,
    payload: &'a QueryPayload,
) -> Option<DehydratePayloadRef<'a>> {
    if meta.has_data()
        && let Some(raw) = payload.raw_response.as_deref()
    {
        return Some(DehydratePayloadRef::Ok(raw));
    }
    if !meta.has_error() {
        return None;
    }
    Some(match payload.raw_error.as_ref()? {
        RawErrorWire::Domain { status, body } => DehydratePayloadRef::DomainErr {
            status: *status,
            body: body.as_ref(),
        },
        RawErrorWire::Protocol { status, message } => DehydratePayloadRef::ProtocolErr {
            status: *status,
            message: message.as_str(),
        },
    })
}

#[inline]
fn compute_age_ms(now_ms: u64, updated_at_ms: i64) -> u32 {
    let now_i64 = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let elapsed = now_i64.saturating_sub(updated_at_ms).max(0) as u64;
    u32::try_from(elapsed).unwrap_or(u32::MAX)
}

/// One slot chosen for dehydration with its collected tags (`pending` emits `StreamPending`).
struct SelectedEntry {
    slot: QuerySlotId,
    tags: Vec<Tag64>,
    pending: bool,
}

/// Single selection/encoding pipeline shared by every `dehydrate_*` entry point.
#[derive(Default)]
struct DehydrateSelection {
    entries: Vec<SelectedEntry>,
}

impl DehydrateSelection {
    #[inline]
    fn contains(&self, slot: QuerySlotId) -> bool {
        self.entries.iter().any(|e| e.slot == slot)
    }

    fn push(&mut self, s: &CacheState, slot: QuerySlotId, pending: bool) {
        let mut tags = Vec::new();
        s.tags
            .collect_tags_for_slot(s.metas[slot.index()].first_tag_edge, &mut tags);
        self.entries.push(SelectedEntry {
            slot,
            tags,
            pending,
        });
    }

    /// Selects every live settled slot.
    fn push_all_settled(&mut self, s: &CacheState) {
        for (idx, &meta) in s.metas.iter().enumerate() {
            if !meta.is_free() && settled_payload_ref(meta, &s.payloads[idx]).is_some() {
                self.push(s, QuerySlotId(idx as u32), false);
            }
        }
    }

    /// Encodes the selection, then records emitted settled revisions in `cursor` (only on
    /// successful encoding).
    fn encode(
        self,
        s: &CacheState,
        now_ms: u64,
        cursor: Option<&mut DehydrateCursor>,
    ) -> Result<Vec<u8>, DehydrateError> {
        let refs: Vec<DehydrateEntryRef<'_>> = self
            .entries
            .iter()
            .filter_map(|e| {
                let idx = e.slot.index();
                let meta = s.metas[idx];
                let payload = &s.payloads[idx];
                let (wire, age_ms) = if e.pending {
                    (DehydratePayloadRef::StreamPending, 0)
                } else {
                    (
                        settled_payload_ref(meta, payload)?,
                        compute_age_ms(now_ms, meta.updated_at_ms),
                    )
                };
                Some(DehydrateEntryRef {
                    proc_id: meta.proc_id,
                    age_ms,
                    subscribed_all: (meta.flags & FLAG_SUBSCRIBED_ALL) != 0,
                    tags: e.tags.as_slice(),
                    encoded_args: &payload.encoded_args,
                    payload: wire,
                })
            })
            .collect();
        let encoded = encode_hydration_entries(&refs)?;
        if let Some(cursor) = cursor {
            for e in self.entries.iter().filter(|e| !e.pending) {
                let idx = e.slot.index();
                cursor
                    .emitted
                    .insert(e.slot, (s.metas[idx].slot_gen, s.payloads[idx].completed_seq));
            }
        }
        Ok(encoded)
    }
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    /// Dehydrates all live settled entries (`Ok`, `DomainErr`, `ProtocolErr`) into a compact
    /// `NVEQ` v2 binary blob for SSR -> client hydration.
    pub fn dehydrate_bytes(&self) -> Result<Vec<u8>, DehydrateError> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut sel = DehydrateSelection::default();
            sel.push_all_settled(s);
            sel.encode(s, now_ms, None)
        })
    }

    /// Dehydrates ONLY live `handles` that have settled (`Ok` or `Err`) and whose
    /// `(slot_gen, completed_seq)` has not yet been emitted in `cursor`, advancing `cursor` only
    /// when encoding succeeds.
    pub fn dehydrate_handles_delta(
        &self,
        handles: &[QueryHandle],
        cursor: &mut DehydrateCursor,
    ) -> Result<Vec<u8>, DehydrateError> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut sel = DehydrateSelection::default();
            for &handle in handles {
                if !s.is_live(handle) || sel.contains(handle.slot) {
                    continue;
                }
                let idx = handle.slot.index();
                let meta = s.metas[idx];
                let payload = &s.payloads[idx];
                let rev = (meta.slot_gen, payload.completed_seq);
                if settled_payload_ref(meta, payload).is_some()
                    && cursor.emitted.get(&handle.slot).copied() != Some(rev)
                {
                    sel.push(s, handle.slot, false);
                }
            }
            sel.encode(s, now_ms, Some(cursor))
        })
    }

    /// Atomically dehydrates the `seq=0` initial streaming SSR payload: includes every currently
    /// settled live entry (`Ok`, `DomainErr`, `ProtocolErr`) AND a `StreamPending` marker for
    /// every unsettled live handle in `pending_handles`, recording emitted settled revisions in
    /// `cursor` so subsequent [`Self::dehydrate_handles_delta`] chunks never duplicate them.
    pub fn dehydrate_initial_stream_chunk(
        &self,
        pending_handles: &[QueryHandle],
        cursor: &mut DehydrateCursor,
    ) -> Result<Vec<u8>, DehydrateError> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut sel = DehydrateSelection::default();
            sel.push_all_settled(s);
            for &handle in pending_handles {
                if !s.is_live(handle) || sel.contains(handle.slot) {
                    continue;
                }
                let meta = s.metas[handle.slot.index()];
                if !meta.has_data() && !meta.has_error() {
                    sel.push(s, handle.slot, true);
                }
            }
            sel.encode(s, now_ms, Some(cursor))
        })
    }

    /// Fill-only monotonic hydration from an initial `NVEQ` v2 snapshot (`seq=0`). Never overwrites
    /// slots that already have committed data/errors or local client activity. Returns the number
    /// of entries actually applied.
    pub fn hydrate_bytes(&self, bytes: &[u8]) -> Result<usize, DehydrateError> {
        self.hydrate_inner(bytes, false)
    }

    /// Fill-only monotonic hydration for an out-of-order streaming `NVEQ` v2 chunk (`seq > 0`).
    /// In addition to the fill-only slot rule, applies nothing once `wildcard_epoch > 0` (the
    /// client called `invalidate_all()` after boot, so every streamed server snapshot is stale —
    /// for absent and existing slots alike). Returns the number of entries applied.
    pub fn hydrate_stream_chunk_bytes(&self, bytes: &[u8]) -> Result<usize, DehydrateError> {
        self.hydrate_inner(bytes, true)
    }

    fn hydrate_inner(&self, bytes: &[u8], is_stream_chunk: bool) -> Result<usize, DehydrateError> {
        let entries = decode_hydration_entries(bytes)?;
        let now_ms = self.inner.clock.now_ms();
        let (applied, wake, waiters) = self.inner.state.with(|s| {
            let mut applied = 0usize;
            let mut waiters = Vec::new();
            let mut wake_cb: Option<WakeCallback> = None;
            if is_stream_chunk && s.wildcard_epoch > 0 {
                return (applied, wake_cb, waiters);
            }

            let now_i64 = i64::try_from(now_ms).unwrap_or(i64::MAX);
            for entry in entries {
                let key_hash = query_key_hash(entry.proc_id, &entry.encoded_args);
                let updated_at = now_i64.saturating_sub(i64::from(entry.age_ms));
                let is_pending_entry = matches!(entry.payload, HydratedPayload::StreamPending);

                let slot = if let Some(existing) =
                    s.lookup_slot(entry.proc_id, key_hash, &entry.encoded_args)
                {
                    let idx = existing.index();
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    // Fill-only monotonic guard: never overwrite committed data/error, invalidated
                    // slots, active client fetches, or slots with prior local commits.
                    let blocked = meta.has_data()
                        || meta.has_error()
                        || meta.is_invalidated(s.wildcard_epoch)
                        || (!meta.is_stream_pending()
                            && (meta.is_fetching() || payload.completed_seq > 0))
                        || (is_pending_entry && meta.is_stream_pending());
                    if blocked {
                        continue;
                    }
                    if entry.subscribed_all && (s.metas[idx].flags & FLAG_SUBSCRIBED_ALL) == 0 {
                        s.metas[idx].flags |= FLAG_SUBSCRIBED_ALL;
                        s.subscribed_all_count += 1;
                    } else if !entry.subscribed_all && !entry.tags.is_empty() {
                        let mut first_edge = s.metas[idx].first_tag_edge;
                        s.tags.attach_tags(existing, &mut first_edge, &entry.tags);
                        s.metas[idx].first_tag_edge = first_edge;
                    }
                    existing
                } else {
                    let target_cap = self.inner.config.max_entries.saturating_sub(1);
                    s.gc_sweep(now_ms, 16, target_cap, &mut waiters);
                    // Pass `now_ms` as `last_inactive_ms` so SSR entries with `age_ms >= gc_time_ms`
                    // are not immediately evicted before the client mounts and reads them.
                    s.insert_slot(
                        entry.proc_id,
                        "",
                        key_hash,
                        entry.encoded_args,
                        self.inner.config.default_options,
                        &entry.tags,
                        entry.subscribed_all,
                        now_ms,
                    )
                    .slot
                };

                let idx = slot.index();
                match entry.payload {
                    HydratedPayload::StreamPending => {
                        s.set_stream_pending(idx);
                        wake_cb = s.mark_dirty(slot);
                    }
                    HydratedPayload::Ok(raw_body) => {
                        let body_hash = xxh64(&raw_body, BODY_HASH_SEED);
                        s.clear_stream_pending(
                            idx,
                            FLAG_HAS_ERROR | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING,
                        );
                        let meta = &mut s.metas[idx];
                        meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
                        meta.optimistic_rev = meta.optimistic_rev.wrapping_add(1);
                        meta.updated_at_ms = updated_at;
                        meta.invalidated_epoch = s.wildcard_epoch;
                        meta.flags |= FLAG_HAS_DATA | FLAG_HYDRATED;

                        let payload = &mut s.payloads[idx];
                        waiters.append(&mut payload.in_flight_waiters);
                        payload.raw_response = Some(raw_body);
                        payload.raw_body_hash = body_hash;
                        payload.raw_error = None;
                        payload.committed = None;
                        payload.committed_error = None;
                        payload.effective_optimistic = None;
                        payload.record_commit(pack_invalidation_version(
                            s.wildcard_epoch,
                            meta.fetch_gen,
                        ));
                        wake_cb = s.mark_dirty(slot);
                    }
                    HydratedPayload::DomainErr { status, body } => {
                        apply_hydrated_error(
                            s,
                            slot,
                            updated_at,
                            RawErrorWire::Domain { status, body },
                            &mut waiters,
                            &mut wake_cb,
                        );
                    }
                    HydratedPayload::ProtocolErr { status, message } => {
                        apply_hydrated_error(
                            s,
                            slot,
                            updated_at,
                            RawErrorWire::Protocol { status, message },
                            &mut waiters,
                            &mut wake_cb,
                        );
                    }
                }
                applied += 1;
            }
            (applied, wake_cb, waiters)
        });
        run_side_effects(wake, waiters);
        Ok(applied)
    }

    /// Finishes an SSR stream cleanly: clears `FLAG_STREAM_PENDING | FLAG_IS_FETCHING` on every
    /// live slot still waiting for a stream chunk **without** advancing `completed_seq`, wakes all
    /// `in_flight_waiters`, and marks those slots dirty so client hooks automatically fall back to
    /// their `Transport` if the stream truncated or timed out. Returns how many slots were cleared.
    pub fn finish_ssr_stream(&self) -> usize {
        let (cleared, wake, waiters) = self.inner.state.with(|s| {
            let mut waiters = Vec::new();
            let (cleared, wake_cb) = s.release_stream_pending(&mut waiters);
            (cleared, wake_cb, waiters)
        });
        run_side_effects(wake, waiters);
        cleared
    }
}
