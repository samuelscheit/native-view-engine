use std::fmt;

/// 4-byte wire magic identifying a binary compact binary `HotPatchMsg` frame (`NVP1`).
pub const HOT_PATCH_WIRE_MAGIC: [u8; 4] = *b"NVP1";

/// Maximum item count accepted per vector inside a binary `NVP1` frame to prevent OOM on corrupt input.
const MAX_WIRE_ITEMS: usize = 65_536;

/// Error returned when decoding a binary `NVP1` `HotPatchMsg` frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HotPatchDecodeError {
    Truncated,
    InvalidMagic,
    UnknownVariant(u8),
    InvalidBool(u8),
    InvalidUtf8,
    LimitExceeded,
    TrailingBytes(usize),
}

impl fmt::Display for HotPatchDecodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Truncated => f.write_str("truncated NVP1 hot-patch buffer"),
            Self::InvalidMagic => f.write_str("invalid NVP1 hot-patch magic"),
            Self::UnknownVariant(v) => write!(f, "unknown NVP1 hot-patch variant tag {v}"),
            Self::InvalidBool(v) => write!(f, "invalid NVP1 boolean tag {v}"),
            Self::InvalidUtf8 => f.write_str("invalid UTF-8 in NVP1 hot-patch string"),
            Self::LimitExceeded => f.write_str("NVP1 collection limit exceeded"),
            Self::TrailingBytes(n) => write!(f, "trailing {n} byte(s) in NVP1 hot-patch buffer"),
        }
    }
}

impl std::error::Error for HotPatchDecodeError {}

/// Unified 2-tier hot-reload protocol broadcast by `nve dev` over WebSocket (binary `NVP1`
/// frames, see [`HotPatchMsg::to_bytes`]) to Desktop Skia, iOS Simulator/Device, Android
/// Emulator/Device, and Web clients, and handed to apps through `NveDevApi::apply_hot_patch`.
/// There is no JSON form.
#[derive(Debug, Clone, PartialEq)]
pub enum HotPatchMsg {
    /// Tier 1 (<10ms, zero recompile, works on physical iOS devices & browsers):
    /// Patches targeted text or style literals inside a live node/template without restarting Rust.
    TemplatePatch {
        generation: u64,
        template_id: String,
        attr_updates: Vec<(String, String)>,
        text_updates: Vec<(usize, String)>,
        /// Optional `test_id` / `data-testid` / `testID` targeting a specific RSX element.
        target_test_id: Option<String>,
        /// Optional structural child index path inside the template tree (from root).
        node_path: Vec<usize>,
        /// Exact `(old_text, new_text)` static text replacements for surgical multi-node patching.
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

fn write_str(out: &mut Vec<u8>, s: &str) {
    let b = s.as_bytes();
    out.extend_from_slice(&(b.len() as u32).to_le_bytes());
    out.extend_from_slice(b);
}

struct WireReader<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> WireReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], HotPatchDecodeError> {
        if self.bytes.len().saturating_sub(self.pos) < n {
            return Err(HotPatchDecodeError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    fn read_u8(&mut self) -> Result<u8, HotPatchDecodeError> {
        Ok(self.take(1)?[0])
    }

    fn read_u32(&mut self) -> Result<u32, HotPatchDecodeError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_u64(&mut self) -> Result<u64, HotPatchDecodeError> {
        let b = self.take(8)?;
        Ok(u64::from_le_bytes([
            b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
        ]))
    }

    fn read_count(&mut self) -> Result<usize, HotPatchDecodeError> {
        let count = self.read_u32()? as usize;
        if count > MAX_WIRE_ITEMS {
            return Err(HotPatchDecodeError::LimitExceeded);
        }
        Ok(count)
    }

    fn read_string(&mut self) -> Result<String, HotPatchDecodeError> {
        let len = self.read_u32()? as usize;
        let slice = self.take(len)?;
        std::str::from_utf8(slice)
            .map(str::to_owned)
            .map_err(|_| HotPatchDecodeError::InvalidUtf8)
    }

    fn finish(self) -> Result<(), HotPatchDecodeError> {
        let rem = self.bytes.len().saturating_sub(self.pos);
        if rem != 0 {
            return Err(HotPatchDecodeError::TrailingBytes(rem));
        }
        Ok(())
    }
}

impl HotPatchMsg {
    /// Encodes this hot-patch message into compact binary `NVP1` framing.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64);
        out.extend_from_slice(&HOT_PATCH_WIRE_MAGIC);
        match self {
            Self::TemplatePatch {
                generation,
                template_id,
                attr_updates,
                text_updates,
                target_test_id,
                node_path,
                text_replacements,
            } => {
                out.push(1);
                out.extend_from_slice(&generation.to_le_bytes());
                write_str(&mut out, template_id);

                out.extend_from_slice(&(attr_updates.len() as u32).to_le_bytes());
                for (k, v) in attr_updates {
                    write_str(&mut out, k);
                    write_str(&mut out, v);
                }

                out.extend_from_slice(&(text_updates.len() as u32).to_le_bytes());
                for (idx, text) in text_updates {
                    out.extend_from_slice(&(*idx as u32).to_le_bytes());
                    write_str(&mut out, text);
                }

                match target_test_id {
                    Some(tid) => {
                        out.push(1);
                        write_str(&mut out, tid);
                    }
                    None => out.push(0),
                }

                out.extend_from_slice(&(node_path.len() as u32).to_le_bytes());
                for &step in node_path {
                    out.extend_from_slice(&(step as u32).to_le_bytes());
                }

                out.extend_from_slice(&(text_replacements.len() as u32).to_le_bytes());
                for (old_text, new_text) in text_replacements {
                    write_str(&mut out, old_text);
                    write_str(&mut out, new_text);
                }
            }
            Self::DylibReload {
                generation,
                dylib_path,
                build_ms,
            } => {
                out.push(2);
                out.extend_from_slice(&generation.to_le_bytes());
                write_str(&mut out, dylib_path);
                out.extend_from_slice(&build_ms.to_le_bytes());
            }
            Self::DevStatus {
                message,
                latency_ms,
            } => {
                out.push(3);
                write_str(&mut out, message);
                out.extend_from_slice(&latency_ms.to_le_bytes());
            }
        }
        out
    }

    /// Decodes a compact binary `NVP1` hot-patch frame.
    pub fn from_bytes(raw: &[u8]) -> Result<Self, HotPatchDecodeError> {
        let mut r = WireReader::new(raw);
        let magic = r.take(4)?;
        if magic != HOT_PATCH_WIRE_MAGIC {
            return Err(HotPatchDecodeError::InvalidMagic);
        }
        let variant = r.read_u8()?;
        let msg = match variant {
            1 => {
                let generation = r.read_u64()?;
                let template_id = r.read_string()?;

                let attr_len = r.read_count()?;
                let mut attr_updates = Vec::with_capacity(attr_len.min(1024));
                for _ in 0..attr_len {
                    let k = r.read_string()?;
                    let v = r.read_string()?;
                    attr_updates.push((k, v));
                }

                let text_len = r.read_count()?;
                let mut text_updates = Vec::with_capacity(text_len.min(1024));
                for _ in 0..text_len {
                    let idx = r.read_u32()? as usize;
                    let text = r.read_string()?;
                    text_updates.push((idx, text));
                }

                let target_test_id = match r.read_u8()? {
                    0 => None,
                    1 => Some(r.read_string()?),
                    other => return Err(HotPatchDecodeError::InvalidBool(other)),
                };

                let path_len = r.read_count()?;
                let mut node_path = Vec::with_capacity(path_len.min(1024));
                for _ in 0..path_len {
                    node_path.push(r.read_u32()? as usize);
                }

                let repl_len = r.read_count()?;
                let mut text_replacements = Vec::with_capacity(repl_len.min(1024));
                for _ in 0..repl_len {
                    let old_t = r.read_string()?;
                    let new_t = r.read_string()?;
                    text_replacements.push((old_t, new_t));
                }

                Self::TemplatePatch {
                    generation,
                    template_id,
                    attr_updates,
                    text_updates,
                    target_test_id,
                    node_path,
                    text_replacements,
                }
            }
            2 => {
                let generation = r.read_u64()?;
                let dylib_path = r.read_string()?;
                let build_ms = r.read_u64()?;
                Self::DylibReload {
                    generation,
                    dylib_path,
                    build_ms,
                }
            }
            3 => {
                let message = r.read_string()?;
                let latency_ms = r.read_u64()?;
                Self::DevStatus {
                    message,
                    latency_ms,
                }
            }
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
    fn nvp1_roundtrips_all_variants_and_rejects_corrupt_inputs() {
        let cases = vec![
            HotPatchMsg::TemplatePatch {
                generation: 42,
                template_id: "examples/studio-workspace/src/ui.rs".into(),
                attr_updates: vec![
                    ("style".into(), "padding: 12px; opacity: 0.9;".into()),
                    ("color".into(), "#38bdf8".into()),
                ],
                text_updates: vec![(0, "AETHER".into()), (3, "東京テスト".into())],
                target_test_id: Some("studio-panel".into()),
                node_path: vec![0, 2, 1],
                text_replacements: vec![("Old".into(), "New".into())],
            },
            HotPatchMsg::TemplatePatch {
                generation: 1,
                template_id: String::new(),
                attr_updates: vec![],
                text_updates: vec![],
                target_test_id: None,
                node_path: vec![],
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
            let bin = msg.to_bytes();
            assert!(bin.starts_with(&HOT_PATCH_WIRE_MAGIC));
            assert_eq!(HotPatchMsg::from_bytes(&bin).unwrap(), *msg);
            // Every truncated prefix must fail with Truncated (never panic)
            for cut in 0..bin.len() {
                assert_eq!(
                    HotPatchMsg::from_bytes(&bin[..cut]),
                    Err(HotPatchDecodeError::Truncated)
                );
            }
            // Trailing byte must fail with TrailingBytes
            let mut extended = bin.clone();
            extended.push(0xAA);
            assert_eq!(
                HotPatchMsg::from_bytes(&extended),
                Err(HotPatchDecodeError::TrailingBytes(1))
            );
        }

        // Corrupt magic, unknown variant, invalid bool, invalid UTF-8, and OOM count guard.
        assert_eq!(
            HotPatchMsg::from_bytes(b"NOPE\x01"),
            Err(HotPatchDecodeError::InvalidMagic)
        );
        assert_eq!(
            HotPatchMsg::from_bytes(b"NVP1\xFF"),
            Err(HotPatchDecodeError::UnknownVariant(0xFF))
        );
        let mut bad_oom = Vec::from(*b"NVP1");
        bad_oom.push(1);
        bad_oom.extend_from_slice(&1u64.to_le_bytes());
        bad_oom.extend_from_slice(&0u32.to_le_bytes()); // template_id
        bad_oom.extend_from_slice(&(u32::MAX).to_le_bytes()); // attr_updates count
        assert_eq!(
            HotPatchMsg::from_bytes(&bad_oom),
            Err(HotPatchDecodeError::LimitExceeded)
        );
        // A JSON text frame (the removed legacy format) is rejected by magic, never parsed.
        assert_eq!(
            HotPatchMsg::from_bytes(br#"{"type":"dev_status","message":"x","latency_ms":0}"#),
            Err(HotPatchDecodeError::InvalidMagic)
        );
        let mut bad_utf8 = Vec::from(*b"NVP1");
        bad_utf8.push(3);
        bad_utf8.extend_from_slice(&2u32.to_le_bytes());
        bad_utf8.extend_from_slice(&[0xFF, 0xFE]);
        bad_utf8.extend_from_slice(&0u64.to_le_bytes());
        assert_eq!(HotPatchMsg::from_bytes(&bad_utf8), Err(HotPatchDecodeError::InvalidUtf8));
        let mut bad_bool = Vec::from(*b"NVP1");
        bad_bool.push(1);
        bad_bool.extend_from_slice(&1u64.to_le_bytes());
        bad_bool.extend_from_slice(&[0; 12]); // template_id, attr count, text count
        bad_bool.push(7);
        assert_eq!(HotPatchMsg::from_bytes(&bad_bool), Err(HotPatchDecodeError::InvalidBool(7)));
    }
}
