// Sticky-error writer for the compact binary `NveBinValue` wire format (layout in `bin_codec.rs`).

use crate::DecodeError;

use super::NVE_BIN_VALUE_MAX_DEPTH;
use super::bin_view::BinValueView;
use super::ffi_types::NveValueTag;

/// Byte length of a container header after its tag: `payload_byte_len: u32` + `count: u32`.
const CONTAINER_HEADER_LEN: usize = 8;

/// Appends `NveBinValue` values to an owned buffer.
///
/// Errors are sticky: the first one (a run or container above `u32::MAX`, nesting deeper than
/// `NVE_BIN_VALUE_MAX_DEPTH`) is recorded, the offending value is skipped, and [`finish`] returns
/// it, so a partial encoding never escapes. Containers patch their element count and payload
/// length when their closure returns, so a count can never disagree with the elements written.
///
/// [`finish`]: BinWriter::finish
#[derive(Debug, Default)]
pub struct BinWriter {
    buf: Vec<u8>,
    /// Open containers, i.e. the nesting depth of the next value.
    depth: usize,
    error: Option<DecodeError>,
}

impl BinWriter {
    #[inline]
    pub fn new() -> Self {
        Self::default()
    }

    #[inline]
    pub fn with_capacity(capacity: usize) -> Self {
        Self::from_vec(Vec::with_capacity(capacity))
    }

    /// Appends after the existing contents of `buf`, reusing its allocation.
    #[inline]
    pub fn from_vec(buf: Vec<u8>) -> Self {
        Self {
            buf,
            depth: 0,
            error: None,
        }
    }

    #[inline]
    pub fn is_ok(&self) -> bool {
        self.error.is_none()
    }

    /// Records `error` unless an earlier one is already recorded.
    #[inline]
    pub fn fail(&mut self, error: DecodeError) {
        if self.error.is_none() {
            self.error = Some(error);
        }
    }

    /// The encoded bytes, or the first recorded error.
    #[inline]
    pub fn finish(self) -> Result<Vec<u8>, DecodeError> {
        match self.error {
            None => Ok(self.buf),
            Some(error) => Err(error),
        }
    }

    #[inline]
    pub fn null(&mut self) {
        self.scalar(&[NveValueTag::Null as u8]);
    }

    #[inline]
    pub fn bool(&mut self, v: bool) {
        self.scalar(&[NveValueTag::Bool as u8, u8::from(v)]);
    }

    #[inline]
    pub fn i64(&mut self, v: i64) {
        self.tagged_word(NveValueTag::I64, v.to_le_bytes());
    }

    #[inline]
    pub fn f64(&mut self, v: f64) {
        self.tagged_word(NveValueTag::F64, v.to_le_bytes());
    }

    #[inline]
    pub fn str(&mut self, s: &str) {
        self.tagged_run(Some(NveValueTag::Str), s.as_bytes());
    }

    #[inline]
    pub fn bytes(&mut self, b: &[u8]) {
        self.tagged_run(Some(NveValueTag::Bytes), b);
    }

    /// Copies an already validated value. Records `MaxDepthExceeded` (writing nothing) when its
    /// containers would nest deeper than `NVE_BIN_VALUE_MAX_DEPTH` at the current depth.
    pub fn value(&mut self, view: BinValueView<'_>) {
        // A validated view nests at most `NVE_BIN_VALUE_MAX_DEPTH` deep, so only a nested copy is walked.
        if self.depth > 0 && self.depth + container_depth(view) > NVE_BIN_VALUE_MAX_DEPTH {
            return self.fail(DecodeError::MaxDepthExceeded);
        }
        self.scalar(view.as_raw_bytes());
    }

    /// Writes an `Array` whose elements `fill` writes, one per [`BinArrayWriter::item`].
    pub fn array(&mut self, fill: impl FnOnce(&mut BinArrayWriter<'_>)) {
        let Some(header) = self.open(NveValueTag::Array) else {
            return;
        };
        let mut items = BinArrayWriter { w: self, count: 0 };
        fill(&mut items);
        let count = items.count;
        self.close(header, count);
    }

    /// Writes a `Map` whose entries `fill` writes, one per [`BinMapWriter::key`].
    pub fn map(&mut self, fill: impl FnOnce(&mut BinMapWriter<'_>)) {
        let Some(header) = self.open(NveValueTag::Map) else {
            return;
        };
        let mut entries = BinMapWriter { w: self, count: 0 };
        fill(&mut entries);
        let count = entries.count;
        self.close(header, count);
    }

    /// Appends `bytes` unless an error is recorded: a failed encode stops growing the buffer.
    #[inline]
    fn scalar(&mut self, bytes: &[u8]) {
        if self.error.is_none() {
            self.buf.extend_from_slice(bytes);
        }
    }

    #[inline]
    fn tagged_word(&mut self, tag: NveValueTag, word: [u8; 8]) {
        if self.error.is_none() {
            self.buf.push(tag as u8);
            self.buf.extend_from_slice(&word);
        }
    }

    /// Appends `len: u32` + `run`; an oversized run records `TooLarge` and writes nothing.
    fn run(&mut self, run: &[u8]) {
        self.tagged_run(None, run);
    }

    fn tagged_run(&mut self, tag: Option<NveValueTag>, run: &[u8]) {
        let Ok(len) = u32::try_from(run.len()) else {
            return self.fail(DecodeError::TooLarge);
        };
        if self.error.is_some() {
            return;
        }
        if let Some(tag) = tag {
            self.buf.push(tag as u8);
        }
        self.buf.extend_from_slice(&len.to_le_bytes());
        self.buf.extend_from_slice(run);
    }

    /// Writes the container tag and a zeroed header; returns the header offset.
    /// Returns `None` (and skips `fill`) once an error is recorded or past the depth limit.
    fn open(&mut self, tag: NveValueTag) -> Option<usize> {
        if self.depth >= NVE_BIN_VALUE_MAX_DEPTH {
            self.fail(DecodeError::MaxDepthExceeded);
            return None;
        }
        if self.error.is_some() {
            return None;
        }
        self.buf.push(tag as u8);
        let header = self.buf.len();
        self.buf.extend_from_slice(&[0; CONTAINER_HEADER_LEN]);
        self.depth += 1;
        Some(header)
    }

    /// Patches `payload_byte_len` (everything after that field) and `count` at `header`.
    fn close(&mut self, header: usize, count: usize) {
        self.depth -= 1;
        let payload = self.buf.len() - header - 4;
        let (Ok(payload), Ok(count)) = (u32::try_from(payload), u32::try_from(count)) else {
            return self.fail(DecodeError::TooLarge);
        };
        self.buf[header..header + 4].copy_from_slice(&payload.to_le_bytes());
        self.buf[header + 4..header + CONTAINER_HEADER_LEN].copy_from_slice(&count.to_le_bytes());
    }
}

/// Element sink of [`BinWriter::array`].
pub struct BinArrayWriter<'w> {
    w: &'w mut BinWriter,
    count: usize,
}

impl BinArrayWriter<'_> {
    /// Starts the next element: write exactly one value into the returned writer.
    #[inline]
    pub fn item(&mut self) -> &mut BinWriter {
        self.count += 1;
        self.w
    }
}

/// Entry sink of [`BinWriter::map`].
pub struct BinMapWriter<'w> {
    w: &'w mut BinWriter,
    count: usize,
}

impl BinMapWriter<'_> {
    /// Writes the next entry's key: write exactly one value into the returned writer.
    #[inline]
    pub fn key(&mut self, key: &str) -> &mut BinWriter {
        self.count += 1;
        self.w.run(key.as_bytes());
        self.w
    }
}

/// Container nesting of `view`: 0 for a scalar, 1 for a container of scalars, and so on.
fn container_depth(view: BinValueView<'_>) -> usize {
    if let Some(items) = view.as_array() {
        1 + items.map(container_depth).max().unwrap_or(0)
    } else if let Some(entries) = view.as_map() {
        1 + entries.map(|(_, v)| container_depth(v)).max().unwrap_or(0)
    } else {
        0
    }
}
