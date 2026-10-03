//! The image-crate contract records (`IMAGE_CRATE_API.md`): the
//! native-layout [`DdsImage`] (one surface — the top-level mip of the
//! first face / slice, or any single surface from [`crate::decode_all`]),
//! its [`DdsPixelFormat`] tag, the shared [`Plane`] / [`ColorInfo`] /
//! [`Metadata`] records, the raw [`RgbImage`] / [`RgbaImage`] results,
//! the header-only [`ImageInfo`] and the multi-surface [`Frame`].
//!
//! Everything here builds without `oxideav-core`; the `registry`
//! feature adds the `VideoFrame` bridge in the `registry` module. The
//! on-disk surface vocabulary (every `D3DFMT` / `DXGI_FORMAT` layout,
//! the mip / face / slice tree) lives in [`crate::surface`].

use std::time::Duration;

use crate::error::{DdsError, Result};
use crate::surface::{CubemapFace, SurfaceFormat};
use crate::types::DxgiFormat;

/// Pixel layout of a [`DdsImage`] — the small set of layouts
/// `oxideav_core::PixelFormat` names, which is what [`crate::decode`]
/// expands every stored DDS surface into.
///
/// Variant names mirror `oxideav_core::PixelFormat` exactly. The
/// stored [`SurfaceFormat`] (one of ~70 `D3DFMT` / `DXGI_FORMAT`
/// layouts) maps onto these as documented on
/// [`SurfaceFormat::contract_format`]; the mapping is exact (byte copy
/// or lossless bit-replication widening) for every integer layout and
/// a `half`/packed-float → `f32` widening for the floating-point ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DdsPixelFormat {
    /// 8-bit grey, 1 byte per pixel (`L8`, `R8_UNORM`, decoded BC4).
    Gray8,
    /// 8-bit grey + alpha, 2 bytes per pixel `[Y, A]` (`A8L8`; `A4L4`
    /// widened by nibble replication).
    Ya8,
    /// 16-bit grey, little-endian `u16` per pixel (`L16`, `R16_UNORM`).
    Gray16Le,
    /// Packed 8-bit RGB, 3 bytes per pixel. DDS has no RGB-ordered
    /// 24-bit layout; this variant is the [`DdsImage::from_rgb8`] input
    /// layout and is written as `R8G8B8` (BGR on disk) by
    /// [`crate::encode`].
    Rgb24,
    /// Packed 8-bit BGR, 3 bytes per pixel — the on-disk order of
    /// `D3DFMT_R8G8B8`.
    Bgr24,
    /// Packed 8-bit RGBA, 4 bytes per pixel (`R8G8B8A8_UNORM` /
    /// `A8B8G8R8`, every decoded BC1 / BC2 / BC3 / BC7 / ASTC surface,
    /// and every 16-bit packed-colour layout widened by bit
    /// replication).
    Rgba,
    /// Packed 8-bit BGRA, 4 bytes per pixel — the on-disk order of
    /// `D3DFMT_A8R8G8B8` / `B8G8R8A8_UNORM`.
    Bgra,
    /// 16-bit RGBA, four little-endian `u16` per pixel
    /// (`R16G16B16A16_UNORM`; `R10G10B10A2_UNORM` widened).
    Rgba64Le,
    /// Single-channel `f32`, little-endian (`R32_FLOAT`, `R16_FLOAT`
    /// widened, the SNORM single-channel layouts normalised).
    GrayF32Le,
    /// Three-channel `f32` RGB, little-endian (`R11G11B10_FLOAT`,
    /// `R9G9B9E5_SHAREDEXP` widened).
    RgbF32Le,
    /// Four-channel `f32` RGBA, little-endian (`R32G32B32A32_FLOAT`,
    /// `R16G16B16A16_FLOAT`, decoded BC6H, the SNORM multi-channel
    /// layouts normalised).
    RgbaF32Le,
}

/// Contract alias: `oxideav_dds::PixelFormat` is [`DdsPixelFormat`].
pub type PixelFormat = DdsPixelFormat;

impl DdsPixelFormat {
    /// Bytes per pixel of the packed plane.
    pub const fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Gray8 => 1,
            Self::Ya8 | Self::Gray16Le => 2,
            Self::Rgb24 | Self::Bgr24 => 3,
            Self::Rgba | Self::Bgra | Self::GrayF32Le => 4,
            Self::Rgba64Le => 8,
            Self::RgbF32Le => 12,
            Self::RgbaF32Le => 16,
        }
    }

    /// `true` for the layouts that carry an alpha channel.
    pub const fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::Ya8 | Self::Rgba | Self::Bgra | Self::Rgba64Le | Self::RgbaF32Le
        )
    }

    /// `true` for the `f32` layouts.
    pub const fn is_float(self) -> bool {
        matches!(self, Self::GrayF32Le | Self::RgbF32Le | Self::RgbaF32Le)
    }

    /// Number of channels (1, 2, 3 or 4).
    pub const fn channels(self) -> usize {
        match self {
            Self::Gray8 | Self::Gray16Le | Self::GrayF32Le => 1,
            Self::Ya8 => 2,
            Self::Rgb24 | Self::Bgr24 | Self::RgbF32Le => 3,
            Self::Rgba | Self::Bgra | Self::Rgba64Le | Self::RgbaF32Le => 4,
        }
    }

    /// Short name (the `oxideav_core::PixelFormat` spelling).
    pub const fn name(self) -> &'static str {
        match self {
            Self::Gray8 => "Gray8",
            Self::Ya8 => "Ya8",
            Self::Gray16Le => "Gray16Le",
            Self::Rgb24 => "Rgb24",
            Self::Bgr24 => "Bgr24",
            Self::Rgba => "Rgba",
            Self::Bgra => "Bgra",
            Self::Rgba64Le => "Rgba64Le",
            Self::GrayF32Le => "GrayF32Le",
            Self::RgbF32Le => "RgbF32Le",
            Self::RgbaF32Le => "RgbaF32Le",
        }
    }
}

/// One pixel plane: `stride` bytes per row, `data` holding at least
/// `stride × height` bytes. Every contract layout is packed, so a
/// [`DdsImage`] has exactly one. The same record carries a stored
/// surface's bytes inside [`crate::DdsSurface`] (block-compressed
/// surfaces: `stride` is one row of blocks).
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Plane {
    /// Bytes per row.
    pub stride: usize,
    /// Row-major bytes, `stride × rows` long.
    pub data: Vec<u8>,
}

impl Plane {
    /// Wrap a plane buffer with its row stride.
    pub fn new(stride: usize, data: Vec<u8>) -> Self {
        Self { stride, data }
    }
}

/// Nominal sample range (H.273 `VideoFullRangeFlag`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum ColorRange {
    /// No range was signalled.
    #[default]
    Unspecified,
    /// Limited (video / studio) range.
    Limited,
    /// Full (PC) range.
    Full,
}

/// Colour signalling of an image: the sample range plus the H.273
/// `ColourPrimaries` / `TransferCharacteristics` / `MatrixCoefficients`
/// code points (`2` = unspecified).
///
/// DDS carries exactly one piece of colour information: a DX10 header
/// whose `DXGI_FORMAT` is one of the `_UNORM_SRGB` variants declares
/// the stored samples sRGB-encoded, and such a file decodes to
/// [`ColorInfo::srgb`]. Every other file — legacy headers, the plain
/// `_UNORM` codes, float and normalised layouts — decodes to
/// [`ColorInfo::dds_default`]: full-range RGB (`matrix` 0) with
/// unspecified primaries and transfer. The default is this crate's
/// convention, not something the format defines, so the registry
/// adapter stamps a colour signal on its frames only for the `_SRGB`
/// case.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct ColorInfo {
    /// Sample range.
    pub range: ColorRange,
    /// H.273 `ColourPrimaries` code point (`1` = BT.709 / sRGB, `2` =
    /// unspecified).
    pub primaries: u8,
    /// H.273 `TransferCharacteristics` code point (`13` = sRGB, `2` =
    /// unspecified).
    pub transfer: u8,
    /// H.273 `MatrixCoefficients` code point (`0` = identity / RGB).
    pub matrix: u8,
}

impl ColorInfo {
    /// H.273 "unspecified" code point.
    pub const UNSPECIFIED: u8 = 2;
    /// H.273 `MatrixCoefficients` identity (RGB) code point.
    pub const MATRIX_IDENTITY: u8 = 0;
    /// H.273 `ColourPrimaries` BT.709 / sRGB code point.
    pub const PRIMARIES_BT709: u8 = 1;
    /// H.273 `TransferCharacteristics` IEC 61966-2-1 sRGB code point.
    pub const TRANSFER_SRGB: u8 = 13;

    /// Build a description from its four parts.
    pub const fn new(range: ColorRange, primaries: u8, transfer: u8, matrix: u8) -> Self {
        Self {
            range,
            primaries,
            transfer,
            matrix,
        }
    }

    /// Every field unspecified.
    pub const fn unspecified() -> Self {
        Self::new(
            ColorRange::Unspecified,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
        )
    }

    /// The crate's default for a file without an `_SRGB` DXGI code:
    /// full-range device RGB (`matrix` 0), primaries and transfer
    /// unspecified.
    pub const fn dds_default() -> Self {
        Self::new(
            ColorRange::Full,
            Self::UNSPECIFIED,
            Self::UNSPECIFIED,
            Self::MATRIX_IDENTITY,
        )
    }

    /// sRGB (IEC 61966-2-1): BT.709 primaries, sRGB transfer, identity
    /// matrix, full range — what a `*_UNORM_SRGB` `DXGI_FORMAT` declares.
    pub const fn srgb() -> Self {
        Self::new(
            ColorRange::Full,
            Self::PRIMARIES_BT709,
            Self::TRANSFER_SRGB,
            Self::MATRIX_IDENTITY,
        )
    }

    /// Set the range.
    pub fn with_range(mut self, range: ColorRange) -> Self {
        self.range = range;
        self
    }

    /// Set the primaries code point.
    pub fn with_primaries(mut self, primaries: u8) -> Self {
        self.primaries = primaries;
        self
    }

    /// Set the transfer code point.
    pub fn with_transfer(mut self, transfer: u8) -> Self {
        self.transfer = transfer;
        self
    }

    /// Set the matrix code point.
    pub fn with_matrix(mut self, matrix: u8) -> Self {
        self.matrix = matrix;
        self
    }

    /// `true` when both primaries and transfer are specified (`!= 2`).
    pub fn is_specified(&self) -> bool {
        self.primaries != Self::UNSPECIFIED && self.transfer != Self::UNSPECIFIED
    }

    /// `true` when this is the sRGB description (transfer 13).
    pub fn is_srgb(&self) -> bool {
        self.transfer == Self::TRANSFER_SRGB
    }
}

impl Default for ColorInfo {
    /// [`ColorInfo::dds_default`].
    fn default() -> Self {
        Self::dds_default()
    }
}

/// The metadata blobs every image crate surfaces. DDS carries none of
/// them — the header has no ICC / Exif / XMP / gamma field — so every
/// field is `None` after a decode and ignored by the encoder.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub struct Metadata {
    /// Embedded ICC profile — DDS has none; always `None`.
    pub icc: Option<Vec<u8>>,
    /// Exif payload — DDS has none; always `None`.
    pub exif: Option<Vec<u8>>,
    /// XMP packet — DDS has none; always `None`.
    pub xmp: Option<Vec<u8>>,
    /// File gamma — DDS has none; always `None`.
    pub gamma: Option<f32>,
}

impl Metadata {
    /// Empty metadata.
    pub fn new() -> Self {
        Self::default()
    }

    /// Set (or clear) the ICC profile.
    pub fn with_icc(mut self, icc: impl Into<Option<Vec<u8>>>) -> Self {
        self.icc = icc.into();
        self
    }

    /// Set (or clear) the Exif payload.
    pub fn with_exif(mut self, exif: impl Into<Option<Vec<u8>>>) -> Self {
        self.exif = exif.into();
        self
    }

    /// Set (or clear) the XMP packet.
    pub fn with_xmp(mut self, xmp: impl Into<Option<Vec<u8>>>) -> Self {
        self.xmp = xmp.into();
        self
    }

    /// Set (or clear) the file gamma.
    pub fn with_gamma(mut self, gamma: impl Into<Option<f32>>) -> Self {
        self.gamma = gamma.into();
        self
    }

    /// `true` when no field is set.
    pub fn is_empty(&self) -> bool {
        self.icc.is_none() && self.exif.is_none() && self.xmp.is_none() && self.gamma.is_none()
    }
}

/// A decoded (or to-be-encoded) DDS picture in a native layout: one
/// surface of the file (the top-level mip of the first face / array
/// slice for [`crate::decode`]) expanded to the [`DdsPixelFormat`] its
/// stored [`SurfaceFormat`] maps to.
///
/// `planes` holds exactly one packed plane; `color` is sRGB for the
/// `_UNORM_SRGB` DXGI codes and [`ColorInfo::dds_default`] otherwise;
/// `metadata` is always empty (DDS has no metadata). DDS has no palette
/// layout this crate decodes, so there is no `palette` field. The
/// stored layout the image came from, and the full mip / face / slice
/// tree, are the depth API: [`crate::parse_dds`] → [`crate::DdsFile`].
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct DdsImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Native pixel layout.
    pub format: PixelFormat,
    /// Pixel planes — exactly one.
    pub planes: Vec<Plane>,
    /// Colour signalling (range + H.273 code points).
    pub color: ColorInfo,
    /// Metadata blobs — always empty for DDS.
    pub metadata: Metadata,
}

impl DdsImage {
    /// Assemble an image from its geometry, layout and planes (exactly
    /// one), validating the geometry: non-zero dimensions, one plane
    /// whose `stride` covers `width × bytes_per_pixel` and whose `data`
    /// covers `stride × height`. Colour is [`ColorInfo::dds_default`]
    /// and metadata empty; the `with_*` builders fill those in.
    pub fn new(width: u32, height: u32, format: PixelFormat, planes: Vec<Plane>) -> Result<Self> {
        if width == 0 || height == 0 {
            return Err(DdsError::invalid("DDS image: zero dimension"));
        }
        if planes.len() != 1 {
            return Err(DdsError::invalid(format!(
                "DDS image: expected exactly one plane, got {}",
                planes.len()
            )));
        }
        let plane = &planes[0];
        let min_stride = (width as usize)
            .checked_mul(format.bytes_per_pixel())
            .ok_or_else(|| DdsError::invalid("DDS image: row size overflows usize"))?;
        if plane.stride < min_stride {
            return Err(DdsError::invalid(format!(
                "DDS image: stride {} is below the {} bytes a {}-pixel row of {} needs",
                plane.stride,
                min_stride,
                width,
                format.name()
            )));
        }
        let needed = plane
            .stride
            .checked_mul(height as usize)
            .ok_or_else(|| DdsError::invalid("DDS image: plane size overflows usize"))?;
        if plane.data.len() < needed {
            return Err(DdsError::invalid(format!(
                "DDS image: plane holds {} bytes, {} × {} rows need {}",
                plane.data.len(),
                plane.stride,
                height,
                needed
            )));
        }
        Ok(Self {
            width,
            height,
            format,
            planes,
            color: ColorInfo::dds_default(),
            metadata: Metadata::default(),
        })
    }

    /// One packed plane with an explicit row stride (validated like
    /// [`Self::new`]).
    pub fn packed(
        width: u32,
        height: u32,
        format: PixelFormat,
        stride: usize,
        data: Vec<u8>,
    ) -> Result<Self> {
        Self::new(width, height, format, vec![Plane::new(stride, data)])
    }

    /// Tightly packed `Rgb24` from exactly `3 × width × height` bytes
    /// (`InvalidData` on a length or geometry mismatch).
    pub fn from_rgb8(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        Self::tight(width, height, PixelFormat::Rgb24, data)
    }

    /// Tightly packed `Rgba` from exactly `4 × width × height` bytes
    /// (`InvalidData` on a length or geometry mismatch).
    pub fn from_rgba8(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        Self::tight(width, height, PixelFormat::Rgba, data)
    }

    /// A tightly packed plane of `format` whose length must be exactly
    /// `width × height × bytes_per_pixel`.
    pub(crate) fn tight(
        width: u32,
        height: u32,
        format: PixelFormat,
        data: Vec<u8>,
    ) -> Result<Self> {
        let stride = (width as usize)
            .checked_mul(format.bytes_per_pixel())
            .ok_or_else(|| DdsError::invalid("DDS image: row size overflows usize"))?;
        let want = stride
            .checked_mul(height as usize)
            .ok_or_else(|| DdsError::invalid("DDS image: plane size overflows usize"))?;
        if data.len() != want {
            return Err(DdsError::invalid(format!(
                "DDS image: {} bytes supplied, {}x{} {} needs exactly {}",
                data.len(),
                width,
                height,
                format.name(),
                want
            )));
        }
        Self::packed(width, height, format, stride, data)
    }

    /// Set the colour signalling.
    pub fn with_color(mut self, color: ColorInfo) -> Self {
        self.color = color;
        self
    }

    /// Set the metadata.
    pub fn with_metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    /// Picture width in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Picture height in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// Native pixel layout.
    pub fn format(&self) -> PixelFormat {
        self.format
    }

    /// Bytes per pixel of [`Self::format`].
    pub fn bytes_per_pixel(&self) -> usize {
        self.format.bytes_per_pixel()
    }

    /// Row stride in bytes of the pixel plane (`0` if the image has no
    /// plane).
    pub fn stride(&self) -> usize {
        self.planes.first().map(|p| p.stride).unwrap_or(0)
    }

    /// The pixel bytes — `Some` for every image that has its plane
    /// (all contract layouts are packed), `None` only for an image
    /// built without planes.
    pub fn as_bytes(&self) -> Option<&[u8]> {
        self.planes.first().map(|p| p.data.as_slice())
    }

    /// Consume the image and return its plane bytes (planes
    /// concatenated in order, strides as reported).
    pub fn into_raw(self) -> Vec<u8> {
        let mut planes = self.planes.into_iter();
        let mut out = planes.next().map(|p| p.data).unwrap_or_default();
        for p in planes {
            out.extend_from_slice(&p.data);
        }
        out
    }

    /// `true` when the layout carries an alpha channel.
    pub fn has_alpha(&self) -> bool {
        self.format.has_alpha()
    }

    /// Tightly packed 8-bit RGB (`3 × width × height` bytes), exact
    /// for every native layout: grey replicated, BGR orders swizzled,
    /// 16-bit samples reduced to their high byte, `f32` samples
    /// clamped to `[0, 1]` and scaled by 255 (round half up). Rows
    /// beyond the plane read as zero, so the call never panics.
    pub fn to_rgb8(&self) -> Vec<u8> {
        self.convert(3)
    }

    /// Tightly packed 8-bit RGBA (`4 × width × height` bytes): as
    /// [`Self::to_rgb8`] with the alpha channel reduced the same way,
    /// `0xFF` for every layout without alpha.
    pub fn to_rgba8(&self) -> Vec<u8> {
        self.convert(4)
    }

    /// [`Self::to_rgb8`] that reports a geometry mismatch instead of
    /// zero-filling (for images not produced by this crate's decoder).
    pub fn try_to_rgb8(&self) -> Result<Vec<u8>> {
        self.check_geometry()?;
        Ok(self.to_rgb8())
    }

    /// [`Self::to_rgba8`] that reports a geometry mismatch instead of
    /// zero-filling.
    pub fn try_to_rgba8(&self) -> Result<Vec<u8>> {
        self.check_geometry()?;
        Ok(self.to_rgba8())
    }

    fn check_geometry(&self) -> Result<()> {
        let plane = self
            .planes
            .first()
            .ok_or_else(|| DdsError::invalid("DDS image: no plane"))?;
        let min_stride = (self.width as usize).saturating_mul(self.format.bytes_per_pixel());
        if plane.stride < min_stride
            || plane.data.len() < plane.stride.saturating_mul(self.height as usize)
        {
            return Err(DdsError::invalid("DDS image: plane geometry mismatch"));
        }
        Ok(())
    }

    /// Row `y` of the plane as a tightly packed slice, or `None` when
    /// the plane is too short.
    pub(crate) fn row(&self, y: usize) -> Option<&[u8]> {
        let plane = self.planes.first()?;
        let row_bytes = (self.width as usize).checked_mul(self.format.bytes_per_pixel())?;
        let start = y.checked_mul(plane.stride)?;
        let end = start.checked_add(row_bytes)?;
        plane.data.get(start..end)
    }

    /// Shared RGB / RGBA conversion kernel. `out_bpp` is 3 or 4.
    fn convert(&self, out_bpp: usize) -> Vec<u8> {
        let w = self.width as usize;
        let h = self.height as usize;
        let mut out = vec![0u8; w.saturating_mul(h).saturating_mul(out_bpp)];
        if out_bpp == 4 && !self.format.has_alpha() {
            for px in out.chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
        }
        for y in 0..h {
            let Some(src) = self.row(y) else {
                break;
            };
            let dst = &mut out[y * w * out_bpp..(y + 1) * w * out_bpp];
            match self.format {
                PixelFormat::Gray8 => {
                    for (d, &s) in dst.chunks_exact_mut(out_bpp).zip(src.iter()) {
                        d[0] = s;
                        d[1] = s;
                        d[2] = s;
                    }
                }
                PixelFormat::Ya8 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(2)) {
                        d[0] = s[0];
                        d[1] = s[0];
                        d[2] = s[0];
                        if out_bpp == 4 {
                            d[3] = s[1];
                        }
                    }
                }
                PixelFormat::Gray16Le => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(2)) {
                        d[0] = s[1];
                        d[1] = s[1];
                        d[2] = s[1];
                    }
                }
                PixelFormat::Rgb24 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(3)) {
                        d[..3].copy_from_slice(s);
                    }
                }
                PixelFormat::Bgr24 => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(3)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                    }
                }
                PixelFormat::Rgba => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(4)) {
                        d[..out_bpp].copy_from_slice(&s[..out_bpp]);
                    }
                }
                PixelFormat::Bgra => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(4)) {
                        d[0] = s[2];
                        d[1] = s[1];
                        d[2] = s[0];
                        if out_bpp == 4 {
                            d[3] = s[3];
                        }
                    }
                }
                PixelFormat::Rgba64Le => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(8)) {
                        d[0] = s[1];
                        d[1] = s[3];
                        d[2] = s[5];
                        if out_bpp == 4 {
                            d[3] = s[7];
                        }
                    }
                }
                PixelFormat::GrayF32Le => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(4)) {
                        let v = float_to_u8(f32::from_le_bytes([s[0], s[1], s[2], s[3]]));
                        d[0] = v;
                        d[1] = v;
                        d[2] = v;
                    }
                }
                PixelFormat::RgbF32Le => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(12)) {
                        for c in 0..3 {
                            d[c] = float_to_u8(f32::from_le_bytes([
                                s[c * 4],
                                s[c * 4 + 1],
                                s[c * 4 + 2],
                                s[c * 4 + 3],
                            ]));
                        }
                    }
                }
                PixelFormat::RgbaF32Le => {
                    for (d, s) in dst.chunks_exact_mut(out_bpp).zip(src.chunks_exact(16)) {
                        for c in 0..out_bpp {
                            d[c] = float_to_u8(f32::from_le_bytes([
                                s[c * 4],
                                s[c * 4 + 1],
                                s[c * 4 + 2],
                                s[c * 4 + 3],
                            ]));
                        }
                    }
                }
            }
        }
        out
    }
}

/// The documented `f32` → 8-bit reduction: clamp to `[0, 1]`, scale by
/// 255, round half up (`NaN` → 0).
#[inline]
pub(crate) fn float_to_u8(v: f32) -> u8 {
    if v.is_nan() {
        return 0;
    }
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Tightly packed 8-bit RGB image: `width × height × 3` bytes,
/// row-major, channel order `R, G, B`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height × 3` bytes.
    pub data: Vec<u8>,
}

impl RgbImage {
    /// Wrap a tightly packed `width × height × 3` RGB buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the image and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Stride (bytes per row) — always `width × 3`.
    pub fn stride(&self) -> usize {
        self.width as usize * 3
    }
}

/// Tightly packed 8-bit RGBA image: `width × height × 4` bytes,
/// row-major, channel order `R, G, B, A`. Opaque source layouts are
/// promoted with `α = 255`.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct RgbaImage {
    /// Image width in pixels.
    pub width: u32,
    /// Image height in pixels.
    pub height: u32,
    /// `width × height × 4` bytes.
    pub data: Vec<u8>,
}

impl RgbaImage {
    /// Wrap a tightly packed `width × height × 4` RGBA buffer.
    pub fn new(width: u32, height: u32, data: Vec<u8>) -> Self {
        Self {
            width,
            height,
            data,
        }
    }

    /// The pixel bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// Consume the image and return the pixel bytes.
    pub fn into_raw(self) -> Vec<u8> {
        self.data
    }

    /// Stride (bytes per row) — always `width × 4`.
    pub fn stride(&self) -> usize {
        self.width as usize * 4
    }
}

/// What [`crate::info`] learns from the magic, `DDS_HEADER` and
/// optional `DDS_HEADER_DXT10` without touching the pixel array.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct ImageInfo {
    /// Top-level (mip 0) width in pixels.
    pub width: u32,
    /// Top-level (mip 0) height in pixels.
    pub height: u32,
    /// The layout [`crate::decode`] would return.
    pub format: PixelFormat,
    /// Number of surfaces [`crate::decode_all`] would return: every
    /// (array slice, cubemap face, mip level, depth slice) the file
    /// carries. `1` for a plain 2D texture without mips.
    pub frames: u32,
    /// `true` when the decoded layout carries an alpha channel.
    pub has_alpha: bool,
    /// Colour signalling, resolved as [`crate::decode`] would.
    pub color: ColorInfo,
    /// DDS has no ICC profile; always `false`.
    pub has_icc: bool,
    /// DDS has no Exif; always `false`.
    pub has_exif: bool,
    /// DDS has no XMP; always `false`.
    pub has_xmp: bool,
    /// The stored on-disk layout (`D3DFMT` / `DXGI_FORMAT` family).
    pub surface_format: SurfaceFormat,
    /// `DXGI_FORMAT` carried by a `DDS_HEADER_DXT10` extension, `None`
    /// for a legacy header.
    pub dxgi_format: Option<DxgiFormat>,
    /// `true` when the file has the `DX10` extension header.
    pub dx10_header: bool,
    /// Mip levels per face / slice (`1` = no mip chain).
    pub mip_levels: u32,
    /// `true` for a cubemap (six faces per array slice).
    pub cubemap: bool,
    /// DX10 texture-array slice count (`1` for a plain texture).
    pub array_size: u32,
    /// Volume (3D) texture depth at mip 0 (`1` for a 2D texture).
    pub depth: u32,
}

impl ImageInfo {
    /// Build a header description from the four facts every DDS file
    /// states; `frames` 1, `mip_levels` / `array_size` / `depth` 1,
    /// no metadata flags, default colour — fill the rest with field
    /// assignment.
    pub fn new(
        width: u32,
        height: u32,
        format: PixelFormat,
        surface_format: SurfaceFormat,
    ) -> Self {
        Self {
            width,
            height,
            format,
            frames: 1,
            has_alpha: format.has_alpha(),
            color: ColorInfo::dds_default(),
            has_icc: false,
            has_exif: false,
            has_xmp: false,
            surface_format,
            dxgi_format: None,
            dx10_header: false,
            mip_levels: 1,
            cubemap: false,
            array_size: 1,
            depth: 1,
        }
    }
}

/// One surface of a DDS file as [`crate::decode_all`] returns it: the
/// decoded image plus its position in the mip / face / slice tree.
/// `delay` is always `None` — DDS surfaces are not an animation.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub struct Frame {
    /// The decoded surface.
    pub image: DdsImage,
    /// Presentation delay — always `None` for DDS.
    pub delay: Option<Duration>,
    /// Mip level (`0` = top level).
    pub mip_level: u32,
    /// DX10 texture-array slice (`0` for a non-array texture).
    pub array_slice: u32,
    /// Cubemap face, `None` for a non-cubemap texture.
    pub face: Option<CubemapFace>,
    /// Volume-texture depth slice (`0` for a 2D texture).
    pub depth_slice: u32,
}

impl Frame {
    /// A top-level, first-face, first-slice frame around `image`.
    pub fn new(image: DdsImage) -> Self {
        Self {
            image,
            delay: None,
            mip_level: 0,
            array_slice: 0,
            face: None,
            depth_slice: 0,
        }
    }

    /// Set the mip level.
    pub fn with_mip_level(mut self, mip_level: u32) -> Self {
        self.mip_level = mip_level;
        self
    }

    /// Set the array slice.
    pub fn with_array_slice(mut self, array_slice: u32) -> Self {
        self.array_slice = array_slice;
        self
    }

    /// Set (or clear) the cubemap face.
    pub fn with_face(mut self, face: impl Into<Option<CubemapFace>>) -> Self {
        self.face = face.into();
        self
    }

    /// Set the depth slice.
    pub fn with_depth_slice(mut self, depth_slice: u32) -> Self {
        self.depth_slice = depth_slice;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructors_validate_geometry() {
        assert!(DdsImage::from_rgb8(2, 2, vec![0; 12]).is_ok());
        assert!(matches!(
            DdsImage::from_rgb8(2, 2, vec![0; 11]),
            Err(DdsError::InvalidData(_))
        ));
        assert!(matches!(
            DdsImage::from_rgba8(0, 2, vec![]),
            Err(DdsError::InvalidData(_))
        ));
        assert!(matches!(
            DdsImage::new(2, 2, PixelFormat::Rgba, vec![]),
            Err(DdsError::InvalidData(_))
        ));
        assert!(matches!(
            DdsImage::packed(2, 2, PixelFormat::Rgba, 7, vec![0; 16]),
            Err(DdsError::InvalidData(_))
        ));
        // A stride wider than the row is fine as long as the plane covers it.
        assert!(DdsImage::packed(2, 2, PixelFormat::Rgba, 12, vec![0; 24]).is_ok());
    }

    #[test]
    fn to_rgba8_per_layout() {
        let g = DdsImage::tight(2, 1, PixelFormat::Gray8, vec![7, 200]).unwrap();
        assert_eq!(g.to_rgba8(), vec![7, 7, 7, 255, 200, 200, 200, 255]);
        assert_eq!(g.to_rgb8(), vec![7, 7, 7, 200, 200, 200]);

        let ya = DdsImage::tight(1, 1, PixelFormat::Ya8, vec![9, 3]).unwrap();
        assert_eq!(ya.to_rgba8(), vec![9, 9, 9, 3]);

        let g16 = DdsImage::tight(1, 1, PixelFormat::Gray16Le, vec![0x34, 0x12]).unwrap();
        assert_eq!(g16.to_rgb8(), vec![0x12, 0x12, 0x12]);

        let bgr = DdsImage::tight(1, 1, PixelFormat::Bgr24, vec![1, 2, 3]).unwrap();
        assert_eq!(bgr.to_rgba8(), vec![3, 2, 1, 255]);

        let bgra = DdsImage::tight(1, 1, PixelFormat::Bgra, vec![1, 2, 3, 4]).unwrap();
        assert_eq!(bgra.to_rgba8(), vec![3, 2, 1, 4]);
        assert_eq!(bgra.to_rgb8(), vec![3, 2, 1]);

        let r64 = DdsImage::tight(
            1,
            1,
            PixelFormat::Rgba64Le,
            vec![0x00, 0x10, 0x00, 0x20, 0x00, 0x30, 0x00, 0x40],
        )
        .unwrap();
        assert_eq!(r64.to_rgba8(), vec![0x10, 0x20, 0x30, 0x40]);

        let mut f = Vec::new();
        for v in [0.5f32, -1.0, 2.0, 1.0] {
            f.extend_from_slice(&v.to_le_bytes());
        }
        let fa = DdsImage::tight(1, 1, PixelFormat::RgbaF32Le, f.clone()).unwrap();
        assert_eq!(fa.to_rgba8(), vec![128, 0, 255, 255]);
        let fg = DdsImage::tight(1, 1, PixelFormat::GrayF32Le, f[..4].to_vec()).unwrap();
        assert_eq!(fg.to_rgba8(), vec![128, 128, 128, 255]);
        let frgb = DdsImage::tight(1, 1, PixelFormat::RgbF32Le, f[..12].to_vec()).unwrap();
        assert_eq!(frgb.to_rgba8(), vec![128, 0, 255, 255]);
    }

    #[test]
    fn float_reduction_rounds_half_up_and_clamps() {
        assert_eq!(float_to_u8(0.0), 0);
        assert_eq!(float_to_u8(1.0), 255);
        assert_eq!(float_to_u8(0.5), 128);
        assert_eq!(float_to_u8(-0.5), 0);
        assert_eq!(float_to_u8(7.0), 255);
        assert_eq!(float_to_u8(f32::NAN), 0);
    }

    #[test]
    fn short_plane_never_panics() {
        // Bypass the validating constructor to build a short image.
        let mut img = DdsImage::from_rgba8(2, 2, vec![1; 16]).unwrap();
        img.planes[0].data.truncate(8);
        let out = img.to_rgba8();
        assert_eq!(out.len(), 16);
        assert_eq!(&out[..8], &[1; 8]);
        assert_eq!(
            &out[8..],
            &[0; 8],
            "missing rows of an alpha layout read as zero"
        );
        assert!(img.try_to_rgba8().is_err());
        let mut grey = DdsImage::tight(1, 2, PixelFormat::Gray8, vec![9, 9]).unwrap();
        grey.planes[0].data.truncate(1);
        assert_eq!(grey.to_rgba8(), vec![9, 9, 9, 255, 0, 0, 0, 255]);
    }
}
