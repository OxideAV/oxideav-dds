//! Decoders for the extended high-bit-depth / floating-point
//! uncompressed DDS surfaces — the 16-bit-per-channel and 32-bit-float
//! formats Microsoft assigns to the legacy D3DFMT numeric FourCC codes
//! (36, 110..=116) and to the matching `DXGI_FORMAT` values.
//!
//! ## Clean-room provenance
//!
//! The byte layout, channel order, and normalisation rules here come
//! entirely from Microsoft's public "Programming guide for DDS" page
//! (the resource-format table that pairs each D3DFMT numeric FourCC with
//! its `DXGI_FORMAT`) and the public `DXGI_FORMAT` enumeration reference.
//! The DXGI naming convention spells out, per format, the channel order
//! (R-then-G-then-B-then-A as written, least-significant channel first in
//! memory), the per-channel bit width, and the numeric interpretation
//! (`_FLOAT` = IEEE-754, `_UNORM` = `value / max`, `_SNORM` =
//! `value / max_pos` clamped at -1). The binary16 → f32 widening reuses
//! [`crate::bc6h::half_to_f32`], itself written from the IEEE-754
//! binary16 definition. No DirectXTex / D3DX / NVTT / other DDS source
//! was consulted.

use crate::bc6h::half_to_f32;
use crate::error::{DdsError, Result};
use crate::image::DdsPixelFormat;

/// Per-channel on-disk sample kind for the extended uncompressed
/// formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleKind {
    /// IEEE-754 binary16 (2 bytes), widened with [`half_to_f32`].
    Float16,
    /// IEEE-754 binary32 (4 bytes), `f32::from_le_bytes`.
    Float32,
    /// Unsigned 16-bit normalised: `value / 65535.0` in `[0, 1]`.
    Unorm16,
    /// Signed 16-bit normalised: `value / 32767.0` in `[-1, 1]`, with
    /// the reserved `-32768` codepoint clamped to `-1.0`.
    Snorm16,
}

impl SampleKind {
    /// Bytes per stored channel.
    fn channel_bytes(self) -> usize {
        match self {
            Self::Float16 | Self::Unorm16 | Self::Snorm16 => 2,
            Self::Float32 => 4,
        }
    }

    /// Decode one little-endian channel sample to f32.
    fn read(self, bytes: &[u8]) -> f32 {
        match self {
            Self::Float16 => half_to_f32(u16::from_le_bytes([bytes[0], bytes[1]])),
            Self::Float32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            Self::Unorm16 => u16::from_le_bytes([bytes[0], bytes[1]]) as f32 / 65535.0,
            Self::Snorm16 => {
                let v = i16::from_le_bytes([bytes[0], bytes[1]]) as f32;
                (v / 32767.0).max(-1.0)
            }
        }
    }
}

/// Resolve a [`DdsPixelFormat`] to its `(channel count, sample kind)`
/// layout, or `None` if it is not one of the extended uncompressed
/// formats.
fn layout(pix: DdsPixelFormat) -> Option<(usize, SampleKind)> {
    Some(match pix {
        DdsPixelFormat::R16G16B16A16Float => (4, SampleKind::Float16),
        DdsPixelFormat::R16G16B16A16Unorm => (4, SampleKind::Unorm16),
        DdsPixelFormat::R16G16B16A16Snorm => (4, SampleKind::Snorm16),
        DdsPixelFormat::R16G16Float => (2, SampleKind::Float16),
        DdsPixelFormat::R16Float => (1, SampleKind::Float16),
        DdsPixelFormat::R32G32B32A32Float => (4, SampleKind::Float32),
        DdsPixelFormat::R32G32Float => (2, SampleKind::Float32),
        DdsPixelFormat::R32Float => (1, SampleKind::Float32),
        _ => return None,
    })
}

/// Decode an extended high-bit-depth / floating-point uncompressed DDS
/// surface to normalised f32 RGBA.
///
/// `src` is the raw on-disk pixel array for a single surface — the bytes
/// found in `DdsImage::surfaces[i].plane.data` (or `planes[0].data`) when
/// `pixel_format` is one of the `is_hdr_uncompressed` formats. `out`
/// receives `width * height * 4` f32 samples in interleaved RGBA order.
///
/// Channels not stored by the source format take the DDS default: an
/// `R16_FLOAT` / `R32_FLOAT` surface fills G and B with `0.0`, and any
/// format without a stored alpha channel fills A with `1.0`. Float
/// channels pass through verbatim (no clamping); `_UNORM` channels are
/// `value / 65535`; `_SNORM` channels are `value / 32767` clamped at
/// `-1.0`.
///
/// Returns [`DdsError::Unsupported`] if `pix` is not an extended
/// uncompressed format and [`DdsError::InvalidData`] if `src` is shorter
/// than `width * height * bytes_per_pixel` or `out` is not exactly
/// `width * height * 4` long.
pub fn decode_hdr_to_f32(
    src: &[u8],
    pix: DdsPixelFormat,
    width: u32,
    height: u32,
    out: &mut [f32],
) -> Result<()> {
    let (channels, kind) = layout(pix).ok_or_else(|| {
        DdsError::unsupported(format!(
            "decode_hdr_to_f32: {} is not an extended uncompressed format",
            pix.name()
        ))
    })?;

    let cb = kind.channel_bytes();
    let pixel_bytes = channels * cb;
    // saturating so adversarial dimensions reject rather than overflow.
    let pixels = (width as usize).saturating_mul(height as usize);
    let need_in = pixels.saturating_mul(pixel_bytes);
    let need_out = pixels.saturating_mul(4);

    if src.len() < need_in {
        return Err(DdsError::invalid(format!(
            "decode_hdr_to_f32: input is {} bytes, need {} for {}x{} {}",
            src.len(),
            need_in,
            width,
            height,
            pix.name()
        )));
    }
    if out.len() != need_out {
        return Err(DdsError::invalid(format!(
            "decode_hdr_to_f32: output is {} f32s, need {} ({}x{} × RGBA)",
            out.len(),
            need_out,
            width,
            height
        )));
    }

    for p in 0..pixels {
        let in_base = p * pixel_bytes;
        let out_base = p * 4;
        // Default RGBA: stored channels overwrite these.
        let mut rgba = [0.0f32, 0.0, 0.0, 1.0];
        for (c, channel) in rgba.iter_mut().enumerate().take(channels) {
            let off = in_base + c * cb;
            *channel = kind.read(&src[off..off + cb]);
        }
        out[out_base..out_base + 4].copy_from_slice(&rgba);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn r16_float_fills_gb_zero_a_one() {
        // 1x1 R16_FLOAT: stored R = 1.0 (0x3c00), G/B default 0, A default 1.
        let src = 0x3c00u16.to_le_bytes();
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R16Float, 1, 1, &mut out).unwrap();
        assert_eq!(out, [1.0, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn rgba16_float_roundtrips_each_channel() {
        // R=1.0 (0x3c00), G=0.5 (0x3800), B=2.0 (0x4000), A=0.0 (0x0000).
        let mut src = Vec::new();
        for h in [0x3c00u16, 0x3800, 0x4000, 0x0000] {
            src.extend_from_slice(&h.to_le_bytes());
        }
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R16G16B16A16Float, 1, 1, &mut out).unwrap();
        assert_eq!(out, [1.0, 0.5, 2.0, 0.0]);
    }

    #[test]
    fn rgba16_unorm_normalises_by_max() {
        // R=65535 -> 1.0, G=0 -> 0.0, B=32768 -> ~0.5, A=65535 -> 1.0.
        let mut src = Vec::new();
        for v in [65535u16, 0, 32768, 65535] {
            src.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R16G16B16A16Unorm, 1, 1, &mut out).unwrap();
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], 0.0);
        assert!((out[2] - 0.5).abs() < 1e-4);
        assert_eq!(out[3], 1.0);
    }

    #[test]
    fn rgba16_snorm_clamps_reserved_min() {
        // R=32767 -> 1.0, G=-32767 -> -1.0, B=-32768 (reserved) -> -1.0, A=0 -> 0.0.
        let mut src = Vec::new();
        for v in [32767i16, -32767, -32768, 0] {
            src.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R16G16B16A16Snorm, 1, 1, &mut out).unwrap();
        assert_eq!(out[0], 1.0);
        assert_eq!(out[1], -1.0);
        assert_eq!(out[2], -1.0);
        assert_eq!(out[3], 0.0);
    }

    #[test]
    fn r32_float_exact_bits() {
        let value = 12.3456f32;
        let src = value.to_le_bytes();
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R32Float, 1, 1, &mut out).unwrap();
        assert_eq!(out, [value, 0.0, 0.0, 1.0]);
    }

    #[test]
    fn rg32_float_two_channels() {
        let mut src = Vec::new();
        src.extend_from_slice(&1.5f32.to_le_bytes());
        src.extend_from_slice(&(-2.5f32).to_le_bytes());
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R32G32Float, 1, 1, &mut out).unwrap();
        assert_eq!(out, [1.5, -2.5, 0.0, 1.0]);
    }

    #[test]
    fn rgba32_float_full() {
        let mut src = Vec::new();
        for v in [0.1f32, 0.2, 0.3, 0.4] {
            src.extend_from_slice(&v.to_le_bytes());
        }
        let mut out = [0.0f32; 4];
        decode_hdr_to_f32(&src, DdsPixelFormat::R32G32B32A32Float, 1, 1, &mut out).unwrap();
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn short_input_rejected() {
        let src = [0u8; 2];
        let mut out = [0.0f32; 8];
        let e = decode_hdr_to_f32(&src, DdsPixelFormat::R16G16B16A16Float, 1, 2, &mut out);
        assert!(matches!(e, Err(DdsError::InvalidData(_))));
    }

    #[test]
    fn wrong_output_len_rejected() {
        let src = [0u8; 8];
        let mut out = [0.0f32; 3];
        let e = decode_hdr_to_f32(&src, DdsPixelFormat::R16G16B16A16Float, 1, 1, &mut out);
        assert!(matches!(e, Err(DdsError::InvalidData(_))));
    }

    #[test]
    fn non_hdr_format_rejected() {
        let src = [0u8; 4];
        let mut out = [0.0f32; 4];
        let e = decode_hdr_to_f32(&src, DdsPixelFormat::A8R8G8B8, 1, 1, &mut out);
        assert!(matches!(e, Err(DdsError::Unsupported(_))));
    }

    #[test]
    fn adversarial_dimensions_do_not_panic() {
        let src = [0u8; 16];
        let mut out = [0.0f32; 4];
        // u32::MAX dimensions must reject (saturating math), not panic.
        let e = decode_hdr_to_f32(
            &src,
            DdsPixelFormat::R16G16B16A16Float,
            u32::MAX,
            u32::MAX,
            &mut out,
        );
        assert!(matches!(e, Err(DdsError::InvalidData(_))));
    }
}
