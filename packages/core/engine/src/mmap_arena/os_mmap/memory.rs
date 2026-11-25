//! In-memory fallback backing for `wasm32` and non-Unix/non-Windows targets.

use std::io;
use std::path::Path;

pub fn page_size() -> usize {
    4096
}

pub struct ReadOnlyMmap {
    buf: Vec<u8>,
}

impl ReadOnlyMmap {
    pub fn open(path: &Path) -> io::Result<Self> {
        let buf = std::fs::read(path)?;
        Ok(Self { buf })
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn base_addr(&self) -> usize {
        self.buf.as_ptr() as usize
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }
}

pub struct ReadWriteMmap {
    buf: Vec<u8>,
}

impl ReadWriteMmap {
    pub fn in_memory(len: usize) -> io::Result<Self> {
        if len == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "cannot create zero-length writable mmap",
            ));
        }
        Ok(Self { buf: vec![0u8; len] })
    }

    pub fn create(_path: &Path, len: usize) -> io::Result<Self> {
        Self::in_memory(len)
    }

    pub fn open(path: &Path) -> io::Result<Self> {
        let buf = std::fs::read(path)?;
        if buf.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "cannot open zero-length writable mmap",
            ));
        }
        Ok(Self { buf })
    }

    #[inline]
    pub fn as_slice(&self) -> &[u8] {
        &self.buf
    }

    #[inline]
    pub fn as_mut_slice(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    #[inline]
    pub fn base_addr(&self) -> usize {
        self.buf.as_ptr() as usize
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    pub fn sync_all(&self) -> io::Result<()> {
        Ok(())
    }

    pub fn sync_range(&self, offset: usize, len: usize) -> io::Result<()> {
        let end = offset.checked_add(len).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "sync range overflow")
        })?;
        if end > self.buf.len() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "sync range exceeds mapping length",
            ));
        }
        Ok(())
    }
}

pub struct AnonReservation {
    buf: Vec<u8>,
}

impl AnonReservation {
    pub fn new(len: usize) -> io::Result<Self> {
        Ok(Self {
            buf: vec![0u8; len.max(4096)],
        })
    }

    pub fn base_addr(&self) -> usize {
        self.buf.as_ptr() as usize
    }
}
