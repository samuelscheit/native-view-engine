//! `NveCrdtDelta` sync adapter bridging [`super::StateReconciler`] with `nve-interop`'s binary
//! `NveCrdtDelta` (`XXH64`) wire protocol so offline optimistic UI edits reconcile
//! deterministically against incoming peer state.

mod crdt_adapter {
    use std::collections::BTreeMap;

    use super::super::{
        ConflictPolicy, LogicalClock, ReconcileBatch, StateReconciler,
    };

    use crate::crdt::{
        CrdtRecord, CrdtValue, CrdtWireError, HlcClock, HlcTimestamp, LwwRegister, NveCrdtDelta,
        OpId, PeerId, VersionVector, crdt_xxh64,
    };

    const MAX_PACKED_PHYSICAL_MS: u64 = (1u64 << 48) - 1;

    /// Projects an [`HlcTimestamp`] into a strictly order-preserving [`LogicalClock`].
    ///
    /// Packs `(physical_ms << 16) | logical` into `counter` (lossless for all timestamps up to
    /// ~8,925 years past epoch) and maps the low 32 bits of `peer_id` as deterministic tie-breaker.
    #[inline]
    pub const fn hlc_to_logical_clock(ts: HlcTimestamp) -> LogicalClock {
        let clamped_ms = if ts.physical_ms > MAX_PACKED_PHYSICAL_MS {
            MAX_PACKED_PHYSICAL_MS
        } else {
            ts.physical_ms
        };
        LogicalClock::at((clamped_ms << 16) | (ts.logical as u64), ts.peer_id as u32)
    }

    /// Bridges [`StateReconciler<String, Vec<u8>>`] with `nve-interop`'s CRDT `LwwRegister`,
    /// `HlcClock`, `VersionVector`, and `NVC1` binary delta wire format.
    #[derive(Debug, Clone)]
    pub struct CrdtReconcileAdapter {
        namespace: u16,
        peer_id: PeerId,
        hlc: HlcClock,
        next_seq: u64,
        version_vector: VersionVector,
        reconciler: StateReconciler<String, Vec<u8>>,
        registers: BTreeMap<String, LwwRegister<Vec<u8>>>,
        entity_to_key: BTreeMap<u64, String>,
        pending_timestamps: BTreeMap<String, HlcTimestamp>,
    }

    impl CrdtReconcileAdapter {
        /// Creates a new adapter scoped to `peer_id` and CRDT `namespace`.
        pub fn new(peer_id: PeerId, namespace: u16) -> Self {
            Self {
                namespace,
                peer_id,
                hlc: HlcClock::new(peer_id),
                next_seq: 0,
                version_vector: VersionVector::new(),
                reconciler: StateReconciler::new(peer_id.as_u64() as u32),
                registers: BTreeMap::new(),
                entity_to_key: BTreeMap::new(),
                pending_timestamps: BTreeMap::new(),
            }
        }

        #[inline]
        pub const fn namespace(&self) -> u16 {
            self.namespace
        }

        #[inline]
        pub const fn peer_id(&self) -> PeerId {
            self.peer_id
        }

        #[inline]
        pub fn version_vector(&self) -> &VersionVector {
            &self.version_vector
        }

        #[inline]
        pub fn reconciler(&self) -> &StateReconciler<String, Vec<u8>> {
            &self.reconciler
        }

        #[inline]
        pub fn dirty_count(&self) -> usize {
            self.reconciler.dirty_count()
        }

        /// Registers `key` so incoming `NveCrdtDelta` `LwwSet` entity hashes resolve back to `key`.
        pub fn register_key(&mut self, key: impl Into<String>) -> u64 {
            let key = key.into();
            let entity_id = crdt_xxh64(key.as_bytes());
            self.entity_to_key.entry(entity_id).or_insert(key);
            entity_id
        }

        /// Seeds a synchronized baseline value and its authoritative CRDT timestamp.
        pub fn seed_synced(
            &mut self,
            key: impl Into<String>,
            value: Vec<u8>,
            timestamp: HlcTimestamp,
        ) {
            let key = key.into();
            self.register_key(key.clone());
            self.reconciler.seed_baseline(key.clone(), value.clone());
            let reg = self
                .registers
                .entry(key)
                .or_insert_with(|| LwwRegister::new(value.clone(), timestamp));
            let _ = reg.set(value, timestamp);
        }

        /// Records an offline/optimistic local mutation at wall-clock `now_ms`.
        pub fn write_optimistic(
            &mut self,
            key: impl Into<String>,
            value: Vec<u8>,
            now_ms: u64,
        ) -> (LogicalClock, HlcTimestamp) {
            let key = key.into();
            self.register_key(key.clone());
            let ts = self.hlc.tick(now_ms);
            let logical = hlc_to_logical_clock(ts);
            self.next_seq = self.next_seq.saturating_add(1);
            self.version_vector.set_max(self.peer_id, self.next_seq);

            self.reconciler
                .record_local_at(key.clone(), value.clone(), logical);
            match self.registers.get_mut(&key) {
                Some(reg) => {
                    let _ = reg.set(value, ts);
                }
                None => {
                    self.registers
                        .insert(key.clone(), LwwRegister::new(value, ts));
                }
            }
            if self.reconciler.dirty_keys().any(|k| k == &key) {
                self.pending_timestamps.insert(key, ts);
            } else {
                self.pending_timestamps.remove(&key);
            }
            (logical, ts)
        }

        /// Returns the current effective local value for `key`.
        pub fn value(&self, key: &str) -> Option<&[u8]> {
            self.reconciler
                .local_value(&key.to_owned())
                .map(Vec::as_slice)
        }

        /// Returns the underlying CRDT [`LwwRegister`] for `key`, if present.
        pub fn register(&self, key: &str) -> Option<&LwwRegister<Vec<u8>>> {
            self.registers.get(key)
        }

        /// Acknowledges that `value` for `key` was durably persisted upstream.
        ///
        /// Clears the dirty flag only when the current local edit still equals `value`.
        pub fn ack_synced(&mut self, key: &str, value: &[u8]) -> bool {
            let k = key.to_owned();
            let cleared = self.reconciler.ack_synced(&k, &value.to_vec());
            if cleared {
                self.pending_timestamps.remove(key);
            }
            cleared
        }

        /// Encodes all pending dirty optimistic mutations into an [`NveCrdtDelta`] binary packet.
        ///
        /// Returns `Ok(None)` when there are no dirty keys.
        pub fn export_crdt_delta(&mut self) -> Result<Option<Vec<u8>>, CrdtWireError> {
            let mut entries: Vec<(String, Vec<u8>)> = self
                .reconciler
                .dirty_entries()
                .map(|(k, v, _)| (k.clone(), v.clone()))
                .collect();
            if entries.is_empty() {
                return Ok(None);
            }
            entries.sort_by(|a, b| a.0.cmp(&b.0));

            let mut records = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                let entity_id = crdt_xxh64(key.as_bytes());
                let timestamp = self
                    .pending_timestamps
                    .get(&key)
                    .copied()
                    .unwrap_or_else(|| self.hlc.last_timestamp());
                self.next_seq = self.next_seq.saturating_add(1);
                let op_id = OpId::new(self.peer_id, self.next_seq);
                self.version_vector.observe(op_id);
                records.push(CrdtRecord::LwwSet {
                    namespace: self.namespace,
                    entity_id,
                    field_id: 0,
                    op_id,
                    timestamp,
                    value: CrdtValue::Bytes(value),
                });
            }

            let packet =
                NveCrdtDelta::new(self.peer_id, self.version_vector.clone(), records);
            packet.encode().map(Some)
        }

        /// Decodes and reconciles an incoming [`NveCrdtDelta`] packet against local optimistic
        /// state using `policy`.
        pub fn apply_crdt_delta(
            &mut self,
            bytes: &[u8],
            now_ms: u64,
            policy: ConflictPolicy,
        ) -> Result<ReconcileBatch<String, Vec<u8>>, CrdtWireError> {
            let packet = NveCrdtDelta::decode(bytes)?;
            let mut incoming: Vec<(String, Vec<u8>, Option<LogicalClock>)> = Vec::new();
            let mut incoming_ts: BTreeMap<String, HlcTimestamp> = BTreeMap::new();

            for record in packet.records {
                if let CrdtRecord::LwwSet {
                    namespace,
                    entity_id,
                    op_id,
                    timestamp,
                    value,
                    ..
                } = record
                {
                    if namespace != self.namespace {
                        continue;
                    }
                    if self.version_vector.contains(op_id) {
                        continue;
                    }
                    self.version_vector.observe(op_id);
                    let _ = self.hlc.receive(now_ms, timestamp);

                    let key = self
                        .entity_to_key
                        .get(&entity_id)
                        .cloned()
                        .unwrap_or_else(|| format!("entity:{entity_id:016x}"));
                    let payload = crdt_value_to_bytes(value);
                    let logical = hlc_to_logical_clock(timestamp);

                    // Coalesce multiple updates to the same key within one packet by highest HLC.
                    let should_use = incoming_ts
                        .get(&key)
                        .is_none_or(|prev| timestamp > *prev);
                    if should_use {
                        incoming_ts.insert(key.clone(), timestamp);
                        incoming.retain(|(k, _, _)| k != &key);
                        incoming.push((key, payload, Some(logical)));
                    }
                }
            }

            self.version_vector.merge(&packet.version_vector);
            let batch = self.reconciler.reconcile_incoming_batch(incoming, policy);

            for decision in &batch.decisions {
                if decision.apply_local {
                    self.pending_timestamps.remove(&decision.key);
                    if let Some(&ts) = incoming_ts.get(&decision.key) {
                        self.registers.insert(
                            decision.key.clone(),
                            LwwRegister::new(decision.resolved.clone(), ts),
                        );
                    }
                } else if !decision.emit_upstream {
                    self.pending_timestamps.remove(&decision.key);
                    if let Some(&ts) = incoming_ts.get(&decision.key)
                        && let Some(reg) = self.registers.get_mut(&decision.key)
                    {
                        let _ = reg.set(decision.resolved.clone(), ts);
                    }
                }
            }

            Ok(batch)
        }
    }

    fn crdt_value_to_bytes(value: CrdtValue) -> Vec<u8> {
        match value {
            CrdtValue::Unit => Vec::new(),
            CrdtValue::Bool(b) => vec![u8::from(b)],
            CrdtValue::U32(v) => v.to_le_bytes().to_vec(),
            CrdtValue::U64(v) => v.to_le_bytes().to_vec(),
            CrdtValue::I64(v) => v.to_le_bytes().to_vec(),
            CrdtValue::Utf8(s) => s.into_bytes(),
            CrdtValue::Bytes(b) => b,
        }
    }
}

pub use crdt_adapter::{CrdtReconcileAdapter, hlc_to_logical_clock};
