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

    #[test]
    fn non_float_format_is_unsupported() {
        let data = [0u8; 64];
        let err = decode_float_surface(DdsPixelFormat::A8R8G8B8, 1, 1, &data).unwrap_err();
        assert!(matches!(err, DdsError::Unsupported(_)));
    }
}
