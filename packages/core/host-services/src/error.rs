//! Typed error model mapping `NveHostStatus` and binary codec failures without JSON.

use std::fmt;

use nve_abi::NveHostStatus;

/// Error returned by host service calls and binary payload codecs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ServiceError {
    /// Operation is pending async completion under `request_id`.
    Pending { request_id: u64 },
    /// Host rejected output buffer capacity; caller needed `required` bytes.
    BufferTooSmall { required: usize },
    /// Capability is not supported on this platform/environment (e.g. headless or unbundled CLI).
    Unsupported,
    /// Host denied permission for the requested capability.
    PermissionDenied,
    /// Caller supplied invalid arguments.
    InvalidArg(String),
    /// Requested resource/key was not found.
    NotFound,
    /// User cancelled an interactive prompt (file picker, auth, etc.).
    Cancelled,
    /// Underlying OS I/O error.
    IoError(String),
    /// Internal host or bridge failure.
    InternalError(String),
    /// Binary wire payload was corrupt, truncated, or invalid UTF-8.
    Codec(String),
}

/// Single source of truth for bidirectional mapping between C-ABI [`NveHostStatus`] codes and
/// [`ServiceError`].
macro_rules! impl_host_status_mapping {
    (
        unit: { $($unit_status:ident <=> $unit_err:ident),* $(,)? }
        payload: {
            $($p_status:ident => $p_make:expr ; $p_pat:pat),* $(,)?
        }
        extra_to_status: {
            $($extra_pat:pat => $extra_status:ident),* $(,)?
        }
    ) => {
        impl ServiceError {
            /// Converts a non-`Ok` [`NveHostStatus`] code into the corresponding [`ServiceError`].
            pub fn from_status(status: NveHostStatus) -> Self {
                match status {
                    NveHostStatus::Ok => {
                        Self::InternalError("unexpected Ok status in error path".into())
                    }
                    $(NveHostStatus::$unit_status => Self::$unit_err,)*
                    $(NveHostStatus::$p_status => $p_make,)*
                }
            }

            /// Maps this error back to a signed C-ABI [`NveHostStatus`] code.
            pub const fn to_status(&self) -> NveHostStatus {
                match self {
                    $(Self::$unit_err => NveHostStatus::$unit_status,)*
                    $($p_pat => NveHostStatus::$p_status,)*
                    $($extra_pat => NveHostStatus::$extra_status,)*
                }
            }
        }
    };
}

impl_host_status_mapping! {
    unit: {
        Unsupported <=> Unsupported,
        PermissionDenied <=> PermissionDenied,
        NotFound <=> NotFound,
        Cancelled <=> Cancelled,
    }
    payload: {
        Pending => Self::Pending { request_id: 0 }; Self::Pending { .. },
        BufferTooSmall => Self::BufferTooSmall { required: 0 }; Self::BufferTooSmall { .. },
        InvalidArg => Self::InvalidArg("invalid argument".into()); Self::InvalidArg(_),
        IoError => Self::IoError("host I/O error".into()); Self::IoError(_),
        InternalError => Self::InternalError("host internal error".into()); Self::InternalError(_),
    }
    extra_to_status: {
        Self::Codec(_) => InvalidArg,
    }
}

impl From<std::io::Error> for ServiceError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
    }
}

impl From<nve_codec::CodecError> for ServiceError {
    fn from(err: nve_codec::CodecError) -> Self {
        match err {
            nve_codec::CodecError::BadEnum { .. } => Self::InvalidArg(err.to_string()),
            other => Self::Codec(other.to_string()),
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending { request_id } => {
                write!(f, "request {request_id} is pending async completion")
            }
            Self::BufferTooSmall { required } => {
                write!(f, "host output buffer too small (required {required} bytes)")
            }
            Self::Unsupported => f.write_str("host capability unsupported"),
            Self::PermissionDenied => f.write_str("host capability permission denied"),
            Self::InvalidArg(msg) => write!(f, "invalid argument: {msg}"),
            Self::NotFound => f.write_str("resource not found"),
            Self::Cancelled => f.write_str("operation cancelled"),
            Self::IoError(msg) => write!(f, "I/O error: {msg}"),
            Self::InternalError(msg) => write!(f, "internal host error: {msg}"),
            Self::Codec(msg) => write!(f, "binary codec error: {msg}"),
        }
    }
}

impl std::error::Error for ServiceError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping_roundtrips_bidirectionally() {
        for status in [
            NveHostStatus::Pending,
            NveHostStatus::BufferTooSmall,
            NveHostStatus::Unsupported,
            NveHostStatus::PermissionDenied,
            NveHostStatus::InvalidArg,
            NveHostStatus::NotFound,
            NveHostStatus::Cancelled,
            NveHostStatus::IoError,
            NveHostStatus::InternalError,
        ] {
            assert_eq!(ServiceError::from_status(status).to_status(), status);
        }
        assert_eq!(
            ServiceError::from_status(NveHostStatus::Ok).to_status(),
            NveHostStatus::InternalError
        );
        assert_eq!(
            ServiceError::Codec("bad payload".into()).to_status(),
            NveHostStatus::InvalidArg
        );
    }
}
