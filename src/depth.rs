//! Depth and depth-stencil surface decoders.
//!
//! The four depth / depth-stencil `DXGI_FORMAT` layouts whose byte
//! packing Microsoft fully specifies in the public DXGI format
//! enumeration:
//!
//! | DXGI value | name | packing |
//! | ---------- | ---- | ------- |
//! | 55 | `D16_UNORM` | one `u16` per texel, depth unsigned-normalised onto `[0, 1]` |
//! | 40 | `D32_FLOAT` | one little-endian `f32` per texel, depth verbatim |
//! | 45 | `D24_UNORM_S8_UINT` | one `u32` per texel: low 24 bits unorm depth, high 8 bits `u8` stencil |
//! | 20 | `D32_FLOAT_S8X24_UINT` | two `u32` words per texel: word 0 is the `f32` depth, word 1 holds the `u8` stencil in its low 8 bits (upper 24 bits unused) |
//!
//! The `R16_UNORM` / `R32_FLOAT` packings are byte-identical to the
//! `D16_UNORM` / `D32_FLOAT` ones (a depth surface is just a
//! single-channel colour surface the GPU treats as depth), and the
//! depth-stencil pair has matching typeless "view" formats
//! (`R24G8_TYPELESS` over `D24_UNORM_S8_UINT`, `R32G8X24_TYPELESS`
//! over `D32_FLOAT_S8X24_UINT`) that share the exact same memory.
//!
//! The depth-stencil pair additionally has **single-aspect view**
//! formats that expose only one of the two components over the same
//! memory, leaving the other aspect as typeless padding:
//!
//! | DXGI value | name | view of | exposes |
//! | ---------- | ---- | ------- | ------- |
//! | 46 | `R24_UNORM_X8_TYPELESS` | `D24_UNORM_S8_UINT` | depth only (low 24 bits, unorm onto `[0, 1]`) → `f32` |
//! | 47 | `X24_TYPELESS_G8_UINT` | `D24_UNORM_S8_UINT` | stencil only (high 8 bits) → `u8` |
//! | 21 | `R32_FLOAT_X8X24_TYPELESS` | `D32_FLOAT_S8X24_UINT` | depth only (word 0, verbatim) → `f32` |
//! | 22 | `X32_TYPELESS_G8X24_UINT` | `D32_FLOAT_S8X24_UINT` | stencil only (word 1, low 8 bits) → `u8` |
//!
//! Every decoder returns the **stored** depth value (normalised onto
//! `[0, 1]` for the UNORM depths, the verbatim `f32` for the float
//! depths) and, for the combined formats, the raw `u8` stencil index.
//! No depth-range remapping (near/far planes) is applied — that is a
//! viewport transform, not part of the surface encoding.
//!
//! Reference: Microsoft's public DXGI format enumeration on
//! learn.microsoft.com (`docs/image/dds/mslearn-dxgi-format-enum.html`).

use crate::error::{DdsError, Result};

/// The 24-bit UNORM denominator `2^24 − 1`.
const UNORM24_MAX: f32 = 16_777_215.0;
/// The 16-bit UNORM denominator `2^16 − 1`.
const UNORM16_MAX: f32 = 65_535.0;

#[inline]
fn read_u16_le(buf: &[u8], off: usize) -> u16 {
    u16::from_le_bytes([buf[off], buf[off + 1]])
}

#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> u32 {
    u32::from_le_bytes([buf[off], buf[off + 1], buf[off + 2], buf[off + 3]])
}

#[inline]
fn pixel_count(width: u32, height: u32, what: &str) -> Result<usize> {
    (width as usize)
        .checked_mul(height as usize)
        .ok_or_else(|| DdsError::invalid(format!("{what}: dimension overflow")))
}

#[inline]
fn require_len(data: &[u8], need: usize, width: u32, height: u32, what: &str) -> Result<()> {
    if data.len() < need {
        return Err(DdsError::invalid(format!(
            "{what}: needs {need} bytes for {width}x{height}, have {}",
            data.len()
        )));
    }
    Ok(())
}

/// Decode a `D16_UNORM` depth surface (`DXGI_FORMAT` value 55) into a
/// flat row-major `Vec<f32>` of `width × height` depth values.
///
/// Each texel is one little-endian `u16`; the unsigned-normalised depth
/// is `stored / (2^16 − 1)`, landing on `[0, 1]`. Same on-disk packing
/// as `R16_UNORM`.
///
/// `data` must hold at least `width × height × 2` bytes.
pub fn decode_depth_d16_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<f32>> {
    let what = "decode_depth_d16";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(2)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        out.push(read_u16_le(data, off) as f32 / UNORM16_MAX);
        off += 2;
    }
    Ok(out)
}

/// Decode a `D32_FLOAT` depth surface (`DXGI_FORMAT` value 40) into a
/// flat row-major `Vec<f32>` of `width × height` depth values.
///
/// Each texel is one little-endian IEEE-754 `f32`, returned verbatim —
/// the stored bits *are* the depth. Same on-disk packing as
/// `R32_FLOAT`.
///
/// `data` must hold at least `width × height × 4` bytes.
pub fn decode_depth_d32_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<f32>> {
    let what = "decode_depth_d32";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        out.push(f32::from_bits(read_u32_le(data, off)));
        off += 4;
    }
    Ok(out)
}

/// A decoded depth+stencil texel: the unsigned-normalised / float depth
/// plus the raw `u8` stencil index.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DepthStencil {
    /// Depth value. For `D24_UNORM_S8_UINT` this is the 24-bit
    /// unsigned-normalised depth on `[0, 1]`; for `D32_FLOAT_S8X24_UINT`
    /// it is the verbatim `f32`.
    pub depth: f32,
    /// Stencil index in `0..=255`, taken from the dedicated 8-bit
    /// stencil field.
    pub stencil: u8,
}

/// Decode a `D24_UNORM_S8_UINT` depth+stencil surface (`DXGI_FORMAT`
/// value 45; the typeless view `R24G8_TYPELESS`, value 44, shares the
/// packing) into a flat row-major `Vec<DepthStencil>` of
/// `width × height` texels.
///
/// Each texel is one little-endian `u32`: the low 24 bits are an
/// unsigned-normalised depth (`stored / (2^24 − 1)` onto `[0, 1]`) and
/// the high 8 bits are the `u8` stencil index.
///
/// `data` must hold at least `width × height × 4` bytes.
pub fn decode_depth_d24s8_surface(
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<DepthStencil>> {
    let what = "decode_depth_d24s8";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        let depth = (word & 0x00ff_ffff) as f32 / UNORM24_MAX;
        let stencil = ((word >> 24) & 0xff) as u8;
        out.push(DepthStencil { depth, stencil });
        off += 4;
    }
    Ok(out)
}

/// Decode a `D32_FLOAT_S8X24_UINT` depth+stencil surface (`DXGI_FORMAT`
/// value 20; the typeless view `R32G8X24_TYPELESS`, value 19, shares
/// the packing) into a flat row-major `Vec<DepthStencil>` of
/// `width × height` texels.
///
/// Each texel is **64 bits** — two little-endian `u32` words. The first
/// word is the IEEE-754 `f32` depth (returned verbatim). The second
/// word holds the `u8` stencil index in its low 8 bits; the upper 24
/// bits are unused padding and are ignored.
///
/// `data` must hold at least `width × height × 8` bytes.
pub fn decode_depth_d32s8_surface(
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<DepthStencil>> {
    let what = "decode_depth_d32s8";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(8)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        let depth = f32::from_bits(read_u32_le(data, off));
        let stencil = (read_u32_le(data, off + 4) & 0xff) as u8;
        out.push(DepthStencil { depth, stencil });
        off += 8;
    }
    Ok(out)
}

/// Decode a `R24_UNORM_X8_TYPELESS` depth-only **view** surface
/// (`DXGI_FORMAT` value 46) into a flat row-major `Vec<f32>` of
/// `width × height` depth values.
///
/// This is the depth aspect of `D24_UNORM_S8_UINT` memory: each texel is
/// one little-endian `u32` whose low 24 bits are an unsigned-normalised
/// depth (`stored / (2^24 − 1)` onto `[0, 1]`) and whose high 8 bits are
/// typeless padding (Microsoft: "24 bits red channel and 8 bits
/// unused"). The padding byte is ignored — only the depth is returned.
///
/// `data` must hold at least `width × height × 4` bytes.
pub fn decode_depth_r24_unorm_x8_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<f32>> {
    let what = "decode_depth_r24_unorm_x8";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        out.push((word & 0x00ff_ffff) as f32 / UNORM24_MAX);
        off += 4;
    }
    Ok(out)
}

/// Decode a `X24_TYPELESS_G8_UINT` stencil-only **view** surface
/// (`DXGI_FORMAT` value 47) into a flat row-major `Vec<u8>` of
/// `width × height` stencil indices.
///
/// This is the stencil aspect of `D24_UNORM_S8_UINT` memory: each texel
/// is one little-endian `u32` whose low 24 bits are typeless padding
/// this view ignores and whose high 8 bits are the `u8` stencil index
/// (Microsoft: "24 bits unused and 8 bits green channel").
///
/// `data` must hold at least `width × height × 4` bytes.
pub fn decode_depth_x24_g8_uint_surface(width: u32, height: u32, data: &[u8]) -> Result<Vec<u8>> {
    let what = "decode_depth_x24_g8_uint";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(4)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        let word = read_u32_le(data, off);
        out.push(((word >> 24) & 0xff) as u8);
        off += 4;
    }
    Ok(out)
}

/// Decode a `R32_FLOAT_X8X24_TYPELESS` depth-only **view** surface
/// (`DXGI_FORMAT` value 21) into a flat row-major `Vec<f32>` of
/// `width × height` depth values.
///
/// This is the depth aspect of `D32_FLOAT_S8X24_UINT` memory: each texel
/// is **64 bits** — two little-endian `u32` words. The first word is the
/// IEEE-754 `f32` depth (returned verbatim); the second word (8 bits
/// stencil + 24 bits padding) is typeless to this view and ignored
/// entirely (Microsoft: "32-bit red channel, 8 bits are unused, and 24
/// bits are unused").
///
/// `data` must hold at least `width × height × 8` bytes.
pub fn decode_depth_r32_float_x8x24_surface(
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<f32>> {
    let what = "decode_depth_r32_float_x8x24";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(8)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        out.push(f32::from_bits(read_u32_le(data, off)));
        off += 8;
    }
    Ok(out)
}

/// Decode a `X32_TYPELESS_G8X24_UINT` stencil-only **view** surface
/// (`DXGI_FORMAT` value 22) into a flat row-major `Vec<u8>` of
/// `width × height` stencil indices.
///
/// This is the stencil aspect of `D32_FLOAT_S8X24_UINT` memory: each
/// texel is **64 bits** — two little-endian `u32` words. The first word
/// (32-bit depth) is typeless to this view and ignored; the second word
/// holds the `u8` stencil index in its low 8 bits with the upper 24 bits
/// unused (Microsoft: "32 bits unused, 8 bits for green channel, and 24
/// bits are unused").
///
/// `data` must hold at least `width × height × 8` bytes.
pub fn decode_depth_x32_g8x24_uint_surface(
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<Vec<u8>> {
    let what = "decode_depth_x32_g8x24_uint";
    let px = pixel_count(width, height, what)?;
    let need = px
        .checked_mul(8)
        .ok_or_else(|| DdsError::invalid(format!("{what}: byte-count overflow")))?;
    require_len(data, need, width, height, what)?;

    let mut out = Vec::with_capacity(px);
    let mut off = 0usize;
    for _ in 0..px {
        out.push((read_u32_le(data, off + 4) & 0xff) as u8);
        off += 8;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn d16_normalises_to_unit_range() {
        // Two texels: 0x0000 (0.0) and 0xffff (1.0).
        let data = [0x00, 0x00, 0xff, 0xff];
        let out = decode_depth_d16_surface(2, 1, &data).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], 0.0);
        assert_eq!(out[1], 1.0);
    }

    #[test]
    fn d16_half_value() {
        // 0x8000 = 32768 / 65535.
        let data = [0x00, 0x80];
        let out = decode_depth_d16_surface(1, 1, &data).unwrap();
        assert!((out[0] - 32768.0 / 65535.0).abs() < 1e-7);
    }

    #[test]
    fn d32_float_verbatim() {
        let v = 0.375f32;
        let data = v.to_le_bytes();
        let out = decode_depth_d32_surface(1, 1, &data).unwrap();
        assert_eq!(out[0], 0.375);
    }

    #[test]
    fn d24s8_splits_depth_and_stencil() {
        // word = stencil 0xab in high byte, depth = 0x00_ffff (low 24 bits).
        let word: u32 = (0xab << 24) | 0x0000_ffff;
        let data = word.to_le_bytes();
        let out = decode_depth_d24s8_surface(1, 1, &data).unwrap();
        assert_eq!(out[0].stencil, 0xab);
        assert!((out[0].depth - 0xffff as f32 / UNORM24_MAX).abs() < 1e-7);
    }

    #[test]
    fn d24s8_full_depth_is_one() {
        let word: u32 = (0x12 << 24) | 0x00ff_ffff;
        let data = word.to_le_bytes();
        let out = decode_depth_d24s8_surface(1, 1, &data).unwrap();
        assert_eq!(out[0].depth, 1.0);
        assert_eq!(out[0].stencil, 0x12);
    }

    #[test]
    fn d32s8_splits_depth_and_stencil() {
        let depth = 0.25f32;
        let mut data = Vec::new();
        data.extend_from_slice(&depth.to_le_bytes());
        // stencil 0x7f in low 8 bits; upper 24 bits set to garbage that
        // must be ignored.
        data.extend_from_slice(&(0xdead_ff7fu32).to_le_bytes());
        let out = decode_depth_d32s8_surface(1, 1, &data).unwrap();
        assert_eq!(out[0].depth, 0.25);
        assert_eq!(out[0].stencil, 0x7f);
    }

    #[test]
    fn short_buffers_error_not_panic() {
        assert!(decode_depth_d16_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_d32_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_d24s8_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_d32s8_surface(4, 4, &[0u8; 4]).is_err());
    }

    #[test]
    fn r24_unorm_x8_view_extracts_depth_ignores_padding() {
        // word = stencil/padding 0xab in high byte, depth = 0x00_ffff.
        // The view must IGNORE the high byte entirely.
        let word: u32 = (0xab << 24) | 0x0000_ffff;
        let data = word.to_le_bytes();
        let out = decode_depth_r24_unorm_x8_surface(1, 1, &data).unwrap();
        assert_eq!(out.len(), 1);
        assert!((out[0] - 0xffff as f32 / UNORM24_MAX).abs() < 1e-7);
    }

    #[test]
    fn r24_unorm_x8_full_depth_is_one_regardless_of_padding() {
        let word: u32 = (0xff << 24) | 0x00ff_ffff;
        let data = word.to_le_bytes();
        let out = decode_depth_r24_unorm_x8_surface(1, 1, &data).unwrap();
        assert_eq!(out[0], 1.0);
    }

    #[test]
    fn x24_g8_view_extracts_stencil_ignores_depth() {
        // depth bits set to garbage in the low 24; stencil 0x5c in high.
        let word: u32 = (0x5c << 24) | 0x00ab_cdef;
        let data = word.to_le_bytes();
        let out = decode_depth_x24_g8_uint_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0x5c]);
    }

    #[test]
    fn r24_and_x24_views_agree_with_combined_d24s8() {
        // Two texels through the combined decoder vs the two views.
        let w0: u32 = (0x11 << 24) | 0x0012_3456;
        let w1: u32 = (0xfe << 24) | 0x00ab_cdef;
        let mut data = Vec::new();
        data.extend_from_slice(&w0.to_le_bytes());
        data.extend_from_slice(&w1.to_le_bytes());
        let combined = decode_depth_d24s8_surface(2, 1, &data).unwrap();
        let depth = decode_depth_r24_unorm_x8_surface(2, 1, &data).unwrap();
        let stencil = decode_depth_x24_g8_uint_surface(2, 1, &data).unwrap();
        for i in 0..2 {
            assert_eq!(combined[i].depth, depth[i]);
            assert_eq!(combined[i].stencil, stencil[i]);
        }
    }

    #[test]
    fn r32_float_x8x24_view_extracts_depth_ignores_second_word() {
        let depth = 0.625f32;
        let mut data = Vec::new();
        data.extend_from_slice(&depth.to_le_bytes());
        // second word: stencil + 24 unused bits, all garbage to be ignored.
        data.extend_from_slice(&0xdead_beefu32.to_le_bytes());
        let out = decode_depth_r32_float_x8x24_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0.625]);
    }

    #[test]
    fn x32_g8x24_view_extracts_stencil_ignores_depth_word() {
        // first word (depth) garbage; stencil 0x7f in low 8 bits of word 1,
        // upper 24 bits of word 1 garbage that must be masked off.
        let mut data = Vec::new();
        data.extend_from_slice(&0x4048_0000u32.to_le_bytes()); // some f32 bits
        data.extend_from_slice(&0xaabb_cc7fu32.to_le_bytes());
        let out = decode_depth_x32_g8x24_uint_surface(1, 1, &data).unwrap();
        assert_eq!(out, vec![0x7f]);
    }

    #[test]
    fn r32_and_x32_views_agree_with_combined_d32s8() {
        let mut data = Vec::new();
        // texel 0
        data.extend_from_slice(&0.25f32.to_le_bytes());
        data.extend_from_slice(&0x0000_0042u32.to_le_bytes());
        // texel 1
        data.extend_from_slice(&(-1.5f32).to_le_bytes());
        data.extend_from_slice(&0xffff_ff80u32.to_le_bytes());
        let combined = decode_depth_d32s8_surface(2, 1, &data).unwrap();
        let depth = decode_depth_r32_float_x8x24_surface(2, 1, &data).unwrap();
        let stencil = decode_depth_x32_g8x24_uint_surface(2, 1, &data).unwrap();
        for i in 0..2 {
            assert_eq!(combined[i].depth, depth[i]);
            assert_eq!(combined[i].stencil, stencil[i]);
        }
    }

    #[test]
    fn view_short_buffers_error_not_panic() {
        assert!(decode_depth_r24_unorm_x8_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_x24_g8_uint_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_r32_float_x8x24_surface(4, 4, &[0u8; 4]).is_err());
        assert!(decode_depth_x32_g8x24_uint_surface(4, 4, &[0u8; 4]).is_err());
    }
}
