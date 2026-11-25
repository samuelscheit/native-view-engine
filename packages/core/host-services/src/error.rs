//! Typed error model mapping `NveHostStatus` and binary codec failures without JSON.

use std::fmt;

use nve_abi::{HostBatchError, NveHostStatus};

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

impl ServiceError {
    /// Converts a non-`Ok` [`NveHostStatus`] code into the corresponding [`ServiceError`].
    pub fn from_status(status: NveHostStatus) -> Self {
        match status {
            NveHostStatus::Ok => Self::InternalError("unexpected Ok status in error path".into()),
            NveHostStatus::Pending => Self::Pending { request_id: 0 },
            NveHostStatus::BufferTooSmall => Self::BufferTooSmall { required: 0 },
            NveHostStatus::Unsupported => Self::Unsupported,
            NveHostStatus::PermissionDenied => Self::PermissionDenied,
            NveHostStatus::InvalidArg => Self::InvalidArg("invalid argument".into()),
            NveHostStatus::NotFound => Self::NotFound,
            NveHostStatus::Cancelled => Self::Cancelled,
            NveHostStatus::IoError => Self::IoError("host I/O error".into()),
            NveHostStatus::InternalError => Self::InternalError("host internal error".into()),
        }
    }

    /// Maps this error back to a signed C-ABI [`NveHostStatus`] code.
    pub const fn to_status(&self) -> NveHostStatus {
        match self {
            Self::Pending { .. } => NveHostStatus::Pending,
            Self::BufferTooSmall { .. } => NveHostStatus::BufferTooSmall,
            Self::Unsupported => NveHostStatus::Unsupported,
            Self::PermissionDenied => NveHostStatus::PermissionDenied,
            Self::InvalidArg(_) | Self::Codec(_) => NveHostStatus::InvalidArg,
            Self::NotFound => NveHostStatus::NotFound,
            Self::Cancelled => NveHostStatus::Cancelled,
            Self::IoError(_) => NveHostStatus::IoError,
            Self::InternalError(_) => NveHostStatus::InternalError,
        }
    }
}

impl From<HostBatchError> for ServiceError {
    fn from(err: HostBatchError) -> Self {
        Self::Codec(err.to_string())
    }
}

impl From<std::io::Error> for ServiceError {
    fn from(err: std::io::Error) -> Self {
        Self::IoError(err.to_string())
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
