//! Decode-side limits / strictness ([`DecodeOptions`]) and the encoder
//! knobs ([`EncodeOptions`]) of the image-crate contract.

use crate::error::{DdsError, Result};
use crate::surface::SurfaceFormat;

/// Limits and strictness for [`crate::decode_with`] /
/// [`crate::decode_all_with`] / [`crate::parse_dds_with`].
///
/// Every limit is checked against the header **before** any surface
/// buffer is allocated, so a hostile header fails with
/// [`DdsError::LimitExceeded`] instead of committing memory. The
/// defaults are: no dimension / pixel-count limit, decoded bytes capped
/// at [`DecodeOptions::DEFAULT_MAX_BYTES`] (1 GiB), `strict = false`.
///
/// `max_bytes` counts what a decode allocates: the stored bytes of
/// every surface the file carries (the parser copies the whole tree)
/// plus the expanded contract plane of the largest surface.
///
/// `strict` turns two tolerances off:
///
/// * trailing bytes after the last surface are normally ignored (some
///   writers pad the file); strict mode rejects them as `InvalidData`;
/// * `dwPitchOrLinearSize` is normally ignored (the surface size is
///   always computed from the layout and the dimensions — many writers
///   leave the field zero or wrong); strict mode requires a non-zero
///   value to agree with the computed row pitch (`DDSD_PITCH`) or
///   top-level surface size (`DDSD_LINEARSIZE`).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct DecodeOptions {
    /// Reject files whose top-level width exceeds this (pixels).
    pub max_width: Option<u32>,
    /// Reject files whose top-level height exceeds this (pixels).
    pub max_height: Option<u32>,
    /// Reject files whose top-level `width × height` exceeds this.
    pub max_pixels: Option<u64>,
    /// Reject files whose decode would allocate more than this many
    /// bytes (see the type docs).
    pub max_bytes: Option<u64>,
    /// Enforce the header consistency rules (see the type docs).
    pub strict: bool,
}

impl DecodeOptions {
    /// Default [`Self::max_bytes`]: 1 GiB.
    pub const DEFAULT_MAX_BYTES: u64 = 1 << 30;

    /// The defaults (see the type docs).
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or lift with `None`) the width limit.
    pub fn with_max_width(mut self, max_width: impl Into<Option<u32>>) -> Self {
        self.max_width = max_width.into();
        self
    }

    /// Set (or lift with `None`) the height limit.
    pub fn with_max_height(mut self, max_height: impl Into<Option<u32>>) -> Self {
        self.max_height = max_height.into();
        self
    }

    /// Set (or lift with `None`) the pixel-count limit.
    pub fn with_max_pixels(mut self, max_pixels: impl Into<Option<u64>>) -> Self {
        self.max_pixels = max_pixels.into();
        self
    }

    /// Set (or lift with `None`) the decoded-bytes limit.
    pub fn with_max_bytes(mut self, max_bytes: impl Into<Option<u64>>) -> Self {
        self.max_bytes = max_bytes.into();
        self
    }

    /// Set strict mode.
    pub fn with_strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Lift every limit (`None` everywhere); strictness unchanged.
    pub fn unlimited(mut self) -> Self {
        self.max_width = None;
        self.max_height = None;
        self.max_pixels = None;
        self.max_bytes = None;
        self
    }

    /// Check the top-level geometry against the dimension / pixel limits.
    pub(crate) fn check_dimensions(&self, width: u32, height: u32) -> Result<()> {
        if let Some(m) = self.max_width {
            if width > m {
                return Err(DdsError::limit(format!(
                    "DDS: width {width} exceeds max_width {m}"
                )));
            }
        }
        if let Some(m) = self.max_height {
            if height > m {
                return Err(DdsError::limit(format!(
                    "DDS: height {height} exceeds max_height {m}"
                )));
            }
        }
        if let Some(m) = self.max_pixels {
            let px = width as u64 * height as u64;
            if px > m {
                return Err(DdsError::limit(format!(
                    "DDS: {width}x{height} = {px} pixels exceeds max_pixels {m}"
                )));
            }
        }
        Ok(())
    }

    /// Check a byte budget against `max_bytes`.
    pub(crate) fn check_bytes(&self, bytes: u64) -> Result<()> {
        if let Some(m) = self.max_bytes {
            if bytes > m {
                return Err(DdsError::limit(format!(
                    "DDS: decode would allocate {bytes} bytes, exceeding max_bytes {m}"
                )));
            }
        }
        Ok(())
    }
}

impl Default for DecodeOptions {
    fn default() -> Self {
        Self {
            max_width: None,
            max_height: None,
            max_pixels: None,
            max_bytes: Some(Self::DEFAULT_MAX_BYTES),
            strict: false,
        }
    }
}

/// Encoder knobs for [`crate::encode`] / [`crate::encode_rgb8`] /
/// [`crate::encode_rgba8`] / [`crate::encode_all`].
///
/// The three choices DDS offers are fields:
///
/// * [`surface_format`](Self::surface_format) — the stored layout. `None`
///   (the default) picks the natural layout of the image's
///   [`crate::PixelFormat`] (see [`crate::encode`]); `Some(..)` converts
///   the image into that layout, including the BC1 / BC2 / BC3 / BC4 /
///   BC5 / BC6H / BC7 and ASTC LDR block encoders, or fails with
///   [`DdsError::Unsupported`] when the conversion does not exist.
/// * [`mip_levels`](Self::mip_levels) — `1` (default) writes the surface
///   alone, `0` writes the full chain down to 1×1, `n` writes `n` levels;
///   missing levels are generated by a 2×2 box filter in the contract
///   layout before conversion to the stored layout.
/// * [`dx10_header`](Self::dx10_header) — force the `DDS_HEADER_DXT10`
///   extension (it is written automatically whenever the layout or the
///   texture shape has no legacy encoding).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct EncodeOptions {
    /// Stored layout; `None` = the natural layout of the image format.
    pub surface_format: Option<SurfaceFormat>,
    /// Mip levels to write: `1` none, `0` full chain, `n` levels.
    pub mip_levels: u32,
    /// Force the `DX10` extension header.
    pub dx10_header: bool,
}

impl EncodeOptions {
    /// The defaults: natural layout, no mip chain, legacy header when
    /// possible.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or clear) the stored layout.
    pub fn with_surface_format(mut self, surface_format: impl Into<Option<SurfaceFormat>>) -> Self {
        self.surface_format = surface_format.into();
        self
    }

    /// Set the mip-level count (`0` = full chain, `1` = none).
    pub fn with_mip_levels(mut self, mip_levels: u32) -> Self {
        self.mip_levels = mip_levels;
        self
    }

    /// Force (or stop forcing) the `DX10` extension header.
    pub fn with_dx10_header(mut self, dx10_header: bool) -> Self {
        self.dx10_header = dx10_header;
        self
    }
}

impl Default for EncodeOptions {
    fn default() -> Self {
        Self {
            surface_format: None,
            mip_levels: 1,
            dx10_header: false,
        }
    }
}
