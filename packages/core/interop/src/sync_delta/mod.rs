//! Zero-copy binary frame-delta wire protocol for live NVE Studio pairing (`< 10 KB/s`).
//!
//! Designed for QUIC datagrams, WebRTC DataChannels, and WebSocket transports:
//! - Fixed 32-byte little-endian header (`b"NVD1"`) with 64-bit FNV-1a integrity checksum over
//!   header + payload and zero-copy packet validation (`DeltaPacketRef<'a>`).
//! - Dual compact encodings automatically selecting the smallest wire representation per frame:
//!   1. **XOR + RLE** (`PacketKind::XorRleDelta`): zero-run RLE over `prev ^ next` with LEB128
//!      varints for byte-level edits to state or frame buffers.
//!   2. **`CommandBuffer` Changed-Op Delta** (`PacketKind::CommandOpDelta`): scans `NVE1` command
//!      boundaries in zero-copy slices, collapses unchanged op runs into 3-byte `CopyRun` tokens,
//!      and ships ONLY the raw bytes of changed/inserted commands — immune to mid-buffer byte shifts!
//! - Reliable sequence numbers + cumulative/selective ACK (`SACK` 64-bit bitmap) + NACK +
//!   bounded retransmit window + out-of-order receiver reorder buffer.

mod cmd_op;
mod session;
#[cfg(test)]
mod tests;
mod wire;
mod xor_rle;

pub use cmd_op::{decode_command_op_delta, encode_command_op_delta};
pub use session::{ReceiveOutcome, SyncDeltaSession, SyncMetrics};
pub use wire::{
    encode_packet, fnv1a_64, fnv1a_64_pair, write_packet_into, DeltaPacketHeader, DeltaPacketRef,
    PacketKind, SyncDeltaError, DEFAULT_SYNC_WINDOW, DELTA_HEADER_SIZE, FLAG_RETRANSMIT,
    MAX_DECODED_FRAME_BYTES, SYNC_DELTA_MAGIC, SYNC_DELTA_VERSION,
};
pub use xor_rle::{
    decode_xor_rle_delta, decode_xor_rle_delta_into, encode_xor_rle_delta,
    encode_xor_rle_delta_into,
};
