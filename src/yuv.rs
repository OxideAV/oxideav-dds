//! Decoders for the YUV (video) DXGI surface formats carried in a DDS
//! `DDS_HEADER_DXT10` trailer.
//!
//! Direct3D 11.1 (Windows 8) added a family of `DXGI_FORMAT` values for
//! video resources whose samples are luma/chroma (Y / U / V) rather than
//! RGB. Microsoft's public `DXGI_FORMAT` enumeration page (staged under
//! `docs/image/dds/mslearn-dxgi-format-enum.html`) specifies, for each
//! one, the per-pixel channel mapping, the plane structure, the
//! chroma-subsampling factor, the byte sizing of a staging resource, and
//! the width/height divisibility constraints. Those descriptions are the
//! sole source for this module.
//!
//! | `DXGI_FORMAT` | Value | Sampling | Bits | Layout |
//! |---------------|------:|----------|-----:|--------|
//! | `AYUV`        | 100   | 4:4:4    | 8    | packed, one 4-byte group per pixel |
//! | `Y410`        | 101   | 4:4:4    | 10   | packed, one 32-bit word per pixel |
//! | `Y416`        | 102   | 4:4:4    | 16   | packed, one 8-byte group per pixel |
//! | `NV12`        | 103   | 4:2:0    | 8    | planar: Y plane + interleaved UV plane |
//! | `P010`        | 104   | 4:2:0    | 10   | planar `u16`: Y plane + interleaved UV plane |
//! | `P016`        | 105   | 4:2:0    | 16   | planar `u16`: Y plane + interleaved UV plane |
//! | `420_OPAQUE`  | 106   | 4:2:0    | 8    | opaque; sized identically to `NV12` |
//! | `YUY2`        | 107   | 4:2:2    | 8    | packed, one 4-byte group per pixel pair |
//! | `Y210`        | 108   | 4:2:2    | 10   | packed `u16`, one 8-byte group per pixel pair |
//! | `Y216`        | 109   | 4:2:2    | 16   | packed `u16`, one 8-byte group per pixel pair |
//! | `NV11`        | 110   | 4:1:1    | 8    | planar: Y plane + interleaved UV plane (¼ width) |
//!
//! # Output representation
//!
//! Every decoder in this module yields **interleaved Y, U, V, A** samples
//! — one quadruple per pixel, row-major — with chroma replicated across
//! the subsampled neighbourhood so the caller receives one full-resolution
//! sample set. The 8-bit formats decode to `Vec<u8>` (`0..=255` per
//! channel); the 10- and 16-bit formats decode to `Vec<u16>` (the stored
//! 16-bit word per channel, with the 10-bit formats keeping the value in
//! the high bits exactly as stored on disk — Microsoft notes the runtime
//! "does not enforce whether the lowest 6 bits are 0").
//!
//! The decoders deliberately do **not** convert luma/chroma to RGB: the
//! DXGI documentation specifies the *memory layout* of these formats but
//! not a YUV→RGB colour matrix (the choice of BT.601 / BT.709 primaries
//! lives in the surrounding video pipeline, not the texture container).
//! Emitting Y/U/V samples keeps the decode lossless and matrix-agnostic,
//! mirroring the way [`crate::hdr`] returns stored channel values rather
//! than tone-mapped colour.
//!
//! For formats without an alpha channel the alpha output is the channel's
//! maximum (`0xff` for 8-bit, `0xffff` for 16-bit), i.e. fully opaque.

use crate::error::{DdsError, Result};

/// Read a little-endian `u16` at `off`.
#[inline]
fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

/// Read a little-endian `u32` at `off`.
#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Chroma-subsampling family of a YUV DXGI format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvSampling {
    /// 4:4:4 — chroma sampled at full resolution (one U/V per pixel).
    S444,
    /// 4:2:2 — chroma sampled at half horizontal resolution (shared
    /// across a horizontal pixel pair).
    S422,
    /// 4:2:0 — chroma sampled at half horizontal *and* half vertical
    /// resolution (shared across a 2×2 pixel block).
    S420,
    /// 4:1:1 — chroma sampled at quarter horizontal resolution (shared
    /// across a horizontal run of four pixels).
    S411,
}

impl YuvSampling {
    /// The required width multiple: pairs (4:2:2 / 4:2:0) need even
    /// width, 4:1:1 needs a multiple of four, 4:4:4 has no constraint.
    pub fn width_multiple(self) -> u32 {
        match self {
            YuvSampling::S444 => 1,
            YuvSampling::S422 | YuvSampling::S420 => 2,
            YuvSampling::S411 => 4,
        }
    }

    /// The required height multiple: only 4:2:0 subsamples vertically and
    /// therefore needs even height.
    pub fn height_multiple(self) -> u32 {
        match self {
            YuvSampling::S420 => 2,
            _ => 1,
        }
    }
}

/// The set of YUV DXGI formats this module decodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YuvFormat {
    /// `DXGI_FORMAT_AYUV` (100): 8-bit 4:4:4 packed, V→R8 U→G8 Y→B8 A→A8.
    Ayuv,
    /// `DXGI_FORMAT_Y410` (101): 10-bit 4:4:4 packed, U→R10 Y→G10 V→B10 A→A2.
    Y410,
    /// `DXGI_FORMAT_Y416` (102): 16-bit 4:4:4 packed, U→R16 Y→G16 V→B16 A→A16.
    Y416,
    /// `DXGI_FORMAT_NV12` (103): 8-bit 4:2:0 planar (Y plane + UV plane).
    Nv12,
    /// `DXGI_FORMAT_P010` (104): 10-bit 4:2:0 planar `u16`.
    P010,
    /// `DXGI_FORMAT_P016` (105): 16-bit 4:2:0 planar `u16`.
    P016,
    /// `DXGI_FORMAT_420_OPAQUE` (106): 8-bit 4:2:0; sized as `NV12`.
    Opaque420,
    /// `DXGI_FORMAT_YUY2` (107): 8-bit 4:2:2 packed, Y0→R8 U0→G8 Y1→B8 V0→A8.
    Yuy2,
    /// `DXGI_FORMAT_Y210` (108): 10-bit 4:2:2 packed `u16`.
    Y210,
    /// `DXGI_FORMAT_Y216` (109): 16-bit 4:2:2 packed `u16`.
    Y216,
    /// `DXGI_FORMAT_NV11` (110): 8-bit 4:1:1 planar (Y plane + UV plane).
    Nv11,
}

impl YuvFormat {
    /// The chroma-subsampling family of this format.
    pub fn sampling(self) -> YuvSampling {
        match self {
            YuvFormat::Ayuv | YuvFormat::Y410 | YuvFormat::Y416 => YuvSampling::S444,
            YuvFormat::Yuy2 | YuvFormat::Y210 | YuvFormat::Y216 => YuvSampling::S422,
            YuvFormat::Nv12 | YuvFormat::P010 | YuvFormat::P016 | YuvFormat::Opaque420 => {
                YuvSampling::S420
            }
            YuvFormat::Nv11 => YuvSampling::S411,
        }
    }

    /// Bits per channel as stored (8 for the byte formats; 16 for the
    /// `u16`-backed formats — 10-bit formats still occupy a full 16-bit
    /// word on disk, value left-justified into the high bits).
    pub fn stored_bits(self) -> u32 {
        match self {
            YuvFormat::Ayuv
            | YuvFormat::Nv12
            | YuvFormat::Opaque420
            | YuvFormat::Yuy2
            | YuvFormat::Nv11 => 8,
            YuvFormat::Y410 => 10,
            YuvFormat::Y416
            | YuvFormat::P010
            | YuvFormat::P016
            | YuvFormat::Y210
            | YuvFormat::Y216 => 16,
        }
    }

    /// `true` if the decoder for this format yields `u16` samples (10-
    /// and 16-bit formats); `false` for the 8-bit `u8` formats.
    pub fn is_u16(self) -> bool {
        !matches!(
            self,
            YuvFormat::Ayuv
                | YuvFormat::Nv12
                | YuvFormat::Opaque420
                | YuvFormat::Yuy2
                | YuvFormat::Nv11
        )
    }

    /// Whether this format carries a per-pixel alpha channel on disk.
    /// Only the 4:4:4 packed formats do (`AYUV`, `Y410`, `Y416`); every
    /// other format is opaque and decodes alpha to its channel maximum.
    pub fn has_alpha(self) -> bool {
        matches!(self, YuvFormat::Ayuv | YuvFormat::Y410 | YuvFormat::Y416)
    }

    /// The exact number of on-disk bytes a mip-0 surface of these
    /// dimensions occupies, per Microsoft's staging-resource sizing
    /// rules. Returns `Err(DdsError::InvalidData)` on dimension overflow.
    pub fn surface_size_bytes(self, width: u32, height: u32) -> Result<u64> {
        let w = width as u64;
        let h = height as u64;
        let size = match self {
            // 4:4:4 packed, one group per pixel.
            YuvFormat::Ayuv => w.checked_mul(h).and_then(|n| n.checked_mul(4)),
            YuvFormat::Y410 => w.checked_mul(h).and_then(|n| n.checked_mul(4)),
            YuvFormat::Y416 => w.checked_mul(h).and_then(|n| n.checked_mul(8)),
            // 4:2:2 packed, one group per pixel pair.
            YuvFormat::Yuy2 => w.checked_mul(h).and_then(|n| n.checked_mul(2)),
            YuvFormat::Y210 | YuvFormat::Y216 => w.checked_mul(h).and_then(|n| n.checked_mul(4)),
            // 4:2:0 planar: Y plane (w*h) + UV plane (w*h/2). Microsoft:
            // "rowPitch * (height + (height / 2))".
            YuvFormat::Nv12 | YuvFormat::Opaque420 => {
                w.checked_mul(h).and_then(|y| y.checked_add(w * (h / 2)))
            }
            // 10/16-bit planar 4:2:0: same shape, two bytes per sample.
            YuvFormat::P010 | YuvFormat::P016 => w
                .checked_mul(h)
                .and_then(|y| y.checked_add(w * (h / 2)))
                .and_then(|n| n.checked_mul(2)),
            // 4:1:1 planar 8-bit. Microsoft: staging size is
            // "rowPitch * height * 2"; the Y plane is w*h and the UV
            // plane (½-width interleaved) is (w/2)*h, total 1.5*w*h,
            // padded to 2*w*h.
            YuvFormat::Nv11 => w.checked_mul(h).and_then(|n| n.checked_mul(2)),
        };
        size.ok_or_else(|| {
            DdsError::invalid(format!("YUV surface size overflow for {width}x{height}"))
        })
    }

    /// Validate the width/height divisibility constraints documented for
    /// this format and return `Err(DdsError::InvalidData)` if violated.
    pub fn check_dims(self, width: u32, height: u32) -> Result<()> {
        if width == 0 || height == 0 {
            return Err(DdsError::invalid("YUV surface has a zero dimension"));
        }
        let wm = self.sampling().width_multiple();
        let hm = self.sampling().height_multiple();
        if width % wm != 0 {
            return Err(DdsError::invalid(format!(
                "{self:?} requires width divisible by {wm}, got {width}"
            )));
        }
        if height % hm != 0 {
            return Err(DdsError::invalid(format!(
                "{self:?} requires height divisible by {hm}, got {height}"
            )));
        }
        Ok(())
    }
}

/// Reject `data` shorter than `need` bytes with a descriptive error.
fn require_len(data: &[u8], need: u64, what: &str, width: u32, height: u32) -> Result<()> {
    if (data.len() as u64) < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    Ok(())
}

// ---------------------------------------------------------------------
// 8-bit decoders (yield interleaved YUVA `u8`).
// ---------------------------------------------------------------------

/// Decode a `DXGI_FORMAT_AYUV` (value 100) surface — 8-bit 4:4:4 packed —
/// into interleaved `[Y, U, V, A]` `u8` samples, `width × height × 4`.
///
/// Microsoft maps the format onto an `R8G8B8A8` view with
/// `V→R8, U→G8, Y→B8, A→A8`. Because `R8G8B8A8` is RGBA byte order on
/// disk, the four bytes of each pixel are `[V, U, Y, A]`; this decoder
/// re-orders them to the module-canonical `[Y, U, V, A]`.
pub fn decode_ayuv_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    YuvFormat::Ayuv.check_dims(width, height)?;
    let need = YuvFormat::Ayuv.surface_size_bytes(width, height)?;
    require_len(data, need, "decode_ayuv", width, height)?;
    let px = (width as usize) * (height as usize);
    let mut out = Vec::with_capacity(px * 4);
    for i in 0..px {
        let o = i * 4;
        let v = data[o];
        let u = data[o + 1];
        let y = data[o + 2];
        let a = data[o + 3];
        out.extend_from_slice(&[y, u, v, a]);
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_YUY2` (value 107) surface — 8-bit 4:2:2 packed —
/// into interleaved `[Y, U, V, A]` `u8` samples, `width × height × 4`
/// (chroma replicated across each horizontal pixel pair; alpha `0xff`).
///
/// Microsoft maps the format onto an `R8G8B8A8` view with
/// `Y0→R8, U0→G8, Y1→B8, V0→A8`. The four bytes of each 32-bit block are
/// therefore `[Y0, U0, Y1, V0]`: two luma samples plus one shared U and
/// one shared V for the pixel pair.
pub fn decode_yuy2_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    YuvFormat::Yuy2.check_dims(width, height)?;
    let need = YuvFormat::Yuy2.surface_size_bytes(width, height)?;
    require_len(data, need, "decode_yuy2", width, height)?;
    let w = width as usize;
    let h = height as usize;
    let mut out = vec![0u8; w * h * 4];
    let row_bytes = w * 2; // 4 bytes per pixel pair = 2 bytes per pixel
    for row in 0..h {
        let src = row * row_bytes;
        let dst = row * w * 4;
        for pair in 0..(w / 2) {
            let s = src + pair * 4;
            let y0 = data[s];
            let u = data[s + 1];
            let y1 = data[s + 2];
            let v = data[s + 3];
            let d = dst + pair * 8;
            out[d..d + 4].copy_from_slice(&[y0, u, v, 0xff]);
            out[d + 4..d + 8].copy_from_slice(&[y1, u, v, 0xff]);
        }
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_NV12` (value 103) surface — 8-bit 4:2:0 planar —
/// into interleaved `[Y, U, V, A]` `u8` samples, `width × height × 4`
/// (chroma replicated across each 2×2 block; alpha `0xff`).
///
/// The surface is two planes: the first `width × height` bytes are the Y
/// plane (`Y→R8`), the remaining `width × (height / 2)` bytes are the UV
/// plane, U and V interleaved (`U→R8, V→G8`), one U/V pair per 2×2 luma
/// block.
pub fn decode_nv12_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode_nv12_like(YuvFormat::Nv12, width, height, data, "decode_nv12")
}

/// Decode a `DXGI_FORMAT_420_OPAQUE` (value 106) surface as an
/// `NV12`-equivalent 8-bit 4:2:0 layout.
///
/// Microsoft documents `420_OPAQUE` as sized identically to `NV12`
/// ("(rowPitch * (height + (height / 2))) bytes") but with a layout that
/// is "completely opaque to applications" — the runtime is free to store
/// any 4:2:0 arrangement (YV12, NV12, …). Since the on-disk arrangement
/// is not defined, this decoder treats the bytes as the canonical NV12
/// plane layout. Callers must understand the result is only meaningful
/// when the producer actually used NV12 ordering.
pub fn decode_420_opaque_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    decode_nv12_like(
        YuvFormat::Opaque420,
        width,
        height,
        data,
        "decode_420_opaque",
    )
}

/// Shared NV12 / 420_OPAQUE plane decode.
fn decode_nv12_like(
    fmt: YuvFormat,
    width: u32,
    height: u32,
    data: &[u8],
    what: &str,
) -> Result<Vec<u8>> {
    fmt.check_dims(width, height)?;
    let need = fmt.surface_size_bytes(width, height)?;
    require_len(data, need, what, width, height)?;
    let w = width as usize;
    let h = height as usize;
    let y_plane = w * h;
    let chroma_w = w / 2;
    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let crow = row / 2;
        for col in 0..w {
            let ccol = col / 2;
            let uv = y_plane + crow * (chroma_w * 2) + ccol * 2;
            let y = data[row * w + col];
            let u = data[uv];
            let v = data[uv + 1];
            let d = (row * w + col) * 4;
            out[d..d + 4].copy_from_slice(&[y, u, v, 0xff]);
        }
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_NV11` (value 110) surface — 8-bit 4:1:1 planar —
/// into interleaved `[Y, U, V, A]` `u8` samples, `width × height × 4`
/// (chroma replicated across each horizontal run of four pixels; alpha
/// `0xff`).
///
/// The surface is two planes: the first `width × height` bytes are the Y
/// plane, the next `(width / 2) × height` bytes are the UV plane with U
/// and V interleaved at quarter horizontal resolution (one U/V pair per
/// four luma samples in a row). Microsoft notes the staging allocation is
/// padded to `rowPitch * height * 2`; only the leading 1.5·w·h bytes
/// carry data.
pub fn decode_nv11_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    YuvFormat::Nv11.check_dims(width, height)?;
    let need = YuvFormat::Nv11.surface_size_bytes(width, height)?;
    require_len(data, need, "decode_nv11", width, height)?;
    let w = width as usize;
    let h = height as usize;
    let y_plane = w * h;
    // UV plane: quarter horizontal resolution → (w/4) U/V pairs per row,
    // stored as 2·(w/4) = w/2 bytes per row.
    let chroma_pairs = w / 4;
    let chroma_row_bytes = chroma_pairs * 2;
    let mut out = vec![0u8; w * h * 4];
    for row in 0..h {
        let crow_base = y_plane + row * chroma_row_bytes;
        for col in 0..w {
            let cgroup = col / 4;
            let uv = crow_base + cgroup * 2;
            let y = data[row * w + col];
            let u = data[uv];
            let v = data[uv + 1];
            let d = (row * w + col) * 4;
            out[d..d + 4].copy_from_slice(&[y, u, v, 0xff]);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------
// 10 / 16-bit decoders (yield interleaved YUVA `u16`).
// ---------------------------------------------------------------------

/// Decode a `DXGI_FORMAT_Y410` (value 101) surface — 10-bit 4:4:4 packed
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`
/// (Y/U/V in `0..=1023`, A in `0..=3`).
///
/// Each pixel is one little-endian 32-bit word viewed as `R10G10B10A2`
/// with `U→R10, Y→G10, V→B10, A→A2`. Following the Direct3D packing rule
/// (first named component in the least-significant bits), the word holds
/// U in bits 0..=9, Y in bits 10..=19, V in bits 20..=29, A in bits
/// 30..=31.
pub fn decode_y410_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    YuvFormat::Y410.check_dims(width, height)?;
    let need = YuvFormat::Y410.surface_size_bytes(width, height)?;
    require_len(data, need, "decode_y410", width, height)?;
    let px = (width as usize) * (height as usize);
    let mut out = Vec::with_capacity(px * 4);
    for i in 0..px {
        let word = read_u32_le(data, i * 4);
        let u = (word & 0x3ff) as u16;
        let y = ((word >> 10) & 0x3ff) as u16;
        let v = ((word >> 20) & 0x3ff) as u16;
        let a = ((word >> 30) & 0x3) as u16;
        out.extend_from_slice(&[y, u, v, a]);
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_Y416` (value 102) surface — 16-bit 4:4:4 packed
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`.
///
/// Each pixel is four little-endian `u16` channels viewed as
/// `R16G16B16A16` with `U→R16, Y→G16, V→B16, A→A16`. The on-disk word
/// order is therefore `[U, Y, V, A]`.
pub fn decode_y416_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    YuvFormat::Y416.check_dims(width, height)?;
    let need = YuvFormat::Y416.surface_size_bytes(width, height)?;
    require_len(data, need, "decode_y416", width, height)?;
    let px = (width as usize) * (height as usize);
    let mut out = Vec::with_capacity(px * 4);
    for i in 0..px {
        let o = i * 8;
        let u = read_u16_le(data, o);
        let y = read_u16_le(data, o + 2);
        let v = read_u16_le(data, o + 4);
        let a = read_u16_le(data, o + 6);
        out.extend_from_slice(&[y, u, v, a]);
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_Y210` (value 108) surface — 10-bit 4:2:2 packed
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`
/// (chroma replicated across each pixel pair; alpha `0xffff`).
///
/// Each pixel pair is four little-endian `u16` words viewed as
/// `R16G16B16A16` with `Y0→R16, U→G16, Y1→B16, V→A16`. The 10-bit value
/// is stored left-justified into the high bits of each 16-bit word; this
/// decoder returns the word as-stored (Microsoft: the runtime "does not
/// enforce whether the lowest 6 bits are 0").
pub fn decode_y210_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_y21x_surface(YuvFormat::Y210, width, height, data, "decode_y210")
}

/// Decode a `DXGI_FORMAT_Y216` (value 109) surface — 16-bit 4:2:2 packed
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`
/// (chroma replicated across each pixel pair; alpha `0xffff`).
///
/// Identical word layout to [`decode_y210_surface`]
/// (`Y0→R16, U→G16, Y1→B16, V→A16`) but every bit of each 16-bit word is
/// significant.
pub fn decode_y216_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_y21x_surface(YuvFormat::Y216, width, height, data, "decode_y216")
}

/// Shared Y210 / Y216 packed 4:2:2 decode (the byte layout is identical;
/// they differ only in how many of the 16 stored bits are significant,
/// which this decoder does not need to interpret).
fn decode_y21x_surface(
    fmt: YuvFormat,
    width: u32,
    height: u32,
    data: &[u8],
    what: &str,
) -> Result<Vec<u16>> {
    fmt.check_dims(width, height)?;
    let need = fmt.surface_size_bytes(width, height)?;
    require_len(data, need, what, width, height)?;
    let w = width as usize;
    let h = height as usize;
    let mut out = vec![0u16; w * h * 4];
    let row_bytes = w * 4; // 8 bytes per pixel pair = 4 bytes per pixel
    for row in 0..h {
        let src = row * row_bytes;
        let dst = row * w * 4;
        for pair in 0..(w / 2) {
            let s = src + pair * 8;
            let y0 = read_u16_le(data, s);
            let u = read_u16_le(data, s + 2);
            let y1 = read_u16_le(data, s + 4);
            let v = read_u16_le(data, s + 6);
            let d = dst + pair * 8;
            out[d..d + 4].copy_from_slice(&[y0, u, v, 0xffff]);
            out[d + 4..d + 8].copy_from_slice(&[y1, u, v, 0xffff]);
        }
    }
    Ok(out)
}

/// Decode a `DXGI_FORMAT_P010` (value 104) surface — 10-bit 4:2:0 planar
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`
/// (chroma replicated across each 2×2 block; alpha `0xffff`).
///
/// Two `u16` planes: the Y plane (`width × height` words) followed by the
/// interleaved UV plane (`width × (height / 2)` words, `U→R16, V→G16`),
/// one U/V pair per 2×2 luma block. The 10-bit value sits in the high
/// bits of each word and is returned as-stored.
pub fn decode_p010_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_p01x_surface(YuvFormat::P010, width, height, data, "decode_p010")
}

/// Decode a `DXGI_FORMAT_P016` (value 105) surface — 16-bit 4:2:0 planar
/// — into interleaved `[Y, U, V, A]` `u16` samples, `width × height × 4`.
///
/// Identical plane layout to [`decode_p010_surface`]; every bit of each
/// 16-bit word is significant.
pub fn decode_p016_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_p01x_surface(YuvFormat::P016, width, height, data, "decode_p016")
}

/// Shared P010 / P016 planar 4:2:0 `u16` decode.
fn decode_p01x_surface(
    fmt: YuvFormat,
    width: u32,
    height: u32,
    data: &[u8],
    what: &str,
) -> Result<Vec<u16>> {
    fmt.check_dims(width, height)?;
    let need = fmt.surface_size_bytes(width, height)?;
    require_len(data, need, what, width, height)?;
    let w = width as usize;
    let h = height as usize;
    // Byte offset of the UV plane (Y plane = w*h u16 = 2*w*h bytes).
    let uv_base = 2 * w * h;
    let chroma_w = w / 2;
    let mut out = vec![0u16; w * h * 4];
    for row in 0..h {
        let crow = row / 2;
        for col in 0..w {
            let ccol = col / 2;
            // UV plane row pitch = chroma_w pairs × 2 channels × 2 bytes.
            let uv = uv_base + (crow * chroma_w * 2 + ccol * 2) * 2;
            let y = read_u16_le(data, (row * w + col) * 2);
            let u = read_u16_le(data, uv);
            let v = read_u16_le(data, uv + 2);
            let d = (row * w + col) * 4;
            out[d..d + 4].copy_from_slice(&[y, u, v, 0xffff]);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ayuv_reorders_to_yuva() {
        // One pixel, on-disk [V, U, Y, A] = [10, 20, 30, 40].
        let data = [10u8, 20, 30, 40];
        let out = decode_ayuv_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![30, 20, 10, 40]); // [Y, U, V, A]
    }

    #[test]
    fn ayuv_size_and_truncation() {
        assert_eq!(YuvFormat::Ayuv.surface_size_bytes(4, 4).unwrap(), 64);
        let short = [0u8; 3];
        assert!(matches!(
            decode_ayuv_surface(1, 1, &short),
            Err(DdsError::InvalidData(_))
        ));
    }

    #[test]
    fn yuy2_pair_shares_chroma() {
        // One pixel pair: [Y0, U0, Y1, V0] = [11, 22, 33, 44].
        let data = [11u8, 22, 33, 44];
        let out = decode_yuy2_surface(2, 1, &data).unwrap();
        // px0 = [Y0,U,V,A], px1 = [Y1,U,V,A]
        assert_eq!(out, vec![11, 22, 44, 0xff, 33, 22, 44, 0xff]);
    }

    #[test]
    fn yuy2_requires_even_width() {
        let data = [0u8; 12];
        assert!(matches!(
            decode_yuy2_surface(3, 1, &data),
            Err(DdsError::InvalidData(_))
        ));
    }

    #[test]
    fn nv12_plane_layout_2x2() {
        // 2x2 luma + one UV pair. Y plane = [1,2,3,4]; UV = [200,201].
        let data = [1u8, 2, 3, 4, 200, 201];
        let out = decode_nv12_surface(2, 2, &data).unwrap();
        // every pixel shares U=200, V=201, A=0xff
        assert_eq!(
            out,
            vec![
                1, 200, 201, 0xff, 2, 200, 201, 0xff, // row 0
                3, 200, 201, 0xff, 4, 200, 201, 0xff, // row 1
            ]
        );
    }

    #[test]
    fn nv12_size() {
        // 4x4: Y=16 + UV=8 = 24 bytes.
        assert_eq!(YuvFormat::Nv12.surface_size_bytes(4, 4).unwrap(), 24);
    }

    #[test]
    fn nv12_requires_even_dims() {
        let data = [0u8; 64];
        assert!(decode_nv12_surface(3, 2, &data).is_err());
        assert!(decode_nv12_surface(2, 3, &data).is_err());
    }

    #[test]
    fn opaque420_decodes_as_nv12() {
        let data = [1u8, 2, 3, 4, 200, 201];
        let a = decode_nv12_surface(2, 2, &data).unwrap();
        let b = decode_420_opaque_surface(2, 2, &data).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn nv11_quarter_width_chroma() {
        // 4x1 luma + one UV pair (quarter horizontal). Y=[1,2,3,4],
        // UV=[50,51]. Surface padded to 2*w*h = 8 bytes.
        let data = [1u8, 2, 3, 4, 50, 51, 0, 0];
        let out = decode_nv11_surface(4, 1, &data).unwrap();
        assert_eq!(
            out,
            vec![1, 50, 51, 0xff, 2, 50, 51, 0xff, 3, 50, 51, 0xff, 4, 50, 51, 0xff,]
        );
    }

    #[test]
    fn nv11_requires_width_multiple_of_four() {
        let data = [0u8; 64];
        assert!(decode_nv11_surface(2, 1, &data).is_err());
    }

    #[test]
    fn y410_bit_unpacking() {
        // U=1 (bits 0..9), Y=2 (bits 10..19), V=3 (bits 20..29), A=2 (30..31)
        let word: u32 = 1 | (2 << 10) | (3 << 20) | (2 << 30);
        let data = word.to_le_bytes();
        let out = decode_y410_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![2u16, 1, 3, 2]); // [Y, U, V, A]
    }

    #[test]
    fn y416_word_order() {
        // On-disk [U, Y, V, A] = [100, 200, 300, 400] as u16 LE.
        let mut data = Vec::new();
        for w in [100u16, 200, 300, 400] {
            data.extend_from_slice(&w.to_le_bytes());
        }
        let out = decode_y416_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![200u16, 100, 300, 400]); // [Y, U, V, A]
    }

    #[test]
    fn y210_pair_shares_chroma() {
        // [Y0, U, Y1, V] = [1000, 2000, 3000, 4000] as u16 LE.
        let mut data = Vec::new();
        for w in [1000u16, 2000, 3000, 4000] {
            data.extend_from_slice(&w.to_le_bytes());
        }
        let out = decode_y210_surface(2, 1, &data).unwrap();
        assert_eq!(
            out,
            vec![1000, 2000, 4000, 0xffff, 3000, 2000, 4000, 0xffff]
        );
    }

    #[test]
    fn y216_matches_y210_layout() {
        let mut data = Vec::new();
        for w in [1u16, 2, 3, 4] {
            data.extend_from_slice(&w.to_le_bytes());
        }
        let a = decode_y210_surface(2, 1, &data).unwrap();
        let b = decode_y216_surface(2, 1, &data).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn p010_plane_layout() {
        // 2x2 Y plane (u16) = [1,2,3,4]; UV pair = [500, 600].
        let mut data = Vec::new();
        for w in [1u16, 2, 3, 4, 500, 600] {
            data.extend_from_slice(&w.to_le_bytes());
        }
        let out = decode_p010_surface(2, 2, &data).unwrap();
        assert_eq!(
            out,
            vec![
                1, 500, 600, 0xffff, 2, 500, 600, 0xffff, 3, 500, 600, 0xffff, 4, 500, 600, 0xffff,
            ]
        );
    }

    #[test]
    fn p016_matches_p010_layout() {
        let mut data = Vec::new();
        for w in [1u16, 2, 3, 4, 5, 6] {
            data.extend_from_slice(&w.to_le_bytes());
        }
        let a = decode_p010_surface(2, 2, &data).unwrap();
        let b = decode_p016_surface(2, 2, &data).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn p010_size() {
        // 4x4 u16: Y=32 bytes + UV=16 bytes = 48.
        assert_eq!(YuvFormat::P010.surface_size_bytes(4, 4).unwrap(), 48);
    }

    #[test]
    fn metadata_consistency() {
        for f in [
            YuvFormat::Ayuv,
            YuvFormat::Y410,
            YuvFormat::Y416,
            YuvFormat::Nv12,
            YuvFormat::P010,
            YuvFormat::P016,
            YuvFormat::Opaque420,
            YuvFormat::Yuy2,
            YuvFormat::Y210,
            YuvFormat::Y216,
            YuvFormat::Nv11,
        ] {
            // u16 formats are exactly the non-8-bit ones.
            assert_eq!(f.is_u16(), f.stored_bits() != 8);
            // alpha only on the 4:4:4 packed formats.
            assert_eq!(f.has_alpha(), f.sampling() == YuvSampling::S444);
        }
    }
}
