//! Session-anchored slot-memory budget for `NVE2` wire decoding.

use crate::id::NodeId;
use crate::store::NodeRow;

use super::WireError;

/// One-time session base allowance for slot-indexed arrays (`NodeStore::nodes`, static dense
/// columns, `ResourceStore::entries`, and sparse `PagedIndex::{dir, pool}`), reset only on
/// `WireDecoder::reset()` or a keyframe against a fresh store — never refilled on delta frames.
pub(crate) const SESSION_BASE_SLOT_BYTES: usize = 16 * 1024 * 1024;
/// Multiplier earning additional slot-memory budget strictly proportional to wire bytes received.
pub(crate) const WIRE_SLOT_MULTIPLIER: usize = 16;
/// Hard per-session ceiling on cumulative slot-array growth across all frames.
pub(crate) const SESSION_MAX_SLOT_BYTES: usize = 256 * 1024 * 1024;
/// Hard upper ceiling on wire `ResourceId` indices (`1 << 20` = 1,048,576 slots), matching the
/// bounded 24-bit `NodeId::MAX_INDEX` space.
pub(crate) const MAX_RESOURCE_SLOTS: u32 = 1 << 20;

/// Session-anchored slot-memory budget preventing both single-frame sparse-index OOM and
/// multi-frame stride amplification across nodes, static dense columns, resources, and sparse
/// `PagedIndex` directories/pages while allowing churned keyframes (`encode_full`) to round-trip.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionBudget {
    started: bool,
    limit_bytes: usize,
    used_bytes: usize,
}

impl SessionBudget {
    pub(crate) fn reset(&mut self) {
        self.started = false;
        self.limit_bytes = 0;
        self.used_bytes = 0;
    }

    /// Updates session budget for `frame`. Returns `true` when the budget started or reset
    /// (first frame or fresh-store keyframe) so the caller can account for any pre-reserved slabs.
    pub(crate) fn begin_frame(&mut self, frame: u64, wire_bytes: usize, fresh_store: bool) -> bool {
        let earned = wire_bytes.saturating_mul(WIRE_SLOT_MULTIPLIER);
        if !self.started || (frame == 1 && fresh_store) {
            self.started = true;
            self.used_bytes = 0;
            self.limit_bytes = SESSION_BASE_SLOT_BYTES
                .saturating_add(earned)
                .min(SESSION_MAX_SLOT_BYTES);
            true
        } else {
            self.limit_bytes = self
                .limit_bytes
                .saturating_add(earned)
                .min(SESSION_MAX_SLOT_BYTES);
            false
        }
    }

    /// Maximum `NodeStore` slot hint a Frame-0 pre-reservation may claim for `wire_bytes` without
    /// exceeding the session slot-memory ceiling before decoding starts.
    #[inline]
    pub(crate) fn max_frame0_node_hint(wire_bytes: usize) -> usize {
        let max_budget = SESSION_BASE_SLOT_BYTES
            .saturating_add(wire_bytes.saturating_mul(WIRE_SLOT_MULTIPLIER))
            .min(SESSION_MAX_SLOT_BYTES);
        (wire_bytes / 4)
            .min(max_budget / size_of::<NodeRow>())
            .min(NodeId::MAX_INDEX as usize)
    }

    #[inline]
    pub(crate) fn remaining(&self) -> usize {
        self.limit_bytes.saturating_sub(self.used_bytes)
    }

    /// Computes and validates the byte cost of growing a slot array from `cur` to `needed`
    /// elements of `unit_bytes` each without mutating `used_bytes`.
    #[inline]
    pub(crate) fn preview_grow(
        &self,
        cur: usize,
        needed: usize,
        unit_bytes: usize,
    ) -> Result<usize, WireError> {
        let Some(delta) = needed.checked_sub(cur) else {
            return Ok(0);
        };
        if delta == 0 {
            return Ok(0);
        }
        let delta_bytes = delta.checked_mul(unit_bytes).ok_or(WireError::Budget)?;
        let next = self
            .used_bytes
            .checked_add(delta_bytes)
            .ok_or(WireError::Budget)?;
        if next > self.limit_bytes {
            return Err(WireError::Budget);
        }
        Ok(delta_bytes)
    }

    /// Commits a previously validated `delta_bytes` slot-growth charge.
    #[inline]
    pub(crate) fn charge(&mut self, delta_bytes: usize) -> Result<(), WireError> {
        let next = self
            .used_bytes
            .checked_add(delta_bytes)
            .ok_or(WireError::Budget)?;
        if next > self.limit_bytes {
            return Err(WireError::Budget);
        }
        self.used_bytes = next;
        Ok(())
    }

    /// Computes an amortized-growth target capacity clamped strictly to what fits inside
    /// `remaining_bytes`, using exact capacity on initial allocation (`cur_len <= 1 || old_cap == 0`)
    /// so Frame-0 bulk pre-sizing allocates exact while incremental growth doubles geometrically
    /// (`O(log N)` reallocations) without ever pushing real heap memory past `SESSION_MAX_SLOT_BYTES`.
    #[inline]
    pub(crate) fn bounded_target_cap(
        old_cap: usize,
        cur_len: usize,
        needed: usize,
        unit_bytes: usize,
        remaining_bytes: usize,
    ) -> Result<(usize, usize), WireError> {
        debug_assert!(unit_bytes > 0);
        if needed <= old_cap {
            return Ok((old_cap, 0));
        }
        let min_delta = (needed - old_cap)
            .checked_mul(unit_bytes)
            .ok_or(WireError::Budget)?;
        if min_delta > remaining_bytes {
            return Err(WireError::Budget);
        }
        let target = if cur_len <= 1 || old_cap == 0 {
            needed
        } else {
            let affordable = old_cap.saturating_add(remaining_bytes / unit_bytes);
            old_cap.saturating_mul(2).max(needed).min(affordable)
        };
        let delta_bytes = (target - old_cap)
            .checked_mul(unit_bytes)
            .ok_or(WireError::Budget)?;
        Ok((target, delta_bytes))
    }

    #[cfg(test)]
    pub(crate) fn with_limit(limit_bytes: usize) -> Self {
        Self {
            started: true,
            limit_bytes,
            used_bytes: 0,
        }
    }
}
