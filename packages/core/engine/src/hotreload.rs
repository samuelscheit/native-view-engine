use std::fmt;

use nve_abi::Xxh64Hasher;
use nve_codec::{ByteReader, ByteWriter, CodecError, DEFAULT_MAX_BYTES};

/// 4-byte wire magic identifying a binary `HotPatchMsg` frame (`NVP1`).
pub const HOT_PATCH_WIRE_MAGIC: [u8; 4] = *b"NVP1";

/// Wire protocol version immediately following [`HOT_PATCH_WIRE_MAGIC`].
/// Version 2 replaces legacy CSS string `attr_updates` with binary `RowPatch` `u32` word payloads
/// and 64-bit `xxh64` template identity keys.
pub const HOT_PATCH_VERSION: u8 = 2;

/// Maximum item count accepted per vector inside a binary `NVP1` frame to prevent OOM on corrupt input.
const MAX_WIRE_ITEMS: usize = 65_536;

/// Computes the deterministic 64-bit `xxh64` template identity key for `(rel_file, ordinal)`.
///
/// Normalizes Windows `\` separators to `/` and strips leading `./` segments with zero heap
/// allocation so compile-time `nve_dioxus::rsx!` anchors and `nve dev` (`rsx_diff`) always agree.
/// Never returns `0` (`0` is reserved for unkeyed patches).
#[inline]
pub fn hot_patch_template_key(rel_file: &str, ordinal: u32) -> u64 {
    let mut h = Xxh64Hasher::new(0);
    let mut bytes = rel_file.as_bytes();
    while bytes.len() >= 2 && bytes[0] == b'.' && (bytes[1] == b'/' || bytes[1] == b'\\') {
        bytes = &bytes[2..];
    }
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        if b == b'\\' {
            if start < i {
                h.update(&bytes[start..i]);
            }
            h.update(b"/");
            start = i + 1;
        }
    }
    if start < bytes.len() {
        h.update(&bytes[start..]);
    }
    h.update(b"#");
    h.update(&ordinal.to_le_bytes());
    let digest = h.digest();
    if digest == 0 { 1 } else { digest }
}

/// Error returned when decoding a binary `NVP1` `HotPatchMsg` frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotPatchDecodeError {
    Truncated,
    InvalidMagic,
    UnsupportedVersion(u8),
    UnknownVariant(u8),
    InvalidBool(u8),
    InvalidUtf8,
    LimitExceeded,
    TrailingBytes(usize),
}

impl From<CodecError> for HotPatchDecodeError {
    #[inline]
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::Truncated { .. } => Self::Truncated,
            CodecError::BadMagic => Self::InvalidMagic,
            CodecError::BadVersion(v) => Self::UnsupportedVersion(v as u8),
            CodecError::BadEnum { raw, .. } => Self::UnknownVariant(raw as u8),
            CodecError::BadBool(b) => Self::InvalidBool(b),
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::LenOverflow { .. } => Self::LimitExceeded,
            CodecError::TrailingBytes(n) => Self::TrailingBytes(n),
            CodecError::NonZeroReserved(_) => Self::InvalidMagic,
            CodecError::DepthExceeded { .. } => Self::LimitExceeded,
        }
    }
}

impl fmt::Display for HotPatchDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated NVP1 hot-patch buffer"),
            Self::InvalidMagic => f.write_str("invalid NVP1 hot-patch magic"),
            Self::UnsupportedVersion(v) => write!(
                f,
                "unsupported hot-patch wire version {v} (expected {HOT_PATCH_VERSION})"
            ),
            Self::UnknownVariant(v) => write!(f, "unknown NVP1 hot-patch variant tag {v}"),
            Self::InvalidBool(v) => write!(f, "invalid NVP1 boolean tag {v}"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 in NVP1 hot-patch string"),
            Self::LimitExceeded => f.write_str("NVP1 collection limit exceeded"),
            Self::TrailingBytes(n) => write!(f, "trailing {n} byte(s) in NVP1 hot-patch buffer"),
        }
    }
}

impl std::error::Error for HotPatchDecodeError {}

/// Unified 2-tier hot-reload protocol broadcast by `nve dev` over WebSocket (binary v2
/// frames, see [`HotPatchMsg::to_bytes`]) to Desktop Skia, iOS Simulator/Device, Android
/// Emulator/Device, and Web clients, and handed to apps through `NveDevApi::apply_hot_patch`.
/// There is no JSON form and no CSS attribute string payload.
#[derive(Debug, Clone, PartialEq)]
pub enum HotPatchMsg {
    /// Tier 1 (<10ms, zero recompile, works on physical iOS devices & browsers):
    /// Applies a binary `nve_store::RowPatch` and/or targeted text update directly onto matching
    /// `NodeStore` nodes identified by `(template_key, node_path)` or `target_test_id`.
    TemplatePatch {
        generation: u64,
        /// Collision-free `xxh64` template identity ([`hot_patch_template_key`]); `0` if unkeyed.
        template_key: u64,
        /// Structural child-index path inside the template tree (from template root).
        node_path: Vec<u16>,
        /// Optional `test_id` / `data-testid` / `testID` fallback targeting a specific RSX element.
        target_test_id: Option<String>,
        /// Flat `u32` words encoded by `nve_store::RowPatch::write_words` (validated via
        /// `RowPatchView::from_words` and applied directly via `Transaction::apply_row_patch`).
        row_patch_words: Vec<u32>,
        /// Positional `(text_index, new_text)` updates.
        text_updates: Vec<(usize, String)>,
        /// Exact `(old_text, new_text)` static text replacements scoped to the target.
        text_replacements: Vec<(String, String)>,
    },
    /// Tier 2 (150-400ms, desktop + iOS Simulator + Android):
    /// Instructs the host shell to hot-swap the compiled Rust app cdylib while preserving state.
    DylibReload {
        generation: u64,
        dylib_path: String,
        build_ms: u64,
    },
    /// Heartbeat / diagnostic banner sent to the live on-screen HUD.
    DevStatus {
        message: String,
        latency_ms: u64,
    },
}

impl HotPatchMsg {
    /// Encodes this hot-patch message into compact binary v2 framing.
    pub fn to_bytes(&self) -> Result<Vec<u8>, CodecError> {
        let mut w = ByteWriter::with_capacity(64);
        w.write_raw(&HOT_PATCH_WIRE_MAGIC);
        w.write_u8(HOT_PATCH_VERSION);
        match self {
            Self::TemplatePatch {
                generation,
                template_key,
                node_path,
                target_test_id,
                row_patch_words,
                text_updates,
                text_replacements,
            } => {
                w.write_u8(1);
                w.write_u64(*generation);
                w.write_u64(*template_key);

                w.write_len_max(node_path.len(), MAX_WIRE_ITEMS)?;
                for &step in node_path {
                    w.write_u16(step);
                }

                w.write_opt(target_test_id.as_deref(), |w, s| {
                    w.write_str_max(s, DEFAULT_MAX_BYTES)
                })?;

                w.write_len_max(row_patch_words.len(), MAX_WIRE_ITEMS)?;
                for &word in row_patch_words {
                    w.write_u32(word);
                }

                w.write_len_max(text_updates.len(), MAX_WIRE_ITEMS)?;
                for (idx, text) in text_updates {
                    w.write_len(*idx)?;
                    w.write_str_max(text, DEFAULT_MAX_BYTES)?;
                }

                w.write_len_max(text_replacements.len(), MAX_WIRE_ITEMS)?;
                for (old_text, new_text) in text_replacements {
                    w.write_str_max(old_text, DEFAULT_MAX_BYTES)?;
                    w.write_str_max(new_text, DEFAULT_MAX_BYTES)?;
                }
            }
            Self::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => {
                w.write_u8(2);
                w.write_u64(*generation);
                w.write_str_max(dylib_path, DEFAULT_MAX_BYTES)?;
                w.write_u64(*build_ms);
            }
            Self::DevStatus {
                message,
                latency_ms,
            } => {
                w.write_u8(3);
                w.write_str_max(message, DEFAULT_MAX_BYTES)?;
                w.write_u64(*latency_ms);
            }
        }
        Ok(w.into_bytes())
    }

    /// Decodes a compact binary v2 hot-patch frame.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, HotPatchDecodeError> {
        let mut r = ByteReader::new(raw);
        r.expect_magic(&HOT_PATCH_WIRE_MAGIC)?;
        let version = r.read_u8()?;
        if version != HOT_PATCH_VERSION {
            return Err(HotPatchDecodeError::UnsupportedVersion(version));
        }
        let variant = r.read_u8()?;
        let msg = match variant {
            1 => {
                let generation = r.read_u64()?;
                let template_key = r.read_u64()?;

                let node_path = r.read_vec(MAX_WIRE_ITEMS, 2, |r| r.read_u16())?;

                let target_test_id = r.read_opt(|r| r.read_string(DEFAULT_MAX_BYTES))?;

                let row_patch_words = r.read_vec(MAX_WIRE_ITEMS, 4, |r| r.read_u32())?;

                let text_updates = r.read_vec(MAX_WIRE_ITEMS, 8, |r| {
                    let idx = r.read_u32()? as usize;
                    let text = r.read_string(DEFAULT_MAX_BYTES)?;
                    Ok((idx, text))
                })?;

                let text_replacements = r.read_vec(MAX_WIRE_ITEMS, 8, |r| {
                    let old_t = r.read_string(DEFAULT_MAX_BYTES)?;
                    let new_t = r.read_string(DEFAULT_MAX_BYTES)?;
                    Ok((old_t, new_t))
                })?;

                Self::TemplatePatch {
                    generation,
                    template_key,
                    node_path,
                    target_test_id,
                    row_patch_words,
                    text_updates,
                    text_replacements,
                }
            }
            2 => Self::DylibReload {
                generation: r.read_u64()?,
                dylib_path: r.read_string(DEFAULT_MAX_BYTES)?,
                build_ms: r.read_u64()?,
            },
            3 => Self::DevStatus {
                message: r.read_string(DEFAULT_MAX_BYTES)?,
                latency_ms: r.read_u64()?,
            },
            other => return Err(HotPatchDecodeError::UnknownVariant(other)),
        };
        r.finish()?;
        Ok(msg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn template_key_normalizes_separators_and_leading_dot_slash() {
        let a = hot_patch_template_key("examples/studio-workspace/src/ui.rs", 3);
        let b = hot_patch_template_key("./examples/studio-workspace/src/ui.rs", 3);
        let c = hot_patch_template_key(".\\examples\\studio-workspace\\src\\ui.rs", 3);
        let d = hot_patch_template_key("examples/studio-workspace/src/ui.rs", 4);
        assert_ne!(a, 0);
        assert_eq!(a, b);
        assert_eq!(a, c);
        assert_ne!(a, d);
    }

    #[test]
    fn nvp1_v2_roundtrips_all_variants_and_rejects_corrupt_inputs() {
        let key = hot_patch_template_key("examples/studio-workspace/src/ui.rs", 2);
        let cases = vec![
            HotPatchMsg::TemplatePatch {
                generation: 42,
                template_key: key,
                node_path: vec![0, 2, 1],
                target_test_id: Some("studio-panel".into()),
                row_patch_words: vec![1, 0x1122_3344, 0x5566_7788, 1, 0, u32::MAX, 0x3F80_0000],
                text_updates: vec![(0, "AETHER".into()), (3, "東京テスト".into())],
                text_replacements: vec![("Old".into(), "New".into())],
            },
            HotPatchMsg::TemplatePatch {
                generation: 1,
                template_key: 0,
                node_path: vec![],
                target_test_id: None,
                row_patch_words: vec![],
                text_updates: vec![],
                text_replacements: vec![],
            },
            HotPatchMsg::DylibReload {
                generation: 99,
                dylib_path: "target/debug/libnve_studio_workspace.dylib".into(),
                build_ms: 287,
            },
            HotPatchMsg::DevStatus {
                message: "watching 4 crates".into(),
                latency_ms: 8,
            },
        ];

        for msg in &cases {
            let bin = msg.to_bytes().unwrap();
            assert!(bin.starts_with(&HOT_PATCH_WIRE_MAGIC));
            assert_eq!(bin[4], HOT_PATCH_VERSION);
            assert_eq!(HotPatchMsg::from_bytes(&bin).unwrap(), *msg);
            for cut in 0..bin.len() {
                assert_eq!(
                    HotPatchMsg::from_bytes(&bin[..cut]),
                    Err(HotPatchDecodeError::Truncated)
                );
            }
            let mut extended = bin.clone();
            extended.push(0xAA);
            assert_eq!(
                HotPatchMsg::from_bytes(&extended),
                Err(HotPatchDecodeError::TrailingBytes(1))
            );
        }

        assert_eq!(
            HotPatchMsg::from_bytes(b"NOPE\x02\x01"),
            Err(HotPatchDecodeError::InvalidMagic)
        );
        // Legacy v1 frames (version byte = 1) are rejected cleanly as UnsupportedVersion.
        assert_eq!(
            HotPatchMsg::from_bytes(b"NVP1\x01\x01"),
            Err(HotPatchDecodeError::UnsupportedVersion(1))
        );
        assert_eq!(
            HotPatchMsg::from_bytes(&[b'N', b'V', b'P', b'1', HOT_PATCH_VERSION, 0xFF]),
            Err(HotPatchDecodeError::UnknownVariant(0xFF))
        );
        let mut bad_oom = Vec::from(*b"NVP1");
        bad_oom.push(HOT_PATCH_VERSION);
        bad_oom.push(1);
        bad_oom.extend_from_slice(&1u64.to_le_bytes()); // generation
        bad_oom.extend_from_slice(&0u64.to_le_bytes()); // template_key
        bad_oom.extend_from_slice(&(u32::MAX).to_le_bytes()); // node_path len
        assert_eq!(
            HotPatchMsg::from_bytes(&bad_oom),
            Err(HotPatchDecodeError::LimitExceeded)
        );
        assert_eq!(
            HotPatchMsg::from_bytes(br#"{"type":"dev_status","message":"x","latency_ms":0}"#),
            Err(HotPatchDecodeError::InvalidMagic)
        );
        let mut bad_utf8 = Vec::from(*b"NVP1");
        bad_utf8.push(HOT_PATCH_VERSION);
        bad_utf8.push(3);
        bad_utf8.extend_from_slice(&2u32.to_le_bytes());
        bad_utf8.extend_from_slice(&[0xFF, 0xFE]);
        bad_utf8.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(
            HotPatchMsg::from_bytes(&bad_utf8),
            Err(HotPatchDecodeError::InvalidUtf8)
        );
        let mut bad_bool = Vec::from(*b"NVP1");
        bad_bool.push(HOT_PATCH_VERSION);
        bad_bool.push(1);
        bad_bool.extend_from_slice(&1u64.to_le_bytes()); // generation
        bad_bool.extend_from_slice(&0u64.to_le_bytes()); // template_key
        bad_bool.extend_from_slice(&0u32.to_le_bytes()); // node_path len = 0
        bad_bool.push(7); // invalid Option tag for target_test_id
        assert_eq!(
            HotPatchMsg::from_bytes(&bad_bool),
            Err(HotPatchDecodeError::InvalidBool(7))
        );

        // Encode-side cap matches decoder MAX_WIRE_ITEMS so oversized frames fail at encode time.
        let oversized = HotPatchMsg::TemplatePatch {
            generation: 1,
            template_key: 0,
            node_path: vec![0; MAX_WIRE_ITEMS + 1],
            target_test_id: None,
            row_patch_words: vec![],
            text_updates: vec![],
            text_replacements: vec![],
        };
        assert_eq!(
            oversized.to_bytes(),
            Err(CodecError::LenOverflow {
                len: MAX_WIRE_ITEMS + 1,
                max: MAX_WIRE_ITEMS,
            })
        );
    }
}
