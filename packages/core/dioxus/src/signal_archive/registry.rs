//! App/surface-scoped `SignalRegistry` for registration, snapshotting, `MmapOffsetArena`
//! persistence, and schema-safe cold hydration.

use nve_abi::hash::HashSet;
use std::collections::BTreeMap;

use dioxus_signals::{ReadableExt, Signal, WritableExt};
use nve_engine::{NativeStateStore, StateSaveOutcome};

use super::codec::{SignalArchive, SignalTableSnapshot};
use super::signal_codec::SignalCodec;
use super::{reconcile_signal_entry, signal_slot_id, SignalArchiveError, SignalValue};
use nve_interop::reconcile::DivergenceKind;

/// Summary returned by [`SignalRegistry::hydrate_from_table`] describing exact schema-evolution
/// behavior on cold start.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HydrationReport {
    /// Registered slots restored successfully from the archive.
    pub hydrated_slots: usize,
    /// Archived slots absent from the current app version (safely ignored).
    pub skipped_unknown_slots: usize,
    /// Archived slots whose stored value type did not match the current slot's type (safely skipped).
    pub skipped_type_mismatches: usize,
    /// Newly added registered slots absent from the archive (retained their default values).
    pub untouched_new_slots: usize,
    /// Schema version recorded in the hydrated archive.
    pub source_schema_version: u32,
}

struct RegisteredSlot {
    slot_id: u64,
    component_path: String,
    signal_name: String,
    read: Box<dyn Fn() -> SignalValue>,
    write: Box<dyn Fn(&SignalValue) -> bool>,
}

/// App/surface-scoped signal table registry for one-time signal registration, deterministic
/// full & incremental snapshotting, WAL delta streaming, and schema-tolerant cold hydration.
pub struct SignalRegistry {
    schema_version: u32,
    slots: BTreeMap<u64, RegisteredSlot>,
    last_persisted: SignalTableSnapshot,
}

impl Default for SignalRegistry {
    fn default() -> Self {
        Self::new(1)
    }
}

impl SignalRegistry {
    pub fn new(schema_version: u32) -> Self {
        Self {
            schema_version,
            slots: BTreeMap::new(),
            last_persisted: SignalTableSnapshot::new(schema_version),
        }
    }

    #[inline]
    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    #[inline]
    pub fn set_schema_version(&mut self, schema_version: u32) {
        self.schema_version = schema_version;
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.slots.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.slots.is_empty()
    }

    /// Registers a typed signal backed by custom getter/setter closures.
    pub fn register_slot<T: SignalCodec + 'static>(
        &mut self,
        component_path: impl Into<String>,
        signal_name: impl Into<String>,
        read: impl Fn() -> T + 'static,
        write: impl Fn(T) + 'static,
    ) -> u64 {
        let component_path = component_path.into();
        let signal_name = signal_name.into();
        let slot_id = signal_slot_id(&component_path, &signal_name);
        self.slots.insert(
            slot_id,
            RegisteredSlot {
                slot_id,
                component_path,
                signal_name,
                read: Box::new(move || read().to_signal_value()),
                write: Box::new(move |val| {
                    if let Some(decoded) = T::from_signal_value(val) {
                        write(decoded);
                        true
                    } else {
                        false
                    }
                }),
            },
        );
        slot_id
    }

    /// Registers a live `dioxus_signals::Signal<T>` directly for snapshot/WAL persistence and
    /// schema-safe cold hydration.
    pub fn register_signal<T: SignalCodec + Clone + 'static>(
        &mut self,
        component_path: impl Into<String>,
        signal_name: impl Into<String>,
        signal: Signal<T>,
    ) -> u64 {
        self.register_slot(
            component_path,
            signal_name,
            move || signal.cloned(),
            move |val| {
                let mut sig = signal;
                sig.set(val);
            },
        )
    }

    /// Unregisters a slot by `(component_path, signal_name)`.
    pub fn unregister(&mut self, component_path: &str, signal_name: &str) -> bool {
        self.slots
            .remove(&signal_slot_id(component_path, signal_name))
            .is_some()
    }

    /// Returns `(slot_id, component_path, signal_name)` for all registered slots.
    pub fn registered_slots(&self) -> Vec<(u64, &str, &str)> {
        self.slots
            .values()
            .map(|s| (s.slot_id, s.component_path.as_str(), s.signal_name.as_str()))
            .collect()
    }

    /// Captures all currently registered signals into a deterministic [`SignalTableSnapshot`].
    pub fn capture(&self) -> SignalTableSnapshot {
        let mut snap = SignalTableSnapshot::new(self.schema_version);
        for (&slot_id, entry) in &self.slots {
            snap.set(slot_id, (entry.read)());
        }
        snap
    }

    /// Encodes a full `NVE_SIG1` snapshot of all registered signals for [`NativeStateStore`].
    pub fn encode_full(&self) -> Result<Vec<u8>, SignalArchiveError> {
        self.capture().encode()
    }

    /// Captures current signal state and marks it as the clean baseline for future incremental diffs.
    pub fn mark_baseline(&mut self) -> SignalTableSnapshot {
        let current = self.capture();
        self.last_persisted = current.clone();
        current
    }

    /// Computes dirty slots since the last persisted baseline and returns `Some(delta_bytes)` if
    /// any registered slot changed, advancing the baseline.
    pub fn encode_delta_since_last(&mut self) -> Result<Option<Vec<u8>>, SignalArchiveError> {
        let current = self.capture();
        let delta = current.diff_since(&self.last_persisted);
        if delta.is_empty() {
            return Ok(None);
        }
        let encoded = delta.encode()?;
        self.last_persisted = current;
        Ok(Some(encoded))
    }

    /// Crash-atomically persists the current signal table into `store` (`MmapOffsetArena`) when
    /// any registered signal changed since the last persisted baseline.
    pub fn persist_if_dirty_to_store(
        &mut self,
        store: &mut NativeStateStore,
    ) -> Result<Option<StateSaveOutcome>, SignalArchiveError> {
        let current = self.capture();
        if current.diff_since(&self.last_persisted).is_empty() {
            return Ok(None);
        }
        let outcome = SignalArchive::save_to_store(store, &current)?;
        self.last_persisted = current;
        Ok(Some(outcome))
    }

    /// Unconditionally persists the full signal table into `store` and refreshes the diff baseline.
    pub fn persist_to_store(
        &mut self,
        store: &mut NativeStateStore,
    ) -> Result<StateSaveOutcome, SignalArchiveError> {
        let current = self.capture();
        let outcome = SignalArchive::save_to_store(store, &current)?;
        self.last_persisted = current;
        Ok(outcome)
    }

    /// Hydrates all matching registered signals from `table` with strict schema-evolution safety:
    /// unknown/removed slots and re-typed slots are safely skipped instead of failing boot.
    pub fn hydrate_from_table(&mut self, table: &SignalTableSnapshot) -> HydrationReport {
        let mut report = HydrationReport {
            source_schema_version: table.schema_version(),
            ..Default::default()
        };
        let mut seen_registered = HashSet::with_capacity_and_hasher(self.slots.len(), Default::default());

        for (slot_id, value) in table.iter() {
            if let Some(entry) = self.slots.get(&slot_id) {
                seen_registered.insert(slot_id);
                if (entry.write)(value) {
                    report.hydrated_slots += 1;
                } else {
                    report.skipped_type_mismatches += 1;
                }
            } else {
                report.skipped_unknown_slots += 1;
            }
        }

        report.untouched_new_slots = self.slots.len().saturating_sub(seen_registered.len());
        self.last_persisted = self.capture();
        report
    }

    /// Reconciles archived hot-reload state against prior `base_defaults` and the currently
    /// registered live code defaults using [`reconcile_signal_entry`].
    ///
    /// User-mutated slots are preserved while untouched slots adopt updated code defaults.
    pub fn hydrate_hot_reload(
        &mut self,
        base_defaults: &SignalTableSnapshot,
        archived_local: &SignalTableSnapshot,
    ) -> HydrationReport {
        let mut report = HydrationReport {
            source_schema_version: archived_local.schema_version(),
            ..Default::default()
        };
        let mut seen_registered =
            HashSet::with_capacity_and_hasher(self.slots.len(), Default::default());

        for (slot_id, archived_val) in archived_local.iter() {
            if let Some(entry) = self.slots.get(&slot_id) {
                seen_registered.insert(slot_id);
                let incoming_default = (entry.read)();
                if let Some(base_default) = base_defaults.get(slot_id) {
                    let decision = reconcile_signal_entry(
                        base_default.clone(),
                        archived_val.clone(),
                        incoming_default,
                    );
                    match decision.kind {
                        DivergenceKind::LocalOnlyDirty | DivergenceKind::Conflict => {
                            if (entry.write)(&decision.resolved) {
                                report.hydrated_slots += 1;
                            } else {
                                report.skipped_type_mismatches += 1;
                            }
                        }
                        DivergenceKind::Clean
                        | DivergenceKind::IncomingOnly
                        | DivergenceKind::Converged => {
                            // Live signal already holds the winning incoming/converged value.
                            report.hydrated_slots += 1;
                        }
                    }
                } else if (entry.write)(archived_val) {
                    report.hydrated_slots += 1;
                } else {
                    report.skipped_type_mismatches += 1;
                }
            } else {
                report.skipped_unknown_slots += 1;
            }
        }

        report.untouched_new_slots = self.slots.len().saturating_sub(seen_registered.len());
        self.last_persisted = self.capture();
        report
    }

    /// Decodes a full `NVE_SIG1` byte slice and hydrates matching registered signals.
    pub fn hydrate_from_bytes(
        &mut self,
        bytes: &[u8],
    ) -> Result<HydrationReport, SignalArchiveError> {
        let table = SignalTableSnapshot::decode(bytes)?;
        Ok(self.hydrate_from_table(&table))
    }

    /// Loads state from a [`NativeStateStore`] (`MmapOffsetArena`) and hydrates all matching
    /// registered signals deterministically.
    pub fn hydrate_from_store(
        &mut self,
        store: &NativeStateStore,
    ) -> Result<HydrationReport, SignalArchiveError> {
        let table = SignalArchive::load_from_store(store)?;
        Ok(self.hydrate_from_table(&table))
    }
}
