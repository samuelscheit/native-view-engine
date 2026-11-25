//! Hybrid Logical Clock (HLC), peer/operation identifiers, and version vectors.
//!
//! All types are zero-dependency, deterministic, and use fixed little-endian binary encodings.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::fmt;

/// Fixed little-endian encoded byte length of [`OpId`] (`peer: u64` + `seq: u64`).
pub const OP_ID_ENCODED_LEN: usize = 16;

/// Fixed little-endian encoded byte length of [`HlcTimestamp`]
/// (`physical_ms: u64` + `logical: u16` + `peer_id: u64`).
pub const HLC_ENCODED_LEN: usize = 18;

/// Unique replica/peer identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct PeerId(pub u64);

impl PeerId {
    /// Reserved deterministic peer ID used for shared initial snapshot seeds.
    pub const SEED: Self = Self(0);

    #[inline]
    pub const fn new(id: u64) -> Self {
        Self(id)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[inline]
    pub const fn to_le_bytes(self) -> [u8; 8] {
        self.0.to_le_bytes()
    }

    #[inline]
    pub const fn from_le_bytes(bytes: [u8; 8]) -> Self {
        Self(u64::from_le_bytes(bytes))
    }
}

impl fmt::Display for PeerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "peer:{}", self.0)
    }
}

/// Globally unique operation identifier `(peer, seq)` with a strict total order
/// `(seq, peer)` suitable for Lamport/RGA tie-breaking.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct OpId {
    pub peer: PeerId,
    pub seq: u64,
}

impl OpId {
    pub const ZERO: Self = Self {
        peer: PeerId(0),
        seq: 0,
    };

    #[inline]
    pub const fn new(peer: PeerId, seq: u64) -> Self {
        Self { peer, seq }
    }

    #[inline]
    pub fn to_le_bytes(self) -> [u8; OP_ID_ENCODED_LEN] {
        let mut out = [0u8; OP_ID_ENCODED_LEN];
        out[0..8].copy_from_slice(&self.peer.0.to_le_bytes());
        out[8..16].copy_from_slice(&self.seq.to_le_bytes());
        out
    }

    #[inline]
    pub fn from_le_bytes(bytes: [u8; OP_ID_ENCODED_LEN]) -> Self {
        let peer = u64::from_le_bytes(bytes[0..8].try_into().expect("8-byte peer slice"));
        let seq = u64::from_le_bytes(bytes[8..16].try_into().expect("8-byte seq slice"));
        Self {
            peer: PeerId(peer),
            seq,
        }
    }
}

impl Ord for OpId {
    #[inline]
    fn cmp(&self, other: &Self) -> Ordering {
        (self.seq, self.peer).cmp(&(other.seq, other.peer))
    }
}

impl PartialOrd for OpId {
    #[inline]
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for OpId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}@{}", self.seq, self.peer.0)
    }
}

/// Hybrid Logical Clock timestamp with strict total order `(physical_ms, logical, peer_id)`
/// and 18-byte fixed little-endian binary encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct HlcTimestamp {
    pub physical_ms: u64,
    pub logical: u16,
    pub peer_id: u64,
}

impl HlcTimestamp {
    pub const ZERO: Self = Self {
        physical_ms: 0,
        logical: 0,
        peer_id: 0,
    };

    #[inline]
    pub const fn new(physical_ms: u64, logical: u16, peer_id: u64) -> Self {
        Self {
            physical_ms,
            logical,
            peer_id,
        }
    }

    #[inline]
    pub fn encode_le(self) -> [u8; HLC_ENCODED_LEN] {
        let mut out = [0u8; HLC_ENCODED_LEN];
        out[0..8].copy_from_slice(&self.physical_ms.to_le_bytes());
        out[8..10].copy_from_slice(&self.logical.to_le_bytes());
        out[10..18].copy_from_slice(&self.peer_id.to_le_bytes());
        out
    }

    #[inline]
    pub fn decode_le(bytes: [u8; HLC_ENCODED_LEN]) -> Self {
        let physical_ms = u64::from_le_bytes(bytes[0..8].try_into().expect("8-byte physical_ms"));
        let logical = u16::from_le_bytes(bytes[8..10].try_into().expect("2-byte logical"));
        let peer_id = u64::from_le_bytes(bytes[10..18].try_into().expect("8-byte peer_id"));
        Self {
            physical_ms,
            logical,
            peer_id,
        }
    }

    #[inline]
    pub fn decode_slice(bytes: &[u8]) -> Option<Self> {
        let arr: [u8; HLC_ENCODED_LEN] = bytes.try_into().ok()?;
        Some(Self::decode_le(arr))
    }
}

/// Per-peer Hybrid Logical Clock generator enforcing strict monotonicity even under
/// backward wall-clock jumps, rapid same-millisecond ticks, and skewed remote clocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HlcClock {
    peer_id: PeerId,
    last_physical_ms: u64,
    last_logical: u16,
}

impl HlcClock {
    #[inline]
    pub const fn new(peer_id: PeerId) -> Self {
        Self {
            peer_id,
            last_physical_ms: 0,
            last_logical: 0,
        }
    }

    #[inline]
    pub const fn with_initial(peer_id: PeerId, physical_ms: u64) -> Self {
        Self {
            peer_id,
            last_physical_ms: physical_ms,
            last_logical: 0,
        }
    }

    #[inline]
    pub const fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    #[inline]
    pub const fn last_timestamp(&self) -> HlcTimestamp {
        HlcTimestamp {
            physical_ms: self.last_physical_ms,
            logical: self.last_logical,
            peer_id: self.peer_id.0,
        }
    }

    /// Generates a new strictly monotonic timestamp for a local event at wall clock `now_ms`.
    pub fn tick(&mut self, now_ms: u64) -> HlcTimestamp {
        if now_ms > self.last_physical_ms {
            self.last_physical_ms = now_ms;
            self.last_logical = 0;
        } else {
            self.bump_logical_from(self.last_physical_ms, self.last_logical);
        }
        self.last_timestamp()
    }

    /// Updates the local clock upon observing `remote` at wall clock `now_ms`, returning a
    /// timestamp strictly greater than both the previous local timestamp and `remote`.
    pub fn receive(&mut self, now_ms: u64, remote: HlcTimestamp) -> HlcTimestamp {
        let max_phys = now_ms.max(self.last_physical_ms).max(remote.physical_ms);
        if max_phys == self.last_physical_ms && max_phys == remote.physical_ms {
            let max_log = self.last_logical.max(remote.logical);
            self.bump_logical_from(max_phys, max_log);
        } else if max_phys == self.last_physical_ms {
            self.bump_logical_from(self.last_physical_ms, self.last_logical);
        } else if max_phys == remote.physical_ms {
            self.bump_logical_from(remote.physical_ms, remote.logical);
        } else {
            self.last_physical_ms = max_phys;
            self.last_logical = 0;
        }
        self.last_timestamp()
    }

    #[inline]
    fn bump_logical_from(&mut self, base_phys: u64, base_logical: u16) {
        if base_logical == u16::MAX {
            self.last_physical_ms = base_phys.saturating_add(1);
            self.last_logical = 0;
        } else {
            self.last_physical_ms = base_phys;
            self.last_logical = base_logical + 1;
        }
    }
}

/// Causal frontier mapping each [`PeerId`] to its highest observed sequence number.
///
/// Backed by a `BTreeMap` for canonical deterministic iteration and wire encoding.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct VersionVector {
    entries: BTreeMap<PeerId, u64>,
}

impl VersionVector {
    #[inline]
    pub fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
        }
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    #[inline]
    pub fn get(&self, peer: PeerId) -> u64 {
        self.entries.get(&peer).copied().unwrap_or(0)
    }

    /// Advances `peer`'s frontier to at least `seq`. Returns `true` if the frontier grew.
    #[inline]
    pub fn set_max(&mut self, peer: PeerId, seq: u64) -> bool {
        if seq == 0 {
            return false;
        }
        let slot = self.entries.entry(peer).or_insert(0);
        if seq > *slot {
            *slot = seq;
            true
        } else {
            false
        }
    }

    /// Records `op` as observed in this version vector.
    #[inline]
    pub fn observe(&mut self, op: OpId) -> bool {
        self.set_max(op.peer, op.seq)
    }

    /// Returns `true` iff `op.seq > 0` and `op` has already been observed.
    #[inline]
    pub fn contains(&self, op: OpId) -> bool {
        op.seq > 0 && self.get(op.peer) >= op.seq
    }

    /// Pointwise maximum merge (commutative, associative, idempotent).
    pub fn merge(&mut self, other: &Self) -> bool {
        let mut changed = false;
        for (&peer, &seq) in &other.entries {
            if self.set_max(peer, seq) {
                changed = true;
            }
        }
        changed
    }

    /// Returns `true` iff `self[p] >= other[p]` for every peer `p`.
    pub fn dominates(&self, other: &Self) -> bool {
        other
            .entries
            .iter()
            .all(|(&peer, &seq)| self.get(peer) >= seq)
    }

    #[inline]
    pub fn iter(&self) -> impl Iterator<Item = (PeerId, u64)> + '_ {
        self.entries.iter().map(|(&p, &s)| (p, s))
    }
}
