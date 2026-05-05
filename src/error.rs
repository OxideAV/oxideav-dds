//! Crate-local error type used by `oxideav-dds`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`DdsError`] gains a
//! `From<DdsError> for oxideav_core::Error` impl (defined in
//! [`crate::registry`]) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-dds`. Standalone (no `oxideav-core`)
/// callers see this; framework callers convert via the gated
/// `From<DdsError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, DdsError>;

/// Error variants returned by `oxideav-dds`'s standalone API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdsError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// pixel-array runs past the end of the file, header `size` field
    /// disagrees with the spec, …).
    InvalidData(String),
    /// The byte stream uses a feature this crate does not implement
    /// yet (e.g. an uncompressed pixel format the round-1 reader
    /// can't lay out, or — on the encoder side — a `DdsPixelFormat`
    /// the encoder doesn't know how to serialise).
    Unsupported(String),
}

impl DdsError {
    /// Construct a [`DdsError::InvalidData`] from a stringy message.
    pub fn invalid(msg: impl Into<String>) -> Self {
        Self::InvalidData(msg.into())
    }

    /// Construct a [`DdsError::Unsupported`] from a stringy message.
    pub fn unsupported(msg: impl Into<String>) -> Self {
        Self::Unsupported(msg.into())
    }
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
        }
    }
}

impl std::error::Error for DdsError {}
