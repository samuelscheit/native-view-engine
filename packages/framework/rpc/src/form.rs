//! Composable `FormDecode` seam, typed `FieldErrors`, and bounded zero-dependency
//! `application/x-www-form-urlencoded` parsing shared between 0-JS HTML forms and RPC actions.

use core::fmt;

/// Standard HTML form media type accepted by progressive `<ActionForm>` submissions.
pub const CONTENT_TYPE_FORM_URLENCODED: &str = "application/x-www-form-urlencoded";

/// Default maximum field pairs accepted by bounded form parsing (`1024`), preventing pair-flood
/// allocation bombs (`"a&a&a&..."`).
pub const DEFAULT_MAX_FORM_FIELDS: usize = 1024;

/// Single field validation or decoding error.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct FieldError {
    #[n(0)]
    pub field: String,
    #[n(1)]
    pub message: String,
}

/// Collection of field-level decoding or validation errors (`422 Unprocessable Entity`).
#[derive(Debug, Clone, Default, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(map)]
pub struct FieldErrors {
    #[n(0)]
    pub errors: Vec<FieldError>,
}

impl FieldErrors {
    #[inline]
    pub const fn new() -> Self {
        Self { errors: Vec::new() }
    }

    #[inline]
    pub fn push(&mut self, field: impl Into<String>, message: impl Into<String>) {
        self.errors.push(FieldError {
            field: field.into(),
            message: message.into(),
        });
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.errors.is_empty()
    }

    #[inline]
    pub fn len(&self) -> usize {
        self.errors.len()
    }

    pub fn into_result<T>(self, value: T) -> Result<T, Self> {
        if self.errors.is_empty() {
            Ok(value)
        } else {
            Err(self)
        }
    }
}

impl fmt::Display for FieldErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (idx, err) in self.errors.iter().enumerate() {
            if idx > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{}: {}", err.field, err.message)?;
        }
        Ok(())
    }
}

impl std::error::Error for FieldErrors {}

/// Parsed form submission pairs preserving key order and repeated fields (e.g. multi-selects
/// and checkboxes).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FormPairs {
    pairs: Vec<(String, String)>,
}

impl FormPairs {
    /// Decodes an `application/x-www-form-urlencoded` body while enforcing `max_fields`.
    pub fn from_urlencoded_limited(body: &str, max_fields: usize) -> Result<Self, &'static str> {
        decode_form_urlencoded_limited(body, max_fields).map(|pairs| Self { pairs })
    }

    pub fn from_pairs(pairs: Vec<(String, String)>) -> Self {
        Self { pairs }
    }

    #[inline]
    pub fn as_slice(&self) -> &[(String, String)] {
        &self.pairs
    }

    /// Returns the last value submitted for `name`, matching HTML form override semantics.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .rev()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Returns all values submitted for `name` in submission order.
    pub fn get_all<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.pairs
            .iter()
            .filter(move |(k, _)| k == name)
            .map(|(_, v)| v.as_str())
    }

    /// Returns `true` for checked HTML checkboxes (`"on"`, `"true"`, `"1"`), `false` when absent.
    pub fn get_bool(&self, name: &str) -> bool {
        self.get(name).is_some_and(|raw| {
            raw.eq_ignore_ascii_case("on")
                || raw.eq_ignore_ascii_case("true")
                || raw == "1"
        })
    }

    /// Returns `None` when the field is absent or blank (HTML empty input semantics).
    pub fn get_optional_non_empty(&self, name: &str) -> Option<&str> {
        self.get(name).map(str::trim).filter(|s| !s.is_empty())
    }
}

/// Composable seam for decoding HTML `<form>` submissions into strongly typed action inputs.
///
/// Kept deliberately outside `minicbor` / payload codecs so HTML coercions (absent checkboxes,
/// empty `Option<T>`, repeated keys, per-field `422` errors) stay explicit and zero-cost on
/// non-form procedures.
pub trait FormDecode: Sized {
    fn from_form(form: &FormPairs) -> Result<Self, FieldErrors>;
}

/// Bounded `application/x-www-form-urlencoded` decoder rejecting submissions with more than
/// `max_fields` non-empty field pairs.
pub fn decode_form_urlencoded_limited(
    body: &str,
    max_fields: usize,
) -> Result<Vec<(String, String)>, &'static str> {
    let mut pairs = Vec::new();
    for segment in body.split('&') {
        if segment.is_empty() {
            continue;
        }
        if pairs.len() >= max_fields {
            return Err("form submission exceeded max_form_fields limit");
        }
        let (raw_k, raw_v) = segment.split_once('=').unwrap_or((segment, ""));
        pairs.push((percent_decode_form(raw_k), percent_decode_form(raw_v)));
    }
    Ok(pairs)
}

fn percent_decode_form(raw: &str) -> String {
    let bytes = raw.as_bytes();
    if !bytes.iter().any(|&b| b == b'+' || b == b'%') {
        return raw.to_owned();
    }
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            b'%' if i + 2 < bytes.len() => {
                if let (Some(hi), Some(lo)) = (hex_val(bytes[i + 1]), hex_val(bytes[i + 2])) {
                    out.push((hi << 4) | lo);
                    i += 3;
                } else {
                    out.push(b'%');
                    i += 1;
                }
            }
            b => {
                out.push(b);
                i += 1;
            }
        }
    }
    match String::from_utf8(out) {
        Ok(valid) => valid,
        Err(err) => String::from_utf8_lossy(err.as_bytes()).into_owned(),
    }
}

#[inline]
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
}
