//! Workload A Baseline 1 codecs:
//! - **Baseline 1a (`json`)**: Zero-dependency `serde_json`-equivalent JSON serializer &
//!   deserializer into owned `Vec<OwnedTask>`.
//! - **Baseline 1b (`nve_snap`)**: `NVE_SNAP` container (`nve_engine::encode_snapshot` +
//!   `MmapSnapshot::open`) wrapping a compact binary encoding deserialized into owned
//!   `Vec<OwnedTask>`.

use std::fmt;
use std::io;
use std::path::Path;

use nve_engine::{encode_snapshot, MmapSnapshot};

use super::os_mmap::{read_file_bytes, write_file_bytes};
use super::OwnedTask;

const SNAP_SCHEMA_VERSION: u32 = 1;
const BIN_MAGIC: [u8; 4] = *b"NVTK";

#[derive(Debug)]
pub enum CodecError {
    Io(io::Error),
    Malformed(&'static str),
    InvalidUtf8,
    Snapshot(String),
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Malformed(msg) => write!(f, "malformed payload: {msg}"),
            Self::InvalidUtf8 => write!(f, "invalid utf-8"),
            Self::Snapshot(msg) => write!(f, "snapshot error: {msg}"),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<io::Error> for CodecError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

// ============================================================================
// Baseline 1a: JSON (serde_json-style text serialize / deserialize)
// ============================================================================

#[inline]
fn push_u64_ascii(out: &mut Vec<u8>, mut v: u64) {
    let mut buf = [0u8; 20];
    let mut pos = buf.len();
    loop {
        pos -= 1;
        buf[pos] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            break;
        }
    }
    out.extend_from_slice(&buf[pos..]);
}

#[inline]
fn push_i64_ascii(out: &mut Vec<u8>, v: i64) {
    if v < 0 {
        out.push(b'-');
        push_u64_ascii(out, v.unsigned_abs());
    } else {
        push_u64_ascii(out, v as u64);
    }
}

#[inline]
fn push_json_str(out: &mut Vec<u8>, s: &str) {
    out.push(b'"');
    let bytes = s.as_bytes();
    let mut start = 0usize;
    for (i, &b) in bytes.iter().enumerate() {
        let esc = match b {
            b'"' => Some(b"\\\"".as_slice()),
            b'\\' => Some(b"\\\\".as_slice()),
            b'\n' => Some(b"\\n".as_slice()),
            b'\r' => Some(b"\\r".as_slice()),
            b'\t' => Some(b"\\t".as_slice()),
            0x00..=0x1F => {
                out.extend_from_slice(&bytes[start..i]);
                let hex = [
                    b'\\',
                    b'u',
                    b'0',
                    b'0',
                    hex_nibble(b >> 4),
                    hex_nibble(b & 0x0F),
                ];
                out.extend_from_slice(&hex);
                start = i + 1;
                None
            }
            _ => None,
        };
        if let Some(seq) = esc {
            out.extend_from_slice(&bytes[start..i]);
            out.extend_from_slice(seq);
            start = i + 1;
        }
    }
    out.extend_from_slice(&bytes[start..]);
    out.push(b'"');
}

#[inline]
const fn hex_nibble(n: u8) -> u8 {
    match n {
        0..=9 => b'0' + n,
        _ => b'a' + (n - 10),
    }
}

/// Serializes `tasks` to compact JSON (`[{"id":...,"created_at_ms":...},...]`).
pub fn encode_tasks_json(tasks: &[OwnedTask]) -> Vec<u8> {
    let mut out = Vec::with_capacity(tasks.len() * 340 + 2);
    out.push(b'[');
    for (idx, t) in tasks.iter().enumerate() {
        if idx > 0 {
            out.push(b',');
        }
        out.extend_from_slice(br#"{"id":"#);
        push_u64_ascii(&mut out, t.id);
        out.extend_from_slice(br#","created_at_ms":"#);
        push_i64_ascii(&mut out, t.created_at_ms);
        out.extend_from_slice(br#","updated_at_ms":"#);
        push_i64_ascii(&mut out, t.updated_at_ms);
        out.extend_from_slice(br#","priority":"#);
        push_u64_ascii(&mut out, t.priority as u64);
        out.extend_from_slice(br#","status":"#);
        push_u64_ascii(&mut out, t.status as u64);
        out.extend_from_slice(br#","assignee_id":"#);
        push_u64_ascii(&mut out, t.assignee_id as u64);
        out.extend_from_slice(br#","flags":"#);
        push_u64_ascii(&mut out, t.flags as u64);
        out.extend_from_slice(br#","title":"#);
        push_json_str(&mut out, &t.title);
        out.extend_from_slice(br#","body":"#);
        push_json_str(&mut out, &t.body);
        out.extend_from_slice(br#","tag":"#);
        push_json_str(&mut out, &t.tag);
        out.push(b'}');
    }
    out.push(b']');
    out
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> JsonParser<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, pos: 0 }
    }

    #[inline]
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                b' ' | b'\n' | b'\r' | b'\t' => self.pos += 1,
                _ => break,
            }
        }
    }

    #[inline]
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    #[inline]
    fn expect(&mut self, ch: u8) -> Result<(), CodecError> {
        self.skip_ws();
        if self.peek() == Some(ch) {
            self.pos += 1;
            Ok(())
        } else {
            Err(CodecError::Malformed("unexpected json token"))
        }
    }

    fn parse_str_slice_or_owned(&mut self) -> Result<String, CodecError> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return Err(CodecError::Malformed("expected json string"));
        }
        self.pos += 1;
        let start = self.pos;
        let mut has_escape = false;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if b == b'"' {
                let raw = &self.bytes[start..self.pos];
                self.pos += 1;
                if !has_escape {
                    let s = std::str::from_utf8(raw).map_err(|_| CodecError::InvalidUtf8)?;
                    return Ok(s.to_owned());
                }
                return unescape_json_string(raw);
            }
            if b == b'\\' {
                has_escape = true;
                self.pos += 2;
                continue;
            }
            self.pos += 1;
        }
        Err(CodecError::Malformed("unterminated json string"))
    }

    fn parse_u64(&mut self) -> Result<u64, CodecError> {
        self.skip_ws();
        let start = self.pos;
        let mut val: u64 = 0;
        while let Some(&b) = self.bytes.get(self.pos) {
            if !b.is_ascii_digit() {
                break;
            }
            val = val
                .checked_mul(10)
                .and_then(|v| v.checked_add((b - b'0') as u64))
                .ok_or(CodecError::Malformed("u64 overflow"))?;
            self.pos += 1;
        }
        if self.pos == start {
            return Err(CodecError::Malformed("expected u64"));
        }
        Ok(val)
    }

    fn parse_i64(&mut self) -> Result<i64, CodecError> {
        self.skip_ws();
        let neg = if self.peek() == Some(b'-') {
            self.pos += 1;
            true
        } else {
            false
        };
        let mag = self.parse_u64()?;
        if neg {
            if mag == (i64::MAX as u64) + 1 {
                Ok(i64::MIN)
            } else {
                i64::try_from(mag)
                    .map(|v| -v)
                    .map_err(|_| CodecError::Malformed("i64 underflow"))
            }
        } else {
            i64::try_from(mag).map_err(|_| CodecError::Malformed("i64 overflow"))
        }
    }

    fn parse_key_borrowed(&mut self) -> Result<&'a str, CodecError> {
        self.skip_ws();
        if self.peek() != Some(b'"') {
            return Err(CodecError::Malformed("expected object key"));
        }
        self.pos += 1;
        let start = self.pos;
        while self.pos < self.bytes.len() {
            if self.bytes[self.pos] == b'"' {
                let slice = &self.bytes[start..self.pos];
                self.pos += 1;
                return std::str::from_utf8(slice).map_err(|_| CodecError::InvalidUtf8);
            }
            self.pos += 1;
        }
        Err(CodecError::Malformed("unterminated object key"))
    }

    fn parse_task(&mut self) -> Result<OwnedTask, CodecError> {
        self.expect(b'{')?;
        let mut id = 0u64;
        let mut created_at_ms = 0i64;
        let mut updated_at_ms = 0i64;
        let mut priority = 0u32;
        let mut status = 0u32;
        let mut assignee_id = 0u32;
        let mut flags = 0u32;
        let mut title = String::new();
        let mut body = String::new();
        let mut tag = String::new();

        self.skip_ws();
        if self.peek() != Some(b'}') {
            loop {
                let key = self.parse_key_borrowed()?;
                self.expect(b':')?;
                match key {
                    "id" => id = self.parse_u64()?,
                    "created_at_ms" => created_at_ms = self.parse_i64()?,
                    "updated_at_ms" => updated_at_ms = self.parse_i64()?,
                    "priority" => priority = self.parse_u64()? as u32,
                    "status" => status = self.parse_u64()? as u32,
                    "assignee_id" => assignee_id = self.parse_u64()? as u32,
                    "flags" => flags = self.parse_u64()? as u32,
                    "title" => title = self.parse_str_slice_or_owned()?,
                    "body" => body = self.parse_str_slice_or_owned()?,
                    "tag" => tag = self.parse_str_slice_or_owned()?,
                    _ => return Err(CodecError::Malformed("unknown task field")),
                }
                self.skip_ws();
                match self.peek() {
                    Some(b',') => self.pos += 1,
                    Some(b'}') => break,
                    _ => return Err(CodecError::Malformed("expected ',' or '}'")),
                }
            }
        }
        self.expect(b'}')?;
        Ok(OwnedTask {
            id,
            created_at_ms,
            updated_at_ms,
            priority,
            status,
            assignee_id,
            flags,
            title,
            body,
            tag,
        })
    }
}

fn unescape_json_string(raw: &[u8]) -> Result<String, CodecError> {
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0usize;
    while i < raw.len() {
        let b = raw[i];
        if b != b'\\' {
            out.push(b);
            i += 1;
            continue;
        }
        i += 1;
        let esc = *raw.get(i).ok_or(CodecError::Malformed("dangling escape"))?;
        match esc {
            b'"' => out.push(b'"'),
            b'\\' => out.push(b'\\'),
            b'/' => out.push(b'/'),
            b'n' => out.push(b'\n'),
            b'r' => out.push(b'\r'),
            b't' => out.push(b'\t'),
            b'u' => {
                let hex = raw
                    .get(i + 1..i + 5)
                    .ok_or(CodecError::Malformed("short \\u escape"))?;
                let hex_str = std::str::from_utf8(hex).map_err(|_| CodecError::InvalidUtf8)?;
                let code = u16::from_str_radix(hex_str, 16)
                    .map_err(|_| CodecError::Malformed("bad \\u hex"))?;
                let ch = char::from_u32(code as u32)
                    .ok_or(CodecError::Malformed("invalid unicode scalar"))?;
                let mut tmp = [0u8; 4];
                out.extend_from_slice(ch.encode_utf8(&mut tmp).as_bytes());
                i += 4;
            }
            _ => return Err(CodecError::Malformed("unknown json escape")),
        }
        i += 1;
    }
    String::from_utf8(out).map_err(|_| CodecError::InvalidUtf8)
}

/// Deserializes a JSON task array into owned `Vec<OwnedTask>`.
pub fn decode_tasks_json(bytes: &[u8]) -> Result<Vec<OwnedTask>, CodecError> {
    let mut p = JsonParser::new(bytes);
    p.expect(b'[')?;
    let mut out = Vec::with_capacity(bytes.len() / 320);
    p.skip_ws();
    if p.peek() != Some(b']') {
        loop {
            out.push(p.parse_task()?);
            p.skip_ws();
            match p.peek() {
                Some(b',') => p.pos += 1,
                Some(b']') => break,
                _ => return Err(CodecError::Malformed("expected ',' or ']'")),
            }
        }
    }
    p.expect(b']')?;
    Ok(out)
}

pub fn write_tasks_json(path: &Path, tasks: &[OwnedTask]) -> Result<usize, CodecError> {
    let bytes = encode_tasks_json(tasks);
    write_file_bytes(path, &bytes)?;
    Ok(bytes.len())
}

pub fn read_tasks_json(path: &Path) -> Result<Vec<OwnedTask>, CodecError> {
    let bytes = read_file_bytes(path)?;
    decode_tasks_json(&bytes)
}

// ============================================================================
// Baseline 1b: NVE_SNAP container + compact binary owned encode/decode
// ============================================================================

/// Encodes `tasks` into a compact binary stream wrapped in an `NVE_SNAP` container
/// (80-byte header + FNV-1a checksum).
pub fn encode_tasks_nve_snap(tasks: &[OwnedTask]) -> Vec<u8> {
    let mut state = Vec::with_capacity(tasks.len() * 240 + 8);
    state.extend_from_slice(&BIN_MAGIC);
    state.extend_from_slice(&(tasks.len() as u32).to_le_bytes());
    for t in tasks {
        state.extend_from_slice(&t.id.to_le_bytes());
        state.extend_from_slice(&t.created_at_ms.to_le_bytes());
        state.extend_from_slice(&t.updated_at_ms.to_le_bytes());
        state.extend_from_slice(&t.priority.to_le_bytes());
        state.extend_from_slice(&t.status.to_le_bytes());
        state.extend_from_slice(&t.assignee_id.to_le_bytes());
        state.extend_from_slice(&t.flags.to_le_bytes());
        write_len_prefixed_str(&mut state, &t.title);
        write_len_prefixed_str(&mut state, &t.body);
        write_len_prefixed_str(&mut state, &t.tag);
    }
    encode_snapshot(SNAP_SCHEMA_VERSION, 0, &[], &state, &[])
}

#[inline]
fn write_len_prefixed_str(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(&(s.len() as u32).to_le_bytes());
    out.extend_from_slice(s.as_bytes());
}

fn read_u32(bytes: &[u8], pos: &mut usize) -> Result<u32, CodecError> {
    let end = pos
        .checked_add(4)
        .ok_or(CodecError::Malformed("offset overflow"))?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(CodecError::Malformed("truncated u32"))?;
    *pos = end;
    Ok(u32::from_le_bytes([slice[0], slice[1], slice[2], slice[3]]))
}

fn read_u64(bytes: &[u8], pos: &mut usize) -> Result<u64, CodecError> {
    let end = pos
        .checked_add(8)
        .ok_or(CodecError::Malformed("offset overflow"))?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(CodecError::Malformed("truncated u64"))?;
    *pos = end;
    Ok(u64::from_le_bytes([
        slice[0], slice[1], slice[2], slice[3], slice[4], slice[5], slice[6], slice[7],
    ]))
}

fn read_len_prefixed_str(bytes: &[u8], pos: &mut usize) -> Result<String, CodecError> {
    let len = read_u32(bytes, pos)? as usize;
    let end = pos
        .checked_add(len)
        .ok_or(CodecError::Malformed("string len overflow"))?;
    let slice = bytes
        .get(*pos..end)
        .ok_or(CodecError::Malformed("truncated string"))?;
    *pos = end;
    let s = std::str::from_utf8(slice).map_err(|_| CodecError::InvalidUtf8)?;
    Ok(s.to_owned())
}

/// Decodes the state section of an `NVE_SNAP` file into owned `Vec<OwnedTask>`.
pub fn decode_tasks_nve_snap_state(state: &[u8]) -> Result<Vec<OwnedTask>, CodecError> {
    if state.len() < 8 || state[0..4] != BIN_MAGIC {
        return Err(CodecError::Malformed("invalid NVTK header"));
    }
    let mut pos = 4usize;
    let count = read_u32(state, &mut pos)? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let id = read_u64(state, &mut pos)?;
        let created_at_ms = read_u64(state, &mut pos)? as i64;
        let updated_at_ms = read_u64(state, &mut pos)? as i64;
        let priority = read_u32(state, &mut pos)?;
        let status = read_u32(state, &mut pos)?;
        let assignee_id = read_u32(state, &mut pos)?;
        let flags = read_u32(state, &mut pos)?;
        let title = read_len_prefixed_str(state, &mut pos)?;
        let body = read_len_prefixed_str(state, &mut pos)?;
        let tag = read_len_prefixed_str(state, &mut pos)?;
        out.push(OwnedTask {
            id,
            created_at_ms,
            updated_at_ms,
            priority,
            status,
            assignee_id,
            flags,
            title,
            body,
            tag,
        });
    }
    Ok(out)
}

pub fn write_tasks_nve_snap(path: &Path, tasks: &[OwnedTask]) -> Result<usize, CodecError> {
    let bytes = encode_tasks_nve_snap(tasks);
    write_file_bytes(path, &bytes)?;
    Ok(bytes.len())
}

pub fn read_tasks_nve_snap(path: &Path) -> Result<Vec<OwnedTask>, CodecError> {
    let snap = MmapSnapshot::open(path).map_err(|e| CodecError::Snapshot(e.to_string()))?;
    decode_tasks_nve_snap_state(snap.state_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::arena_mmap::{generate_workspace_tasks, TempFileGuard};

    #[test]
    fn json_round_trip_handles_escapes_and_bulk() {
        let mut tasks = generate_workspace_tasks(128);
        tasks[0].title = "Quote \"here\" \\ slash\nnewline\ttab".to_owned();
        tasks[0].created_at_ms = -42_000;
        let encoded = encode_tasks_json(&tasks);
        let decoded = decode_tasks_json(&encoded).expect("json decode");
        assert_eq!(decoded, tasks);

        let guard = TempFileGuard::new("json_rt", "json");
        write_tasks_json(guard.path(), &tasks).expect("write json");
        let from_disk = read_tasks_json(guard.path()).expect("read json");
        assert_eq!(from_disk, tasks);
    }

    #[test]
    fn nve_snap_round_trip_matches_owned_tasks() {
        let tasks = generate_workspace_tasks(256);
        let guard = TempFileGuard::new("snap_rt", "nvesnap");
        write_tasks_nve_snap(guard.path(), &tasks).expect("write nve_snap");
        let from_disk = read_tasks_nve_snap(guard.path()).expect("read nve_snap");
        assert_eq!(from_disk, tasks);
    }
}
