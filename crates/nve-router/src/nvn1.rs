//! Compact zero-JSON binary `NVN1` wire codec for cross-surface and host navigation messages.

use std::fmt;

use crate::route::Routable;
use crate::stack::{NavigationAction, NavigationTransition};

/// 4-byte `NVN1` magic header.
pub const NVN1_MAGIC: [u8; 4] = *b"NVN1";
/// Current wire format version.
pub const NVN1_VERSION: u8 = 1;
/// Minimum packet length (`18`-byte fixed header + `2`-byte `tab_id_len`).
pub const NVN1_MIN_PACKET_BYTES: usize = 20;

pub const NVN1_FLAG_CAN_GO_BACK: u8 = 1 << 0;
pub const NVN1_FLAG_CAN_GO_FORWARD: u8 = 1 << 1;
pub const NVN1_FLAG_IS_MODAL: u8 = 1 << 2;
const NVN1_KNOWN_FLAGS_MASK: u8 =
    NVN1_FLAG_CAN_GO_BACK | NVN1_FLAG_CAN_GO_FORWARD | NVN1_FLAG_IS_MODAL;

/// Error returned when encoding a [`NavPacket`] into `NVN1` bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nvn1EncodeError {
    UriTooLong(usize),
    TabIdTooLong(usize),
}

impl fmt::Display for Nvn1EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UriTooLong(len) => write!(f, "NVN1 URI length {len} exceeds u16::MAX"),
            Self::TabIdTooLong(len) => write!(f, "NVN1 tab_id length {len} exceeds u16::MAX"),
        }
    }
}

impl std::error::Error for Nvn1EncodeError {}

/// Error returned when decoding an `NVN1` packet from untrusted bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Nvn1DecodeError {
    Truncated,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u8),
    InvalidAction(u8),
    InvalidFlags(u8),
    NonZeroReserved(u8),
    InvalidUtf8(&'static str),
    TrailingBytes(usize),
}

impl fmt::Display for Nvn1DecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => write!(f, "truncated NVN1 packet"),
            Self::InvalidMagic(m) => write!(f, "invalid NVN1 magic {m:?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVN1 version {v}"),
            Self::InvalidAction(a) => write!(f, "invalid NVN1 navigation action {a}"),
            Self::InvalidFlags(fl) => write!(f, "unknown NVN1 flag bits 0x{fl:02x}"),
            Self::NonZeroReserved(r) => write!(f, "non-zero NVN1 reserved byte 0x{r:02x}"),
            Self::InvalidUtf8(field) => write!(f, "invalid UTF-8 in NVN1 field {field}"),
            Self::TrailingBytes(n) => write!(f, "NVN1 packet has {n} unexpected trailing byte(s)"),
        }
    }
}

impl std::error::Error for Nvn1DecodeError {}

/// Zero-copy borrowed view over a decoded `NVN1` navigation packet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavPacketRef<'a> {
    pub action: NavigationAction,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_modal: bool,
    pub stack_depth: u32,
    pub active_tab_index: u32,
    pub uri: &'a str,
    pub tab_id: &'a str,
}

impl<'a> NavPacketRef<'a> {
    pub fn to_owned(self) -> NavPacket {
        NavPacket {
            action: self.action,
            can_go_back: self.can_go_back,
            can_go_forward: self.can_go_forward,
            is_modal: self.is_modal,
            stack_depth: self.stack_depth,
            active_tab_index: self.active_tab_index,
            uri: self.uri.to_owned(),
            tab_id: self.tab_id.to_owned(),
        }
    }
}

/// Owned `NVN1` navigation state/command packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavPacket {
    pub action: NavigationAction,
    pub can_go_back: bool,
    pub can_go_forward: bool,
    pub is_modal: bool,
    pub stack_depth: u32,
    pub active_tab_index: u32,
    pub uri: String,
    pub tab_id: String,
}

impl NavPacket {
    /// Builds a wire packet directly from a committed [`NavigationTransition`].
    pub fn from_transition<R: Routable>(tr: &NavigationTransition<R>) -> Self {
        Self {
            action: tr.action,
            can_go_back: tr.can_go_back,
            can_go_forward: tr.can_go_forward,
            is_modal: tr.is_modal,
            stack_depth: tr.stack_depth,
            active_tab_index: tr.active_tab_index,
            uri: tr.route.to_path(),
            tab_id: tr.active_tab_id.clone(),
        }
    }

    pub fn as_ref(&self) -> NavPacketRef<'_> {
        NavPacketRef {
            action: self.action,
            can_go_back: self.can_go_back,
            can_go_forward: self.can_go_forward,
            is_modal: self.is_modal,
            stack_depth: self.stack_depth,
            active_tab_index: self.active_tab_index,
            uri: &self.uri,
            tab_id: &self.tab_id,
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>, Nvn1EncodeError> {
        encode_nav_packet(self.as_ref())
    }
}

/// Encodes `packet` into a fresh compact binary `NVN1` buffer.
pub fn encode_nav_packet(packet: NavPacketRef<'_>) -> Result<Vec<u8>, Nvn1EncodeError> {
    let mut buf = Vec::with_capacity(NVN1_MIN_PACKET_BYTES + packet.uri.len() + packet.tab_id.len());
    encode_nav_packet_into(packet, &mut buf)?;
    Ok(buf)
}

/// Appends the compact binary `NVN1` representation of `packet` into `dst`.
pub fn encode_nav_packet_into(
    packet: NavPacketRef<'_>,
    dst: &mut Vec<u8>,
) -> Result<(), Nvn1EncodeError> {
    let uri_len =
        u16::try_from(packet.uri.len()).map_err(|_| Nvn1EncodeError::UriTooLong(packet.uri.len()))?;
    let tab_id_len = u16::try_from(packet.tab_id.len())
        .map_err(|_| Nvn1EncodeError::TabIdTooLong(packet.tab_id.len()))?;

    let mut flags = 0u8;
    if packet.can_go_back {
        flags |= NVN1_FLAG_CAN_GO_BACK;
    }
    if packet.can_go_forward {
        flags |= NVN1_FLAG_CAN_GO_FORWARD;
    }
    if packet.is_modal {
        flags |= NVN1_FLAG_IS_MODAL;
    }

    dst.reserve(NVN1_MIN_PACKET_BYTES + packet.uri.len() + packet.tab_id.len());
    dst.extend_from_slice(&NVN1_MAGIC);
    dst.push(NVN1_VERSION);
    dst.push(packet.action as u8);
    dst.push(flags);
    dst.push(0);
    dst.extend_from_slice(&packet.stack_depth.to_le_bytes());
    dst.extend_from_slice(&packet.active_tab_index.to_le_bytes());
    dst.extend_from_slice(&uri_len.to_le_bytes());
    dst.extend_from_slice(packet.uri.as_bytes());
    dst.extend_from_slice(&tab_id_len.to_le_bytes());
    dst.extend_from_slice(packet.tab_id.as_bytes());
    Ok(())
}

/// Decodes a binary `NVN1` slice with zero heap allocations and strict bounds/UTF-8 validation.
pub fn decode_nav_packet_ref(bytes: &[u8]) -> Result<NavPacketRef<'_>, Nvn1DecodeError> {
    if bytes.len() < NVN1_MIN_PACKET_BYTES {
        return Err(Nvn1DecodeError::Truncated);
    }

    let magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
    if magic != NVN1_MAGIC {
        return Err(Nvn1DecodeError::InvalidMagic(magic));
    }
    let version = bytes[4];
    if version != NVN1_VERSION {
        return Err(Nvn1DecodeError::UnsupportedVersion(version));
    }
    let action =
        NavigationAction::from_u8(bytes[5]).ok_or(Nvn1DecodeError::InvalidAction(bytes[5]))?;
    let flags = bytes[6];
    if (flags & !NVN1_KNOWN_FLAGS_MASK) != 0 {
        return Err(Nvn1DecodeError::InvalidFlags(flags));
    }
    let reserved = bytes[7];
    if reserved != 0 {
        return Err(Nvn1DecodeError::NonZeroReserved(reserved));
    }

    let stack_depth = u32::from_le_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    let active_tab_index = u32::from_le_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    let uri_len = u16::from_le_bytes([bytes[16], bytes[17]]) as usize;

    let uri_end = 18usize.checked_add(uri_len).ok_or(Nvn1DecodeError::Truncated)?;
    let tab_len_end = uri_end.checked_add(2).ok_or(Nvn1DecodeError::Truncated)?;
    if bytes.len() < tab_len_end {
        return Err(Nvn1DecodeError::Truncated);
    }
    let uri = std::str::from_utf8(&bytes[18..uri_end])
        .map_err(|_| Nvn1DecodeError::InvalidUtf8("uri"))?;

    let tab_id_len = u16::from_le_bytes([bytes[uri_end], bytes[uri_end + 1]]) as usize;
    let tab_end = tab_len_end
        .checked_add(tab_id_len)
        .ok_or(Nvn1DecodeError::Truncated)?;
    if bytes.len() < tab_end {
        return Err(Nvn1DecodeError::Truncated);
    }
    if bytes.len() > tab_end {
        return Err(Nvn1DecodeError::TrailingBytes(bytes.len() - tab_end));
    }
    let tab_id = std::str::from_utf8(&bytes[tab_len_end..tab_end])
        .map_err(|_| Nvn1DecodeError::InvalidUtf8("tab_id"))?;

    Ok(NavPacketRef {
        action,
        can_go_back: (flags & NVN1_FLAG_CAN_GO_BACK) != 0,
        can_go_forward: (flags & NVN1_FLAG_CAN_GO_FORWARD) != 0,
        is_modal: (flags & NVN1_FLAG_IS_MODAL) != 0,
        stack_depth,
        active_tab_index,
        uri,
        tab_id,
    })
}

/// Decodes a binary `NVN1` slice into an owned [`NavPacket`].
pub fn decode_nav_packet(bytes: &[u8]) -> Result<NavPacket, Nvn1DecodeError> {
    decode_nav_packet_ref(bytes).map(NavPacketRef::to_owned)
}
