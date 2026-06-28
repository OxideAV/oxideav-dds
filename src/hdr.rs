//! Decoders for the extended high-bit-depth and floating-point
//! uncompressed DDS surfaces.
//!
//! These are the layouts Microsoft assigns to the legacy `D3DFMT`
//! numeric FourCC codes 36 / 110..=116 and to the matching
//! `DXGI_FORMAT` values:
//!
//! | FourCC | `D3DFMT`             | `DXGI_FORMAT`           | Channels       |
//! |-------:|----------------------|-------------------------|----------------|
//! | 36     | `A16B16G16R16`       | `R16G16B16A16_UNORM`    | 4 × `u16`      |
//! | 110    | `Q16W16V16U16`       | `R16G16B16A16_SNORM`    | 4 × `i16`      |
//! | 111    | `R16F`               | `R16_FLOAT`             | 1 × half-float |
//! | 112    | `G16R16F`            | `R16G16_FLOAT`          | 2 × half-float |
//! | 113    | `A16B16G16R16F`      | `R16G16B16A16_FLOAT`    | 4 × half-float |
//! | 114    | `R32F`               | `R32_FLOAT`             | 1 × `f32`      |
//! | 115    | `G32R32F`            | `R32G32_FLOAT`          | 2 × `f32`      |
//! | 116    | `A32B32G32R32F`      | `R32G32B32A32_FLOAT`    | 4 × `f32`      |
//!
//! Channel order, bit count, and the FourCC ↔ DXGI ↔ `D3DFMT`
//! correspondence come from Microsoft's public DDS / DXGI
//! programming-guide pages. Each sample is stored little-endian, with
//! the channels packed in the order the DXGI name lists them (the first
//! letter is at the lowest memory address — i.e. `R16G16B16A16` stores
//! R, then G, then B, then A).
//!
//! The decoders below return the stored sample values directly:
//!
//! * The half-float (`*_FLOAT` 16-bit) and `f32` (`*_FLOAT` 32-bit)
//!   layouts are decoded to `f32` per channel — the half-float path
//!   reuses the crate's IEEE-754 binary16 → `f32` widening
//!   ([`crate::bc6h::half_to_f32`]); the 32-bit path reinterprets the
//!   four little-endian bytes as an IEEE-754 binary32.
//! * The `R16G16B16A16_UNORM` and `R16G16B16A16_SNORM` layouts are
//!   decoded to their stored 16-bit integers per channel (`u16` /
//!   `i16`). Mapping the unsigned-normalised / signed-normalised
//!   integers onto a real range is left to the caller; see the
//!   crate-level docs for the open documentation gap on that point.
//!
//! A further packed floating-point layout, `R11G11B10_FLOAT`
//! (`DXGI_FORMAT` value 26), encodes three partial-precision
//! floating-point channels into a single little-endian 32-bit word.
//! Microsoft's public DXGI format reference specifies the exact
//! packing: there are no sign bits, each channel carries a 5-bit
//! biased-by-15 exponent, R and G carry a 6-bit mantissa, and B carries
//! a 5-bit mantissa. With the "first named component occupies the
//! least-significant bits" rule the 32-bit word holds R in bits 0..=10,
//! G in bits 11..=21, and B in bits 22..=31. [`decode_r11g11b10_float_surface`]
//! widens each channel to `f32`. That format's table entry carries the
//! reference's footnotes 5 and 7, so the mantissa has an *implied
//! leading one* (footnote 5: "If the exponent is not 0, 1.0 is added to
//! the mantissa before applying the exponent") and the channels support
//! denormals (footnote 7).
//!
//! The shared-exponent layout `R9G9B9E5_SHAREDEXP` (`DXGI_FORMAT` value
//! 67) also packs three sign-less partial-precision channels into one
//! little-endian 32-bit word, but the three channels *share* a single
//! 5-bit biased-by-15 exponent and each carries a 9-bit mantissa. The
//! reference describes it as a "variant of s10e5" with no sign bit, a
//! shared 5-bit biased-by-15 exponent and a 9-bit mantissa per channel.
//! Its table entry carries footnotes 6 and 7: footnote 6 states "These
//! float formats do not have an implied 1 added to their mantissa", and
//! footnote 7 grants denormal support. Because there is no implied
//! leading one, the value of each channel is the *pure* fraction
//! `mantissa / 2^9` scaled by the shared exponent — a single linear
//! expression `mantissa × 2^(exp − 15 − 9)` = `mantissa × 2^(exp − 24)`
//! covers every exponent (including the all-zero word, which is +0).
//! Following the same "first named component occupies the
//! least-significant bits" rule the 32-bit word holds R in bits 0..=8,
//! G in bits 9..=17, B in bits 18..=26, and the shared 5-bit exponent
//! in bits 27..=31. [`decode_r9g9b9e5_sharedexp_surface`] widens each
//! channel to `f32`.
//!
//! A packed integer layout, `R10G10B10A2_UNORM` (`DXGI_FORMAT` value 24,
//! legacy `D3DFMT_A2B10G10R10`), stores three 10-bit colour channels and
//! one 2-bit alpha channel in a single little-endian 32-bit word. The
//! programming guide's pixel-format table gives the bit masks
//! (R = `0x000003ff`, G = `0x000ffc00`, B = `0x3ff00000`,
//! A = `0xc0000000`), so with the "first named component occupies the
//! least-significant bits" rule R is in bits 0..=9, G in bits 10..=19,
//! B in bits 20..=29, and A in bits 30..=31.
//! [`decode_r10g10b10a2_unorm_surface`] returns the stored
//! unsigned-normalised integers directly (R / G / B in `0..=1023`,
//! A in `0..=3`); as with the `R16G16B16A16_UNORM` path the crate does
//! not scale them onto a real range, leaving the per-channel division
//! (`/ 1023` for colour, `/ 3` for alpha) to the caller.
//!
//! The sibling integer layout `R10G10B10A2_UINT` (`DXGI_FORMAT` value
//! 25) uses the *same* bit packing as value 24 but the reference
//! describes it as a "four-component, 32-bit unsigned-integer format"
//! rather than "unsigned-normalized-integer": the stored channels are
//! plain unsigned integers, so there is no `[0, 1]` normalisation step
//! at all. [`decode_r10g10b10a2_uint_surface`] returns the stored
//! integers (R / G / B in `0..=1023`, A in `0..=3`) as the values
//! themselves. The format has no legacy `D3DFMT` four-cc — it is
//! DX10-header only.

use crate::bc6h::half_to_f32;
use crate::error::{DdsError, Result};
use crate::image::DdsPixelFormat;

/// Read a little-endian `u16` at `off`.
#[inline]
fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

/// Read a little-endian `f32` at `off`.
#[inline]
fn read_f32_le(buf: &[u8], off: usize) -> f32 {
    f32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Read a little-endian `u32` at `off`.
#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

/// Widen one packed `R11G11B10_FLOAT` channel to `f32`.
///
/// `mantissa_bits` is 6 for the R / G channels and 5 for the B channel;
/// the exponent is always 5 bits, biased by 15, and there is no sign
/// bit (the value is always non-negative). The mapping mirrors the
/// IEEE-754 half-precision rules with the sign held at zero:
///
/// * exponent 0, mantissa 0 → +0.
/// * exponent 0, mantissa ≠ 0 → a subnormal: normalise the mantissa
///   into the implied-leading-one form and emit the corresponding
///   binary32 (these small floats support denormals per the format
///   reference, so they are not flushed to zero).
/// * exponent all-ones (31) → infinity (mantissa 0) or NaN
///   (mantissa ≠ 0), re-expressed with the binary32 all-ones exponent.
/// * otherwise → a normal value: re-bias the exponent from 15 to 127
///   and left-align the mantissa into binary32's 23-bit field.
#[inline]
fn packed_float_to_f32(bits: u32, mantissa_bits: u32) -> f32 {
    let mant_mask = (1u32 << mantissa_bits) - 1;
    let exp = (bits >> mantissa_bits) & 0x1f;
    let mant = bits & mant_mask;
    // Number of low bits to shift the source mantissa up so its top bit
    // lands at binary32 mantissa bit 22.
    let mant_shift = 23 - mantissa_bits;
    let out_bits = if exp == 0 {
        if mant == 0 {
            0
        } else {
            // Subnormal: shift the mantissa left until its implied
            // leading one appears, decrementing the unbiased exponent
            // accordingly. The unbiased exponent starts at 1 - 15 for
            // the smallest normal and drops by one per shift.
            let mut m = mant;
            let mut e: i32 = 1 - 15;
            while (m & (1 << mantissa_bits)) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= mant_mask;
            let exp_f32 = (e + 127) as u32;
            (exp_f32 << 23) | (m << mant_shift)
        }
    } else if exp == 0x1f {
        0x7f80_0000 | (mant << mant_shift)
    } else {
        let exp_f32 = (exp as i32 - 15 + 127) as u32;
        (exp_f32 << 23) | (mant << mant_shift)
    };
    f32::from_bits(out_bits)
}

/// Decode a tightly-packed surface of one of the floating-point
/// formats (`R16_FLOAT`, `R16G16_FLOAT`, `R16G16B16A16_FLOAT`,
/// `R32_FLOAT`, `R32G32_FLOAT`, `R32G32B32A32_FLOAT`) into a flat,
/// interleaved `Vec<f32>` of `width × height × channel_count` samples.
///
/// `data` must be at least `width × height × bytes_per_pixel` bytes.
/// Samples are emitted in row-major order; within a pixel the channels
/// are emitted in stored order (R, then G, then B, then A — exactly as
/// they appear on disk).
///
/// Returns [`DdsError::Unsupported`] if `pix` is not one of the
/// floating-point layouts, and [`DdsError::InvalidData`] if `data` is
/// shorter than the format requires.
pub fn decode_float_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<f32>> {
    let (channels, half) = match pix {
        DdsPixelFormat::R16Float => (1u32, true),
        DdsPixelFormat::R16G16Float => (2, true),
        DdsPixelFormat::R16G16B16A16Float => (4, true),
        DdsPixelFormat::R32Float => (1, false),
        DdsPixelFormat::R32G32Float => (2, false),
        DdsPixelFormat::R32G32B32A32Float => (4, false),
        _ => {
            return Err(DdsError::unsupported(format!(
                "decode_float_surface: {} is not a floating-point format",
                pix.name()
            )))
        }
    };

    let sample_bytes = if half { 2usize } else { 4 };
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_float_surface: dimension overflow"))?;
    let total_samples = px
        .checked_mul(channels as usize)
        .ok_or_else(|| DdsError::invalid("decode_float_surface: sample-count overflow"))?;
    let need = total_samples
        .checked_mul(sample_bytes)
        .ok_or_else(|| DdsError::invalid("decode_float_surface: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_float_surface: {} needs {need} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }

    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..total_samples {
        let v = if half {
            half_to_f32(read_u16_le(data, off))
        } else {
            read_f32_le(data, off)
        };
        out.push(v);
        off += sample_bytes;
    }
    Ok(out)
}

/// Decode a tightly-packed `R16G16B16A16_UNORM` surface into a flat,
/// interleaved `Vec<u16>` of `width × height × 4` stored samples (R, G,
/// B, A per pixel, row-major).
///
/// The values are the raw stored unsigned-normalised integers; the
/// crate does not scale them onto `[0, 1]` (see the crate-level docs
/// for the documentation gap on the normalisation arithmetic).
pub fn decode_rgba16_unorm_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_rgba16_raw(DdsPixelFormat::R16G16B16A16Unorm, width, height, data)
}

/// Decode a tightly-packed `R16G16B16A16_SNORM` surface into a flat,
/// interleaved `Vec<i16>` of `width × height × 4` stored samples (R, G,
/// B, A per pixel, row-major).
///
/// The values are the raw stored signed-normalised integers; the crate
/// does not scale them onto `[-1, 1]` (see the crate-level docs for the
/// documentation gap on the normalisation arithmetic).
pub fn decode_rgba16_snorm_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<i16>> {
    let raw = decode_rgba16_raw(DdsPixelFormat::R16G16B16A16Snorm, width, height, data)?;
    Ok(raw.into_iter().map(|u| u as i16).collect())
}

fn decode_rgba16_raw(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u16>> {
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_rgba16: dimension overflow"))?;
    let total_samples = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_rgba16: sample-count overflow"))?;
    let need = total_samples
        .checked_mul(2)
        .ok_or_else(|| DdsError::invalid("decode_rgba16: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_rgba16: {} needs {need} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..total_samples {
        out.push(read_u16_le(data, off));
        off += 2;
    }
    Ok(out)
}

/// Channels carried by a 16-bit plain-integer format.
fn uint16_channels(pix: DdsPixelFormat) -> Option<u32> {
    Some(match pix {
        DdsPixelFormat::R16Uint | DdsPixelFormat::R16Sint => 1,
        DdsPixelFormat::R16G16Uint | DdsPixelFormat::R16G16Sint => 2,
        DdsPixelFormat::R16G16B16A16Uint | DdsPixelFormat::R16G16B16A16Sint => 4,
        _ => return None,
    })
}

/// Read every tightly-packed little-endian `u16` sample of a 16-bit
/// integer surface (1, 2 or 4 channels per pixel) into a flat,
/// interleaved, row-major buffer. Shared by the UINT and SINT paths;
/// the SINT wrapper reinterprets the words as `i16`.
fn decode_uint16_raw(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u16>> {
    let channels = uint16_channels(pix).ok_or_else(|| {
        DdsError::unsupported(format!(
            "decode_uint16_surface: {} is not a 16-bit integer format",
            pix.name()
        ))
    })? as usize;
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_uint16: dimension overflow"))?;
    let total_samples = px
        .checked_mul(channels)
        .ok_or_else(|| DdsError::invalid("decode_uint16: sample-count overflow"))?;
    let need = total_samples
        .checked_mul(2)
        .ok_or_else(|| DdsError::invalid("decode_uint16: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_uint16: {} needs {need} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..total_samples {
        out.push(read_u16_le(data, off));
        off += 2;
    }
    Ok(out)
}

/// Decode a tightly-packed 16-bit **unsigned-integer** surface
/// (`R16_UINT`, `R16G16_UINT`, or `R16G16B16A16_UINT`) into a flat,
/// interleaved `Vec<u16>` of `width × height × channels` stored samples,
/// row-major, channels in the named order (R, then G, then B, A).
///
/// These are plain little-endian integers — there is no `[0, 1]`
/// normalisation, so the decoded words ARE the values. Returns
/// [`DdsError::Unsupported`] for a non-`_UINT` format and
/// [`DdsError::InvalidData`] when `data` is shorter than
/// `width × height × channels × 2` bytes.
pub fn decode_uint16_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u16>> {
    match pix {
        DdsPixelFormat::R16Uint | DdsPixelFormat::R16G16Uint | DdsPixelFormat::R16G16B16A16Uint => {
            decode_uint16_raw(pix, width, height, data)
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_uint16_surface: {} is not a 16-bit unsigned-integer format",
            pix.name()
        ))),
    }
}

/// Decode a tightly-packed 16-bit **signed-integer** surface
/// (`R16_SINT`, `R16G16_SINT`, or `R16G16B16A16_SINT`) into a flat,
/// interleaved `Vec<i16>` of `width × height × channels` stored samples,
/// row-major, channels in the named order (R, then G, then B, A).
///
/// The stored two's-complement words are returned verbatim (no
/// normalisation onto `[-1, 1]`). Returns [`DdsError::Unsupported`] for a
/// non-`_SINT` format and [`DdsError::InvalidData`] when `data` is
/// shorter than `width × height × channels × 2` bytes.
pub fn decode_sint16_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<i16>> {
    match pix {
        DdsPixelFormat::R16Sint | DdsPixelFormat::R16G16Sint | DdsPixelFormat::R16G16B16A16Sint => {
            let raw = decode_uint16_raw(pix, width, height, data)?;
            Ok(raw.into_iter().map(|u| u as i16).collect())
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_sint16_surface: {} is not a 16-bit signed-integer format",
            pix.name()
        ))),
    }
}

/// Channels carried by an 8-bit plain-integer format.
fn uint8_channels(pix: DdsPixelFormat) -> Option<u32> {
    Some(match pix {
        DdsPixelFormat::R8Uint | DdsPixelFormat::R8Sint => 1,
        DdsPixelFormat::R8G8Uint | DdsPixelFormat::R8G8Sint => 2,
        DdsPixelFormat::R8G8B8A8Uint | DdsPixelFormat::R8G8B8A8Sint => 4,
        _ => return None,
    })
}

/// Read every tightly-packed `u8` sample of an 8-bit integer surface
/// (1, 2 or 4 channels per pixel) into a flat, interleaved, row-major
/// buffer. Shared by the UINT and SINT paths; the SINT wrapper
/// reinterprets the bytes as `i8`.
fn decode_uint8_raw(pix: DdsPixelFormat, width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    let channels = uint8_channels(pix).ok_or_else(|| {
        DdsError::unsupported(format!(
            "decode_uint8_surface: {} is not an 8-bit integer format",
            pix.name()
        ))
    })? as usize;
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_uint8: dimension overflow"))?;
    let total_samples = px
        .checked_mul(channels)
        .ok_or_else(|| DdsError::invalid("decode_uint8: sample-count overflow"))?;
    // Each sample is exactly one byte, so the byte count equals the
    // sample count.
    if data.len() < total_samples {
        return Err(DdsError::invalid(format!(
            "decode_uint8: {} needs {total_samples} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }
    Ok(data[..total_samples].to_vec())
}

/// Decode a tightly-packed 8-bit **unsigned-integer** surface
/// (`R8_UINT`, `R8G8_UINT`, or `R8G8B8A8_UINT`) into a flat, interleaved
/// `Vec<u8>` of `width × height × channels` stored samples, row-major,
/// channels in the named order (R, then G, then B, A).
///
/// These are plain bytes — there is no `[0, 1]` normalisation, so the
/// decoded bytes ARE the values. Returns [`DdsError::Unsupported`] for a
/// non-`_UINT` format and [`DdsError::InvalidData`] when `data` is shorter
/// than `width × height × channels` bytes.
pub fn decode_uint8_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>> {
    match pix {
        DdsPixelFormat::R8Uint | DdsPixelFormat::R8G8Uint | DdsPixelFormat::R8G8B8A8Uint => {
            decode_uint8_raw(pix, width, height, data)
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_uint8_surface: {} is not an 8-bit unsigned-integer format",
            pix.name()
        ))),
    }
}

/// Decode a tightly-packed 8-bit **signed-integer** surface (`R8_SINT`,
/// `R8G8_SINT`, or `R8G8B8A8_SINT`) into a flat, interleaved `Vec<i8>` of
/// `width × height × channels` stored samples, row-major, channels in the
/// named order (R, then G, then B, A).
///
/// The stored two's-complement bytes are returned verbatim (no
/// normalisation onto `[-1, 1]`). Returns [`DdsError::Unsupported`] for a
/// non-`_SINT` format and [`DdsError::InvalidData`] when `data` is shorter
/// than `width × height × channels` bytes.
pub fn decode_sint8_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<i8>> {
    match pix {
        DdsPixelFormat::R8Sint | DdsPixelFormat::R8G8Sint | DdsPixelFormat::R8G8B8A8Sint => {
            let raw = decode_uint8_raw(pix, width, height, data)?;
            Ok(raw.into_iter().map(|u| u as i8).collect())
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_sint8_surface: {} is not an 8-bit signed-integer format",
            pix.name()
        ))),
    }
}

/// Channels carried by a 32-bit plain-integer format.
fn uint32_channels(pix: DdsPixelFormat) -> Option<u32> {
    Some(match pix {
        DdsPixelFormat::R32Uint | DdsPixelFormat::R32Sint => 1,
        DdsPixelFormat::R32G32Uint | DdsPixelFormat::R32G32Sint => 2,
        DdsPixelFormat::R32G32B32Uint | DdsPixelFormat::R32G32B32Sint => 3,
        DdsPixelFormat::R32G32B32A32Uint | DdsPixelFormat::R32G32B32A32Sint => 4,
        _ => return None,
    })
}

/// Read every tightly-packed little-endian `u32` sample of a 32-bit
/// integer surface (1, 2, 3 or 4 channels per pixel) into a flat,
/// interleaved, row-major buffer. Shared by the UINT and SINT paths; the
/// SINT wrapper reinterprets the words as `i32`.
fn decode_uint32_raw(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u32>> {
    let channels = uint32_channels(pix).ok_or_else(|| {
        DdsError::unsupported(format!(
            "decode_uint32_surface: {} is not a 32-bit integer format",
            pix.name()
        ))
    })? as usize;
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_uint32: dimension overflow"))?;
    let total_samples = px
        .checked_mul(channels)
        .ok_or_else(|| DdsError::invalid("decode_uint32: sample-count overflow"))?;
    let need = total_samples
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_uint32: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_uint32: {} needs {need} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..total_samples {
        out.push(read_u32_le(data, off));
        off += 4;
    }
    Ok(out)
}

/// Decode a tightly-packed 32-bit **unsigned-integer** surface (`R32_UINT`,
/// `R32G32_UINT`, `R32G32B32_UINT`, or `R32G32B32A32_UINT`) into a flat,
/// interleaved `Vec<u32>` of `width × height × channels` stored samples,
/// row-major, channels in the named order (R, then G, then B, A).
///
/// These are plain little-endian integers — there is no `[0, 1]`
/// normalisation, so the decoded words ARE the values. Returns
/// [`DdsError::Unsupported`] for a non-`_UINT` format and
/// [`DdsError::InvalidData`] when `data` is shorter than
/// `width × height × channels × 4` bytes.
pub fn decode_uint32_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u32>> {
    match pix {
        DdsPixelFormat::R32Uint
        | DdsPixelFormat::R32G32Uint
        | DdsPixelFormat::R32G32B32Uint
        | DdsPixelFormat::R32G32B32A32Uint => decode_uint32_raw(pix, width, height, data),
        _ => Err(DdsError::unsupported(format!(
            "decode_uint32_surface: {} is not a 32-bit unsigned-integer format",
            pix.name()
        ))),
    }
}

/// Decode a tightly-packed 32-bit **signed-integer** surface (`R32_SINT`,
/// `R32G32_SINT`, `R32G32B32_SINT`, or `R32G32B32A32_SINT`) into a flat,
/// interleaved `Vec<i32>` of `width × height × channels` stored samples,
/// row-major, channels in the named order (R, then G, then B, A).
///
/// The stored two's-complement words are returned verbatim (no
/// normalisation onto `[-1, 1]`). Returns [`DdsError::Unsupported`] for a
/// non-`_SINT` format and [`DdsError::InvalidData`] when `data` is shorter
/// than `width × height × channels × 4` bytes.
pub fn decode_sint32_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<i32>> {
    match pix {
        DdsPixelFormat::R32Sint
        | DdsPixelFormat::R32G32Sint
        | DdsPixelFormat::R32G32B32Sint
        | DdsPixelFormat::R32G32B32A32Sint => {
            let raw = decode_uint32_raw(pix, width, height, data)?;
            Ok(raw.into_iter().map(|u| u as i32).collect())
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_sint32_surface: {} is not a 32-bit signed-integer format",
            pix.name()
        ))),
    }
}

/// Channels and per-channel bit width for a normalised single- / dual-
/// channel layout. `None` for any other format.
fn norm_layout(pix: DdsPixelFormat) -> Option<(usize, u32)> {
    Some(match pix {
        // (channels, bits-per-channel)
        DdsPixelFormat::R8Unorm | DdsPixelFormat::L8 | DdsPixelFormat::R8Snorm => (1, 8),
        DdsPixelFormat::R8G8Snorm => (2, 8),
        DdsPixelFormat::R8G8B8A8Snorm => (4, 8),
        DdsPixelFormat::R16Unorm | DdsPixelFormat::R16Snorm => (1, 16),
        DdsPixelFormat::R16G16Unorm | DdsPixelFormat::R16G16Snorm => (2, 16),
        _ => return None,
    })
}

/// Read one little-endian unsigned channel sample of `bits` width (8 or
/// 16) from `data` at byte offset `off`.
fn read_norm_sample(data: &[u8], off: usize, bits: u32) -> u32 {
    if bits == 8 {
        data[off] as u32
    } else {
        read_u16_le(data, off) as u32
    }
}

/// Read every tightly-packed normalised sample of a 1/2/4-channel 8- or
/// 16-bit surface and convert each to `f32`. `signed == false` applies the
/// UNORM rule (`v / (2^bits − 1)`, range `[0, 1]`); `signed == true`
/// applies the SNORM rule (two's-complement, `v / (2^(bits−1) − 1)`,
/// clamped so both the minimum and second-minimum encodings map to
/// `-1.0`, range `[-1, 1]`).
fn decode_norm_raw(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
    signed: bool,
    what: &str,
) -> Result<Vec<f32>> {
    let (channels, bits) = norm_layout(pix).ok_or_else(|| {
        DdsError::unsupported(format!(
            "{what}: {} is not a normalised 8/16-bit format",
            pix.name()
        ))
    })?;
    let bytes_per_sample = (bits / 8) as usize;
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))?;
    let total_samples = px
        .checked_mul(channels)
        .ok_or_else(|| DdsError::invalid(format!("{what}: sample-count overflow")))?;
    let need = total_samples
        .checked_mul(bytes_per_sample)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: {} needs {need} bytes for {width}x{height}, have {}",
            pix.name(),
            data.len()
        )));
    }
    // UNORM divisor 2^bits − 1; SNORM divisor 2^(bits−1) − 1.
    let unorm_div = ((1u64 << bits) - 1) as f32;
    let snorm_div = ((1u64 << (bits - 1)) - 1) as f32;
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..total_samples {
        let raw = read_norm_sample(data, off, bits);
        off += bytes_per_sample;
        let v = if signed {
            // Sign-extend the `bits`-wide two's-complement value to i32.
            let shift = 32 - bits;
            let signed_val = ((raw << shift) as i32) >> shift;
            (signed_val as f32 / snorm_div).max(-1.0)
        } else {
            raw as f32 / unorm_div
        };
        out.push(v);
    }
    Ok(out)
}

/// Decode a normalised **unsigned** single- / dual-channel surface
/// (`R8_UNORM`, `R16_UNORM`, or `R16G16_UNORM`) into a flat, interleaved
/// `Vec<f32>` of `width × height × channels` samples, row-major, channels
/// in the named order.
///
/// Each stored unsigned integer is mapped onto `[0, 1]` by dividing by
/// `2^bits − 1` (all-zero → `0.0`, all-one → `1.0`), the DXGI UNORM rule.
/// `R8_UNORM` shares its byte layout with the legacy `L8` luminance
/// format, so [`DdsPixelFormat::L8`] is accepted here too. Returns
/// [`DdsError::Unsupported`] for a non-UNORM format and
/// [`DdsError::InvalidData`] when `data` is shorter than the surface needs.
pub fn decode_unorm_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<f32>> {
    match pix {
        DdsPixelFormat::R8Unorm
        | DdsPixelFormat::L8
        | DdsPixelFormat::R16Unorm
        | DdsPixelFormat::R16G16Unorm => {
            decode_norm_raw(pix, width, height, data, false, "decode_unorm_surface")
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_unorm_surface: {} is not a normalised unsigned 8/16-bit format",
            pix.name()
        ))),
    }
}

/// Decode a normalised **signed** single- / dual- / four-channel surface
/// (`R8_SNORM`, `R8G8_SNORM`, `R8G8B8A8_SNORM`, `R16_SNORM`, or
/// `R16G16_SNORM`) into a flat, interleaved `Vec<f32>` of
/// `width × height × channels` samples, row-major, channels in the named
/// order.
///
/// Each stored two's-complement integer is mapped onto `[-1, 1]` by
/// dividing by `2^(bits−1) − 1`, with the result clamped at `-1.0` so that
/// both the minimum (`-2^(bits−1)`) and second-minimum (`-2^(bits−1) + 1`)
/// encodings map to `-1.0` — the DXGI SNORM rule. Returns
/// [`DdsError::Unsupported`] for a non-SNORM format and
/// [`DdsError::InvalidData`] when `data` is shorter than the surface needs.
pub fn decode_snorm_surface(
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<f32>> {
    match pix {
        DdsPixelFormat::R8Snorm
        | DdsPixelFormat::R8G8Snorm
        | DdsPixelFormat::R8G8B8A8Snorm
        | DdsPixelFormat::R16Snorm
        | DdsPixelFormat::R16G16Snorm => {
            decode_norm_raw(pix, width, height, data, true, "decode_snorm_surface")
        }
        _ => Err(DdsError::unsupported(format!(
            "decode_snorm_surface: {} is not a normalised signed 8/16-bit format",
            pix.name()
        ))),
    }
}

/// Decode a tightly-packed `R11G11B10_FLOAT` surface into a flat,
/// interleaved `Vec<f32>` of `width × height × 3` samples (R, G, B per
/// pixel, row-major).
///
/// Each pixel is one little-endian 32-bit word; R occupies bits 0..=10,
/// G bits 11..=21, and B bits 22..=31. Each channel is an unsigned
/// partial-precision float (5-bit biased-by-15 exponent, no sign bit;
/// 6-bit mantissa for R and G, 5-bit mantissa for B) widened to `f32`
/// by [`packed_float_to_f32`].
///
/// `data` must be at least `width × height × 4` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_r11g11b10_float_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<f32>> {
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_r11g11b10_float: dimension overflow"))?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_r11g11b10_float: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_r11g11b10_float: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let total_samples = px
        .checked_mul(3)
        .ok_or_else(|| DdsError::invalid("decode_r11g11b10_float: sample-count overflow"))?;
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        let r = word & 0x7ff;
        let g = (word >> 11) & 0x7ff;
        let b = (word >> 22) & 0x3ff;
        out.push(packed_float_to_f32(r, 6));
        out.push(packed_float_to_f32(g, 6));
        out.push(packed_float_to_f32(b, 5));
        off += 4;
    }
    Ok(out)
}

/// Decode a tightly-packed `R9G9B9E5_SHAREDEXP` surface into a flat,
/// interleaved `Vec<f32>` of `width × height × 3` samples (R, G, B per
/// pixel, row-major).
///
/// Each pixel is one little-endian 32-bit word. The three channels share
/// a single 5-bit biased-by-15 exponent (bits 27..=31) and each carries
/// its own 9-bit mantissa: R in bits 0..=8, G in bits 9..=17, and B in
/// bits 18..=26. There is no sign bit and there is no implied leading
/// one on the mantissa (per the format reference's footnote 6), so each
/// channel reconstructs to
///
/// ```text
/// value = mantissa × 2^(exp − 15 − 9) = mantissa × 2^(exp − 24)
/// ```
///
/// which is a single linear expression that covers every exponent value
/// — there is no separate normal / subnormal split, and the all-zero
/// word decodes to `+0`. The result is always non-negative.
///
/// `data` must be at least `width × height × 4` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_r9g9b9e5_sharedexp_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<f32>> {
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_r9g9b9e5_sharedexp: dimension overflow"))?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_r9g9b9e5_sharedexp: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_r9g9b9e5_sharedexp: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let total_samples = px
        .checked_mul(3)
        .ok_or_else(|| DdsError::invalid("decode_r9g9b9e5_sharedexp: sample-count overflow"))?;
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        let r = (word & 0x1ff) as f32;
        let g = ((word >> 9) & 0x1ff) as f32;
        let b = ((word >> 18) & 0x1ff) as f32;
        let exp = ((word >> 27) & 0x1f) as i32;
        // value = mantissa × 2^(exp − 24); a single scale factor applies
        // to all three channels because they share the exponent.
        let scale = 2.0f32.powi(exp - 24);
        out.push(r * scale);
        out.push(g * scale);
        out.push(b * scale);
        off += 4;
    }
    Ok(out)
}

/// Decode a tightly-packed `R10G10B10A2_UNORM` surface into a flat,
/// interleaved `Vec<u16>` of `width × height × 4` stored samples (R, G,
/// B, A per pixel, row-major).
///
/// Each pixel is one little-endian 32-bit word: R occupies bits 0..=9,
/// G bits 10..=19, B bits 20..=29, and A bits 30..=31. The returned
/// values are the raw stored unsigned-normalised integers — R / G / B in
/// `0..=1023` and A in `0..=3`. The crate does not scale them onto a real
/// range; the caller divides colour channels by `1023` and alpha by `3`
/// to obtain the `[0, 1]` normalised values.
///
/// `data` must be at least `width × height × 4` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_r10g10b10a2_unorm_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_r10g10b10a2_raw(width, height, data, "decode_r10g10b10a2_unorm")
}

/// Decode a tightly-packed `R10G10B10A2_UINT` surface into a flat,
/// interleaved `Vec<u16>` of `width × height × 4` stored samples (R, G,
/// B, A per pixel, row-major).
///
/// The bit packing is identical to [`decode_r10g10b10a2_unorm_surface`]
/// (`DXGI_FORMAT` value 24): R occupies bits 0..=9, G bits 10..=19, B
/// bits 20..=29, and A bits 30..=31 of one little-endian 32-bit word.
/// The difference is purely semantic — the format reference describes
/// value 25 as a four-component, 32-bit *unsigned-integer* format, so
/// the returned values (R / G / B in `0..=1023`, A in `0..=3`) are the
/// integers themselves, with no `[0, 1]` normalisation step. The format
/// has no legacy `D3DFMT` four-cc; it is DX10-header only.
///
/// `data` must be at least `width × height × 4` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_r10g10b10a2_uint_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    decode_r10g10b10a2_raw(width, height, data, "decode_r10g10b10a2_uint")
}

/// Shared bit-extraction for the two `R10G10B10A2` packed layouts. The
/// UNORM (value 24) and UINT (value 25) variants share the exact same
/// little-endian 32-bit word packing — R in bits 0..=9, G in 10..=19, B
/// in 20..=29, A in 30..=31 — and differ only in how the caller is
/// expected to interpret the returned integers, so both return the raw
/// stored channels here.
fn decode_r10g10b10a2_raw(width: u32, height: u32, data: &[u8], what: &str) -> Result<Vec<u16>> {
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let total_samples = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: sample-count overflow")))?;
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        let r = (word & 0x3ff) as u16;
        let g = ((word >> 10) & 0x3ff) as u16;
        let b = ((word >> 20) & 0x3ff) as u16;
        let a = ((word >> 30) & 0x3) as u16;
        out.push(r);
        out.push(g);
        out.push(b);
        out.push(a);
        off += 4;
    }
    Ok(out)
}

/// Decode a tightly-packed legacy `D3DFMT_A2R10G10B10` surface into a
/// flat, interleaved `Vec<u16>` of `width × height × 4` stored samples
/// (R, G, B, A per pixel, row-major).
///
/// This is the BGR-ordered sibling of
/// [`decode_r10g10b10a2_unorm_surface`]: the channels are packed in the
/// reverse order inside each little-endian 32-bit word. Per Microsoft's
/// "Common DDS File Resource Formats" table the masks are
/// R = `0x3ff00000`, G = `0x000ffc00`, B = `0x000003ff`, A =
/// `0xc0000000`, so the red channel occupies the *most*-significant 10
/// colour bits (bits 20..=29), green bits 10..=19, blue the least
/// significant (bits 0..=9), and alpha bits 30..=31. The returned values
/// are the raw stored unsigned-normalised integers — R / G / B in
/// `0..=1023` and A in `0..=3`; the caller divides colour channels by
/// `1023` and alpha by `3` to obtain the `[0, 1]` normalised values.
///
/// `data` must be at least `width × height × 4` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_a2r10g10b10_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u16>> {
    let what = "decode_a2r10g10b10";
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let total_samples = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: sample-count overflow")))?;
    let mut out = Vec::with_capacity(total_samples);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        let b = (word & 0x3ff) as u16;
        let g = ((word >> 10) & 0x3ff) as u16;
        let r = ((word >> 20) & 0x3ff) as u16;
        let a = ((word >> 30) & 0x3) as u16;
        out.push(r);
        out.push(g);
        out.push(b);
        out.push(a);
        off += 4;
    }
    Ok(out)
}

/// Decode a tightly-packed legacy `D3DFMT_A8R3G3B2` surface into a flat,
/// interleaved RGBA8 `Vec<u8>` of `width × height × 4` bytes (R, G, B, A
/// per pixel, row-major).
///
/// Each pixel is one little-endian 16-bit word: the low byte holds 3:3:2
/// RGB (red bits 5..=7, green bits 2..=4, blue bits 0..=1) and the high
/// byte an 8-bit alpha, per Microsoft's "Common DDS File Resource
/// Formats" table (R=`0x00e0`, G=`0x001c`, B=`0x0003`, A=`0xff00`). The
/// 3-bit and 2-bit colour channels are widened to 8 bits by the standard
/// bit-replication rule (the channel's high bits repeated into the low
/// bits) so that an all-ones field maps to `0xff`.
///
/// `data` must be at least `width × height × 2` bytes. Returns
/// [`DdsError::InvalidData`] if it is shorter.
pub fn decode_a8r3g3b2_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    let what = "decode_a8r3g3b2";
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))?;
    let need = px
        .checked_mul(2)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let out_len = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: sample-count overflow")))?;
    let mut out = Vec::with_capacity(out_len);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u16_le(data, off);
        let r3 = ((word >> 5) & 0x7) as u8;
        let g3 = ((word >> 2) & 0x7) as u8;
        let b2 = (word & 0x3) as u8;
        let a = ((word >> 8) & 0xff) as u8;
        // 3-bit → 8-bit replication: vvv vvv vv (high bits repeated).
        let r = (r3 << 5) | (r3 << 2) | (r3 >> 1);
        let g = (g3 << 5) | (g3 << 2) | (g3 >> 1);
        // 2-bit → 8-bit replication: vv vv vv vv.
        let b = (b2 << 6) | (b2 << 4) | (b2 << 2) | b2;
        out.push(r);
        out.push(g);
        out.push(b);
        out.push(a);
        off += 2;
    }
    Ok(out)
}

/// Decode a tightly-packed `R8G8_B8G8_UNORM` surface (`DXGI_FORMAT`
/// value 68) into a flat, interleaved RGBA8 `Vec<u8>` of
/// `width × height × 4` bytes (R, G, B, A per pixel, row-major;
/// A is forced to `0xff` since the layout carries no alpha channel).
///
/// This is one of the two packed, horizontally sub-sampled RGB layouts
/// in the DXGI format table. Microsoft's `DXGI_FORMAT` reference
/// describes value 68 as a four-component, 32-bit unsigned-normalized
/// format in which "each 32-bit block describes a pair of pixels:
/// `(R8, G8, B8)` and `(R8, G8, B8)` where the `R8`/`B8` values are
/// repeated, and the `G8` values are unique to each pixel" — i.e. the
/// red and blue channels are shared across the horizontal pixel pair
/// and only green is sampled per pixel. The on-disk byte order follows
/// the format name `R8G8_B8G8`: the four bytes of each block are
/// `[R, G0, B, G1]`, so pixel 0 reconstructs to `(R, G0, B)` and pixel
/// 1 to `(R, G1, B)`.
///
/// Width must be even (the layout pairs adjacent pixels); an odd width
/// is rejected with [`DdsError::InvalidData`]. `data` must hold at least
/// `(width / 2) × height × 4` bytes.
pub fn decode_r8g8_b8g8_unorm_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    // Byte 0 carries the shared red, bytes 1/3 the two greens, byte 2
    // the shared blue.
    decode_packed_subsampled_rgb(width, height, data, 0, 1, 3, 2, "decode_r8g8_b8g8_unorm")
}

/// Decode a tightly-packed `G8R8_G8B8_UNORM` surface (`DXGI_FORMAT`
/// value 69) into a flat, interleaved RGBA8 `Vec<u8>` of
/// `width × height × 4` bytes (R, G, B, A per pixel, row-major;
/// A is forced to `0xff`).
///
/// This is the sibling of [`decode_r8g8_b8g8_unorm_surface`] with the
/// channels reordered within each 32-bit block. Microsoft's
/// `DXGI_FORMAT` reference describes value 69 with the same pair-of-
/// pixels reconstruction — `R8`/`B8` repeated, `G8` unique per pixel —
/// but the on-disk byte order follows the format name `G8R8_G8B8`: the
/// four bytes of each block are `[G0, R, G1, B]`, so pixel 0
/// reconstructs to `(R, G0, B)` and pixel 1 to `(R, G1, B)`.
///
/// Width must be even; an odd width is rejected with
/// [`DdsError::InvalidData`]. `data` must hold at least
/// `(width / 2) × height × 4` bytes.
pub fn decode_g8r8_g8b8_unorm_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    // Byte 1 carries the shared red, bytes 0/2 the two greens, byte 3
    // the shared blue.
    decode_packed_subsampled_rgb(width, height, data, 1, 0, 2, 3, "decode_g8r8_g8b8_unorm")
}

/// Shared reconstruction for the two horizontally sub-sampled packed
/// RGB layouts (`DXGI_FORMAT` values 68 / 69). Each 32-bit block on disk
/// encodes a left/right pixel pair that shares one red byte and one blue
/// byte but carries an independent green byte each. The `r_off`,
/// `g0_off`, `g1_off`, and `b_off` arguments give the within-block byte
/// offsets of the shared red, the left green, the right green, and the
/// shared blue respectively, which is the only thing that distinguishes
/// the two layouts. Output is interleaved RGBA8 with the alpha byte set
/// to `0xff`.
#[allow(clippy::too_many_arguments)]
fn decode_packed_subsampled_rgb(
    width: u32,
    height: u32,
    data: &[u8],
    r_off: usize,
    g0_off: usize,
    g1_off: usize,
    b_off: usize,
    what: &str,
) -> Result<Vec<u8>> {
    if width % 2 != 0 {
        return Err(DdsError::invalid(format!(
            "{what}: width must be even (sub-sampled pixel pairs), got {width}"
        )));
    }
    let pairs_per_row = (width / 2) as usize;
    let rows = height as usize;
    let blocks = pairs_per_row
        .checked_mul(rows)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))?;
    let need = blocks
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let px = (width as usize)
        .checked_mul(rows)
        .ok_or_else(|| DdsError::invalid(format!("{what}: pixel-count overflow")))?;
    let total = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: sample-count overflow")))?;
    let mut out = Vec::with_capacity(total);
    let mut off = 0usize;
    for _ in 0..blocks {
        let r = data[off + r_off];
        let g0 = data[off + g0_off];
        let g1 = data[off + g1_off];
        let b = data[off + b_off];
        // Left pixel: (R, G0, B), right pixel: (R, G1, B).
        out.push(r);
        out.push(g0);
        out.push(b);
        out.push(0xff);
        out.push(r);
        out.push(g1);
        out.push(b);
        out.push(0xff);
        off += 4;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r32_float_round_trip_values() {
        // Two pixels (1x2): 1.0 then -2.5, little-endian f32.
        let mut data = Vec::new();
        data.extend_from_slice(&1.0f32.to_le_bytes());
        data.extend_from_slice(&(-2.5f32).to_le_bytes());
        let out = decode_float_surface(DdsPixelFormat::R32Float, 1, 2, &data).unwrap();
        assert_eq!(out, vec![1.0, -2.5]);
    }

    #[test]
    fn r32g32b32a32_float_channel_order() {
        // One pixel, channels R=1, G=2, B=3, A=4 stored in that order.
        let mut data = Vec::new();
        for v in [1.0f32, 2.0, 3.0, 4.0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_float_surface(DdsPixelFormat::R32G32B32A32Float, 1, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn r16_float_uses_half_widening() {
        // Half-float 1.0 = 0x3c00, 0.5 = 0x3800, 0.0 = 0x0000.
        let mut data = Vec::new();
        for h in [0x3c00u16, 0x3800, 0x0000] {
            data.extend_from_slice(&h.to_le_bytes());
        }
        let out = decode_float_surface(DdsPixelFormat::R16Float, 1, 3, &data).unwrap();
        assert_eq!(out, vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn r16g16b16a16_float_four_channels() {
        let mut data = Vec::new();
        // R=1.0, G=0.5, B=0.0, A=1.0 as half-floats.
        for h in [0x3c00u16, 0x3800, 0x0000, 0x3c00] {
            data.extend_from_slice(&h.to_le_bytes());
        }
        let out = decode_float_surface(DdsPixelFormat::R16G16B16A16Float, 1, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 0.5, 0.0, 1.0]);
    }

    #[test]
    fn rgba16_unorm_raw_channels() {
        let mut data = Vec::new();
        for v in [0u16, 0x8000, 0xffff, 0x0001] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_rgba16_unorm_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0, 0x8000, 0xffff, 0x0001]);
    }

    #[test]
    fn rgba16_snorm_sign_interpretation() {
        let mut data = Vec::new();
        // 0x7fff = 32767, 0x8001 = -32767, 0x0000 = 0, 0xffff = -1.
        for v in [0x7fffu16, 0x8001, 0x0000, 0xffff] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_rgba16_snorm_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![32767, -32767, 0, -1]);
    }

    #[test]
    fn truncated_input_is_invalid() {
        let data = [0u8; 3];
        let err = decode_float_surface(DdsPixelFormat::R32Float, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    /// Build the little-endian 32-bit word for an `R11G11B10_FLOAT`
    /// pixel from raw per-channel bit patterns.
    fn pack_r11g11b10(r: u32, g: u32, b: u32) -> [u8; 4] {
        let word = (r & 0x7ff) | ((g & 0x7ff) << 11) | ((b & 0x3ff) << 22);
        word.to_le_bytes()
    }

    #[test]
    fn r11g11b10_one_in_each_channel() {
        // 1.0 = exponent 15 (biased), mantissa 0. R/G use a 6-bit
        // mantissa so 1.0 = 15<<6 = 0x3c0; B uses 5 so 1.0 = 15<<5 = 0x1e0.
        let data = pack_r11g11b10(0x3c0, 0x3c0, 0x1e0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn r11g11b10_zero_is_zero() {
        let data = pack_r11g11b10(0, 0, 0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn r11g11b10_channel_independence() {
        // R = 2.0 (exp 16, mant 0 → 16<<6 = 0x400), G = 0.5 (exp 14,
        // mant 0 → 14<<6 = 0x380), B = 0.0. Confirms each channel is
        // read from its own bit field with the right mantissa width.
        let data = pack_r11g11b10(0x400, 0x380, 0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![2.0, 0.5, 0.0]);
    }

    #[test]
    fn r11g11b10_b_channel_quarter() {
        // B = 0.25 (exp 13, mant 0 → 13<<5 = 0x1a0) with R/G zero.
        let data = pack_r11g11b10(0, 0, 0x1a0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0.0, 0.0, 0.25]);
    }

    #[test]
    fn r11g11b10_subnormal_supported() {
        // R: exponent 0, mantissa 1 (smallest subnormal, 6-bit mantissa).
        // Value = 2^(1-15) * (1/64) = 2^-14 / 64 = 2^-20.
        let data = pack_r11g11b10(1, 0, 0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert_eq!(out[0], 2.0f32.powi(-20));
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 0.0);
    }

    #[test]
    fn r11g11b10_inf_and_nan() {
        // R: exponent 31, mantissa 0 → +inf. G: exponent 31, mantissa
        // non-zero → NaN. B: 0.
        let data = pack_r11g11b10(0x7c0, 0x7c1, 0);
        let out = decode_r11g11b10_float_surface(1, 1, &data).unwrap();
        assert!(out[0].is_infinite() && out[0] > 0.0);
        assert!(out[1].is_nan());
        assert_eq!(out[2], 0.0);
    }

    #[test]
    fn r11g11b10_two_pixels_row_major() {
        let mut data = Vec::new();
        data.extend_from_slice(&pack_r11g11b10(0x3c0, 0, 0)); // R=1
        data.extend_from_slice(&pack_r11g11b10(0, 0x3c0, 0)); // G=1
        let out = decode_r11g11b10_float_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn r11g11b10_truncated_input_is_invalid() {
        let data = [0u8; 3];
        let err = decode_r11g11b10_float_surface(1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    #[test]
    fn non_float_format_is_unsupported() {
        let data = [0u8; 64];
        let err = decode_float_surface(DdsPixelFormat::A8R8G8B8, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    /// Build the little-endian 32-bit word for an `R9G9B9E5_SHAREDEXP`
    /// pixel from raw per-channel mantissa bit patterns and the shared
    /// 5-bit exponent.
    fn pack_r9g9b9e5(r: u32, g: u32, b: u32, e: u32) -> [u8; 4] {
        let word = (r & 0x1ff) | ((g & 0x1ff) << 9) | ((b & 0x1ff) << 18) | ((e & 0x1f) << 27);
        word.to_le_bytes()
    }

    #[test]
    fn r9g9b9e5_one_in_each_channel() {
        // value = mantissa × 2^(exp − 24). With exp = 16 the scale is
        // 2^-8 = 1/256, so mantissa 256 gives 1.0 on every channel; the
        // shared exponent applies to all three at once.
        let data = pack_r9g9b9e5(256, 256, 256, 16);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 1.0, 1.0]);
    }

    #[test]
    fn r9g9b9e5_zero_word_is_zero() {
        let data = pack_r9g9b9e5(0, 0, 0, 0);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn r9g9b9e5_shared_exponent_scales_all_channels() {
        // mantissas 256 / 128 / 64 with exp 16 → 1.0 / 0.5 / 0.25.
        // Confirms one shared exponent multiplies every channel and the
        // per-channel mantissa bit fields are read independently.
        let data = pack_r9g9b9e5(256, 128, 64, 16);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 0.5, 0.25]);
    }

    #[test]
    fn r9g9b9e5_exponent_bump_doubles() {
        // Same mantissas as the unit-channel case but exp 17 → all 2.0.
        let data = pack_r9g9b9e5(256, 256, 256, 17);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![2.0, 2.0, 2.0]);
    }

    #[test]
    fn r9g9b9e5_no_implied_one() {
        // With no implied leading one, mantissa 0 is exactly +0 even when
        // the shared exponent is large; only the mantissa carries
        // magnitude. R = 511 (max 9-bit) at exp 24 = 511 × 2^0 = 511.
        let data = pack_r9g9b9e5(511, 0, 0, 24);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![511.0, 0.0, 0.0]);
    }

    #[test]
    fn r9g9b9e5_smallest_denorm() {
        // mantissa 1, exp 0 → 1 × 2^-24, the smallest representable
        // non-zero value (denormals are supported per footnote 7, so it
        // is not flushed to zero).
        let data = pack_r9g9b9e5(1, 0, 0, 0);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out[0], 2.0f32.powi(-24));
        assert_eq!(out[1], 0.0);
        assert_eq!(out[2], 0.0);
    }

    #[test]
    fn r9g9b9e5_max_value() {
        // mantissa 511, exp 31 → 511 × 2^7 = 65408 on each channel.
        let data = pack_r9g9b9e5(511, 511, 511, 31);
        let out = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![65408.0, 65408.0, 65408.0]);
    }

    #[test]
    fn r9g9b9e5_two_pixels_row_major() {
        let mut data = Vec::new();
        data.extend_from_slice(&pack_r9g9b9e5(256, 0, 0, 16)); // R=1
        data.extend_from_slice(&pack_r9g9b9e5(0, 256, 0, 16)); // G=1
        let out = decode_r9g9b9e5_sharedexp_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![1.0, 0.0, 0.0, 0.0, 1.0, 0.0]);
    }

    #[test]
    fn r9g9b9e5_truncated_input_is_invalid() {
        let data = [0u8; 3];
        let err = decode_r9g9b9e5_sharedexp_surface(1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    /// Build the little-endian 32-bit word for an `R10G10B10A2_UNORM`
    /// pixel from raw per-channel integers (R/G/B in 0..=1023, A in 0..=3).
    fn pack_r10g10b10a2(r: u32, g: u32, b: u32, a: u32) -> [u8; 4] {
        let word = (r & 0x3ff) | ((g & 0x3ff) << 10) | ((b & 0x3ff) << 20) | ((a & 0x3) << 30);
        word.to_le_bytes()
    }

    #[test]
    fn r10g10b10a2_channel_order_and_widths() {
        // R=1023 (max), G=512, B=1, A=2 — confirms each channel is read
        // from its own bit field with the right width and the right
        // least-significant-bits-first ordering.
        let data = pack_r10g10b10a2(1023, 512, 1, 2);
        let out = decode_r10g10b10a2_unorm_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 512, 1, 2]);
    }

    #[test]
    fn r10g10b10a2_zero_word_is_zero() {
        let data = pack_r10g10b10a2(0, 0, 0, 0);
        let out = decode_r10g10b10a2_unorm_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn r10g10b10a2_all_ones_word() {
        // 0xffffffff → R=G=B=1023, A=3 (every bit set).
        let data = 0xffff_ffffu32.to_le_bytes();
        let out = decode_r10g10b10a2_unorm_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 1023, 1023, 3]);
    }

    #[test]
    fn r10g10b10a2_two_pixels_row_major() {
        let mut data = Vec::new();
        data.extend_from_slice(&pack_r10g10b10a2(1023, 0, 0, 0)); // R max
        data.extend_from_slice(&pack_r10g10b10a2(0, 0, 0, 3)); // A max
        let out = decode_r10g10b10a2_unorm_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 0, 0, 0, 0, 0, 0, 3]);
    }

    #[test]
    fn r10g10b10a2_truncated_input_is_invalid() {
        let data = [0u8; 3];
        let err = decode_r10g10b10a2_unorm_surface(1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    #[test]
    fn r10g10b10a2_uint_channel_order_and_widths() {
        // R=1023 (max), G=512, B=1, A=2 — same packing as the UNORM
        // variant; confirms each channel reads from its own bit field
        // with the least-significant-bits-first ordering.
        let data = pack_r10g10b10a2(1023, 512, 1, 2);
        let out = decode_r10g10b10a2_uint_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 512, 1, 2]);
    }

    #[test]
    fn r10g10b10a2_uint_zero_word_is_zero() {
        let data = pack_r10g10b10a2(0, 0, 0, 0);
        let out = decode_r10g10b10a2_uint_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0, 0, 0, 0]);
    }

    #[test]
    fn r10g10b10a2_uint_all_ones_word() {
        // 0xffffffff → R=G=B=1023, A=3 (every bit set).
        let data = 0xffff_ffffu32.to_le_bytes();
        let out = decode_r10g10b10a2_uint_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 1023, 1023, 3]);
    }

    #[test]
    fn r10g10b10a2_uint_matches_unorm_bit_extraction() {
        // The two layouts share the exact same packing, so the raw
        // extracted integers are identical for any word.
        let data = pack_r10g10b10a2(700, 300, 42, 1);
        let uint = decode_r10g10b10a2_uint_surface(1, 1, &data).unwrap();
        let unorm = decode_r10g10b10a2_unorm_surface(1, 1, &data).unwrap();
        assert_eq!(uint, unorm);
    }

    #[test]
    fn r10g10b10a2_uint_two_pixels_row_major() {
        let mut data = Vec::new();
        data.extend_from_slice(&pack_r10g10b10a2(1023, 0, 0, 0)); // R max
        data.extend_from_slice(&pack_r10g10b10a2(0, 0, 0, 3)); // A max
        let out = decode_r10g10b10a2_uint_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![1023, 0, 0, 0, 0, 0, 0, 3]);
    }

    #[test]
    fn r10g10b10a2_uint_truncated_input_is_invalid() {
        let data = [0u8; 3];
        let err = decode_r10g10b10a2_uint_surface(1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    #[test]
    fn r8g8_b8g8_reconstructs_pixel_pair() {
        // One 32-bit block, byte order [R, G0, B, G1] = [10, 20, 30, 40].
        // Pixel 0 = (R=10, G=20, B=30), pixel 1 = (R=10, G=40, B=30),
        // alpha forced to 0xff. 2x1 surface.
        let data = [10u8, 20, 30, 40];
        let out = decode_r8g8_b8g8_unorm_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![10, 20, 30, 0xff, 10, 40, 30, 0xff]);
    }

    #[test]
    fn g8r8_g8b8_reconstructs_pixel_pair() {
        // One 32-bit block, byte order [G0, R, G1, B] = [20, 10, 40, 30].
        // Same reconstruction as the R8G8_B8G8 case: pixel 0 = (10,20,30),
        // pixel 1 = (10,40,30).
        let data = [20u8, 10, 40, 30];
        let out = decode_g8r8_g8b8_unorm_surface(2, 1, &data).unwrap();
        assert_eq!(out, vec![10, 20, 30, 0xff, 10, 40, 30, 0xff]);
    }

    #[test]
    fn subsampled_two_rows_are_row_major() {
        // 2x2 R8G8_B8G8: row 0 block [1,2,3,4], row 1 block [5,6,7,8].
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let out = decode_r8g8_b8g8_unorm_surface(2, 2, &data).unwrap();
        assert_eq!(
            out,
            vec![
                1, 2, 3, 0xff, 1, 4, 3, 0xff, // row 0
                5, 6, 7, 0xff, 5, 8, 7, 0xff, // row 1
            ]
        );
    }

    #[test]
    fn subsampled_odd_width_rejected() {
        let data = [0u8; 4];
        let err = decode_r8g8_b8g8_unorm_surface(1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
        let err = decode_g8r8_g8b8_unorm_surface(3, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    #[test]
    fn subsampled_truncated_input_is_invalid() {
        // 4x1 needs two 4-byte blocks (8 bytes); give only 5.
        let data = [0u8; 5];
        let err = decode_r8g8_b8g8_unorm_surface(4, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    // --- 16-bit integer surfaces ----------------------------------------

    #[test]
    fn r16_uint_single_channel_row_major() {
        // 2x1 R16_UINT: little-endian words 0x0102, 0xfffe.
        let data = [0x02u8, 0x01, 0xfe, 0xff];
        let out = decode_uint16_surface(DdsPixelFormat::R16Uint, 2, 1, &data).unwrap();
        assert_eq!(out, vec![0x0102, 0xfffe]);
    }

    #[test]
    fn r16g16_uint_two_channels() {
        // One pixel, R=0x1234, G=0x5678.
        let data = [0x34u8, 0x12, 0x78, 0x56];
        let out = decode_uint16_surface(DdsPixelFormat::R16G16Uint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![0x1234, 0x5678]);
    }

    #[test]
    fn r16g16b16a16_uint_four_channels_row_major() {
        // Two pixels: (1,2,3,4) then (5,6,7,8) as raw u16.
        let mut data = Vec::new();
        for v in [1u16, 2, 3, 4, 5, 6, 7, 8] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_uint16_surface(DdsPixelFormat::R16G16B16A16Uint, 2, 1, &data).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn r16_sint_sign_interpretation() {
        // 0xffff -> -1, 0x8000 -> i16::MIN, 0x7fff -> i16::MAX.
        let data = [0xffu8, 0xff, 0x00, 0x80, 0xff, 0x7f];
        let out = decode_sint16_surface(DdsPixelFormat::R16Sint, 3, 1, &data).unwrap();
        assert_eq!(out, vec![-1, i16::MIN, i16::MAX]);
    }

    #[test]
    fn r16g16b16a16_sint_four_channels() {
        // One pixel: R=-2, G=2, B=-32768, A=32767.
        let mut data = Vec::new();
        for v in [-2i16, 2, i16::MIN, i16::MAX] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_sint16_surface(DdsPixelFormat::R16G16B16A16Sint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![-2, 2, i16::MIN, i16::MAX]);
    }

    #[test]
    fn uint16_rejects_non_uint_format() {
        let data = [0u8; 8];
        let err = decode_uint16_surface(DdsPixelFormat::R16Sint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
        // A float format is likewise rejected.
        let err = decode_uint16_surface(DdsPixelFormat::R16Float, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn sint16_rejects_non_sint_format() {
        let data = [0u8; 8];
        let err = decode_sint16_surface(DdsPixelFormat::R16Uint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn uint16_truncated_input_is_invalid() {
        // 2x2 R16G16B16A16_UINT needs 4 px * 4 ch * 2 = 32 bytes; give 31.
        let data = [0u8; 31];
        let err = decode_uint16_surface(DdsPixelFormat::R16G16B16A16Uint, 2, 2, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    // --- 8-bit integer surfaces -----------------------------------------

    #[test]
    fn r8_uint_single_channel_row_major() {
        // 3x1 R8_UINT: bytes 1, 200, 255.
        let data = [1u8, 200, 255];
        let out = decode_uint8_surface(DdsPixelFormat::R8Uint, 3, 1, &data).unwrap();
        assert_eq!(out, vec![1, 200, 255]);
    }

    #[test]
    fn r8g8_uint_two_channels() {
        // One pixel, R=0x12, G=0x34.
        let data = [0x12u8, 0x34];
        let out = decode_uint8_surface(DdsPixelFormat::R8G8Uint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![0x12, 0x34]);
    }

    #[test]
    fn r8g8b8a8_uint_four_channels_row_major() {
        // Two pixels: (1,2,3,4) then (5,6,7,8).
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let out = decode_uint8_surface(DdsPixelFormat::R8G8B8A8Uint, 2, 1, &data).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn r8_sint_sign_interpretation() {
        // 0xff -> -1, 0x80 -> i8::MIN, 0x7f -> i8::MAX.
        let data = [0xffu8, 0x80, 0x7f];
        let out = decode_sint8_surface(DdsPixelFormat::R8Sint, 3, 1, &data).unwrap();
        assert_eq!(out, vec![-1, i8::MIN, i8::MAX]);
    }

    #[test]
    fn r8g8b8a8_sint_four_channels() {
        // One pixel: R=-2, G=2, B=-128, A=127.
        let data = [(-2i8) as u8, 2, (i8::MIN) as u8, (i8::MAX) as u8];
        let out = decode_sint8_surface(DdsPixelFormat::R8G8B8A8Sint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![-2, 2, i8::MIN, i8::MAX]);
    }

    #[test]
    fn uint8_rejects_non_uint_format() {
        let data = [0u8; 8];
        let err = decode_uint8_surface(DdsPixelFormat::R8Sint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
        let err = decode_uint8_surface(DdsPixelFormat::R16Uint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn sint8_rejects_non_sint_format() {
        let data = [0u8; 8];
        let err = decode_sint8_surface(DdsPixelFormat::R8Uint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn uint8_truncated_input_is_invalid() {
        // 2x2 R8G8B8A8_UINT needs 4 px * 4 ch = 16 bytes; give 15.
        let data = [0u8; 15];
        let err = decode_uint8_surface(DdsPixelFormat::R8G8B8A8Uint, 2, 2, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }

    // --- 32-bit integer surfaces ----------------------------------------

    #[test]
    fn r32_uint_single_channel_row_major() {
        // 2x1 R32_UINT: little-endian words 0x0102_0304, 0xffff_fffe.
        let mut data = Vec::new();
        for v in [0x0102_0304u32, 0xffff_fffe] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_uint32_surface(DdsPixelFormat::R32Uint, 2, 1, &data).unwrap();
        assert_eq!(out, vec![0x0102_0304, 0xffff_fffe]);
    }

    #[test]
    fn r32g32_uint_two_channels() {
        // One pixel, R=0x1234_5678, G=0x9abc_def0.
        let mut data = Vec::new();
        for v in [0x1234_5678u32, 0x9abc_def0] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_uint32_surface(DdsPixelFormat::R32G32Uint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![0x1234_5678, 0x9abc_def0]);
    }

    #[test]
    fn r32g32b32_uint_three_channels() {
        // One pixel, R=1, G=2, B=3 — confirms the 96-bit (12-byte) stride
        // of the three-component family.
        let mut data = Vec::new();
        for v in [1u32, 2, 3] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_uint32_surface(DdsPixelFormat::R32G32B32Uint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![1, 2, 3]);
    }

    #[test]
    fn r32g32b32a32_uint_four_channels_row_major() {
        // Two pixels: (1,2,3,4) then (5,6,7,8) as raw u32.
        let mut data = Vec::new();
        for v in [1u32, 2, 3, 4, 5, 6, 7, 8] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_uint32_surface(DdsPixelFormat::R32G32B32A32Uint, 2, 1, &data).unwrap();
        assert_eq!(out, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn r32_sint_sign_interpretation() {
        // 0xffff_ffff -> -1, 0x8000_0000 -> i32::MIN, 0x7fff_ffff -> i32::MAX.
        let mut data = Vec::new();
        for v in [0xffff_ffffu32, 0x8000_0000, 0x7fff_ffff] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_sint32_surface(DdsPixelFormat::R32Sint, 3, 1, &data).unwrap();
        assert_eq!(out, vec![-1, i32::MIN, i32::MAX]);
    }

    #[test]
    fn r32g32b32a32_sint_four_channels() {
        // One pixel: R=-2, G=2, B=i32::MIN, A=i32::MAX.
        let mut data = Vec::new();
        for v in [-2i32, 2, i32::MIN, i32::MAX] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_sint32_surface(DdsPixelFormat::R32G32B32A32Sint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![-2, 2, i32::MIN, i32::MAX]);
    }

    #[test]
    fn r32g32b32_sint_three_channels() {
        // One pixel: R=-1, G=0, B=i32::MAX over the 12-byte stride.
        let mut data = Vec::new();
        for v in [-1i32, 0, i32::MAX] {
            data.extend_from_slice(&v.to_le_bytes());
        }
        let out = decode_sint32_surface(DdsPixelFormat::R32G32B32Sint, 1, 1, &data).unwrap();
        assert_eq!(out, vec![-1, 0, i32::MAX]);
    }

    #[test]
    fn uint32_rejects_non_uint_format() {
        let data = [0u8; 16];
        let err = decode_uint32_surface(DdsPixelFormat::R32Sint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
        let err = decode_uint32_surface(DdsPixelFormat::R32Float, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn sint32_rejects_non_sint_format() {
        let data = [0u8; 16];
        let err = decode_sint32_surface(DdsPixelFormat::R32Uint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }

    #[test]
    fn uint32_truncated_input_is_invalid() {
        // 1x1 R32G32B32A32_UINT needs 4 ch * 4 = 16 bytes; give 15.
        let data = [0u8; 15];
        let err = decode_uint32_surface(DdsPixelFormat::R32G32B32A32Uint, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::InvalidData(_)));
    }
}
