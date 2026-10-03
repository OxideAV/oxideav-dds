//! Crate-local error type used by `oxideav-dds`'s standalone (no
//! `oxideav-core`) public API.
//!
//! When the `registry` feature is enabled, [`DdsError`] gains a
//! `From<DdsError> for oxideav_core::Error` impl (defined in
//! the `registry` module) so the trait-side surface (`Decoder` /
//! `Encoder`) can keep returning `oxideav_core::Result<T>` while the
//! underlying decode/encode functions stay framework-free.

use core::fmt;

/// `Result` alias scoped to `oxideav-dds`. Standalone (no `oxideav-core`)
/// callers see this; framework callers convert via the gated
/// `From<DdsError> for oxideav_core::Error` impl.
pub type Result<T> = core::result::Result<T, DdsError>;

/// Contract alias: `oxideav_dds::Error` is [`DdsError`].
pub type Error = DdsError;

/// Error variants returned by `oxideav-dds`'s standalone API.
///
/// Carries a `std::io::Error` in [`DdsError::Io`], so the enum is
/// neither `Clone` nor `PartialEq`; tests match on variants or on
/// `Display`.
#[derive(Debug)]
#[non_exhaustive]
pub enum DdsError {
    /// The byte stream is malformed (bad magic, truncated header,
    /// pixel-array runs past the end of the file, header `size` field
    /// disagrees with the spec, …) or a caller-assembled image has an
    /// inconsistent geometry.
    InvalidData(String),
    /// The byte stream uses a feature this crate does not implement
    /// (a `DXGI_FORMAT` the parser cannot lay out, a stored layout the
    /// contract decode cannot expand, or — on the encoder side — a
    /// target layout the encoder cannot produce from the given input).
    Unsupported(String),
    /// A [`crate::DecodeOptions`] limit (`max_width`, `max_height`,
    /// `max_pixels`, `max_bytes`) would be exceeded; reported before any
    /// pixel allocation.
    LimitExceeded(String),
    /// An I/O error from [`crate::decode_from`] / [`crate::encode_to`].
    Io(std::io::Error),
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

    /// Construct a [`DdsError::LimitExceeded`] from a stringy message.
    pub fn limit(msg: impl Into<String>) -> Self {
        Self::LimitExceeded(msg.into())
    }
}

impl fmt::Display for DdsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidData(s) => write!(f, "invalid data: {s}"),
            Self::Unsupported(s) => write!(f, "unsupported: {s}"),
            Self::LimitExceeded(s) => write!(f, "limit exceeded: {s}"),
            Self::Io(e) => write!(f, "I/O error: {e}"),
        }
    }
}

impl std::error::Error for DdsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for DdsError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}
