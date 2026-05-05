//! BC1..BC5 block decompression to RGBA8 / RG8 / R8.
//!
//! Reference: Microsoft's public "BCn texture compression" articles on
//! learn.microsoft.com (specifically "BC1, BC2 and BC3" + "BC4" + "BC5"
//! pages under direct3d11 / direct3d10 reference) and the public
//! D3DSDK BCn block-layout descriptions. No DirectXTex, NVTT,
//! libsquish, basisu, bc7enc, or any other implementation source was
//! consulted; only the public spec text + tables + the worked-example
//! pseudo-code from the Microsoft articles.
//!
//! Block layout summary (every BCn block is 4×4 pixels):
//!
//! * **BC1** (8 bytes / block): two RGB565 endpoint colours `c0` /
//!   `c1` followed by 16 × 2-bit indices. If `c0 > c1` the block
//!   uses 4 interpolated colours `(c0, c1, (2c0+c1)/3, (c0+2c1)/3)`.
//!   If `c0 <= c1` the block uses 3 colours plus a transparent black
//!   at index 3 (`(c0, c1, (c0+c1)/2, 0,0,0,0)`). 1-bit alpha mode.
//!
//! * **BC2** (16 bytes / block): 8 bytes of explicit 4-bit alpha (16
//!   nibbles, row-major, low nibble first) followed by an
//!   always-4-colour BC1 colour block (no transparent index).
//!
//! * **BC3** (16 bytes / block): 8 bytes of interpolated alpha (two
//!   8-bit alpha endpoints + 16 × 3-bit indices) followed by an
//!   always-4-colour BC1 colour block. Interpolated-alpha rules
//!   mirror BC4: if `a0 > a1`, 8-value mode `(a0, a1, (6a0+a1)/7,
//!   ..., (a0+6a1)/7)`; else 6-value mode plus 0 + 255 for indices
//!   6/7.
//!
//! * **BC4** (8 bytes / block): two 8-bit endpoints + 16 × 3-bit
//!   indices for a single channel, with the same `e0 > e1` switch
//!   as BC3 alpha. Unsigned (`BC4_UNORM`) or signed (`BC4_SNORM`).
//!
//! * **BC5** (16 bytes / block): two BC4 blocks back-to-back (red
//!   then green). Used for two-channel data like tangent-space
//!   normal maps.
//!
//! All decoders write into a target RGBA8 surface (BC1/BC2/BC3) or
//! R/RG surface (BC4/BC5) with the caller-supplied row stride.
//! Out-of-bounds pixels (when width/height aren't multiples of 4)
//! are skipped.

use crate::error::{DdsError, Result};

/// Total bytes for an RGBA8 (4 bytes/pixel) surface of `width × height`.
pub(crate) fn rgba8_surface_bytes(width: u32, height: u32) -> usize {
    width as usize * height as usize * 4
}

/// Total bytes for a single-channel R8 surface of `width × height`.
pub(crate) fn r8_surface_bytes(width: u32, height: u32) -> usize {
    width as usize * height as usize
}

/// Total bytes for a two-channel RG8 surface of `width × height`.
pub(crate) fn rg8_surface_bytes(width: u32, height: u32) -> usize {
    width as usize * height as usize * 2
}

/// Expand a 16-bit RGB565 endpoint to an `(r, g, b)` 8-bit triple
/// using the bit-replication scheme Direct3D mandates (top bits
/// replicated into the LSBs so the endpoint range covers the full
/// 0..=255).
#[inline]
fn rgb565_to_rgb888(c: u16) -> (u8, u8, u8) {
    let r5 = ((c >> 11) & 0x1f) as u8;
    let g6 = ((c >> 5) & 0x3f) as u8;
    let b5 = (c & 0x1f) as u8;
    let r = (r5 << 3) | (r5 >> 2);
    let g = (g6 << 2) | (g6 >> 4);
    let b = (b5 << 3) | (b5 >> 2);
    (r, g, b)
}

/// Decode a single BC1 colour block (8 bytes) into a 4×4 RGBA8 grid.
fn decode_bc1_block(block: &[u8; 8], punchthrough_alpha: bool) -> [[u8; 4]; 16] {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);

    let (r0, g0, b0) = rgb565_to_rgb888(c0);
    let (r1, g1, b1) = rgb565_to_rgb888(c1);

    let mut palette = [[0u8; 4]; 4];
    palette[0] = [r0, g0, b0, 255];
    palette[1] = [r1, g1, b1, 255];

    // For BC2 / BC3 (`punchthrough_alpha = false`) the colour
    // sub-block ALWAYS uses the 4-colour interpolation regardless of
    // c0/c1 ordering (Microsoft BC2/BC3 reference). Only "real" BC1
    // honours the c0 <= c1 punch-through-alpha branch.
    let four_colour = c0 > c1 || !punchthrough_alpha;
    if four_colour {
        // 4-colour interpolation, opaque alpha.
        // c2 = (2c0 + c1) / 3, c3 = (c0 + 2c1) / 3 — channel-wise.
        // Range loop touches multiple palette slots so it doesn't
        // collapse to an iterator chain.
        #[allow(clippy::needless_range_loop)]
        for ch in 0..3 {
            let e0 = palette[0][ch] as u32;
            let e1 = palette[1][ch] as u32;
            palette[2][ch] = ((2 * e0 + e1) / 3) as u8;
            palette[3][ch] = ((e0 + 2 * e1) / 3) as u8;
        }
        palette[2][3] = 255;
        palette[3][3] = 255;
    } else {
        // BC1, c0 <= c1 → 3-colour mode + transparent black at idx 3.
        #[allow(clippy::needless_range_loop)]
        for ch in 0..3 {
            let e0 = palette[0][ch] as u32;
            let e1 = palette[1][ch] as u32;
            palette[2][ch] = ((e0 + e1) / 2) as u8;
        }
        palette[2][3] = 255;
        palette[3] = [0, 0, 0, 0];
    }

    let mut out = [[0u8; 4]; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let idx = ((indices >> (i * 2)) & 0x3) as usize;
        *slot = palette[idx];
    }
    out
}

/// Decode an 8-byte interpolated-alpha block (BC3 alpha sub-block /
/// BC4) into 16 unsigned 8-bit alpha values.
fn decode_bc4_unorm_block(block: &[u8; 8]) -> [u8; 16] {
    let a0 = block[0];
    let a1 = block[1];

    // 48-bit packed indices live in bytes 2..=7. Read them as a
    // little-endian u64 with the top 16 bits zero, then peel off
    // 3 bits per pixel.
    let mut packed: u64 = 0;
    for (i, &b) in block[2..].iter().enumerate() {
        packed |= (b as u64) << (i * 8);
    }

    let mut palette = [0u16; 8];
    palette[0] = a0 as u16;
    palette[1] = a1 as u16;
    if a0 > a1 {
        // (6a0+a1)/7, (5a0+2a1)/7, ..., (a0+6a1)/7 (floor division
        // — matches the Microsoft BC3/BC4 reference table).
        for k in 1..=6u16 {
            let num = (7 - k) * a0 as u16 + k * a1 as u16;
            palette[(k + 1) as usize] = num / 7;
        }
    } else {
        // 6-value interpolation, fixed 0 and 255 endpoints at idx 6/7.
        for k in 1..=4u16 {
            let num = (5 - k) * a0 as u16 + k * a1 as u16;
            palette[(k + 1) as usize] = num / 5;
        }
        palette[6] = 0;
        palette[7] = 255;
    }

    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let idx = ((packed >> (i * 3)) & 0x7) as usize;
        *slot = palette[idx] as u8;
    }
    out
}

/// Decode an 8-byte interpolated-alpha block (BC4_SNORM) into 16
/// signed 8-bit values stored as `i8`.
fn decode_bc4_snorm_block(block: &[u8; 8]) -> [i8; 16] {
    let a0 = block[0] as i8;
    let a1 = block[1] as i8;

    let mut packed: u64 = 0;
    for (i, &b) in block[2..].iter().enumerate() {
        packed |= (b as u64) << (i * 8);
    }

    let mut palette = [0i16; 8];
    palette[0] = a0 as i16;
    palette[1] = a1 as i16;
    if a0 > a1 {
        for k in 1..=6i16 {
            let num = (7 - k) * a0 as i16 + k * a1 as i16;
            palette[(k + 1) as usize] = num.div_euclid(7);
        }
    } else {
        for k in 1..=4i16 {
            let num = (5 - k) * a0 as i16 + k * a1 as i16;
            palette[(k + 1) as usize] = num.div_euclid(5);
        }
        // SNORM endpoints are -127 and 127 (the -128 value is
        // reserved and Microsoft tells encoders to clamp it to -127).
        palette[6] = -127;
        palette[7] = 127;
    }

    let mut out = [0i8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let idx = ((packed >> (i * 3)) & 0x7) as usize;
        *slot = palette[idx].clamp(-127, 127) as i8;
    }
    out
}

/// Decode a BC2 explicit 4-bit alpha block (8 bytes) into 16 alpha
/// values. The 4-bit value is bit-replicated to 8 bits
/// (`a4 << 4 | a4`).
fn decode_bc2_alpha_block(block: &[u8; 8]) -> [u8; 16] {
    let mut out = [0u8; 16];
    for (i, slot) in out.iter_mut().enumerate() {
        let nibble = if i & 1 == 0 {
            block[i / 2] & 0x0f
        } else {
            (block[i / 2] >> 4) & 0x0f
        };
        *slot = (nibble << 4) | nibble;
    }
    out
}

// ---- Public surface-level decoders --------------------------------------

/// Decode a BC1 surface to RGBA8. Width and height are the logical
/// pixel dimensions; the input must hold
/// `ceil(w/4) × ceil(h/4) × 8` bytes. Output buffer must be
/// `width × height × 4` bytes.
pub fn decode_bc1(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    decode_bc1_inner(input, width, height, output, /*alpha*/ true)
}

fn decode_bc1_inner(
    input: &[u8],
    width: u32,
    height: u32,
    output: &mut [u8],
    punchthrough_alpha: bool,
) -> Result<()> {
    let want_in = block_input_bytes(width, height, 8);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC1 input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = rgba8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC1 output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 8;
            let block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let pixels = decode_bc1_block(&block, punchthrough_alpha);
            write_rgba_block(&pixels, bx, by, width, height, stride, output);
        }
    }
    Ok(())
}

/// Decode a BC2 (DXT3) surface to RGBA8. Same layout/contract as BC1
/// with 16-byte blocks (8 alpha + 8 colour).
pub fn decode_bc2(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 16);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC2 input {} bytes < expected {} bytes",
            input.len(),
            want_in
        )));
    }
    let want_out = rgba8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC2 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize * 4;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let alpha_block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let colour_block: [u8; 8] = input[off + 8..off + 16].try_into().unwrap();
            let alpha = decode_bc2_alpha_block(&alpha_block);
            let mut pixels = decode_bc1_block(&colour_block, /*punchthrough*/ false);
            for (px, &a) in pixels.iter_mut().zip(alpha.iter()) {
                px[3] = a;
            }
            write_rgba_block(&pixels, bx, by, width, height, stride, output);
        }
    }
    Ok(())
}

/// Decode a BC3 (DXT5) surface to RGBA8. 16-byte blocks (8
/// interpolated-alpha + 8 colour).
pub fn decode_bc3(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 16);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC3 input {} bytes < expected {} bytes",
            input.len(),
            want_in
        )));
    }
    let want_out = rgba8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC3 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize * 4;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let alpha_block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let colour_block: [u8; 8] = input[off + 8..off + 16].try_into().unwrap();
            let alpha = decode_bc4_unorm_block(&alpha_block);
            let mut pixels = decode_bc1_block(&colour_block, /*punchthrough*/ false);
            for (px, &a) in pixels.iter_mut().zip(alpha.iter()) {
                px[3] = a;
            }
            write_rgba_block(&pixels, bx, by, width, height, stride, output);
        }
    }
    Ok(())
}

/// Decode a BC4 unsigned single-channel surface to R8.
pub fn decode_bc4_unorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 8);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC4 input {} bytes < expected {}",
            input.len(),
            want_in
        )));
    }
    let want_out = r8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC4 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 8;
            let block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let pixels = decode_bc4_unorm_block(&block);
            for py in 0..4u32 {
                let yy = (by as u32) * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = (bx as u32) * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    output[yy as usize * stride + xx as usize] = pixels[(py * 4 + px) as usize];
                }
            }
        }
    }
    Ok(())
}

/// Decode a BC4 signed single-channel surface to R8 (i8 reinterpreted as u8).
pub fn decode_bc4_snorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 8);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC4 input {} bytes < expected {}",
            input.len(),
            want_in
        )));
    }
    let want_out = r8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC4 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 8;
            let block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let pixels = decode_bc4_snorm_block(&block);
            for py in 0..4u32 {
                let yy = (by as u32) * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = (bx as u32) * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    output[yy as usize * stride + xx as usize] =
                        pixels[(py * 4 + px) as usize] as u8;
                }
            }
        }
    }
    Ok(())
}

/// Decode a BC5 unsigned two-channel surface to RG8 (interleaved
/// `[r0, g0, r1, g1, ...]`).
pub fn decode_bc5_unorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 16);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC5 input {} bytes < expected {}",
            input.len(),
            want_in
        )));
    }
    let want_out = rg8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC5 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize * 2;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let r_block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let g_block: [u8; 8] = input[off + 8..off + 16].try_into().unwrap();
            let reds = decode_bc4_unorm_block(&r_block);
            let greens = decode_bc4_unorm_block(&g_block);
            for py in 0..4u32 {
                let yy = (by as u32) * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = (bx as u32) * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    let i = (py * 4 + px) as usize;
                    let dst = yy as usize * stride + xx as usize * 2;
                    output[dst] = reds[i];
                    output[dst + 1] = greens[i];
                }
            }
        }
    }
    Ok(())
}

/// Decode a BC5 signed two-channel surface to RG8 (i8 reinterpreted as u8).
pub fn decode_bc5_snorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let want_in = block_input_bytes(width, height, 16);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC5 input {} bytes < expected {}",
            input.len(),
            want_in
        )));
    }
    let want_out = rg8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC5 output {} bytes < expected {}",
            output.len(),
            want_out
        )));
    }
    let stride = width as usize * 2;
    let bw = width.div_ceil(4) as usize;
    let bh = height.div_ceil(4) as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let r_block: [u8; 8] = input[off..off + 8].try_into().unwrap();
            let g_block: [u8; 8] = input[off + 8..off + 16].try_into().unwrap();
            let reds = decode_bc4_snorm_block(&r_block);
            let greens = decode_bc4_snorm_block(&g_block);
            for py in 0..4u32 {
                let yy = (by as u32) * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = (bx as u32) * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    let i = (py * 4 + px) as usize;
                    let dst = yy as usize * stride + xx as usize * 2;
                    output[dst] = reds[i] as u8;
                    output[dst + 1] = greens[i] as u8;
                }
            }
        }
    }
    Ok(())
}

// ---- Helpers ------------------------------------------------------------

#[inline]
fn block_input_bytes(width: u32, height: u32, block_bytes: u32) -> usize {
    (width.max(1).div_ceil(4) as usize)
        * (height.max(1).div_ceil(4) as usize)
        * (block_bytes as usize)
}

/// Splat a 4×4 RGBA8 grid (`pixels[i] = [r, g, b, a]`) into the
/// destination RGBA8 buffer at `(bx*4, by*4)`. Pixels falling outside
/// the logical width/height are skipped (handles the Microsoft "round
/// up to 4-multiples" surface size for non-multiple-of-4 textures).
fn write_rgba_block(
    pixels: &[[u8; 4]; 16],
    bx: usize,
    by: usize,
    width: u32,
    height: u32,
    stride: usize,
    out: &mut [u8],
) {
    for py in 0..4u32 {
        let yy = (by as u32) * 4 + py;
        if yy >= height {
            continue;
        }
        for px in 0..4u32 {
            let xx = (bx as u32) * 4 + px;
            if xx >= width {
                continue;
            }
            let dst = yy as usize * stride + xx as usize * 4;
            let p = pixels[(py * 4 + px) as usize];
            out[dst..dst + 4].copy_from_slice(&p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a BC1 block with c0 = white (0xffff, RGB565 = 31,63,31)
    /// and c1 = black (0x0000), all 16 indices = 0 (every pixel = white).
    #[test]
    fn bc1_solid_white_block() {
        let block = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
        let pixels = decode_bc1_block(&block, true);
        for p in pixels.iter() {
            assert_eq!(p, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc1_solid_black_indices_one() {
        // c0 = 0xffff white, c1 = 0x0000 black, indices all = 0b01.
        // 0b01010101 repeated 4× = 0x55555555.
        let block = [0xff, 0xff, 0x00, 0x00, 0x55, 0x55, 0x55, 0x55];
        let pixels = decode_bc1_block(&block, true);
        for p in pixels.iter() {
            assert_eq!(p, &[0, 0, 0, 255]);
        }
    }

    #[test]
    fn bc1_punchthrough_transparent_index_3() {
        // c0 = 0x0000 black, c1 = 0xffff white. Since c0 <= c1,
        // indices 0/1 = c0/c1 (black/white), index 2 = (c0+c1)/2 =
        // grey, index 3 = transparent black.
        // 16 indices = 0b11 → packed 0xffff_ffff.
        let block = [0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff];
        let pixels = decode_bc1_block(&block, true);
        for p in pixels.iter() {
            assert_eq!(p, &[0, 0, 0, 0]);
        }
    }

    #[test]
    fn bc4_unorm_solid_endpoints() {
        // a0 = 200, a1 = 100; indices all 0 → all = a0.
        let block = [200, 100, 0, 0, 0, 0, 0, 0];
        let p = decode_bc4_unorm_block(&block);
        for v in p.iter() {
            assert_eq!(*v, 200);
        }
    }

    #[test]
    fn bc4_unorm_interpolated_index_2() {
        // a0 = 255, a1 = 0, 8-value mode (a0 > a1).
        // palette[2] (k=1) = (6*a0 + 1*a1)/7 = 1530/7 = 218.
        let mut idx: u64 = 0;
        for i in 0..16 {
            idx |= 0b010_u64 << (i * 3);
        }
        let bytes = idx.to_le_bytes();
        let block = [
            255, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ];
        let p = decode_bc4_unorm_block(&block);
        for v in p.iter() {
            assert_eq!(*v, ((6 * 255_u32) / 7) as u8);
        }
    }

    #[test]
    fn bc4_unorm_six_value_mode_endpoints() {
        // a0 = 0, a1 = 100 (a0 < a1, 6-value mode).
        // Index 6 → 0, index 7 → 255.
        let mut idx: u64 = 0;
        idx |= 0b110;
        idx |= 0b111 << 3;
        let bytes = idx.to_le_bytes();
        let block = [
            0, 100, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
        ];
        let p = decode_bc4_unorm_block(&block);
        assert_eq!(p[0], 0);
        assert_eq!(p[1], 255);
        assert_eq!(p[2], 0);
    }

    #[test]
    fn bc1_decode_surface_solid_white_8x4() {
        let block = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
        let mut input = Vec::new();
        input.extend_from_slice(&block);
        input.extend_from_slice(&block);
        let mut out = vec![0u8; 8 * 4 * 4];
        decode_bc1(&input, 8, 4, &mut out).unwrap();
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc1_handles_non_multiple_of_4() {
        // 5x5 surface — needs 2x2 = 4 blocks. All white.
        let block = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
        let mut input = Vec::new();
        for _ in 0..4 {
            input.extend_from_slice(&block);
        }
        let mut out = vec![0u8; 5 * 5 * 4];
        decode_bc1(&input, 5, 5, &mut out).unwrap();
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc2_alpha_block_bit_replication() {
        // First nibble = 0xf → 0xff, rest = 0x0 → 0x00.
        let block = [0x0f, 0, 0, 0, 0, 0, 0, 0];
        let a = decode_bc2_alpha_block(&block);
        assert_eq!(a[0], 0xff);
        for &v in a.iter().skip(1) {
            assert_eq!(v, 0x00);
        }
    }

    #[test]
    fn bc2_decode_surface_solid_opaque_white() {
        let mut input = vec![0xff; 8]; // alpha all 0xff
        input.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0]); // colour
        let mut out = vec![0u8; 4 * 4 * 4];
        decode_bc2(&input, 4, 4, &mut out).unwrap();
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc3_decode_surface_solid_opaque_white() {
        let mut input = vec![255, 0, 0, 0, 0, 0, 0, 0]; // alpha block
        input.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0]); // colour
        let mut out = vec![0u8; 4 * 4 * 4];
        decode_bc3(&input, 4, 4, &mut out).unwrap();
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc4_unorm_decode_surface() {
        let block = [200, 100, 0, 0, 0, 0, 0, 0];
        let mut out = vec![0u8; 4 * 4];
        decode_bc4_unorm(&block, 4, 4, &mut out).unwrap();
        for &v in out.iter() {
            assert_eq!(v, 200);
        }
    }

    #[test]
    fn bc5_unorm_decode_surface_interleaves_rg() {
        let r_block = [200, 100, 0, 0, 0, 0, 0, 0];
        let g_block = [50, 25, 0, 0, 0, 0, 0, 0];
        let mut input = Vec::new();
        input.extend_from_slice(&r_block);
        input.extend_from_slice(&g_block);
        let mut out = vec![0u8; 4 * 4 * 2];
        decode_bc5_unorm(&input, 4, 4, &mut out).unwrap();
        for pair in out.chunks_exact(2) {
            assert_eq!(pair, &[200, 50]);
        }
    }
}
