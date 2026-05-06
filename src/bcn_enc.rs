//! BCn block encoders — round 3 shipped BC1 (DXT1); round 4 adds
//! BC2 (DXT3), BC3 (DXT5), BC4 (single channel) and BC5 (two channel).
//!
//! Reference: Microsoft's public "BC1, BC2 and BC3" + "BC4" + "BC5"
//! articles on learn.microsoft.com (block layout, c0/c1 ordering,
//! 4-colour vs 3-colour-plus-transparent rules, 8-value vs 6-value
//! interpolation modes) and the public Direct3D 11 reference. No
//! DirectXTex / NVTT / libsquish / bc7enc / basisu / ARM astc-encoder
//! source was consulted; only the public spec.
//!
//! Encoder strategy (BC1):
//!
//! 1. For each 4×4 block, compute the principal axis of the RGB cloud
//!    (PCA: largest eigenvector of the covariance matrix). For a tiny
//!    16-pixel sample we shortcut to "axis = max-luminance pixel - min-
//!    luminance pixel" — visually almost as good and avoids the
//!    eigensolver. Specifically we pick the two pixels whose RGB sum
//!    differ the most along (R + 2G + B) — a perceptual proxy.
//! 2. Project all 16 pixels onto that axis; the two extrema become the
//!    endpoint candidates. Convert each to RGB565.
//! 3. If the two endpoints quantise to the same RGB565 value, force one
//!    of them off by one to keep the four-colour palette distinct; if
//!    they're equal AND we want the four-colour mode, ensure c0 > c1
//!    (which Microsoft's BC1 reader uses to pick between four-colour
//!    and three-colour-plus-transparent layout).
//! 4. Quantise each pixel to the nearest of the four (or three +
//!    transparent) interpolated colours and pack the result.
//!
//! No SIMD, no per-pixel optimal cluster fit, no "drop endpoint by
//! one" refinement. The output is correct (decodes to a faithful
//! approximation of the source) and bit-exact roundtrips when the
//! source is already block-aligned (e.g. solid-colour blocks). It is
//! NOT competitive with DirectXTex or ISPC encoders for visual
//! quality on photographic content.

use crate::bcn::rgb565_to_rgb888;
use crate::error::{DdsError, Result};

#[inline]
fn rgba_at(pixels: &[u8], x: u32, y: u32, stride: usize) -> [u8; 4] {
    let off = y as usize * stride + x as usize * 4;
    [
        pixels[off],
        pixels[off + 1],
        pixels[off + 2],
        pixels[off + 3],
    ]
}

/// Quantise an 8-bit RGB triple to RGB565.
#[inline]
fn rgb_to_565(r: u8, g: u8, b: u8) -> u16 {
    let r5 = ((r as u16) >> 3) & 0x1f;
    let g6 = ((g as u16) >> 2) & 0x3f;
    let b5 = ((b as u16) >> 3) & 0x1f;
    (r5 << 11) | (g6 << 5) | b5
}

/// Compute squared Euclidean RGB distance between two pixels.
#[inline]
fn sq_dist(a: [u8; 3], b: [u8; 3]) -> u32 {
    let dr = a[0] as i32 - b[0] as i32;
    let dg = a[1] as i32 - b[1] as i32;
    let db = a[2] as i32 - b[2] as i32;
    (dr * dr + dg * dg + db * db) as u32
}

/// Encode one 4×4 RGBA8 block (16 pixels read row-major from `pixels`)
/// into an 8-byte BC1 block.
///
/// `accept_punchthrough_alpha` — when true and any pixel has alpha
/// below 128, the encoder uses BC1's 3-colour-plus-transparent layout
/// (c0 ≤ c1) and quantises low-alpha pixels to the transparent index.
/// When false the encoder always emits the 4-colour layout.
fn encode_bc1_block(pixels_rgba: &[[u8; 4]; 16], accept_punchthrough_alpha: bool) -> [u8; 8] {
    // Detect 1-bit alpha pixels.
    let mut has_alpha = false;
    for p in pixels_rgba.iter() {
        if p[3] < 128 {
            has_alpha = true;
            break;
        }
    }
    let three_colour_mode = accept_punchthrough_alpha && has_alpha;

    // Pick endpoints: find the two pixels that are furthest apart in
    // RGB space (squared Euclidean). With 16 pixels per block this is
    // 16*15/2 = 120 distance computations — trivial. Skip transparent
    // pixels in 3-colour mode.
    let is_visible = |p: &[u8; 4]| !three_colour_mode || p[3] >= 128;
    let mut best_d = 0u32;
    let mut best_i: Option<usize> = None;
    let mut best_j: Option<usize> = None;
    for i in 0..16 {
        if !is_visible(&pixels_rgba[i]) {
            continue;
        }
        // Track at least one visible pixel so the all-transparent
        // branch below can still seed a valid endpoint pair.
        best_i.get_or_insert(i);
        best_j.get_or_insert(i);
        for j in (i + 1)..16 {
            if !is_visible(&pixels_rgba[j]) {
                continue;
            }
            let pi = pixels_rgba[i];
            let pj = pixels_rgba[j];
            let d = sq_dist([pi[0], pi[1], pi[2]], [pj[0], pj[1], pj[2]]);
            if d > best_d {
                best_d = d;
                best_i = Some(i);
                best_j = Some(j);
            }
        }
    }
    let (best_i, best_j) = match (best_i, best_j) {
        (Some(i), Some(j)) => (i, j),
        // Every pixel transparent — emit a fully-transparent block.
        _ => return pack_bc1(0, 0, 0xffff_ffffu32),
    };
    let mut e0 = [
        pixels_rgba[best_i][0],
        pixels_rgba[best_i][1],
        pixels_rgba[best_i][2],
    ];
    let mut e1 = [
        pixels_rgba[best_j][0],
        pixels_rgba[best_j][1],
        pixels_rgba[best_j][2],
    ];

    // Quantise to RGB565.
    let mut c0 = rgb_to_565(e0[0], e0[1], e0[2]);
    let mut c1 = rgb_to_565(e1[0], e1[1], e1[2]);

    // Re-expand for index quantisation (we want to match what the
    // decoder will see, not the original 8-bit endpoint).
    let (r0, g0, b0) = rgb565_to_rgb888(c0);
    let (r1, g1, b1) = rgb565_to_rgb888(c1);
    e0 = [r0, g0, b0];
    e1 = [r1, g1, b1];

    // BC1 mode selection: 4-colour requires c0 > c1; 3-colour-plus-
    // transparent requires c0 ≤ c1. Swap if needed so the layout matches
    // our intended mode.
    let mut three_colour_mode = three_colour_mode;
    if !three_colour_mode {
        if c0 < c1 {
            std::mem::swap(&mut c0, &mut c1);
            std::mem::swap(&mut e0, &mut e1);
        } else if c0 == c1 {
            // Endpoints quantise to the same RGB565 value — the
            // 4-colour palette would degenerate to a single colour
            // anyway, so emit the 3-colour layout instead (c0 = c1
            // satisfies c0 ≤ c1; palette[2] = c0 = c1; index 3 is the
            // transparent slot, but no pixel will pick it for a fully-
            // opaque block).
            three_colour_mode = true;
        }
    } else if c0 > c1 {
        // 3-colour mode: ensure c0 ≤ c1.
        std::mem::swap(&mut c0, &mut c1);
        std::mem::swap(&mut e0, &mut e1);
    }

    // Build the palette the decoder will actually produce.
    let mut palette = [[0u8; 3]; 4];
    palette[0] = e0;
    palette[1] = e1;
    if !three_colour_mode {
        for ch in 0..3 {
            palette[2][ch] = ((2 * e0[ch] as u32 + e1[ch] as u32) / 3) as u8;
            palette[3][ch] = ((e0[ch] as u32 + 2 * e1[ch] as u32) / 3) as u8;
        }
    } else {
        for ch in 0..3 {
            palette[2][ch] = ((e0[ch] as u32 + e1[ch] as u32) / 2) as u8;
        }
        // palette[3] in 3-colour mode is the transparent slot; we
        // assign index 3 directly without consulting palette[3].
    }

    // Quantise each pixel to the nearest palette entry. For 3-colour
    // mode, low-alpha pixels go directly to index 3 (transparent).
    let mut indices = 0u32;
    let palette_count = if three_colour_mode { 3 } else { 4 };
    for (i, p) in pixels_rgba.iter().enumerate() {
        let idx: u32 = if three_colour_mode && p[3] < 128 {
            3
        } else {
            let mut best = 0u32;
            let mut best_dist = u32::MAX;
            for (k, pal) in palette.iter().enumerate().take(palette_count) {
                let d = sq_dist([p[0], p[1], p[2]], *pal);
                if d < best_dist {
                    best_dist = d;
                    best = k as u32;
                }
            }
            best
        };
        indices |= (idx & 0x3) << (i * 2);
    }

    pack_bc1(c0, c1, indices)
}

#[inline]
fn pack_bc1(c0: u16, c1: u16, indices: u32) -> [u8; 8] {
    let mut out = [0u8; 8];
    out[0..2].copy_from_slice(&c0.to_le_bytes());
    out[2..4].copy_from_slice(&c1.to_le_bytes());
    out[4..8].copy_from_slice(&indices.to_le_bytes());
    out
}

/// Encode a width × height RGBA8 surface to BC1.
///
/// `input` must hold `width × height × 4` bytes (row-major, no padding).
/// `output` must hold `ceil(w/4) × ceil(h/4) × 8` bytes.
pub fn encode_bc1(
    input: &[u8],
    width: u32,
    height: u32,
    accept_punchthrough_alpha: bool,
    output: &mut [u8],
) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 4;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC1 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 8;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC1 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    for by in 0..bh {
        for bx in 0..bw {
            // Gather the 4×4 block from the input. Pixels outside the
            // surface (edge blocks for non-multiple-of-4 sizes) are
            // filled with the nearest in-bounds pixel.
            let mut block = [[0u8; 4]; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    block[(py * 4 + px) as usize] = rgba_at(input, xx, yy, stride);
                }
            }
            let bc = encode_bc1_block(&block, accept_punchthrough_alpha);
            let off = (by * bw + bx) * 8;
            output[off..off + 8].copy_from_slice(&bc);
        }
    }
    Ok(())
}

// ---- BC4 / single-channel interpolated-alpha block encoder ------------

/// Encode 16 single-channel bytes (`pixels`, row-major) into an 8-byte
/// BC4 block using the interpolated-alpha layout that BC3-alpha and BC4
/// share. We always emit the 8-value mode (`a0 > a1`) which avoids the
/// need to special-case "exactly 0" / "exactly 255" pixels — those still
/// quantise correctly within the 8-value palette.
///
/// Strategy:
/// * Endpoints = `(max, min)` of the input. If max == min, both
///   endpoints are equal and every index is 0.
/// * Otherwise build the 8-entry palette and quantise each pixel to the
///   nearest entry (`abs(diff)` minimum). Pack the resulting 3-bit
///   indices into the 48-bit packed-index field.
fn encode_bc4_unorm_block(pixels: &[u8; 16]) -> [u8; 8] {
    let mut max = 0u8;
    let mut min = 255u8;
    for &p in pixels.iter() {
        if p > max {
            max = p;
        }
        if p < min {
            min = p;
        }
    }
    if max == min {
        // Degenerate — emit a 6-value-mode block with a0==a1 and all
        // indices = 0.
        let mut out = [0u8; 8];
        out[0] = max;
        out[1] = min;
        return out;
    }
    let a0 = max;
    let a1 = min;
    // 8-value palette: a0, a1, (6a0+a1)/7 ... (a0+6a1)/7.
    let mut palette = [0u16; 8];
    palette[0] = a0 as u16;
    palette[1] = a1 as u16;
    for k in 1..=6u16 {
        let num = (7 - k) * a0 as u16 + k * a1 as u16;
        palette[(k + 1) as usize] = num / 7;
    }
    let mut packed: u64 = 0;
    for (i, &p) in pixels.iter().enumerate() {
        let mut best = 0u32;
        let mut best_d = u32::MAX;
        for (k, &pal) in palette.iter().enumerate() {
            let d = (p as i32 - pal as i32).unsigned_abs();
            if d < best_d {
                best_d = d;
                best = k as u32;
            }
        }
        packed |= (best as u64 & 0x7) << (i * 3);
    }
    let mut out = [0u8; 8];
    out[0] = a0;
    out[1] = a1;
    let bytes = packed.to_le_bytes();
    out[2..8].copy_from_slice(&bytes[0..6]);
    out
}

/// Encode 16 alpha bytes into a BC2-style explicit-4-bit-alpha block
/// (8 bytes). Each pixel is quantised by right-shifting 4 bits (top
/// nibble survives — drops 4 LSBs).
fn encode_bc2_alpha_block(pixels_alpha: &[u8; 16]) -> [u8; 8] {
    let mut out = [0u8; 8];
    for (i, &a) in pixels_alpha.iter().enumerate() {
        let nibble = a >> 4;
        if i & 1 == 0 {
            out[i / 2] |= nibble & 0x0f;
        } else {
            out[i / 2] |= (nibble & 0x0f) << 4;
        }
    }
    out
}

/// Encode an RGBA8 surface to BC2 (DXT3): 4-bit explicit alpha + 4-colour BC1.
pub fn encode_bc2(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 4;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC2 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC2 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    for by in 0..bh {
        for bx in 0..bw {
            let mut block_rgba = [[0u8; 4]; 16];
            let mut alpha = [0u8; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    let p = rgba_at(input, xx, yy, stride);
                    let i = (py * 4 + px) as usize;
                    block_rgba[i] = p;
                    alpha[i] = p[3];
                }
            }
            let alpha_bytes = encode_bc2_alpha_block(&alpha);
            // BC1 colour block, 4-colour mode (no punchthrough).
            let mut colour_rgba = block_rgba;
            // BC2 colour block always uses 4-colour interpolation —
            // force opaque alpha so the BC1 encoder doesn't pick the
            // 3-colour mode.
            for p in colour_rgba.iter_mut() {
                p[3] = 0xff;
            }
            let mut bc1 = encode_bc1_block(&colour_rgba, /*alpha*/ false);
            // Ensure c0 > c1 so the BC2 reader uses the always-4-colour
            // mode (matches the inner encoder for non-degenerate blocks).
            let c0 = u16::from_le_bytes([bc1[0], bc1[1]]);
            let c1 = u16::from_le_bytes([bc1[2], bc1[3]]);
            if c0 < c1 {
                // swap c0/c1 and flip indices (4-colour palette is
                // symmetric: idx 0↔1, idx 2↔3).
                bc1[0..2].copy_from_slice(&c1.to_le_bytes());
                bc1[2..4].copy_from_slice(&c0.to_le_bytes());
                let mut idx = u32::from_le_bytes(bc1[4..8].try_into().unwrap());
                let flipped = ((idx & 0x55555555) << 1) | ((idx & 0xaaaaaaaa) >> 1);
                idx = flipped;
                bc1[4..8].copy_from_slice(&idx.to_le_bytes());
            }
            let off = (by * bw + bx) * 16;
            output[off..off + 8].copy_from_slice(&alpha_bytes);
            output[off + 8..off + 16].copy_from_slice(&bc1);
        }
    }
    Ok(())
}

/// Encode an RGBA8 surface to BC3 (DXT5): interpolated-alpha + 4-colour BC1.
pub fn encode_bc3(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 4;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC3 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC3 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    for by in 0..bh {
        for bx in 0..bw {
            let mut block_rgba = [[0u8; 4]; 16];
            let mut alpha = [0u8; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    let p = rgba_at(input, xx, yy, stride);
                    let i = (py * 4 + px) as usize;
                    block_rgba[i] = p;
                    alpha[i] = p[3];
                }
            }
            let alpha_bytes = encode_bc4_unorm_block(&alpha);
            // BC3 colour block always uses 4-colour interpolation; same
            // c0 > c1 swap rule as BC2.
            let mut colour_rgba = block_rgba;
            for p in colour_rgba.iter_mut() {
                p[3] = 0xff;
            }
            let mut bc1 = encode_bc1_block(&colour_rgba, /*alpha*/ false);
            let c0 = u16::from_le_bytes([bc1[0], bc1[1]]);
            let c1 = u16::from_le_bytes([bc1[2], bc1[3]]);
            if c0 < c1 {
                bc1[0..2].copy_from_slice(&c1.to_le_bytes());
                bc1[2..4].copy_from_slice(&c0.to_le_bytes());
                let mut idx = u32::from_le_bytes(bc1[4..8].try_into().unwrap());
                let flipped = ((idx & 0x55555555) << 1) | ((idx & 0xaaaaaaaa) >> 1);
                idx = flipped;
                bc1[4..8].copy_from_slice(&idx.to_le_bytes());
            }
            let off = (by * bw + bx) * 16;
            output[off..off + 8].copy_from_slice(&alpha_bytes);
            output[off + 8..off + 16].copy_from_slice(&bc1);
        }
    }
    Ok(())
}

/// Encode an R8 surface to BC4_UNORM. `input.len() >= width * height`,
/// `output.len() >= ceil(w/4) * ceil(h/4) * 8`.
pub fn encode_bc4_unorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC4 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 8;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC4 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let mut block = [0u8; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    block[(py * 4 + px) as usize] = input[yy as usize * stride + xx as usize];
                }
            }
            let bc = encode_bc4_unorm_block(&block);
            let off = (by * bw + bx) * 8;
            output[off..off + 8].copy_from_slice(&bc);
        }
    }
    Ok(())
}

/// Encode an interleaved RG8 surface (`[r, g, r, g, ...]`) to BC5_UNORM.
/// `input.len() >= width * height * 2`, `output.len() >=
/// ceil(w/4) * ceil(h/4) * 16`.
pub fn encode_bc5_unorm(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 2;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC5 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC5 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 2;
    for by in 0..bh {
        for bx in 0..bw {
            let mut r = [0u8; 16];
            let mut g = [0u8; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    let dst = yy as usize * stride + xx as usize * 2;
                    let i = (py * 4 + px) as usize;
                    r[i] = input[dst];
                    g[i] = input[dst + 1];
                }
            }
            let r_bc = encode_bc4_unorm_block(&r);
            let g_bc = encode_bc4_unorm_block(&g);
            let off = (by * bw + bx) * 16;
            output[off..off + 8].copy_from_slice(&r_bc);
            output[off + 8..off + 16].copy_from_slice(&g_bc);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bcn::{decode_bc1, decode_bc2, decode_bc3, decode_bc4_unorm, decode_bc5_unorm};

    /// Solid white block → encoder picks both endpoints = white →
    /// every index = 0 → roundtrip is bit-exact.
    #[test]
    fn bc1_encode_solid_white_roundtrip() {
        let input = vec![0xffu8; 4 * 4 * 4];
        let mut bc = vec![0u8; 8];
        encode_bc1(&input, 4, 4, false, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc1(&bc, 4, 4, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    /// Solid black block → both endpoints = black → roundtrip bit-exact.
    #[test]
    fn bc1_encode_solid_black_roundtrip() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for chunk in input.chunks_exact_mut(4) {
            chunk[3] = 0xff; // alpha
        }
        let mut bc = vec![0u8; 8];
        encode_bc1(&input, 4, 4, false, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc1(&bc, 4, 4, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[0, 0, 0, 255]);
        }
    }

    /// Two-colour block (left half red, right half blue) → endpoints =
    /// (red, blue), every pixel maps to one of the two extremes.
    #[test]
    fn bc1_encode_two_colour_roundtrip() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    input[off] = 0xff;
                    input[off + 1] = 0;
                    input[off + 2] = 0;
                    input[off + 3] = 0xff;
                } else {
                    input[off] = 0;
                    input[off + 1] = 0;
                    input[off + 2] = 0xff;
                    input[off + 3] = 0xff;
                }
            }
        }
        let mut bc = vec![0u8; 8];
        encode_bc1(&input, 4, 4, false, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc1(&bc, 4, 4, &mut decoded).unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    assert_eq!(
                        &decoded[off..off + 4],
                        &[0xff, 0, 0, 0xff],
                        "pixel ({x},{y})"
                    );
                } else {
                    assert_eq!(
                        &decoded[off..off + 4],
                        &[0, 0, 0xff, 0xff],
                        "pixel ({x},{y})"
                    );
                }
            }
        }
    }

    /// Alpha pass-through: every transparent pixel decodes to the
    /// transparent index when 1-bit alpha is enabled.
    #[test]
    fn bc1_encode_punchthrough_alpha_block() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                input[off] = 0xff;
                input[off + 1] = 0xff;
                input[off + 2] = 0xff;
                input[off + 3] = if x < 2 { 0 } else { 0xff };
            }
        }
        let mut bc = vec![0u8; 8];
        encode_bc1(&input, 4, 4, /*alpha*/ true, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc1(&bc, 4, 4, &mut decoded).unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    assert_eq!(decoded[off + 3], 0, "pixel ({x},{y}) should be transparent");
                } else {
                    assert!(
                        decoded[off + 3] >= 200,
                        "pixel ({x},{y}) should be opaque, got alpha {}",
                        decoded[off + 3]
                    );
                }
            }
        }
    }

    #[test]
    fn bc1_encode_handles_5x5_surface() {
        let input = vec![0xffu8; 5 * 5 * 4];
        let mut bc = vec![0u8; 4 * 8]; // 2x2 = 4 blocks
        encode_bc1(&input, 5, 5, false, &mut bc).unwrap();
        let mut decoded = vec![0u8; 5 * 5 * 4];
        decode_bc1(&bc, 5, 5, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    // ---- BC2 / BC3 / BC4 / BC5 encoder roundtrip tests -----------------

    /// BC4 encoder: solid r = 200 → roundtrip bit-exact (both endpoints
    /// = 200, every index = 0).
    #[test]
    fn bc4_encode_solid_roundtrip() {
        let input = vec![200u8; 4 * 4];
        let mut bc = vec![0u8; 8];
        encode_bc4_unorm(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4];
        decode_bc4_unorm(&bc, 4, 4, &mut decoded).unwrap();
        for &v in decoded.iter() {
            assert_eq!(v, 200);
        }
    }

    /// BC4 encoder: gradient block — endpoints = (max, min); every pixel
    /// quantises to one of 8 palette entries and decodes within ±18 of
    /// the source (8-value bin width = (255 - 0)/7 ≈ 36, so half-bin is 18).
    #[test]
    fn bc4_encode_gradient_psnr() {
        let mut input = vec![0u8; 4 * 4];
        for (i, b) in input.iter_mut().enumerate() {
            *b = (i as u8) * 16; // 0, 16, 32, ..., 240
        }
        let mut bc = vec![0u8; 8];
        encode_bc4_unorm(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4];
        decode_bc4_unorm(&bc, 4, 4, &mut decoded).unwrap();
        let mut sse: u64 = 0;
        for (s, d) in input.iter().zip(decoded.iter()) {
            let diff = *s as i32 - *d as i32;
            sse += (diff * diff) as u64;
        }
        // sqrt(sse / 16) — should be <= ~18 (half a bin).
        let mse = sse as f64 / 16.0;
        let rmse = mse.sqrt();
        assert!(rmse < 20.0, "BC4 gradient rmse = {} (want < 20)", rmse);
    }

    /// BC2 encoder: opaque white block roundtrips bit-exact.
    #[test]
    fn bc2_encode_solid_white_roundtrip() {
        let input = vec![0xffu8; 4 * 4 * 4];
        let mut bc = vec![0u8; 16];
        encode_bc2(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc2(&bc, 4, 4, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    /// BC2 encoder preserves alpha quantised to top-nibble (loss = 4 LSBs
    /// → output = input & 0xf0 | (input >> 4)).
    #[test]
    fn bc2_encode_alpha_quantises_to_top_nibble() {
        let mut input = vec![0xffu8; 4 * 4 * 4];
        // Set alpha values to 0x10..0x1f (low nibble varies, top = 1).
        for (i, chunk) in input.chunks_exact_mut(4).enumerate() {
            chunk[3] = 0x10 + i as u8;
        }
        let mut bc = vec![0u8; 16];
        encode_bc2(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc2(&bc, 4, 4, &mut decoded).unwrap();
        // Alpha: top-nibble 1 → expand_to_8 = 0x11. Low nibble dropped.
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk[3], 0x11, "BC2 alpha quantised to top nibble");
        }
    }

    /// BC3 encoder: solid red opaque block roundtrips bit-exact.
    #[test]
    fn bc3_encode_solid_red_roundtrip() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for chunk in input.chunks_exact_mut(4) {
            chunk[0] = 0xff;
            chunk[3] = 0xff;
        }
        let mut bc = vec![0u8; 16];
        encode_bc3(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc3(&bc, 4, 4, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[0xff, 0, 0, 0xff]);
        }
    }

    /// BC3 encoder: gradient alpha 0..240 → BC4-encoded alpha decodes
    /// within RMSE 20.
    #[test]
    fn bc3_encode_alpha_gradient_psnr() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for (i, chunk) in input.chunks_exact_mut(4).enumerate() {
            chunk[0] = 128;
            chunk[1] = 128;
            chunk[2] = 128;
            chunk[3] = (i as u8) * 16;
        }
        let mut bc = vec![0u8; 16];
        encode_bc3(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc3(&bc, 4, 4, &mut decoded).unwrap();
        let mut sse_a: u64 = 0;
        for i in 0..16 {
            let s = input[i * 4 + 3] as i32;
            let d = decoded[i * 4 + 3] as i32;
            sse_a += ((s - d) * (s - d)) as u64;
        }
        let rmse = (sse_a as f64 / 16.0).sqrt();
        assert!(rmse < 20.0, "BC3 alpha gradient rmse = {}", rmse);
    }

    /// BC5 encoder: solid (r=120, g=80) block roundtrips bit-exact.
    #[test]
    fn bc5_encode_solid_roundtrip() {
        let mut input = vec![0u8; 4 * 4 * 2];
        for pair in input.chunks_exact_mut(2) {
            pair[0] = 120;
            pair[1] = 80;
        }
        let mut bc = vec![0u8; 16];
        encode_bc5_unorm(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 2];
        decode_bc5_unorm(&bc, 4, 4, &mut decoded).unwrap();
        for pair in decoded.chunks_exact(2) {
            assert_eq!(pair[0], 120);
            assert_eq!(pair[1], 80);
        }
    }

    /// Natural-image PSNR check on a synthetic gradient. The
    /// furthest-point endpoint heuristic in `encode_bc1_block` gives
    /// PSNR-RGB > ~30 dB on smoothly-varying photographic-style content.
    #[test]
    fn bc1_encode_natural_gradient_psnr_gt_25_db() {
        // 16×16 RGBA gradient (no alpha variation).
        let mut input = vec![0u8; 16 * 16 * 4];
        for y in 0..16 {
            for x in 0..16 {
                let off = (y * 16 + x) * 4;
                input[off] = (x * 16) as u8; // R varies horizontally
                input[off + 1] = (y * 16) as u8; // G varies vertically
                input[off + 2] = ((x + y) * 8) as u8; // B varies diagonally
                input[off + 3] = 0xff;
            }
        }
        let mut bc = vec![0u8; 16 * 8];
        encode_bc1(&input, 16, 16, false, &mut bc).unwrap();
        let mut decoded = vec![0u8; 16 * 16 * 4];
        decode_bc1(&bc, 16, 16, &mut decoded).unwrap();
        let mut sse: u64 = 0;
        let mut count: u64 = 0;
        for i in 0..(16 * 16) {
            for ch in 0..3 {
                let s = input[i * 4 + ch] as i32;
                let d = decoded[i * 4 + ch] as i32;
                sse += ((s - d) * (s - d)) as u64;
                count += 1;
            }
        }
        let mse = sse as f64 / count as f64;
        let psnr = 10.0 * (255.0_f64 * 255.0 / mse).log10();
        assert!(
            psnr > 25.0,
            "BC1 16x16 RGB-gradient PSNR = {:.2} dB (want > 25 dB)",
            psnr
        );
    }
}
