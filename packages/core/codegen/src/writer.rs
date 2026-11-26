//! Infallible `String`-backed emitter writer for `nve-codegen`.
//!
//! Writing to a `String` via [`std::fmt::Write`] is infallible (`Ok(())` always).
//! [`CodeWriter`] exposes an inherent `write_fmt` returning `()` so `write!` and
//! `writeln!` calls across emitters never need `.unwrap()` or `let _ = ...`.

use std::fmt;
use std::ops::Deref;

/// Infallible buffer for emitting generated source code.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct CodeWriter(String);

impl CodeWriter {
    /// Creates an empty buffer with at least `capacity` bytes preallocated.
    #[inline]
    pub(crate) fn with_capacity(capacity: usize) -> Self {
        Self(String::with_capacity(capacity))
    }

    /// Formats `args` into the underlying buffer. Called directly by `write!` / `writeln!`.
    #[inline]
    pub(crate) fn write_fmt(&mut self, args: fmt::Arguments<'_>) {
        // `std::fmt::Write` for `String` is infallible.
        let _ = fmt::Write::write_fmt(&mut self.0, args);
    }

    /// Appends a string slice.
    #[inline]
    pub(crate) fn push_str(&mut self, s: &str) {
        self.0.push_str(s);
    }

    /// Consumes the writer and returns the accumulated `String`.
    #[inline]
    pub(crate) fn into_string(self) -> String {
        self.0
    }
}

impl Deref for CodeWriter {
    type Target = str;

    #[inline]
    fn deref(&self) -> &str {
        &self.0
    }
}

impl From<String> for CodeWriter {
    #[inline]
    fn from(s: String) -> Self {
        Self(s)
    }
}

impl From<CodeWriter> for String {
    #[inline]
    fn from(writer: CodeWriter) -> Self {
        writer.0
    }
}
