//! Little-endian `NVS2` binary encoder and bounds-checked decoder with an XXH64 trailer.

use std::fmt;

use nve_codec::{ByteReader, ByteWriter, CodecError, DEFAULT_MAX_BYTES};
use nve_hash::xxh64;

use crate::search::{Posting, SearchDoc, SearchIndex};

pub const NVS2_MAGIC: &[u8; 4] = b"NVS2";
pub const NVS2_VERSION: u16 = 1;

const HEADER_BYTES: usize = 20;
const CHECKSUM_BYTES: usize = 8;
const MIN_DOC_BYTES: usize = 20;
const MIN_TOKEN_BYTES: usize = 8;
const MIN_TRIGRAM_BYTES: usize = 5;
const POSTING_BYTES: usize = 6;

/// Typed decoding error for `NVS2` binary buffers (never panics).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SearchIndexError {
    TooShort,
    InvalidMagic([u8; 4]),
    UnsupportedVersion(u16),
    ChecksumMismatch { expected: u64, actual: u64 },
    UnexpectedEof,
    InvalidUtf8,
    InvalidDocId { doc_id: u32, doc_count: u32 },
    NonZeroReserved(u16),
    TrailingBytes(usize),
}

impl From<CodecError> for SearchIndexError {
    #[inline]
    fn from(err: CodecError) -> Self {
        match err {
            CodecError::InvalidUtf8 => Self::InvalidUtf8,
            CodecError::NonZeroReserved(v) => Self::NonZeroReserved(v as u16),
            CodecError::TrailingBytes(n) => Self::TrailingBytes(n),
            _ => Self::UnexpectedEof,
        }
    }
}

impl fmt::Display for SearchIndexError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "NVS2 buffer too short"),
            Self::InvalidMagic(m) => write!(f, "invalid NVS2 magic: {m:?}"),
            Self::UnsupportedVersion(v) => write!(f, "unsupported NVS2 version: {v}"),
            Self::ChecksumMismatch { expected, actual } => write!(
                f,
                "NVS2 checksum mismatch: expected {expected:#018x}, got {actual:#018x}"
            ),
            Self::UnexpectedEof => write!(f, "unexpected end of NVS2 buffer"),
            Self::InvalidUtf8 => write!(f, "invalid UTF-8 string in NVS2 buffer"),
            Self::InvalidDocId { doc_id, doc_count } => {
                write!(f, "posting doc_id {doc_id} out of range (doc_count={doc_count})")
            }
            Self::NonZeroReserved(v) => write!(f, "NVS2 reserved header bytes must be 0, got {v}"),
            Self::TrailingBytes(n) => write!(f, "NVS2 buffer has {n} unexpected trailing bytes"),
        }
    }
}

impl std::error::Error for SearchIndexError {}

pub(crate) fn encode_index(index: &SearchIndex) -> Result<Vec<u8>, CodecError> {
    let mut w = ByteWriter::with_capacity(
        HEADER_BYTES + index.docs.len() * 64 + index.tokens.len() * 16 + CHECKSUM_BYTES,
    );
    w.write_raw(NVS2_MAGIC);
    w.write_u16(NVS2_VERSION);
    w.write_u16(0);
    w.write_len(index.docs.len())?;
    w.write_len(index.tokens.len())?;
    w.write_len(index.trigrams.len())?;

    for doc in &index.docs {
        w.write_str(&doc.route)?;
        w.write_str(&doc.title)?;
        w.write_str(&doc.section)?;
        w.write_str(&doc.anchor)?;
        w.write_str(&doc.excerpt)?;
    }

    for (term, postings) in &index.tokens {
        w.write_str(term)?;
        write_postings(&mut w, postings)?;
    }

    for (tri, postings) in &index.trigrams {
        w.write_str_u8(tri)?;
        write_postings(&mut w, postings)?;
    }

    let checksum = xxh64(w.as_slice(), 0);
    w.write_u64(checksum);
    Ok(w.into_bytes())
}

pub(crate) fn decode_index(bytes: &[u8]) -> Result<SearchIndex, SearchIndexError> {
    if bytes.len() < HEADER_BYTES + CHECKSUM_BYTES {
        return Err(SearchIndexError::TooShort);
    }
    let payload_len = bytes.len() - CHECKSUM_BYTES;
    let payload = &bytes[..payload_len];
    let mut r = ByteReader::new(payload);

    let magic = r.read_array::<4>()?;
    if &magic != NVS2_MAGIC {
        return Err(SearchIndexError::InvalidMagic(magic));
    }
    let version = r.read_u16()?;
    if version != NVS2_VERSION {
        return Err(SearchIndexError::UnsupportedVersion(version));
    }
    r.expect_zero(2)?;

    let expected_ck = ByteReader::new(&bytes[payload_len..]).read_u64()?;
    let actual_ck = xxh64(payload, 0);
    if expected_ck != actual_ck {
        return Err(SearchIndexError::ChecksumMismatch {
            expected: expected_ck,
            actual: actual_ck,
        });
    }

    let doc_count = r.read_u32()? as usize;
    let token_count = r.read_u32()? as usize;
    let trigram_count = r.read_u32()? as usize;

    if doc_count > r.remaining() / MIN_DOC_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut docs = Vec::with_capacity(doc_count);
    for _ in 0..doc_count {
        docs.push(SearchDoc {
            route: r.read_string(DEFAULT_MAX_BYTES)?,
            title: r.read_string(DEFAULT_MAX_BYTES)?,
            section: r.read_string(DEFAULT_MAX_BYTES)?,
            anchor: r.read_string(DEFAULT_MAX_BYTES)?,
            excerpt: r.read_string(DEFAULT_MAX_BYTES)?,
        });
    }

    if token_count > r.remaining() / MIN_TOKEN_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut tokens = Vec::with_capacity(token_count);
    for _ in 0..token_count {
        let term = r.read_string(DEFAULT_MAX_BYTES)?;
        let postings = read_postings(&mut r, doc_count as u32)?;
        tokens.push((term, postings));
    }

    if trigram_count > r.remaining() / MIN_TRIGRAM_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut trigrams = Vec::with_capacity(trigram_count);
    for _ in 0..trigram_count {
        let tri = r.read_str_u8(u8::MAX as usize)?.to_owned();
        let postings = read_postings(&mut r, doc_count as u32)?;
        trigrams.push((tri, postings));
    }

    r.finish()?;

    Ok(SearchIndex {
        docs,
        tokens,
        trigrams,
    })
}

fn write_postings(w: &mut ByteWriter, postings: &[Posting]) -> Result<(), CodecError> {
    w.write_len(postings.len())?;
    for p in postings {
        w.write_u32(p.doc_id);
        w.write_u16(p.weight_x10);
    }
    Ok(())
}

fn read_postings(r: &mut ByteReader<'_>, doc_count: u32) -> Result<Vec<Posting>, SearchIndexError> {
    let count = r.read_u32()? as usize;
    if count > r.remaining() / POSTING_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let doc_id = r.read_u32()?;
        if doc_id >= doc_count {
            return Err(SearchIndexError::InvalidDocId { doc_id, doc_count });
        }
        let weight_x10 = r.read_u16()?;
        out.push(Posting { doc_id, weight_x10 });
    }
    Ok(out)
}
