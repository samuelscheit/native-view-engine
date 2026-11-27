//! Compact zero-`serde_json` `NVEQ` v2 dehydration, per-boundary delta streaming cursors,
//! monotonic fill-only hydration, and truncated-stream recovery for [`crate::QueryClient`].

mod codec;

use std::collections::HashMap;

use nve_hash::xxh64;
use nve_rpc::{MaybeSend, MaybeSync, Tag64, Transport};

pub use self::codec::{DehydrateError, NVEQ_VERSION};
use self::codec::{
    decode_hydration_entries, encode_hydration_entries, DehydrateEntryRef, DehydratePayloadRef,
    HydratedPayload,
};
use crate::engine::{query_key_hash, run_side_effects, QueryClient, BODY_HASH_SEED};
use crate::slab::{
    pack_invalidation_version, QueryMeta, QueryPayload, QuerySlotId, RawErrorWire, FLAG_HAS_DATA,
    FLAG_HAS_ERROR, FLAG_HYDRATED, FLAG_IS_FETCHING, FLAG_IS_INVALIDATED, FLAG_STREAM_PENDING,
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
    let meta = &mut s.metas[idx];
    meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
    meta.optimistic_rev = meta.optimistic_rev.wrapping_add(1);
    meta.updated_at_ms = updated_at;
    meta.invalidated_epoch = s.wildcard_epoch;
    meta.flags |= FLAG_HAS_ERROR | FLAG_HYDRATED;
    meta.flags &= !(FLAG_HAS_DATA | FLAG_IS_INVALIDATED | FLAG_IS_FETCHING | FLAG_STREAM_PENDING);

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

#[inline]
fn settled_payload_ref<'a>(
    meta: QueryMeta,
    payload: &'a QueryPayload,
) -> Option<DehydratePayloadRef<'a>> {
    if meta.has_data()
        && !meta.has_error()
        && let Some(raw) = payload.raw_response.as_deref()
    {
        return Some(DehydratePayloadRef::Ok(raw));
    }
    if meta.has_error()
        && let Some(raw_err) = payload.raw_error.as_ref()
    {
        return Some(match raw_err {
            RawErrorWire::Domain { status, body } => DehydratePayloadRef::DomainErr {
                status: *status,
                body: body.as_ref(),
            },
            RawErrorWire::Protocol { status, message } => DehydratePayloadRef::ProtocolErr {
                status: *status,
                message: message.as_str(),
            },
        });
    }
    if meta.has_data()
        && let Some(raw) = payload.raw_response.as_deref()
    {
        return Some(DehydratePayloadRef::Ok(raw));
    }
    None
}

#[inline]
fn compute_age_ms(now_ms: u64, updated_at_ms: i64) -> u32 {
    let now_i64 = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let elapsed = now_i64.saturating_sub(updated_at_ms).max(0) as u64;
    u32::try_from(elapsed).unwrap_or(u32::MAX)
}

impl<T: Transport, C: Clone + MaybeSend + MaybeSync + 'static> QueryClient<T, C> {
    /// Dehydrates all live settled entries (`Ok`, `DomainErr`, `ProtocolErr`) into a compact
    /// `NVEQ` v2 binary blob for SSR -> client hydration.
    pub fn dehydrate_bytes(&self) -> Result<Vec<u8>, DehydrateError> {
        let now_ms = self.inner.clock.now_ms();
        self.inner.state.with(|s| {
            let mut tag_bufs: Vec<Vec<Tag64>> = Vec::new();
            let mut slot_indices: Vec<usize> = Vec::new();
            for (idx, &meta) in s.metas.iter().enumerate() {
                if meta.is_free() {
                    continue;
                }
                if settled_payload_ref(meta, &s.payloads[idx]).is_some() {
                    let mut tags = Vec::new();
                    s.tags.collect_tags_for_slot(meta.first_tag_edge, &mut tags);
                    tag_bufs.push(tags);
                    slot_indices.push(idx);
                }
            }
            let refs: Vec<DehydrateEntryRef<'_>> = slot_indices
                .iter()
                .zip(tag_bufs.iter())
                .filter_map(|(&idx, tags)| {
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    let wire = settled_payload_ref(meta, payload)?;
                    Some(DehydrateEntryRef {
                        proc_id: meta.proc_id,
                        age_ms: compute_age_ms(now_ms, meta.updated_at_ms),
                        subscribed_all: (meta.flags & FLAG_SUBSCRIBED_ALL) != 0,
                        tags: tags.as_slice(),
                        encoded_args: &payload.encoded_args,
                        payload: wire,
                    })
                })
                .collect();
            encode_hydration_entries(&refs)
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
            let mut tag_bufs: Vec<Vec<Tag64>> = Vec::new();
            let mut selected: Vec<(QuerySlotId, u32, u32)> = Vec::new();
            for &handle in handles {
                if !s.is_live(handle) {
                    continue;
                }
                if selected.iter().any(|&(slot, _, _)| slot == handle.slot) {
                    continue;
                }
                let idx = handle.slot.index();
                let meta = s.metas[idx];
                let payload = &s.payloads[idx];
                if settled_payload_ref(meta, payload).is_none() {
                    continue;
                }
                let rev = (meta.slot_gen, payload.completed_seq);
                if cursor.emitted.get(&handle.slot).copied() == Some(rev) {
                    continue;
                }
                let mut tags = Vec::new();
                s.tags.collect_tags_for_slot(meta.first_tag_edge, &mut tags);
                tag_bufs.push(tags);
                selected.push((handle.slot, rev.0, rev.1));
            }
            let refs: Vec<DehydrateEntryRef<'_>> = selected
                .iter()
                .zip(tag_bufs.iter())
                .filter_map(|(&(slot, _, _), tags)| {
                    let idx = slot.index();
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    let wire = settled_payload_ref(meta, payload)?;
                    Some(DehydrateEntryRef {
                        proc_id: meta.proc_id,
                        age_ms: compute_age_ms(now_ms, meta.updated_at_ms),
                        subscribed_all: (meta.flags & FLAG_SUBSCRIBED_ALL) != 0,
                        tags: tags.as_slice(),
                        encoded_args: &payload.encoded_args,
                        payload: wire,
                    })
                })
                .collect();
            let encoded = encode_hydration_entries(&refs)?;
            for (slot, slot_gen, completed_seq) in selected {
                cursor.emitted.insert(slot, (slot_gen, completed_seq));
            }
            Ok(encoded)
        })
    }

    /// Dehydrates `StreamPending` markers for live `handles` that have not yet settled (for `seq=0`
    /// mid-stream client boot so client hooks suspend on the stream rather than firing duplicate
    /// RPCs).
    pub fn dehydrate_pending_handles(
        &self,
        handles: &[QueryHandle],
    ) -> Result<Vec<u8>, DehydrateError> {
        self.inner.state.with(|s| {
            let mut tag_bufs: Vec<Vec<Tag64>> = Vec::new();
            let mut selected: Vec<QuerySlotId> = Vec::new();
            for &handle in handles {
                if !s.is_live(handle) || selected.contains(&handle.slot) {
                    continue;
                }
                let idx = handle.slot.index();
                let meta = s.metas[idx];
                if meta.has_data() || meta.has_error() {
                    continue;
                }
                let mut tags = Vec::new();
                s.tags.collect_tags_for_slot(meta.first_tag_edge, &mut tags);
                tag_bufs.push(tags);
                selected.push(handle.slot);
            }
            let refs: Vec<DehydrateEntryRef<'_>> = selected
                .iter()
                .zip(tag_bufs.iter())
                .map(|(&slot, tags)| {
                    let idx = slot.index();
                    let meta = s.metas[idx];
                    let payload = &s.payloads[idx];
                    DehydrateEntryRef {
                        proc_id: meta.proc_id,
                        age_ms: 0,
                        subscribed_all: (meta.flags & FLAG_SUBSCRIBED_ALL) != 0,
                        tags: tags.as_slice(),
                        encoded_args: &payload.encoded_args,
                        payload: DehydratePayloadRef::StreamPending,
                    }
                })
                .collect();
            encode_hydration_entries(&refs)
        })
    }

    /// Fill-only monotonic hydration from an initial `NVEQ` v2 snapshot (`seq=0`). Never overwrites
    /// slots that already have committed data/errors or local client activity. Returns the number
    /// of entries actually applied.
    pub fn hydrate_bytes(&self, bytes: &[u8]) -> Result<usize, DehydrateError> {
        self.hydrate_inner(bytes, false)
    }

    /// Fill-only monotonic hydration for an out-of-order streaming `NVEQ` v2 chunk (`seq > 0`).
    /// In addition to the fill-only slot rule, refuses to create absent slots if `wildcard_epoch > 0`
    /// (client called `invalidate_all()` after boot). Returns the number of entries applied.
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
                    if is_stream_chunk && s.wildcard_epoch > 0 {
                        continue;
                    }
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
                        let meta = &mut s.metas[idx];
                        meta.flags |= FLAG_IS_FETCHING | FLAG_STREAM_PENDING;
                        wake_cb = s.mark_dirty(slot);
                    }
                    HydratedPayload::Ok(raw_body) => {
                        let body_hash = xxh64(&raw_body, BODY_HASH_SEED);
                        let meta = &mut s.metas[idx];
                        meta.fetch_gen = meta.fetch_gen.wrapping_add(1).max(1);
                        meta.optimistic_rev = meta.optimistic_rev.wrapping_add(1);
                        meta.updated_at_ms = updated_at;
                        meta.invalidated_epoch = s.wildcard_epoch;
                        meta.flags |= FLAG_HAS_DATA | FLAG_HYDRATED;
                        meta.flags &= !(FLAG_HAS_ERROR
                            | FLAG_IS_INVALIDATED
                            | FLAG_IS_FETCHING
                            | FLAG_STREAM_PENDING);

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
            let mut cleared = 0usize;
            let mut waiters = Vec::new();
            let mut wake_cb: Option<WakeCallback> = None;
            for idx in 0..s.metas.len() {
                let meta = &mut s.metas[idx];
                if !meta.is_free() && meta.is_stream_pending() {
                    meta.flags &= !(FLAG_STREAM_PENDING | FLAG_IS_FETCHING);
                    waiters.append(&mut s.payloads[idx].in_flight_waiters);
                    wake_cb = s.mark_dirty(QuerySlotId(idx as u32));
                    cleared += 1;
                }
            }
            (cleared, wake_cb, waiters)
        });
        run_side_effects(wake, waiters);
        cleared
    }
}
