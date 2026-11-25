//! Sans-I/O core wire framing (`NveWireFrame` v2), Bearer-token handshake, compact
//! `NveNodePatchDelta` codec, URI parser, and client state machine.
//!
//! Zero async/runtime dependencies (`tokio`/`quinn`/`rustls`-free) and 100% `wasm32`-clean so
//! browser WebTransport/WebSocket clients, native QUIC/WSS peers, and C/Swift/Dart FFI adapters
//! share a single canonical wire format.

pub mod frame;
pub mod handshake;
pub mod patch;
pub mod session;
#[cfg(test)]
mod tests;
pub mod uri;

pub use frame::{
    NveWireChannel, NveWireError, NveWireFrame, NveWireFrameRef, NveWireHeader, NveWireKind,
    DEFAULT_MAX_WIRE_FRAME_BYTES, MAX_WIRE_FRAME_BYTES, NVE_WIRE_FLAG_DATAGRAM,
    NVE_WIRE_FLAG_FINAL_CHUNK, NVE_WIRE_FLAG_XXH64, NVE_WIRE_HEADER_SIZE, NVE_WIRE_MAGIC,
    NVE_WIRE_TRAILER_SIZE, NVE_WIRE_VERSION, NVE_WIRE_XXH64_SEED,
};
pub use handshake::{
    constant_time_eq, NveWireHello, NveWireHelloAck, NveWireRejectReason, NveWireSlotChunk,
    DEFAULT_WIRE_SCHEMA_VERSION, NVE_WIRE_CAP_DATAGRAMS, NVE_WIRE_CAP_NODE_PATCHES,
    NVE_WIRE_CAP_SCENE_ARCHIVE, NVE_WIRE_CAP_SLOT_CHUNKS,
};
pub use nve_engine::{
    NveNodePatch, NveNodePatchField, NveNodePatchValue, NveSceneArchive, NveSceneArchiveView,
    NveSignalSnapshot, NveStateArena, NveTimelineArchive, NveTimelineArchiveView, NveTimelineEntry,
    NveTimelineEntryInput, NveTimelinePatchInput, NveTimelineRoot,
};
pub use patch::{NveNodePatchDeltaBuilder, NveNodePatchDeltaView, NVE_NODE_PATCH_DELTA_HEADER_SIZE};
pub use session::{
    NveWireClientSession, NveWireClientState, NveWireIngestOutcome, NveWireMetrics,
};
pub use uri::{
    format_cert_hash_hex, format_wire_uri, parse_cert_hash_hex, parse_wire_uri,
    NveWireTransportKind, NveWireUri, DEFAULT_WIRE_QUIC_ALPN, DEFAULT_WIRE_QUIC_PORT,
    DEFAULT_WIRE_WSS_PATH, DEFAULT_WIRE_WSS_PORT,
};

/// Self-documenting alias for an encoded incremental `CommandBuffer` frame batch.
pub type NveCommandBatch = Vec<u8>;
/// Self-documenting alias for an encoded `EventRing` input event batch.
pub type NveEventBatch = Vec<u8>;
