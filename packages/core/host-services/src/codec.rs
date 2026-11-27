//! URL validation and shared host-service binary framing limits (backed by `nve-codec`).

use crate::error::ServiceError;

/// Upper bound on element count in a single decoded sequence to prevent OOM DoS on corrupt buffers.
pub const MAX_SEQ_ITEMS: usize = nve_codec::DEFAULT_MAX_ITEMS;

/// Upper bound on a single length-prefixed byte slice or UTF-8 string (`16 MiB`).
pub const MAX_PAYLOAD_BYTES: usize = nve_codec::DEFAULT_MAX_BYTES;

/// Validates a non-empty URL containing a scheme (`scheme:...`) without control characters.
pub fn validate_url(url: &str) -> Result<(), ServiceError> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return Err(ServiceError::InvalidArg("URL must not be empty".into()));
    }
    if trimmed.bytes().any(|b| b < 0x20 || b == 0x7F) {
        return Err(ServiceError::InvalidArg(
            "URL must not contain ASCII control characters".into(),
        ));
    }
    let Some((scheme, rest)) = trimmed.split_once(':') else {
        return Err(ServiceError::InvalidArg("URL must include a scheme".into()));
    };
    if scheme.is_empty()
        || rest.is_empty()
        || !scheme.as_bytes()[0].is_ascii_alphabetic()
        || !scheme
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'-' | b'.'))
    {
        return Err(ServiceError::InvalidArg(format!(
            "invalid URL scheme '{scheme}'"
        )));
    }
    Ok(())
}
