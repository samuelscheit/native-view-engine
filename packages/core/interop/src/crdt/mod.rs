//! Zero-dependency, zero-JSON binary CRDT primitives and `NVC1` delta wire protocol.

pub mod hlc;
pub mod lww;
pub mod orset;
pub mod rga;
pub mod wire;

pub use hlc::{
    HLC_ENCODED_LEN, HlcClock, HlcTimestamp, OP_ID_ENCODED_LEN, OpId, PeerId, VersionVector,
};
pub use lww::{GCounter, LwwRegister, PnCounter};
pub use orset::{OrSet, OrSetDelta};
pub use rga::{RgaDelta, RgaList, RgaNode};
pub use wire::{
    CRDT_WIRE_MAGIC, CRDT_WIRE_MIN_LEN, CRDT_WIRE_VERSION, CrdtDeltaPacket, CrdtRecord, CrdtValue,
    CrdtWireError, MAX_FIELD_BYTES, MAX_RECORDS, MAX_VV_ENTRIES, MAX_WIRE_BYTES, fnv1a_64,
};
