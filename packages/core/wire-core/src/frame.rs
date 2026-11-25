//! `NveWireFrame` v2 binary framing: 16-byte fixed little-endian header + payload + optional 8-byte
//! `XXH64` integrity trailer (`nve_engine::xxh64`).
//!
//! Zero custom ARQ, zero SACK bitmaps, zero hand-rolled UDP fragmentation — QUIC / WebTransport /
//! TLS WebSocket provides transport encryption, congestion control, multiplexing, and framing.

use std::fmt;

use nve_engine::xxh64;

/// 2-byte ASCII magic identifying `NveWireFrame` v2 packets (`"NW"`).
pub const NVE_WIRE_MAGIC: [u8; 2] = *b"NW";

/// Current `NveWireFrame` protocol version (`2`).
pub const NVE_WIRE_VERSION: u8 = 2;

/// Fixed 16-byte little-endian header size (8-byte aligned).
pub const NVE_WIRE_HEADER_SIZE: usize = 16;

/// Optional trailing `XXH64` checksum length in bytes.
pub const NVE_WIRE_TRAILER_SIZE: usize = 8;

/// Domain-separated `XXH64` seed for `NveWireFrame` v2 trailers.
pub const NVE_WIRE_XXH64_SEED: u64 = 0x4E56_4557_4952_4532; // "NVEWIRE2"

/// Default per-frame payload allocation guard (`16 MiB`).
pub const DEFAULT_MAX_WIRE_FRAME_BYTES: usize = 16 * 1024 * 1024;

/// Absolute upper cap on frame payload length (`64 MiB`).
pub const MAX_WIRE_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Header flag: an 8-byte little-endian `XXH64` checksum trailer follows the payload.
pub const NVE_WIRE_FLAG_XXH64: u8 = 1 << 0;

/// Header flag: frame is intended for RFC 9221 unreliable QUIC/WebTransport datagrams.
pub const NVE_WIRE_FLAG_DATAGRAM: u8 = 1 << 1;

/// Header flag: final chunk of a streaming `AsyncSlot` (`SlotChunk`) sequence.
pub const NVE_WIRE_FLAG_FINAL_CHUNK: u8 = 1 << 2;

/// Logical multiplexed channel identifier for `NveWireFrame`.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NveWireChannel {
    Control = 0,
    UiScene = 1,
    AsyncSlot = 2,
    InputEvents = 3,
    StateSync = 4,
    Presence = 5,
}

impl NveWireChannel {
    #[inline]
    pub const fn from_u8(raw: u8) -> Result<Self, NveWireError> {
        match raw {
            0 => Ok(Self::Control),
            1 => Ok(Self::UiScene),
            2 => Ok(Self::AsyncSlot),
            3 => Ok(Self::InputEvents),
            4 => Ok(Self::StateSync),
            5 => Ok(Self::Presence),
            other => Err(NveWireError::UnknownChannel(other)),
        }
    }
}

/// Frame message kind discriminator for `NveWireFrame` v2.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NveWireKind {
    Hello = 1,
    HelloAck = 2,
    Reject = 3,
    /// Complete zero-copy `NveSceneArchive` (`RelocBuf`, already `XXH64`-verified in its 64B header).
    SceneKeyframe = 4,
    /// Direct incremental `CommandBuffer` batch (zero reverse-parsing).
    CommandDelta = 5,
    /// Compact 24-byte `NveNodePatch` slice + UTF-8 string table.
    NodePatchDelta = 6,
    /// Out-of-order async / GenUI slot chunk.
    SlotChunk = 7,
    /// Raw binary `EventRing` (`NveEventBatch`) sent upstream from client to host.
    EventBatch = 8,
    /// `NveCrdtDelta` binary state synchronization delta.
    CrdtDelta = 9,
    /// Low-latency ephemeral presence / cursor payload (RFC 9221 datagram friendly).
    Presence = 10,
    /// Tier-1 live hot-patch payload.
    HotPatch = 11,
    Ping = 12,
    Pong = 13,
    KeyframeRequest = 14,
    Bye = 15,
}

impl NveWireKind {
    #[inline]
    pub const fn from_u8(raw: u8) -> Result<Self, NveWireError> {
        match raw {
            1 => Ok(Self::Hello),
            2 => Ok(Self::HelloAck),
            3 => Ok(Self::Reject),
            4 => Ok(Self::SceneKeyframe),
            5 => Ok(Self::CommandDelta),
            6 => Ok(Self::NodePatchDelta),
            7 => Ok(Self::SlotChunk),
            8 => Ok(Self::EventBatch),
            9 => Ok(Self::CrdtDelta),
            10 => Ok(Self::Presence),
            11 => Ok(Self::HotPatch),
            12 => Ok(Self::Ping),
            13 => Ok(Self::Pong),
            14 => Ok(Self::KeyframeRequest),
            15 => Ok(Self::Bye),
            other => Err(NveWireError::UnknownKind(other)),
        }
    }

    /// Default logical channel associated with this frame kind.
    #[inline]
    pub const fn default_channel(self) -> NveWireChannel {
        match self {
            Self::Hello
            | Self::HelloAck
            | Self::Reject
            | Self::Ping
            | Self::Pong
            | Self::KeyframeRequest
            | Self::Bye
            | Self::HotPatch => NveWireChannel::Control,
            Self::SceneKeyframe | Self::CommandDelta | Self::NodePatchDelta => {
                NveWireChannel::UiScene
            }
            Self::SlotChunk => NveWireChannel::AsyncSlot,
            Self::EventBatch => NveWireChannel::InputEvents,
            Self::CrdtDelta => NveWireChannel::StateSync,
            Self::Presence => NveWireChannel::Presence,
        }
    }
}

/// Errors produced while decoding, validating, or building `NveWireFrame` v2 messages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NveWireError {
    Truncated { expected: usize, actual: usize },
    InvalidMagic([u8; 2]),
    UnsupportedVersion(u8),
    UnknownChannel(u8),
    UnknownKind(u8),
    NonZeroReserved(u16),
    FrameTooLarge { len: usize, max: usize },
    ChecksumMismatch { expected: u64, actual: u64 },
    TrailingBytes(usize),
    InvalidUtf8,
    InvalidPayload(&'static str),
}

impl fmt::Display for NveWireError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated { expected, actual } => {
                write!(f, "NveWireFrame truncated: expected at least {expected}B, got {actual}B")
            }
            Self::InvalidMagic(m) => write!(f, "NveWireFrame invalid magic: {m:02x?}"),
            Self::UnsupportedVersion(v) => write!(f, "NveWireFrame unsupported version: {v}"),
            Self::UnknownChannel(c) => write!(f, "NveWireFrame unknown channel: {c}"),
            Self::UnknownKind(k) => write!(f, "NveWireFrame unknown kind: {k}"),
            Self::NonZeroReserved(r) => write!(f, "NveWireFrame reserved field must be 0, got {r}"),
            Self::FrameTooLarge { len, max } => {
                write!(f, "NveWireFrame payload {len}B exceeds max_frame_bytes ({max}B)")
            }
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "NveWireFrame XXH64 checksum mismatch: expected 0x{expected:016x}, got 0x{actual:016x}"
            ),
            Self::TrailingBytes(n) => write!(f, "NveWireFrame has {n} unexpected trailing bytes"),
            Self::InvalidUtf8 => write!(f, "NveWireFrame payload contains invalid UTF-8"),
            Self::InvalidPayload(msg) => write!(f, "NveWireFrame invalid payload: {msg}"),
        }
    }
}

impl std::error::Error for NveWireError {}

/// Decoded 16-byte fixed little-endian `NveWireFrame` header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveWireHeader {
    pub channel: NveWireChannel,
    pub kind: NveWireKind,
    pub flags: u8,
    pub sequence: u32,
    pub payload_len: u32,
}

impl NveWireHeader {
    #[inline]
    pub const fn has_xxh64(&self) -> bool {
        (self.flags & NVE_WIRE_FLAG_XXH64) != 0
    }

    #[inline]
    pub const fn is_datagram(&self) -> bool {
        (self.flags & NVE_WIRE_FLAG_DATAGRAM) != 0
    }

    #[inline]
    pub const fn is_final_chunk(&self) -> bool {
        (self.flags & NVE_WIRE_FLAG_FINAL_CHUNK) != 0
    }

    /// Total wire byte length (header + payload + optional 8-byte `XXH64` trailer).
    #[inline]
    pub const fn total_wire_len(&self) -> usize {
        NVE_WIRE_HEADER_SIZE
            + self.payload_len as usize
            + if self.has_xxh64() {
                NVE_WIRE_TRAILER_SIZE
            } else {
                0
            }
    }

    /// Serializes the 16-byte little-endian header.
    #[inline]
    pub fn to_bytes(&self) -> [u8; NVE_WIRE_HEADER_SIZE] {
        let mut out = [0u8; NVE_WIRE_HEADER_SIZE];
        out[0..2].copy_from_slice(&NVE_WIRE_MAGIC);
        out[2] = NVE_WIRE_VERSION;
        out[3] = self.channel as u8;
        out[4] = self.kind as u8;
        out[5] = self.flags;
        out[6..8].copy_from_slice(&0u16.to_le_bytes());
        out[8..12].copy_from_slice(&self.sequence.to_le_bytes());
        out[12..16].copy_from_slice(&self.payload_len.to_le_bytes());
        out
    }

    /// Parses and validates a 16-byte header BEFORE any payload allocation.
    pub fn decode(bytes: &[u8], max_frame_bytes: usize) -> Result<Self, NveWireError> {
        if bytes.len() < NVE_WIRE_HEADER_SIZE {
            return Err(NveWireError::Truncated {
                expected: NVE_WIRE_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let magic = [bytes[0], bytes[1]];
        if magic != NVE_WIRE_MAGIC {
            return Err(NveWireError::InvalidMagic(magic));
        }
        if bytes[2] != NVE_WIRE_VERSION {
            return Err(NveWireError::UnsupportedVersion(bytes[2]));
        }
        let channel = NveWireChannel::from_u8(bytes[3])?;
        let kind = NveWireKind::from_u8(bytes[4])?;
        let flags = bytes[5];
        let reserved = u16::from_le_bytes([bytes[6], bytes[7]]);
        if reserved != 0 {
            return Err(NveWireError::NonZeroReserved(reserved));
        }
        let sequence = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
        let payload_len = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
        let effective_max = max_frame_bytes.min(MAX_WIRE_FRAME_BYTES);
        if (payload_len as usize) > effective_max {
            return Err(NveWireError::FrameTooLarge {
                len: payload_len as usize,
                max: effective_max,
            });
        }
        Ok(Self {
            channel,
            kind,
            flags,
            sequence,
            payload_len,
        })
    }
}

/// Zero-copy borrowed view of one validated `NveWireFrame`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NveWireFrameRef<'a> {
    pub header: NveWireHeader,
    pub payload: &'a [u8],
}

impl<'a> NveWireFrameRef<'a> {
    /// Decodes and validates a single complete wire frame from `bytes`.
    pub fn decode(bytes: &'a [u8], max_frame_bytes: usize) -> Result<Self, NveWireError> {
        let header = NveWireHeader::decode(bytes, max_frame_bytes)?;
        let expected_total = header.total_wire_len();
        if bytes.len() < expected_total {
            return Err(NveWireError::Truncated {
                expected: expected_total,
                actual: bytes.len(),
            });
        }
        if bytes.len() > expected_total {
            return Err(NveWireError::TrailingBytes(bytes.len() - expected_total));
        }
        let payload_end = NVE_WIRE_HEADER_SIZE + header.payload_len as usize;
        if header.has_xxh64() {
            let expected = u64::from_le_bytes(
                bytes[payload_end..payload_end + NVE_WIRE_TRAILER_SIZE]
                    .try_into()
                    .expect("8-byte trailer"),
            );
            let actual = xxh64(&bytes[..payload_end], NVE_WIRE_XXH64_SEED);
            if actual != expected {
                return Err(NveWireError::ChecksumMismatch { expected, actual });
            }
        }
        Ok(Self {
            header,
            payload: &bytes[NVE_WIRE_HEADER_SIZE..payload_end],
        })
    }

    pub fn to_owned(&self) -> NveWireFrame {
        NveWireFrame {
            channel: self.header.channel,
            kind: self.header.kind,
            flags: self.header.flags,
            sequence: self.header.sequence,
            payload: self.payload.to_vec(),
        }
    }
}

/// Owned `NveWireFrame` v2 packet ready to encode onto QUIC streams, WebTransport, or WSS.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NveWireFrame {
    pub channel: NveWireChannel,
    pub kind: NveWireKind,
    pub flags: u8,
    pub sequence: u32,
    pub payload: Vec<u8>,
}

impl NveWireFrame {
    /// Creates a new frame on `kind.default_channel()` with `XXH64` enabled by default
    /// (except `SceneKeyframe`, whose `NveSceneArchive` 64-byte header is already `XXH64`-verified).
    pub fn new(kind: NveWireKind, sequence: u32, payload: Vec<u8>) -> Self {
        let flags = if kind == NveWireKind::SceneKeyframe {
            0
        } else {
            NVE_WIRE_FLAG_XXH64
        };
        Self {
            channel: kind.default_channel(),
            kind,
            flags,
            sequence,
            payload,
        }
    }

    pub fn with_channel(mut self, channel: NveWireChannel) -> Self {
        self.channel = channel;
        self
    }

    pub fn with_xxh64(mut self, enabled: bool) -> Self {
        if enabled {
            self.flags |= NVE_WIRE_FLAG_XXH64;
        } else {
            self.flags &= !NVE_WIRE_FLAG_XXH64;
        }
        self
    }

    pub fn with_datagram(mut self, enabled: bool) -> Self {
        if enabled {
            self.flags |= NVE_WIRE_FLAG_DATAGRAM;
        } else {
            self.flags &= !NVE_WIRE_FLAG_DATAGRAM;
        }
        self
    }

    pub fn header(&self) -> Result<NveWireHeader, NveWireError> {
        let payload_len = u32::try_from(self.payload.len()).map_err(|_| {
            NveWireError::FrameTooLarge {
                len: self.payload.len(),
                max: MAX_WIRE_FRAME_BYTES,
            }
        })?;
        if self.payload.len() > MAX_WIRE_FRAME_BYTES {
            return Err(NveWireError::FrameTooLarge {
                len: self.payload.len(),
                max: MAX_WIRE_FRAME_BYTES,
            });
        }
        Ok(NveWireHeader {
            channel: self.channel,
            kind: self.kind,
            flags: self.flags,
            sequence: self.sequence,
            payload_len,
        })
    }

    /// Encodes the complete frame into a new byte vector.
    pub fn encode(&self) -> Result<Vec<u8>, NveWireError> {
        let header = self.header()?;
        let mut out = Vec::with_capacity(header.total_wire_len());
        self.encode_into(&mut out)?;
        Ok(out)
    }

    /// Encodes the complete frame into `out` (clearing `out` first).
    pub fn encode_into(&self, out: &mut Vec<u8>) -> Result<(), NveWireError> {
        let header = self.header()?;
        out.clear();
        out.reserve(header.total_wire_len());
        out.extend_from_slice(&header.to_bytes());
        out.extend_from_slice(&self.payload);
        if header.has_xxh64() {
            let digest = xxh64(out, NVE_WIRE_XXH64_SEED);
            out.extend_from_slice(&digest.to_le_bytes());
        }
        Ok(())
    }

    /// Decodes a complete wire frame into an owned `NveWireFrame`.
    pub fn decode(bytes: &[u8], max_frame_bytes: usize) -> Result<Self, NveWireError> {
        Ok(NveWireFrameRef::decode(bytes, max_frame_bytes)?.to_owned())
    }
}
