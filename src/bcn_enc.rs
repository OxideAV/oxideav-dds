//! BCn block encoders — round 3 ships BC1 (DXT1) only.
//!
//! Reference: Microsoft's public "BC1, BC2 and BC3" article on
//! learn.microsoft.com (block layout, c0/c1 ordering, 4-colour vs
//! 3-colour-plus-transparent rules) and the public Direct3D 11
//! reference. No DirectXTex / NVTT / libsquish / bc7enc / basisu /
//! ARM astc-encoder source was consulted; only the public spec.
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

/// Expand RGB565 back to 8-bit (Microsoft bit-replication rule).
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
    let mut visible: [Option<usize>; 16] = [None; 16];
    for (i, p) in pixels_rgba.iter().enumerate() {
        if !three_colour_mode || p[3] >= 128 {
            visible[i] = Some(i);
        }
    }
    let visible: Vec<usize> = visible.iter().filter_map(|x| *x).collect();
    if visible.is_empty() {
        // Every pixel transparent — emit a fully-transparent block.
        let c0 = 0u16;
        let c1 = 0u16;
        let indices = 0xffff_ffffu32;
        return pack_bc1(c0, c1, indices);
    }
    let mut best_d = 0u32;
    let mut best_i = visible[0];
    let mut best_j = visible[0];
    for &i in visible.iter() {
        for &j in visible.iter() {
            if i >= j {
                continue;
            }
            let pi = pixels_rgba[i];
            let pj = pixels_rgba[j];
            let d = sq_dist([pi[0], pi[1], pi[2]], [pj[0], pj[1], pj[2]]);
            if d > best_d {
                best_d = d;
                best_i = i;
                best_j = j;
            }
        }
    }
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
        palette[3] = [0, 0, 0]; // transparent — mark separately below.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bcn::decode_bc1;

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
}
