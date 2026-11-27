//! Target-independent sequencing, byte-cap, and Base64 pre-validation state machine for Option B
//! `<script type="application/x-nve-query" data-nve-seq="N">` SSR hydration streams.

use std::borrow::Cow;
use std::fmt;

/// Hard cap on the number of `application/x-nve-query` chunks ingested per document.
pub(crate) const MAX_STREAM_CHUNKS: usize = 256;

/// Hard cap on total decoded `NVEQ` payload bytes ingested per document (8 MiB).
pub(crate) const MAX_STREAM_TOTAL_BYTES: usize = 8 * 1024 * 1024;

/// Reason a streaming query chunk was rejected before or after Base64 decoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamReject {
    NonMonotonicSeq { seq: u64, last_seq: u64 },
    ChunkCapExceeded { max_chunks: usize },
    EmptyPayload,
    ByteCapExceeded { current_bytes: usize, max_bytes: usize },
}

impl fmt::Display for StreamReject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            Self::NonMonotonicSeq { seq, last_seq } => {
                write!(f, "non-monotonic or duplicate seq {seq} (last={last_seq})")
            }
            Self::ChunkCapExceeded { max_chunks } => {
                write!(f, "exceeded MAX_STREAM_CHUNKS ({max_chunks})")
            }
            Self::EmptyPayload => write!(f, "empty Base64 payload"),
            Self::ByteCapExceeded {
                current_bytes,
                max_bytes,
            } => write!(
                f,
                "exceeded MAX_STREAM_TOTAL_BYTES ({current_bytes} / {max_bytes} bytes)"
            ),
        }
    }
}

/// Tracks monotonic sequence numbers and safety caps across Phase 1 and mid-stream chunks.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct StreamIngestState {
    pub(crate) last_seq: Option<u64>,
    pub(crate) chunks_ingested: usize,
    pub(crate) total_bytes: usize,
}

impl StreamIngestState {
    /// Verifies `seq` is strictly monotonic and the chunk count cap has not been reached.
    pub(crate) fn check_header(&self, seq: u64) -> Result<(), StreamReject> {
        if let Some(prev) = self.last_seq
            && seq <= prev
        {
            return Err(StreamReject::NonMonotonicSeq {
                seq,
                last_seq: prev,
            });
        }
        if self.chunks_ingested >= MAX_STREAM_CHUNKS {
            return Err(StreamReject::ChunkCapExceeded {
                max_chunks: MAX_STREAM_CHUNKS,
            });
        }
        Ok(())
    }

    /// Strips ASCII whitespace from `raw` while enforcing a pre-decode byte-cap bound so an
    /// oversize Base64 block is rejected before allocating or decoding.
    pub(crate) fn prepare_base64<'a>(&self, raw: &'a str) -> Result<Cow<'a, [u8]>, StreamReject> {
        let mut non_ws = 0usize;
        let mut has_ws = false;
        for &b in raw.as_bytes() {
            if b.is_ascii_whitespace() {
                has_ws = true;
            } else {
                non_ws += 1;
            }
        }
        if non_ws == 0 {
            return Err(StreamReject::EmptyPayload);
        }
        let remaining = MAX_STREAM_TOTAL_BYTES.saturating_sub(self.total_bytes);
        let max_b64_len = remaining.div_ceil(3).saturating_mul(4);
        if non_ws > max_b64_len {
            return Err(StreamReject::ByteCapExceeded {
                current_bytes: self.total_bytes,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            });
        }
        if !has_ws {
            return Ok(Cow::Borrowed(raw.as_bytes()));
        }
        let mut cleaned = Vec::with_capacity(non_ws);
        for &b in raw.as_bytes() {
            if !b.is_ascii_whitespace() {
                cleaned.push(b);
            }
        }
        Ok(Cow::Owned(cleaned))
    }

    /// Verifies `total_bytes + decoded_len <= MAX_STREAM_TOTAL_BYTES` without overflow.
    pub(crate) fn check_decoded_len(&self, decoded_len: usize) -> Result<(), StreamReject> {
        if self
            .total_bytes
            .checked_add(decoded_len)
            .is_some_and(|total| total <= MAX_STREAM_TOTAL_BYTES)
        {
            Ok(())
        } else {
            Err(StreamReject::ByteCapExceeded {
                current_bytes: self.total_bytes,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            })
        }
    }

    /// Records one successfully hydrated chunk.
    pub(crate) fn record_ingested(&mut self, seq: u64, decoded_len: usize) {
        self.last_seq = Some(seq);
        self.chunks_ingested = self.chunks_ingested.saturating_add(1);
        self.total_bytes = self.total_bytes.saturating_add(decoded_len);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn enforces_strict_monotonic_seq() {
        let mut state = StreamIngestState::default();
        assert_eq!(state.check_header(0), Ok(()));
        state.record_ingested(0, 16);

        assert_eq!(
            state.check_header(0),
            Err(StreamReject::NonMonotonicSeq { seq: 0, last_seq: 0 })
        );
        assert_eq!(state.check_header(1), Ok(()));
        state.record_ingested(1, 32);

        assert_eq!(state.check_header(5), Ok(()));
        state.record_ingested(5, 64);

        assert_eq!(
            state.check_header(3),
            Err(StreamReject::NonMonotonicSeq { seq: 3, last_seq: 5 })
        );
        assert_eq!(
            state.check_header(5),
            Err(StreamReject::NonMonotonicSeq { seq: 5, last_seq: 5 })
        );
        assert_eq!(state.check_header(6), Ok(()));
    }

    #[test]
    fn enforces_max_stream_chunks_cap() {
        let mut state = StreamIngestState::default();
        for seq in 0..(MAX_STREAM_CHUNKS as u64) {
            assert_eq!(state.check_header(seq), Ok(()));
            state.record_ingested(seq, 1);
        }
        assert_eq!(state.chunks_ingested, MAX_STREAM_CHUNKS);
        assert_eq!(
            state.check_header(MAX_STREAM_CHUNKS as u64),
            Err(StreamReject::ChunkCapExceeded {
                max_chunks: MAX_STREAM_CHUNKS,
            })
        );
    }

    #[test]
    fn enforces_byte_cap_and_overflow_safety() {
        let mut state = StreamIngestState::default();
        assert_eq!(state.check_decoded_len(MAX_STREAM_TOTAL_BYTES), Ok(()));
        assert_eq!(
            state.check_decoded_len(MAX_STREAM_TOTAL_BYTES + 1),
            Err(StreamReject::ByteCapExceeded {
                current_bytes: 0,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            })
        );
        assert_eq!(
            state.check_decoded_len(usize::MAX),
            Err(StreamReject::ByteCapExceeded {
                current_bytes: 0,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            })
        );

        state.record_ingested(0, MAX_STREAM_TOTAL_BYTES - 3);
        assert_eq!(state.check_decoded_len(3), Ok(()));
        assert_eq!(
            state.check_decoded_len(4),
            Err(StreamReject::ByteCapExceeded {
                current_bytes: MAX_STREAM_TOTAL_BYTES - 3,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            })
        );
        // 4 base64 chars fit in 3 remaining bytes; 8 base64 chars are rejected pre-decode.
        assert!(state.prepare_base64("AAAA").is_ok());
        assert_eq!(
            state.prepare_base64("AAAAAAAA"),
            Err(StreamReject::ByteCapExceeded {
                current_bytes: MAX_STREAM_TOTAL_BYTES - 3,
                max_bytes: MAX_STREAM_TOTAL_BYTES,
            })
        );
    }

    #[test]
    fn strips_ascii_whitespace_and_rejects_empty_payloads() {
        let state = StreamIngestState::default();
        assert_eq!(state.prepare_base64(""), Err(StreamReject::EmptyPayload));
        assert_eq!(
            state.prepare_base64(" \t\r\n "),
            Err(StreamReject::EmptyPayload)
        );

        let clean = state.prepare_base64("AQID").expect("clean base64");
        assert!(matches!(clean, Cow::Borrowed(b"AQID")));

        let spaced = state
            .prepare_base64("  AQ\n\tID\r\n")
            .expect("whitespace-stripped base64");
        assert!(matches!(spaced, Cow::Owned(ref v) if v == b"AQID"));
    }

    #[test]
    fn reject_display_messages_are_non_empty() {
        let cases = [
            StreamReject::NonMonotonicSeq { seq: 1, last_seq: 2 },
            StreamReject::ChunkCapExceeded { max_chunks: 256 },
            StreamReject::EmptyPayload,
            StreamReject::ByteCapExceeded {
                current_bytes: 10,
                max_bytes: 20,
            },
        ];
        for err in cases {
            assert!(!err.to_string().is_empty());
        }
    }

    #[cfg(feature = "app-issues")]
    #[test]
    fn base64_whitespace_strip_decodes_cleanly_and_rejects_corrupt_input() {
        use base64::Engine as _;

        let mut state = StreamIngestState::default();
        let raw = b"NVEQ-test-payload";
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        let wrapped = format!("  {}\n\t{}  \r\n", &encoded[..8], &encoded[8..]);
        let prepared = state.prepare_base64(&wrapped).expect("valid prepared b64");
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(prepared.as_ref())
            .expect("valid base64");
        assert_eq!(decoded, raw);
        assert_eq!(state.check_decoded_len(decoded.len()), Ok(()));
        state.record_ingested(0, decoded.len());
        assert_eq!(state.last_seq, Some(0));
        assert_eq!(state.chunks_ingested, 1);
        assert_eq!(state.total_bytes, raw.len());

        let bad = state.prepare_base64("!!!not-base64!!!").expect("within cap");
        assert!(base64::engine::general_purpose::STANDARD
            .decode(bad.as_ref())
            .is_err());
    }
}
