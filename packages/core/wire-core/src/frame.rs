//! `NveWireFrame` v2 binary framing: 16-byte fixed little-endian header + payload + optional 8-byte
//! `XXH64` integrity trailer (`nve_abi::xxh64`).
//!
//! Zero custom ARQ, zero SACK bitmaps, zero hand-rolled UDP fragmentation — QUIC / WebTransport /
//! TLS WebSocket provides transport encryption, congestion control, multiplexing, and framing.

use std::fmt;

use nve_abi::xxh64;
use nve_codec::{write_fixed, ByteReader, ByteSink, ByteWriter, CodecError, NveCodec};

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
pub enum NveWireChannel {
    Control = 0,
    UiScene = 1,
    AsyncSlot = 2,
    InputEvents = 3,
    StateSync = 4,
    Presence = 5,
}


/// Frame message kind discriminator for `NveWireFrame` v2.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, NveCodec)]
pub enum NveWireKind {
    Hello = 1,
    HelloAck = 2,
    Reject = 3,
    /// Complete zero-copy `NveSceneArchive` (`RelocBuf`, already `XXH64`-verified in its 64B header).
    SceneKeyframe = 4,
    /// Direct incremental `NVE2` frame batch (zero reverse-parsing).
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

impl From<CodecError> for NveWireError {
    #[inline]
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::Truncated { needed, remaining } => Self::Truncated {
                expected: needed,
                actual: remaining,
            },
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::TrailingBytes(n) => Self::TrailingBytes(n),
            CodecError::NonZeroReserved(v) => Self::NonZeroReserved(v as u16),
            CodecError::BadMagic => Self::InvalidPayload("invalid wire magic"),
            CodecError::BadVersion(_) => Self::InvalidPayload("unsupported wire version"),
            CodecError::BadEnum { .. } => Self::InvalidPayload("invalid wire enum tag"),
            CodecError::BadBool(_) => Self::InvalidPayload("invalid wire bool byte"),
            CodecError::LenOverflow { len, max } => Self::FrameTooLarge { len, max },
            CodecError::DepthExceeded { .. } => Self::InvalidPayload("wire nesting depth exceeded"),
        }
    }
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

    #[inline]
    fn write_to_sink<S: ByteSink>(&self, w: &mut S) -> Result<(), CodecError> {
        w.put_raw(&NVE_WIRE_MAGIC)?;
        w.put_u8(NVE_WIRE_VERSION)?;
        w.put_u8(self.channel as u8)?;
        w.put_u8(self.kind as u8)?;
        w.put_u8(self.flags)?;
        w.put_zeroes(2)?;
        w.put_u32(self.sequence)?;
        w.put_u32(self.payload_len)
    }

    /// Serializes the 16-byte little-endian header with zero heap allocation.
    #[inline]
    pub fn to_bytes(&self) -> [u8; NVE_WIRE_HEADER_SIZE] {
        write_fixed::<NVE_WIRE_HEADER_SIZE>(|w| self.write_to_sink(w))
    }

    /// Parses and validates a 16-byte header BEFORE any payload allocation.
    pub fn decode(bytes: &[u8], max_frame_bytes: usize) -> Result<Self, NveWireError> {
        if bytes.len() < NVE_WIRE_HEADER_SIZE {
            return Err(NveWireError::Truncated {
                expected: NVE_WIRE_HEADER_SIZE,
                actual: bytes.len(),
            });
        }
        let mut r = ByteReader::new(bytes);
        let magic = r.read_array::<2>()?;
        if magic != NVE_WIRE_MAGIC {
            return Err(NveWireError::InvalidMagic(magic));
        }
        let version = r.read_u8()?;
        if version != NVE_WIRE_VERSION {
            return Err(NveWireError::UnsupportedVersion(version));
        }
        let raw_channel = r.read_u8()?;
        let channel =
            NveWireChannel::from_repr(raw_channel).ok_or(NveWireError::UnknownChannel(raw_channel))?;
        let raw_kind = r.read_u8()?;
        let kind = NveWireKind::from_repr(raw_kind).ok_or(NveWireError::UnknownKind(raw_kind))?;
        let flags = r.read_u8()?;
        let reserved = r.read_u16()?;
        if reserved != 0 {
            return Err(NveWireError::NonZeroReserved(reserved));
        }
        let sequence = r.read_u32()?;
        let payload_len = r.read_u32()?;
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
            let expected = ByteReader::new(&bytes[payload_end..]).read_u64()?;
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
        ByteWriter::with_buf(out, |w| -> Result<(), CodecError> {
            w.reserve(header.total_wire_len());
            header.write_to_sink(w)?;
            w.write_raw(&self.payload);
            if header.has_xxh64() {
                let digest = xxh64(w.as_slice(), NVE_WIRE_XXH64_SEED);
                w.write_u64(digest);
            }
            Ok(())
        })?;
        Ok(())
    }

    /// Decodes a complete wire frame into an owned `NveWireFrame`.
    pub fn decode(bytes: &[u8], max_frame_bytes: usize) -> Result<Self, NveWireError> {
        Ok(NveWireFrameRef::decode(bytes, max_frame_bytes)?.to_owned())
    }
}
