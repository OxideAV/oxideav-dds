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
    let px = (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid("decode_r10g10b10a2_unorm: dimension overflow"))?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_r10g10b10a2_unorm: byte-count overflow"))?;
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "decode_r10g10b10a2_unorm: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    let total_samples = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid("decode_r10g10b10a2_unorm: sample-count overflow"))?;
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
}
