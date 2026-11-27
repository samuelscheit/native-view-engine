//! Little-endian `NVS2` binary encoder and bounds-checked decoder with an XXH64 trailer.

use std::fmt;

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
    TrailingBytes(usize),
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
            Self::TrailingBytes(n) => write!(f, "NVS2 buffer has {n} unexpected trailing bytes"),
        }
    }
}

impl std::error::Error for SearchIndexError {}

pub(crate) fn encode_index(index: &SearchIndex) -> Vec<u8> {
    let mut buf = Vec::with_capacity(
        HEADER_BYTES + index.docs.len() * 64 + index.tokens.len() * 16 + CHECKSUM_BYTES,
    );
    buf.extend_from_slice(NVS2_MAGIC);
    buf.extend_from_slice(&NVS2_VERSION.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes());
    buf.extend_from_slice(&(index.docs.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(index.tokens.len() as u32).to_le_bytes());
    buf.extend_from_slice(&(index.trigrams.len() as u32).to_le_bytes());

    for doc in &index.docs {
        write_str_u32(&mut buf, &doc.route);
        write_str_u32(&mut buf, &doc.title);
        write_str_u32(&mut buf, &doc.section);
        write_str_u32(&mut buf, &doc.anchor);
        write_str_u32(&mut buf, &doc.excerpt);
    }

    for (term, postings) in &index.tokens {
        write_str_u32(&mut buf, term);
        write_postings(&mut buf, postings);
    }

    for (tri, postings) in &index.trigrams {
        let tri_bytes = tri.as_bytes();
        let len = tri_bytes.len().min(u8::MAX as usize);
        buf.push(len as u8);
        buf.extend_from_slice(&tri_bytes[..len]);
        write_postings(&mut buf, postings);
    }

    let checksum = xxh64(&buf, 0);
    buf.extend_from_slice(&checksum.to_le_bytes());
    buf
}

pub(crate) fn decode_index(bytes: &[u8]) -> Result<SearchIndex, SearchIndexError> {
    if bytes.len() < HEADER_BYTES + CHECKSUM_BYTES {
        return Err(SearchIndexError::TooShort);
    }
    let magic: [u8; 4] = bytes[0..4].try_into().unwrap();
    if &magic != NVS2_MAGIC {
        return Err(SearchIndexError::InvalidMagic(magic));
    }
    let version = u16::from_le_bytes([bytes[4], bytes[5]]);
    if version != NVS2_VERSION {
        return Err(SearchIndexError::UnsupportedVersion(version));
    }

    let payload_len = bytes.len() - CHECKSUM_BYTES;
    let payload = &bytes[..payload_len];
    let expected_ck = u64::from_le_bytes(bytes[payload_len..].try_into().unwrap());
    let actual_ck = xxh64(payload, 0);
    if expected_ck != actual_ck {
        return Err(SearchIndexError::ChecksumMismatch {
            expected: expected_ck,
            actual: actual_ck,
        });
    }

    let mut r = Reader::new(&payload[8..]);
    let doc_count = r.read_u32()? as usize;
    let token_count = r.read_u32()? as usize;
    let trigram_count = r.read_u32()? as usize;

    if doc_count > r.remaining() / MIN_DOC_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut docs = Vec::with_capacity(doc_count);
    for _ in 0..doc_count {
        docs.push(SearchDoc {
            route: r.read_str_u32()?,
            title: r.read_str_u32()?,
            section: r.read_str_u32()?,
            anchor: r.read_str_u32()?,
            excerpt: r.read_str_u32()?,
        });
    }

    if token_count > r.remaining() / MIN_TOKEN_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut tokens = Vec::with_capacity(token_count);
    for _ in 0..token_count {
        let term = r.read_str_u32()?;
        let postings = r.read_postings(doc_count as u32)?;
        tokens.push((term, postings));
    }

    if trigram_count > r.remaining() / MIN_TRIGRAM_BYTES {
        return Err(SearchIndexError::UnexpectedEof);
    }
    let mut trigrams = Vec::with_capacity(trigram_count);
    for _ in 0..trigram_count {
        let tri = r.read_str_u8()?;
        let postings = r.read_postings(doc_count as u32)?;
        trigrams.push((tri, postings));
    }

    if r.remaining() != 0 {
        return Err(SearchIndexError::TrailingBytes(r.remaining()));
    }

    Ok(SearchIndex {
        docs,
        tokens,
        trigrams,
    })
}

fn write_str_u32(buf: &mut Vec<u8>, s: &str) {
    let bytes = s.as_bytes();
    buf.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
    buf.extend_from_slice(bytes);
}

fn write_postings(buf: &mut Vec<u8>, postings: &[Posting]) {
    buf.extend_from_slice(&(postings.len() as u32).to_le_bytes());
    for p in postings {
        buf.extend_from_slice(&p.doc_id.to_le_bytes());
        buf.extend_from_slice(&p.weight_x10.to_le_bytes());
    }
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    fn read_exact(&mut self, len: usize) -> Result<&'a [u8], SearchIndexError> {
        let end = self
            .pos
            .checked_add(len)
            .ok_or(SearchIndexError::UnexpectedEof)?;
        if end > self.data.len() {
            return Err(SearchIndexError::UnexpectedEof);
        }
        let slice = &self.data[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn read_u8(&mut self) -> Result<u8, SearchIndexError> {
        Ok(self.read_exact(1)?[0])
    }

    fn read_u16(&mut self) -> Result<u16, SearchIndexError> {
        let b = self.read_exact(2)?;
        Ok(u16::from_le_bytes([b[0], b[1]]))
    }

    fn read_u32(&mut self) -> Result<u32, SearchIndexError> {
        let b = self.read_exact(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    fn read_str_u32(&mut self) -> Result<String, SearchIndexError> {
        let len = self.read_u32()? as usize;
        let bytes = self.read_exact(len)?;
        std::str::from_utf8(bytes)
            .map(ToOwned::to_owned)
            .map_err(|_| SearchIndexError::InvalidUtf8)
    }

    fn read_str_u8(&mut self) -> Result<String, SearchIndexError> {
        let len = self.read_u8()? as usize;
        let bytes = self.read_exact(len)?;
        std::str::from_utf8(bytes)
            .map(ToOwned::to_owned)
            .map_err(|_| SearchIndexError::InvalidUtf8)
    }

    fn read_postings(&mut self, doc_count: u32) -> Result<Vec<Posting>, SearchIndexError> {
        let count = self.read_u32()? as usize;
        if count > self.remaining() / POSTING_BYTES {
            return Err(SearchIndexError::UnexpectedEof);
        }
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            let doc_id = self.read_u32()?;
            if doc_id >= doc_count {
                return Err(SearchIndexError::InvalidDocId { doc_id, doc_count });
            }
            let weight_x10 = self.read_u16()?;
            out.push(Posting { doc_id, weight_x10 });
        }
        Ok(out)
    }
}
