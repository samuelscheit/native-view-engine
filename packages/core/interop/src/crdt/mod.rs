//! Zero-dependency, binary CRDT primitives and `NveCrdtDelta` (`XXH64`) wire protocol.

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
    CrdtRecord, CrdtValue, CrdtWireError, MAX_FIELD_BYTES, MAX_RECORDS, MAX_VV_ENTRIES,
    MAX_WIRE_BYTES, NVE_CRDT_DELTA_MAGIC, NVE_CRDT_DELTA_MIN_LEN, NVE_CRDT_DELTA_VERSION,
    NVE_CRDT_XXH64_SEED, NveCrdtDelta, crdt_xxh64,
};
