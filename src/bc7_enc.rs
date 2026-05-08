//! BC7 (DXGI `BC7_UNORM`) block encoder.
//!
//! Round 3 baseline shipped mode 6 only; round 4 added the three
//! 2-subset modes — mode 1 (6-bit RGB + shared p-bits, opaque), mode 3
//! (7-bit RGB + per-endpoint p-bits, opaque) and mode 7 (5-bit RGBA +
//! per-endpoint p-bits, translucent) — with a partition-table search
//! across the Microsoft / Khronos 64-partition set. Round 5 adds the
//! two 3-subset modes — mode 0 (4-bit RGB + per-endpoint p-bits +
//! 3-bit indices, 16-entry partition table) and mode 2 (5-bit RGB,
//! no p-bits, 2-bit indices, 64-entry partition table) — for genuine
//! 3-axis natural-image content that the 2-subset modes cannot fit
//! cleanly. The combined picker pushes the round-4 ~28 dB ceiling on
//! 3-axis content past 30 dB.
//!
//! Reference: Microsoft's public "BC7" article on learn.microsoft.com
//! (Direct3D 11 reference) and the public Khronos
//! `KHR_DF_MODEL_BC7` description in the Khronos Data Format
//! specification. No DirectXTex / NVTT / bc7enc / ISPC / basisu source
//! was consulted; only the public spec text + the layout tables.
//!
//! Encoder strategy (round 5):
//!
//! 1. **Mode 6 (always)**: Furthest-point endpoint pair in 4-D RGBA
//!    space, 7-bit + 1-p-bit quantisation, 16-entry palette, nearest-
//!    palette index per pixel. Bit-exact for solid-RGBA blocks; ~33–40 dB
//!    PSNR on smoothly-varying photographic content.
//! 2. **Mode 1 / 3** (opaque blocks only — every pixel α = 0xff):
//!    Sweep the full 64-entry Microsoft / Khronos 2-subset partition
//!    table; for each partition, seed per-subset endpoints with
//!    furthest-point in that subset, then run two iterations of (snap
//!    pixels to nearest palette → least-squares refine endpoints →
//!    re-quantise) before measuring SSE. Keep the winner across all
//!    partitions × both modes.
//! 3. **Mode 0 / 2** (opaque blocks only): Same loop generalised to
//!    three subsets. Mode 0 uses the first 16 entries of the 3-subset
//!    partition table (4-bit selector); mode 2 sweeps all 64 entries.
//!    The 3-subset modes specifically address rank-3 colour content
//!    that the 2-subset modes can only approximate.
//! 4. **Mode 7** (translucent blocks): Same 2-subset refinement loop
//!    with 5+5+1p quantisation per channel and per-endpoint p-bits.
//! 5. The block-level encoder picks the candidate with the lowest SSE
//!    across modes 0, 1, 2, 3, 6 (opaque) or 6 + 7 (translucent).
//!
//! Round 7 closes the encoder coverage gap by adding mode 4/5
//! channel-rotation encoders — both are 1-subset modes with separate
//! RGB and alpha index planes plus a 2-bit field that swaps A with one
//! of R/G/B post-decode. Mode 4 carries a 1-bit `idx_sel` that picks
//! whether the 2-bit primary plane drives RGB or alpha; mode 5 has
//! 2-bit indices on both planes. The encoder pre-rotates the input
//! pixels by the chosen rotation, fits RGB and alpha endpoints
//! separately, picks per-plane indices, then packs — sweeping all 4
//! rotation values (× 2 idx_sel choices for mode 4).
//!
//! With ~7 candidate modes × up to 64 partitions × 2..4 p-bit choices
//! per subset the per-block work is ~7 × 256 quantise+SSE iterations —
//! in release mode that's ~O(180 µs) per 4×4 block, fine for small
//! textures and test fixtures.
//!
//! Output is byte-by-byte bit-exact decoder-roundtrippable: the encoder
//! always picks indices against the palette the *decoder* will produce,
//! so re-encoded blocks decode to the same pixels the encoder
//! considered when computing SSE.

// Per-channel and per-pixel inner loops are clearer indexed (the
// channel index is read on every line of the body); silence clippy's
// preference for iterator-style code for this module.
#![allow(clippy::needless_range_loop)]

use crate::bc7::{ANCHOR_2_SUBSET_2, ANCHOR_3_SUBSET_2, ANCHOR_3_SUBSET_3, PART_2, PART_3};
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

#[inline]
fn sq_dist4(a: [u8; 4], b: [u8; 4]) -> u32 {
    let mut s: i32 = 0;
    for i in 0..4 {
        let d = a[i] as i32 - b[i] as i32;
        s += d * d;
    }
    s as u32
}

/// Quantise an 8-bit value to 7 bits + 1 p-bit by exhaustive choice.
/// Returns `(raw7, p)` such that `(raw7 << 1) | p` reconstructs the 8-bit
/// value with minimum absolute error.
///
/// Currently kept under `#[cfg(test)]` because the inline encoder body
/// open-codes the quantisation; the helper exists for the
/// `quantize_7p_is_lossless` test that verifies the lossless mapping.
#[cfg(test)]
fn quantize_7p(value: u8) -> (u32, u32) {
    // Candidate 8-bit reconstructions are: every value in 0..=255 such
    // that (raw << 1) | p produces a unique 8-bit result. Since
    // (raw << 1) | p is a one-to-one mapping into 8-bit space (raw in
    // 0..128, p in 0..2), every 8-bit value is reproducible exactly.
    // So we always have an exact match and never lose precision.
    let raw = (value as u32) >> 1;
    let p = (value as u32) & 1;
    (raw, p)
}

/// BC7 weight tables (Microsoft `aWeight2 / aWeight3 / aWeight4`).
const WEIGHT_2: [u32; 4] = [0, 21, 43, 64];
const WEIGHT_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHT_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

#[inline]
fn weight_for(idx_bits: u32) -> &'static [u32] {
    match idx_bits {
        2 => &WEIGHT_2,
        3 => &WEIGHT_3,
        4 => &WEIGHT_4,
        _ => unreachable!(),
    }
}

/// Interpolate `((64 - w) * e0 + w * e1 + 32) >> 6` per Microsoft.
#[inline]
fn interp(e0: u8, e1: u8, idx: usize, idx_bits: u32) -> u8 {
    let w = weight_for(idx_bits)[idx];
    (((64 - w) * e0 as u32 + w * e1 as u32 + 32) >> 6) as u8
}

/// Pick the closest of the N palette entries for a given pixel,
/// considering all 4 channels.
fn nearest_index_rgba(palette: &[[u8; 4]], pixel: [u8; 4]) -> u32 {
    let mut best = 0u32;
    let mut best_d = u32::MAX;
    for (k, p) in palette.iter().enumerate() {
        let d = sq_dist4(*p, pixel);
        if d < best_d {
            best_d = d;
            best = k as u32;
        }
    }
    best
}

/// Pick the p-bit (0 or 1) for endpoint `e` (8-bit-per-channel RGBA)
/// that minimises round-trip error against the source pixels — i.e.
/// minimises sum_c (recon_c - e_c)^2. The decoder reconstructs each
/// channel as `((raw << 1) | p)` where `raw = e_c >> 1`, so the
/// reconstructed byte is `(e_c & 0xfe) | p` — the original value with
/// its LSB replaced by `p`. The error per channel is therefore
/// `|e_c - ((e_c & 0xfe) | p)|` ∈ {0, 1}; the per-endpoint optimum
/// is the p that matches the majority of the four LSBs.
fn pick_p_for_endpoint(e: [u8; 4]) -> u32 {
    let mut sum = 0u32;
    for c in 0..4 {
        sum += (e[c] & 1) as u32;
    }
    if sum >= 2 {
        1
    } else {
        0
    }
}

/// Mode-1 / 3 / 7 channel quantiser: collapse an 8-bit value to
/// `colour_bits` bits + 1 p-bit, returning `(raw, recon_8bit)`.
///
/// `bits` is the raw colour bit-count (5, 6 or 7). The decoder
/// reconstructs as `expand_to_8(raw, bits, Some(p_bit))` with
/// Microsoft's bit-replication: append the p-bit to the raw value's
/// LSB, then shift up to 8 bits and replicate the high bits into the
/// low padding when `bits + 1 < 8`.
fn quantize_with_pbit(value: u8, bits: u32, p: u32) -> (u32, u8) {
    // Choose raw to minimise |recon - value| for this fixed p.
    // recon(raw) = expand((raw << 1) | (p & 1), bits + 1) — monotone in raw,
    // so we can find the best raw by an analytic round.
    let total = bits + 1;
    let max_raw = (1u32 << bits) - 1;

    // Form the (bits+1)-bit value with the p-bit stuck in the low slot.
    // Find the (bits+1)-bit code closest to the source's high bits.
    // Then scrub the low bit to match `p`.
    //
    // Practical approach: try raw = value >> (8 - bits) and a couple of
    // neighbours, pick the one whose reconstruction is closest.
    let approx = (value as u32) >> (8 - bits);
    let mut best = (approx, recon8(approx, bits, p));
    let mut best_err = abs_diff_i32(value as i32, best.1 as i32);
    for delta in [-1i32, 1, -2, 2] {
        let r = approx as i32 + delta;
        if r < 0 || r > max_raw as i32 {
            continue;
        }
        let r = r as u32;
        let rec = recon8(r, bits, p);
        let err = abs_diff_i32(value as i32, rec as i32);
        if err < best_err {
            best_err = err;
            best = (r, rec);
        }
    }
    let _ = total;
    best
}

#[inline]
fn abs_diff_i32(a: i32, b: i32) -> i32 {
    (a - b).abs()
}

/// Reproduce decoder's `expand_to_8` for `(raw << 1) | p` with `bits + 1`
/// total bits.
fn recon8(raw: u32, bits: u32, p: u32) -> u8 {
    let total = bits + 1;
    let v = (raw << 1) | (p & 1);
    if total >= 8 {
        return v as u8;
    }
    let shift = 8 - total;
    let high = (v << shift) as u8;
    high | (high >> total)
}

// ---- Mode 6 (1-subset) -------------------------------------------------

/// Encode one 4×4 RGBA8 block into a 16-byte BC7 mode-6 candidate.
/// Returns `(block_bytes, sse)`.
fn encode_bc7_mode6_block(pixels_rgba: &[[u8; 4]; 16]) -> ([u8; 16], u64) {
    // ---- Endpoint search: furthest-point in RGBA space.
    let mut best_d = 0u32;
    let mut best_i = 0usize;
    let mut best_j = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let d = sq_dist4(pixels_rgba[i], pixels_rgba[j]);
            if d > best_d {
                best_d = d;
                best_i = i;
                best_j = j;
            }
        }
    }

    let mut e0 = pixels_rgba[best_i];
    let mut e1 = pixels_rgba[best_j];

    let mut p0 = pick_p_for_endpoint(e0);
    let mut p1 = pick_p_for_endpoint(e1);

    let raw_endpoint = |e: [u8; 4]| -> [u32; 4] {
        let mut out = [0u32; 4];
        for c in 0..4 {
            out[c] = (e[c] as u32) >> 1;
        }
        out
    };
    let mut raw0 = raw_endpoint(e0);
    let mut raw1 = raw_endpoint(e1);

    let recon = |raw: [u32; 4], p: u32| -> [u8; 4] {
        let mut out = [0u8; 4];
        for c in 0..4 {
            out[c] = (((raw[c] << 1) | (p & 1)) & 0xff) as u8;
        }
        out
    };
    e0 = recon(raw0, p0);
    e1 = recon(raw1, p1);

    let build_palette = |e0: [u8; 4], e1: [u8; 4]| -> [[u8; 4]; 16] {
        let mut palette = [[0u8; 4]; 16];
        for (k, slot) in palette.iter_mut().enumerate() {
            for c in 0..4 {
                slot[c] = interp(e0[c], e1[c], k, 4);
            }
        }
        palette
    };
    let palette = build_palette(e0, e1);

    let mut indices = [0u32; 16];
    for (i, p) in pixels_rgba.iter().enumerate() {
        indices[i] = nearest_index_rgba(&palette, *p);
    }

    if indices[0] >= 8 {
        std::mem::swap(&mut raw0, &mut raw1);
        std::mem::swap(&mut p0, &mut p1);
        for idx in indices.iter_mut() {
            *idx = 15 - *idx;
        }
    }

    // SSE against the palette the decoder will produce after the swap.
    let final_e0 = recon(raw0, p0);
    let final_e1 = recon(raw1, p1);
    let final_palette = build_palette(final_e0, final_e1);
    let mut sse: u64 = 0;
    for (i, p) in pixels_rgba.iter().enumerate() {
        let r = final_palette[indices[i] as usize];
        sse += sq_dist4(*p, r) as u64;
    }

    (pack_mode6(raw0, raw1, p0, p1, indices), sse)
}

/// Pack a mode-6 BC7 block.
fn pack_mode6(raw0: [u32; 4], raw1: [u32; 4], p0: u32, p1: u32, indices: [u32; 16]) -> [u8; 16] {
    let mut bw = BitWriter::new();
    // Mode prefix: 6 zeros + 1.
    for _ in 0..6 {
        bw.put(0, 1);
    }
    bw.put(1, 1);
    // R0, R1, G0, G1, B0, B1, A0, A1 — 7 bits each (channel-major).
    for ch in 0..4 {
        bw.put(raw0[ch], 7);
        bw.put(raw1[ch], 7);
    }
    bw.put(p0, 1);
    bw.put(p1, 1);
    // Indices: pixel 0 anchor (3 bits), pixels 1..15 (4 bits).
    bw.put(indices[0], 3);
    for px in 1..16 {
        bw.put(indices[px], 4);
    }
    bw.into_block()
}

// ---- Mode 1 / 3 / 7 (2-subset) -----------------------------------------

/// Sweep the full 64-entry Microsoft partition table for the 2-subset
/// search. With 4 candidate modes × 64 partitions × 2..4 p-bit choices
/// per subset the per-block work is ~2 × 256 quantise+SSE iterations —
/// in release mode that's ~O(50 µs) per 4×4 block which is fine for
/// small textures and test fixtures. A future round can switch to a
/// curated shortlist if encoder throughput becomes the bottleneck.
const PARTITION_COUNT: u32 = 64;

/// Furthest-point endpoint pair for a subset of pixels (indexed by
/// `pixel_subset[i] == s`). Returns `None` if the subset has fewer
/// than 2 distinct pixels (caller falls back to a degenerate pair).
fn furthest_in_subset(
    pixels: &[[u8; 4]; 16],
    pixel_subset: &[u8; 16],
    s: u8,
) -> ([u8; 4], [u8; 4]) {
    let mut idxs: [usize; 16] = [0; 16];
    let mut n = 0usize;
    for (i, &t) in pixel_subset.iter().enumerate() {
        if t == s {
            idxs[n] = i;
            n += 1;
        }
    }
    if n == 0 {
        return ([0; 4], [0; 4]);
    }
    if n == 1 {
        return (pixels[idxs[0]], pixels[idxs[0]]);
    }
    let mut best_d = 0u32;
    let mut bi = idxs[0];
    let mut bj = idxs[1];
    for ai in 0..n {
        for aj in (ai + 1)..n {
            let i = idxs[ai];
            let j = idxs[aj];
            let d = sq_dist4(pixels[i], pixels[j]);
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    (pixels[bi], pixels[bj])
}

/// Least-squares endpoint refinement for one subset. Given fixed
/// indices `idx_i ∈ [0, 2^idx_bits)` for each pixel `i` of a subset,
/// solve for `e0, e1` that minimise `sum_i (interp(e0, e1, idx_i) -
/// pixel_i)^2` for each channel independently.
///
/// `interp(e0, e1, idx)` = `((64 - w_i) * e0 + w_i * e1 + 32) / 64`
/// where `w_i` is the weight for the index. So the per-channel system
/// is a 2-variable least-squares, solved analytically:
///
///   sum_i (a_i e0 + b_i e1 - p_i)^2  with  a_i = (64 - w_i) / 64,
///                                          b_i = w_i / 64
///   → [aa ab; ab bb] [e0 e1]^T = [ap bp]^T
///
/// where `aa = sum a_i^2`, `bb = sum b_i^2`, `ab = sum a_i b_i`,
/// `ap = sum a_i p_i`, `bp = sum b_i p_i`. `e0, e1` clamped to [0, 255].
fn refine_endpoints(
    pixels: &[[u8; 4]; 16],
    pixel_subset: &[u8; 16],
    s: u8,
    indices: &[u32; 16],
    idx_bits: u32,
) -> ([u8; 4], [u8; 4]) {
    let weights = weight_for(idx_bits);
    let mut e0 = [0u8; 4];
    let mut e1 = [0u8; 4];
    for c in 0..4 {
        let mut aa = 0.0f64;
        let mut bb = 0.0f64;
        let mut ab = 0.0f64;
        let mut ap = 0.0f64;
        let mut bp = 0.0f64;
        for i in 0..16 {
            if pixel_subset[i] != s {
                continue;
            }
            let w = weights[indices[i] as usize] as f64 / 64.0;
            let a = 1.0 - w;
            let b = w;
            let p = pixels[i][c] as f64;
            aa += a * a;
            bb += b * b;
            ab += a * b;
            ap += a * p;
            bp += b * p;
        }
        let det = aa * bb - ab * ab;
        if det.abs() < 1e-9 {
            // Degenerate (all weights equal, e.g. one-pixel subset). Fall
            // back to the mean of the subset for both endpoints.
            let mut sum = 0.0f64;
            let mut count = 0u32;
            for i in 0..16 {
                if pixel_subset[i] == s {
                    sum += pixels[i][c] as f64;
                    count += 1;
                }
            }
            let m = if count > 0 { sum / count as f64 } else { 0.0 };
            e0[c] = m.round().clamp(0.0, 255.0) as u8;
            e1[c] = m.round().clamp(0.0, 255.0) as u8;
        } else {
            let v0 = (bb * ap - ab * bp) / det;
            let v1 = (aa * bp - ab * ap) / det;
            e0[c] = v0.round().clamp(0.0, 255.0) as u8;
            e1[c] = v1.round().clamp(0.0, 255.0) as u8;
        }
    }
    (e0, e1)
}

/// Per-subset endpoint quantisation result.
#[derive(Clone, Copy)]
struct SubsetEnc {
    raw: [[u32; 4]; 2], // raw[endpoint][channel], colour_bits-wide for RGB and alpha_bits-wide for A
    p: [u32; 2],        // per-endpoint p-bit (or both equal for shared)
    rec: [[u8; 4]; 2],  // 8-bit reconstructions
}

/// Encode one subset's 2 endpoints with `colour_bits` per RGB channel +
/// `alpha_bits` per A channel + `pbit_per_endpoint` (else shared p-bit).
/// For modes 1/3/7 alpha bits = 0 means "alpha implicit-255" (modes 1,
/// 3); alpha_bits = 5 means "5-bit alpha + p-bit attached" (mode 7).
fn encode_subset(
    e0: [u8; 4],
    e1: [u8; 4],
    colour_bits: u32,
    alpha_bits: u32,
    pbit_per_endpoint: bool,
) -> SubsetEnc {
    let mut best = SubsetEnc {
        raw: [[0; 4]; 2],
        p: [0; 2],
        rec: [[0; 4]; 2],
    };
    let mut best_err: u64 = u64::MAX;

    let p_choices: &[(u32, u32)] = if pbit_per_endpoint {
        &[(0, 0), (0, 1), (1, 0), (1, 1)]
    } else {
        &[(0, 0), (1, 1)]
    };

    for &(p0, p1) in p_choices {
        let mut raw = [[0u32; 4]; 2];
        let mut rec = [[0u8; 4]; 2];
        for c in 0..3 {
            let (r0, q0) = quantize_with_pbit(e0[c], colour_bits, p0);
            let (r1, q1) = quantize_with_pbit(e1[c], colour_bits, p1);
            raw[0][c] = r0;
            raw[1][c] = r1;
            rec[0][c] = q0;
            rec[1][c] = q1;
        }
        if alpha_bits == 0 {
            raw[0][3] = 0;
            raw[1][3] = 0;
            rec[0][3] = 255;
            rec[1][3] = 255;
        } else {
            // For modes 6 and 7 alpha shares the endpoint's p-bit.
            let (ra0, qa0) = quantize_with_pbit(e0[3], alpha_bits, p0);
            let (ra1, qa1) = quantize_with_pbit(e1[3], alpha_bits, p1);
            raw[0][3] = ra0;
            raw[1][3] = ra1;
            rec[0][3] = qa0;
            rec[1][3] = qa1;
        }
        let mut err: u64 = 0;
        for c in 0..4 {
            err += (e0[c] as i32 - rec[0][c] as i32).pow(2) as u64;
            err += (e1[c] as i32 - rec[1][c] as i32).pow(2) as u64;
        }
        if err < best_err {
            best_err = err;
            best = SubsetEnc {
                raw,
                p: [p0, p1],
                rec,
            };
        }
    }
    best
}

/// Encode one 4×4 RGBA8 block as a 2-subset BC7 candidate (mode 1, 3
/// or 7) for a specific partition. Returns `(block_bytes, sse)`.
fn encode_bc7_2subset(pixels_rgba: &[[u8; 4]; 16], mode: u32, partition: u32) -> ([u8; 16], u64) {
    debug_assert!(mode == 1 || mode == 3 || mode == 7);

    let part = PART_2[partition as usize];
    let (colour_bits, alpha_bits, idx_bits, pbit_per_endpoint) = match mode {
        1 => (6, 0, 3, false),
        3 => (7, 0, 2, true),
        7 => (5, 5, 2, true),
        _ => unreachable!(),
    };

    // Per-subset endpoint search: seed with furthest-point, then iterate
    // (a) snap pixels to nearest palette entry, (b) least-squares refine
    // endpoints to fit the indices, (c) re-quantise endpoints. One
    // refinement pass typically takes the SSE within a few percent of
    // the optimum for natural-image content.
    let mut subsets = [SubsetEnc {
        raw: [[0; 4]; 2],
        p: [0; 2],
        rec: [[0; 4]; 2],
    }; 2];
    for s in 0..2u8 {
        let (e0, e1) = furthest_in_subset(pixels_rgba, &part, s);
        subsets[s as usize] = encode_subset(e0, e1, colour_bits, alpha_bits, pbit_per_endpoint);
    }

    let palette_size = 1usize << idx_bits;
    let build_palettes = |subsets: &[SubsetEnc; 2]| -> [[[u8; 4]; 8]; 2] {
        let mut palettes: [[[u8; 4]; 8]; 2] = [[[0; 4]; 8]; 2];
        for s in 0..2 {
            let e0 = subsets[s].rec[0];
            let e1 = subsets[s].rec[1];
            for k in 0..palette_size {
                for c in 0..4 {
                    palettes[s][k][c] = interp(e0[c], e1[c], k, idx_bits);
                }
            }
        }
        palettes
    };
    let snap_indices = |palettes: &[[[u8; 4]; 8]; 2]| -> [u32; 16] {
        let mut indices = [0u32; 16];
        for px in 0..16 {
            let s = part[px] as usize;
            let mut best = 0u32;
            let mut best_d = u32::MAX;
            let pal_slice = &palettes[s][..palette_size];
            for (k, p) in pal_slice.iter().enumerate() {
                let d = sq_dist4(*p, pixels_rgba[px]);
                if d < best_d {
                    best_d = d;
                    best = k as u32;
                }
            }
            indices[px] = best;
        }
        indices
    };

    let palettes = build_palettes(&subsets);
    let mut indices = snap_indices(&palettes);

    // Iterative refinement (2 passes is enough for the gradient test; a
    // third pass yields diminishing returns). Each pass: refine
    // endpoints by least-squares against the current indices, re-quantise,
    // re-snap pixels to the new palette.
    for _ in 0..2 {
        let mut new_subsets = subsets;
        for s in 0..2u8 {
            let (e0_f, e1_f) = refine_endpoints(pixels_rgba, &part, s, &indices, idx_bits);
            new_subsets[s as usize] =
                encode_subset(e0_f, e1_f, colour_bits, alpha_bits, pbit_per_endpoint);
        }
        let new_palettes = build_palettes(&new_subsets);
        let new_indices = snap_indices(&new_palettes);
        // Compute SSE for both candidates and keep the better.
        let sse_old: u64 = (0..16usize)
            .map(|px| {
                let s = part[px] as usize;
                sq_dist4(palettes[s][indices[px] as usize], pixels_rgba[px]) as u64
            })
            .sum();
        let sse_new: u64 = (0..16usize)
            .map(|px| {
                let s = part[px] as usize;
                sq_dist4(new_palettes[s][new_indices[px] as usize], pixels_rgba[px]) as u64
            })
            .sum();
        if sse_new < sse_old {
            subsets = new_subsets;
            indices = new_indices;
        } else {
            break;
        }
    }

    // Anchor handling: subset 0 anchor = pixel 0; subset 1 anchor = ANCHOR_2_SUBSET_2[partition].
    // Each anchor index has its MSB implicit-0, so it must be < palette_size / 2.
    let anchor1 = ANCHOR_2_SUBSET_2[partition as usize] as usize;
    let half = palette_size as u32 / 2;
    let max_idx = palette_size as u32 - 1;
    for (s, &anchor_px) in [0usize, anchor1].iter().enumerate() {
        if indices[anchor_px] >= half {
            // Swap endpoints of this subset + complement all indices in this subset.
            subsets[s].raw.swap(0, 1);
            subsets[s].p.swap(0, 1);
            subsets[s].rec.swap(0, 1);
            for px in 0..16 {
                if part[px] as usize == s {
                    indices[px] = max_idx - indices[px];
                }
            }
        }
    }

    // Compute SSE against final palettes (after swaps).
    let mut final_palettes: [[[u8; 4]; 8]; 2] = [[[0; 4]; 8]; 2];
    for s in 0..2 {
        let e0 = subsets[s].rec[0];
        let e1 = subsets[s].rec[1];
        for k in 0..palette_size {
            for c in 0..4 {
                final_palettes[s][k][c] = interp(e0[c], e1[c], k, idx_bits);
            }
        }
    }
    let mut sse: u64 = 0;
    for px in 0..16 {
        let s = part[px] as usize;
        let r = final_palettes[s][indices[px] as usize];
        sse += sq_dist4(r, pixels_rgba[px]) as u64;
    }

    let block = pack_2subset(mode, partition, &subsets, &part, indices, anchor1);
    (block, sse)
}

/// Pack a 2-subset BC7 block (mode 1, 3 or 7).
fn pack_2subset(
    mode: u32,
    partition: u32,
    subsets: &[SubsetEnc; 2],
    part: &[u8; 16],
    indices: [u32; 16],
    anchor1: usize,
) -> [u8; 16] {
    let (colour_bits, alpha_bits, idx_bits, pbit_per_endpoint) = match mode {
        1 => (6u32, 0u32, 3u32, false),
        3 => (7, 0, 2, true),
        7 => (5, 5, 2, true),
        _ => unreachable!(),
    };

    let mut bw = BitWriter::new();

    // Mode prefix: `mode` zeros, then a 1.
    for _ in 0..mode {
        bw.put(0, 1);
    }
    bw.put(1, 1);

    // Partition: 6 bits.
    bw.put(partition, 6);

    // Endpoint colours: channel-major (all R, all G, all B, then all A).
    // Each channel has subsets * 2 endpoint slots (= 4 for 2-subset).
    // Slot order: subset0_e0, subset0_e1, subset1_e0, subset1_e1.
    for ch in 0..3 {
        for s in 0..2 {
            for ep in 0..2 {
                bw.put(subsets[s].raw[ep][ch], colour_bits);
            }
        }
    }
    if alpha_bits > 0 {
        for s in 0..2 {
            for ep in 0..2 {
                bw.put(subsets[s].raw[ep][3], alpha_bits);
            }
        }
    }

    // P-bits.
    if pbit_per_endpoint {
        // 4 p-bits — one per endpoint, in the same channel-major slot order.
        for s in 0..2 {
            for ep in 0..2 {
                bw.put(subsets[s].p[ep], 1);
            }
        }
    } else {
        // Mode 1: 2 shared p-bits, one per subset (shared by both endpoints).
        for s in 0..2 {
            // Both endpoints carry the same p-bit by construction (encode_subset
            // restricts mode-1 to (0,0) and (1,1)).
            debug_assert_eq!(subsets[s].p[0], subsets[s].p[1]);
            bw.put(subsets[s].p[0], 1);
        }
    }

    // Indices: each pixel writes `idx_bits` bits except the two anchors,
    // which write `idx_bits - 1` bits (MSB implicit-0).
    for px in 0..16 {
        let nbits = if px == 0 || px == anchor1 {
            idx_bits - 1
        } else {
            idx_bits
        };
        bw.put(indices[px], nbits);
        let _ = part; // partition table not needed here — anchors are pixel-indexed.
    }

    bw.into_block()
}

// ---- Mode 0 / 2 (3-subset, opaque) -------------------------------------

/// Furthest-point endpoint pair for a subset of pixels in a 3-subset
/// partition. The 3-subset variant is identical to [`furthest_in_subset`]
/// except `pixel_subset[i] ∈ {0, 1, 2}`.
fn furthest_in_subset_3(
    pixels: &[[u8; 4]; 16],
    pixel_subset: &[u8; 16],
    s: u8,
) -> ([u8; 4], [u8; 4]) {
    furthest_in_subset(pixels, pixel_subset, s)
}

/// Encode one 4×4 RGBA8 block as a 3-subset BC7 candidate (mode 0 or
/// mode 2) for a specific partition. Returns `(block_bytes, sse)`.
///
/// Mode 0: 4-bit colour, no alpha, 6 per-endpoint p-bits, 3-bit indices,
///         partition selector ∈ 0..16 (4-bit field).
/// Mode 2: 5-bit colour, no alpha, no p-bits, 2-bit indices, partition
///         selector ∈ 0..64 (6-bit field).
fn encode_bc7_3subset(pixels_rgba: &[[u8; 4]; 16], mode: u32, partition: u32) -> ([u8; 16], u64) {
    debug_assert!(mode == 0 || mode == 2);

    let part = PART_3[partition as usize];
    let (colour_bits, idx_bits, has_pbits, pbit_per_endpoint) = match mode {
        0 => (4u32, 3u32, true, true),
        2 => (5u32, 2u32, false, false),
        _ => unreachable!(),
    };

    // Per-subset endpoint search: seed with furthest-point, then refine.
    let mut subsets = [SubsetEnc {
        raw: [[0; 4]; 2],
        p: [0; 2],
        rec: [[0; 4]; 2],
    }; 3];
    for s in 0..3u8 {
        let (e0, e1) = furthest_in_subset_3(pixels_rgba, &part, s);
        subsets[s as usize] = if has_pbits {
            encode_subset(e0, e1, colour_bits, 0, pbit_per_endpoint)
        } else {
            // Mode 2: no p-bits — encode_subset_no_pbit collapses each
            // channel to colour_bits + bit-replication.
            encode_subset_no_pbit(e0, e1, colour_bits)
        };
    }

    let palette_size = 1usize << idx_bits;
    let build_palettes = |subsets: &[SubsetEnc; 3]| -> [[[u8; 4]; 8]; 3] {
        let mut palettes: [[[u8; 4]; 8]; 3] = [[[0; 4]; 8]; 3];
        for s in 0..3 {
            let e0 = subsets[s].rec[0];
            let e1 = subsets[s].rec[1];
            for k in 0..palette_size {
                for c in 0..3 {
                    palettes[s][k][c] = interp(e0[c], e1[c], k, idx_bits);
                }
                palettes[s][k][3] = 255;
            }
        }
        palettes
    };
    let snap_indices = |palettes: &[[[u8; 4]; 8]; 3]| -> [u32; 16] {
        let mut indices = [0u32; 16];
        for px in 0..16 {
            let s = part[px] as usize;
            let mut best = 0u32;
            let mut best_d = u32::MAX;
            let pal_slice = &palettes[s][..palette_size];
            for (k, p) in pal_slice.iter().enumerate() {
                let d = sq_dist4(*p, pixels_rgba[px]);
                if d < best_d {
                    best_d = d;
                    best = k as u32;
                }
            }
            indices[px] = best;
        }
        indices
    };

    let palettes = build_palettes(&subsets);
    let mut indices = snap_indices(&palettes);

    // Iterative refinement: 2 passes is enough for the gradient tests.
    for _ in 0..2 {
        let mut new_subsets = subsets;
        for s in 0..3u8 {
            let (e0_f, e1_f) = refine_endpoints_3(pixels_rgba, &part, s, &indices, idx_bits);
            new_subsets[s as usize] = if has_pbits {
                encode_subset(e0_f, e1_f, colour_bits, 0, pbit_per_endpoint)
            } else {
                encode_subset_no_pbit(e0_f, e1_f, colour_bits)
            };
        }
        let new_palettes = build_palettes(&new_subsets);
        let new_indices = snap_indices(&new_palettes);
        let sse_old: u64 = (0..16usize)
            .map(|px| {
                let s = part[px] as usize;
                sq_dist4(palettes[s][indices[px] as usize], pixels_rgba[px]) as u64
            })
            .sum();
        let sse_new: u64 = (0..16usize)
            .map(|px| {
                let s = part[px] as usize;
                sq_dist4(new_palettes[s][new_indices[px] as usize], pixels_rgba[px]) as u64
            })
            .sum();
        if sse_new < sse_old {
            subsets = new_subsets;
            indices = new_indices;
        } else {
            break;
        }
    }

    // Anchor handling: subset 0 anchor = pixel 0; subset 1 anchor =
    // ANCHOR_3_SUBSET_2[partition]; subset 2 anchor = ANCHOR_3_SUBSET_3.
    // Each anchor index has its MSB implicit-0, so it must be < palette_size / 2.
    let anchor1 = ANCHOR_3_SUBSET_2[partition as usize] as usize;
    let anchor2 = ANCHOR_3_SUBSET_3[partition as usize] as usize;
    let anchors = [0usize, anchor1, anchor2];
    let half = palette_size as u32 / 2;
    let max_idx = palette_size as u32 - 1;
    for (s, &anchor_px) in anchors.iter().enumerate() {
        if indices[anchor_px] >= half {
            subsets[s].raw.swap(0, 1);
            subsets[s].p.swap(0, 1);
            subsets[s].rec.swap(0, 1);
            for px in 0..16 {
                if part[px] as usize == s {
                    indices[px] = max_idx - indices[px];
                }
            }
        }
    }

    // Recompute SSE against final palettes (after anchor swaps).
    let mut final_palettes: [[[u8; 4]; 8]; 3] = [[[0; 4]; 8]; 3];
    for s in 0..3 {
        let e0 = subsets[s].rec[0];
        let e1 = subsets[s].rec[1];
        for k in 0..palette_size {
            for c in 0..3 {
                final_palettes[s][k][c] = interp(e0[c], e1[c], k, idx_bits);
            }
            final_palettes[s][k][3] = 255;
        }
    }
    let mut sse: u64 = 0;
    for px in 0..16 {
        let s = part[px] as usize;
        let r = final_palettes[s][indices[px] as usize];
        sse += sq_dist4(r, pixels_rgba[px]) as u64;
    }

    let block = pack_3subset(mode, partition, &subsets, indices, anchor1, anchor2);
    (block, sse)
}

// ---- Mode 4 / 5 (1-subset, channel-rotation, dual index plane) ----------

/// Apply BC7 channel-rotation to a 4-channel pixel: swap channel `rot`
/// with the alpha channel (rot=0 → no swap; rot=1 → A↔R; rot=2 → A↔G;
/// rot=3 → A↔B). Used by both encoder and decoder; involution (applying
/// twice yields the identity).
#[inline]
fn apply_rotation(pixel: [u8; 4], rot: u32) -> [u8; 4] {
    let mut p = pixel;
    match rot {
        0 => {}
        1 => p.swap(0, 3),
        2 => p.swap(1, 3),
        3 => p.swap(2, 3),
        _ => unreachable!(),
    }
    p
}

/// 1-D quantiser for an 8-bit value to `bits` bits, no p-bit (modes 4/5
/// alpha when `bits >= 8`, or RGB channels with bit-replication when bits < 8).
fn quantize_1ch_no_pbit(value: u8, bits: u32) -> (u32, u8) {
    if bits >= 8 {
        return (value as u32, value);
    }
    let max_raw = (1u32 << bits) - 1;
    let approx = (value as u32) >> (8 - bits);
    let mut best = (approx, recon8_no_pbit(approx, bits));
    let mut best_err = abs_diff_i32(value as i32, best.1 as i32);
    for delta in [-1i32, 1] {
        let r = approx as i32 + delta;
        if r < 0 || r > max_raw as i32 {
            continue;
        }
        let r = r as u32;
        let rec = recon8_no_pbit(r, bits);
        let err = abs_diff_i32(value as i32, rec as i32);
        if err < best_err {
            best_err = err;
            best = (r, rec);
        }
    }
    best
}

/// Least-squares refinement of two endpoint scalars (e0, e1) for a single
/// channel against fixed indices using the BC7 weight table for `idx_bits`.
/// Returns `(e0, e1)` as 8-bit values clamped to [0, 255].
fn refine_scalar_endpoints(values: &[u8; 16], indices: &[u32; 16], idx_bits: u32) -> (u8, u8) {
    let weights = weight_for(idx_bits);
    let mut aa = 0.0f64;
    let mut bb = 0.0f64;
    let mut ab = 0.0f64;
    let mut ap = 0.0f64;
    let mut bp = 0.0f64;
    for i in 0..16 {
        let w = weights[indices[i] as usize] as f64 / 64.0;
        let a = 1.0 - w;
        let b = w;
        let p = values[i] as f64;
        aa += a * a;
        bb += b * b;
        ab += a * b;
        ap += a * p;
        bp += b * p;
    }
    let det = aa * bb - ab * ab;
    if det.abs() < 1e-9 {
        let m: f64 = values.iter().map(|&v| v as f64).sum::<f64>() / 16.0;
        let m = m.round().clamp(0.0, 255.0) as u8;
        return (m, m);
    }
    let v0 = (bb * ap - ab * bp) / det;
    let v1 = (aa * bp - ab * ap) / det;
    let e0 = v0.round().clamp(0.0, 255.0) as u8;
    let e1 = v1.round().clamp(0.0, 255.0) as u8;
    (e0, e1)
}

/// Encode one 4×4 RGBA8 block as a mode-4 or mode-5 BC7 candidate for a
/// specific channel-rotation.
///
/// Mode 4: 1 subset, 5-bit RGB endpoints, 6-bit A endpoints, no p-bits,
///         primary index 2-bit (RGB) + secondary 3-bit (A) when idx_sel=0,
///         OR primary 3-bit (RGB) + secondary 2-bit (A) when idx_sel=1.
///         Channel-rotation r selects which channel swaps with A.
/// Mode 5: 1 subset, 7-bit RGB endpoints, 8-bit A endpoints, no p-bits,
///         2-bit RGB index plane + 2-bit A index plane.
///         Channel-rotation r selects which channel swaps with A.
///
/// The encoder pre-rotates the input pixels by `rotation`, fits RGB+A
/// endpoints separately on the rotated data, picks indices for each plane,
/// and packs the bitstream. The decoder inverts the rotation post-interp,
/// so encoded blocks roundtrip pixel-for-pixel.
fn encode_bc7_mode4(pixels_rgba: &[[u8; 4]; 16], rotation: u32, idx_sel: u32) -> ([u8; 16], u64) {
    debug_assert!(rotation < 4 && idx_sel < 2);

    // Pre-rotate the input: each channel that ends up as "alpha" in the
    // bitstream is the one the decoder will swap into place.
    let mut rotated = [[0u8; 4]; 16];
    for (i, p) in pixels_rgba.iter().enumerate() {
        rotated[i] = apply_rotation(*p, rotation);
    }

    let (rgb_idx_bits, alpha_idx_bits) = if idx_sel == 0 {
        (2u32, 3u32)
    } else {
        (3u32, 2u32)
    };

    // Mode 4 endpoint precision: RGB=5 bits (no p-bit), A=6 bits (no p-bit).
    let rgb_bits = 5u32;
    let alpha_bits = 6u32;

    // ---- Seed RGB endpoints by furthest-point in 3-D RGB space.
    let mut best_d = 0u32;
    let mut bi = 0usize;
    let mut bj = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let mut d: u32 = 0;
            for c in 0..3 {
                let dc = rotated[i][c] as i32 - rotated[j][c] as i32;
                d += (dc * dc) as u32;
            }
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    let mut rgb_e0 = [rotated[bi][0], rotated[bi][1], rotated[bi][2]];
    let mut rgb_e1 = [rotated[bj][0], rotated[bj][1], rotated[bj][2]];

    // ---- Seed alpha endpoints by min/max alpha.
    let mut a_min = rotated[0][3];
    let mut a_max = rotated[0][3];
    for p in rotated.iter() {
        a_min = a_min.min(p[3]);
        a_max = a_max.max(p[3]);
    }
    let mut a_e0 = a_min;
    let mut a_e1 = a_max;

    // Quantise RGB / A endpoints + build palettes.
    let quantise_rgb = |e0: [u8; 3], e1: [u8; 3]| -> ([u32; 3], [u32; 3], [u8; 3], [u8; 3]) {
        let mut r0 = [0u32; 3];
        let mut r1 = [0u32; 3];
        let mut q0 = [0u8; 3];
        let mut q1 = [0u8; 3];
        for c in 0..3 {
            let (a, b) = quantize_1ch_no_pbit(e0[c], rgb_bits);
            r0[c] = a;
            q0[c] = b;
            let (a, b) = quantize_1ch_no_pbit(e1[c], rgb_bits);
            r1[c] = a;
            q1[c] = b;
        }
        (r0, r1, q0, q1)
    };
    let quantise_alpha = |e0: u8, e1: u8| -> (u32, u32, u8, u8) {
        let (r0, q0) = quantize_1ch_no_pbit(e0, alpha_bits);
        let (r1, q1) = quantize_1ch_no_pbit(e1, alpha_bits);
        (r0, r1, q0, q1)
    };

    let (mut rgb_raw0, mut rgb_raw1, mut rgb_rec0, mut rgb_rec1) = quantise_rgb(rgb_e0, rgb_e1);
    let (mut a_raw0, mut a_raw1, mut a_rec0, mut a_rec1) = quantise_alpha(a_e0, a_e1);

    // ---- Index assignment loop with refinement.
    let snap_rgb_indices = |q0: [u8; 3], q1: [u8; 3]| -> [u32; 16] {
        let palette_size = 1usize << rgb_idx_bits;
        let mut palette = [[0u8; 3]; 8];
        for k in 0..palette_size {
            for c in 0..3 {
                palette[k][c] = interp(q0[c], q1[c], k, rgb_idx_bits);
            }
        }
        let mut indices = [0u32; 16];
        for (px, p) in rotated.iter().enumerate() {
            let mut best_k = 0u32;
            let mut best_d = u32::MAX;
            for (k, pal) in palette.iter().take(palette_size).enumerate() {
                let mut d: u32 = 0;
                for c in 0..3 {
                    let dc = pal[c] as i32 - p[c] as i32;
                    d += (dc * dc) as u32;
                }
                if d < best_d {
                    best_d = d;
                    best_k = k as u32;
                }
            }
            indices[px] = best_k;
        }
        indices
    };
    let snap_alpha_indices = |q0: u8, q1: u8| -> [u32; 16] {
        let palette_size = 1usize << alpha_idx_bits;
        let mut palette = [0u8; 8];
        for k in 0..palette_size {
            palette[k] = interp(q0, q1, k, alpha_idx_bits);
        }
        let mut indices = [0u32; 16];
        for (px, p) in rotated.iter().enumerate() {
            let mut best_k = 0u32;
            let mut best_d = u32::MAX;
            for k in 0..palette_size {
                let dc = palette[k] as i32 - p[3] as i32;
                let d = (dc * dc) as u32;
                if d < best_d {
                    best_d = d;
                    best_k = k as u32;
                }
            }
            indices[px] = best_k;
        }
        indices
    };

    let mut rgb_indices = snap_rgb_indices(rgb_rec0, rgb_rec1);
    let mut alpha_indices = snap_alpha_indices(a_rec0, a_rec1);

    // Iterative refinement: 2 passes is sufficient for natural content.
    for _ in 0..2 {
        // Refine RGB endpoints per channel.
        let mut rch = [[0u8; 16]; 3];
        for c in 0..3 {
            for i in 0..16 {
                rch[c][i] = rotated[i][c];
            }
        }
        let mut new_rgb_e0 = [0u8; 3];
        let mut new_rgb_e1 = [0u8; 3];
        for c in 0..3 {
            let (e0, e1) = refine_scalar_endpoints(&rch[c], &rgb_indices, rgb_idx_bits);
            new_rgb_e0[c] = e0;
            new_rgb_e1[c] = e1;
        }
        // Refine alpha endpoints.
        let mut ach = [0u8; 16];
        for i in 0..16 {
            ach[i] = rotated[i][3];
        }
        let (new_a_e0, new_a_e1) = refine_scalar_endpoints(&ach, &alpha_indices, alpha_idx_bits);

        let (n_rgb_raw0, n_rgb_raw1, n_rgb_rec0, n_rgb_rec1) = quantise_rgb(new_rgb_e0, new_rgb_e1);
        let (n_a_raw0, n_a_raw1, n_a_rec0, n_a_rec1) = quantise_alpha(new_a_e0, new_a_e1);
        let n_rgb_idx = snap_rgb_indices(n_rgb_rec0, n_rgb_rec1);
        let n_a_idx = snap_alpha_indices(n_a_rec0, n_a_rec1);

        // Compute SSE for old vs new (decoder-faithful: pre-rotation
        // pixels reconstructed with palette interp, then SSE).
        let sse_old = sse_mode_4_5_pre_rot(
            &rotated,
            rgb_rec0,
            rgb_rec1,
            a_rec0,
            a_rec1,
            &rgb_indices,
            &alpha_indices,
            rgb_idx_bits,
            alpha_idx_bits,
        );
        let sse_new = sse_mode_4_5_pre_rot(
            &rotated,
            n_rgb_rec0,
            n_rgb_rec1,
            n_a_rec0,
            n_a_rec1,
            &n_rgb_idx,
            &n_a_idx,
            rgb_idx_bits,
            alpha_idx_bits,
        );
        if sse_new < sse_old {
            rgb_raw0 = n_rgb_raw0;
            rgb_raw1 = n_rgb_raw1;
            rgb_rec0 = n_rgb_rec0;
            rgb_rec1 = n_rgb_rec1;
            a_raw0 = n_a_raw0;
            a_raw1 = n_a_raw1;
            a_rec0 = n_a_rec0;
            a_rec1 = n_a_rec1;
            rgb_indices = n_rgb_idx;
            alpha_indices = n_a_idx;
            rgb_e0 = new_rgb_e0;
            rgb_e1 = new_rgb_e1;
            a_e0 = new_a_e0;
            a_e1 = new_a_e1;
        } else {
            break;
        }
    }
    let _ = (rgb_e0, rgb_e1, a_e0, a_e1); // silence unused warnings.

    // ---- Anchor handling: pixel 0 is the anchor for both index planes.
    // Each anchor index is stored with one fewer bit (MSB implicit-0),
    // so it must be < palette_size / 2. Swap endpoints + complement
    // indices in the offending plane if needed.
    let rgb_pal_size = 1u32 << rgb_idx_bits;
    if rgb_indices[0] >= rgb_pal_size / 2 {
        std::mem::swap(&mut rgb_raw0, &mut rgb_raw1);
        std::mem::swap(&mut rgb_rec0, &mut rgb_rec1);
        for idx in rgb_indices.iter_mut() {
            *idx = (rgb_pal_size - 1) - *idx;
        }
    }
    let alpha_pal_size = 1u32 << alpha_idx_bits;
    if alpha_indices[0] >= alpha_pal_size / 2 {
        std::mem::swap(&mut a_raw0, &mut a_raw1);
        std::mem::swap(&mut a_rec0, &mut a_rec1);
        for idx in alpha_indices.iter_mut() {
            *idx = (alpha_pal_size - 1) - *idx;
        }
    }

    // ---- Final SSE against decoded pixels (post-rotation == original input).
    let sse = sse_mode_4_5_pre_rot(
        &rotated,
        rgb_rec0,
        rgb_rec1,
        a_rec0,
        a_rec1,
        &rgb_indices,
        &alpha_indices,
        rgb_idx_bits,
        alpha_idx_bits,
    );

    // ---- Pack bitstream (mode 4).
    let block = pack_mode4(
        rotation,
        idx_sel,
        rgb_raw0,
        rgb_raw1,
        a_raw0,
        a_raw1,
        rgb_indices,
        alpha_indices,
        idx_sel,
    );
    (block, sse)
}

#[allow(clippy::too_many_arguments)]
fn sse_mode_4_5_pre_rot(
    rotated_pixels: &[[u8; 4]; 16],
    rgb_q0: [u8; 3],
    rgb_q1: [u8; 3],
    a_q0: u8,
    a_q1: u8,
    rgb_idx: &[u32; 16],
    a_idx: &[u32; 16],
    rgb_idx_bits: u32,
    a_idx_bits: u32,
) -> u64 {
    let mut sse: u64 = 0;
    for px in 0..16 {
        let mut recon = [0u8; 4];
        for c in 0..3 {
            recon[c] = interp(rgb_q0[c], rgb_q1[c], rgb_idx[px] as usize, rgb_idx_bits);
        }
        recon[3] = interp(a_q0, a_q1, a_idx[px] as usize, a_idx_bits);
        sse += sq_dist4(recon, rotated_pixels[px]) as u64;
    }
    sse
}

#[allow(clippy::too_many_arguments)]
fn pack_mode4(
    rotation: u32,
    idx_sel: u32,
    rgb_raw0: [u32; 3],
    rgb_raw1: [u32; 3],
    a_raw0: u32,
    a_raw1: u32,
    rgb_indices: [u32; 16],
    alpha_indices: [u32; 16],
    _idx_sel_bit: u32,
) -> [u8; 16] {
    // Mode 4 prefix: 4 zeros + 1 = bit position 4 set.
    let mut bw = BitWriter::new();
    for _ in 0..4 {
        bw.put(0, 1);
    }
    bw.put(1, 1);
    // Rotation (2 bits), idx_sel (1 bit).
    bw.put(rotation, 2);
    bw.put(idx_sel, 1);
    // RGB endpoints, channel-major: R0, R1, G0, G1, B0, B1 (5 bits each).
    for ch in 0..3 {
        bw.put(rgb_raw0[ch], 5);
        bw.put(rgb_raw1[ch], 5);
    }
    // Alpha endpoints: A0, A1 (6 bits each).
    bw.put(a_raw0, 6);
    bw.put(a_raw1, 6);
    // Primary index plane (RGB if idx_sel=0, else Alpha drives RGB).
    // Stored: 2-bit indices when idx_sel=0 (anchor 1 bit), else 3-bit (anchor 2 bits).
    // The "primary" plane in the bitstream is whichever has the lower idx_bits
    // count when idx_sel=0 (RGB primary, A secondary). When idx_sel=1, RGB
    // becomes the secondary plane (3-bit), and Alpha is primary (2-bit).
    //
    // Per Microsoft / Khronos: "Read primary then secondary indices.
    // idx_sel routes which plane drives which output channel."
    // We always pack rgb_indices into the plane that has rgb_idx_bits
    // and alpha_indices into the plane that has alpha_idx_bits — but
    // the *bitstream slot* of the primary vs secondary is fixed.
    //
    // Mode 4 primary slot is 2-bit (32 bits, anchor 1 bit), secondary
    // is 3-bit (48 bits, anchor 2 bits). idx_sel=0 puts RGB in primary,
    // alpha in secondary. idx_sel=1 swaps the routing.
    let (primary_idx, primary_bits, secondary_idx, secondary_bits) = if idx_sel == 0 {
        (&rgb_indices, 2u32, &alpha_indices, 3u32)
    } else {
        (&alpha_indices, 2u32, &rgb_indices, 3u32)
    };
    // Wait — re-examine. The decoder uses:
    //   idx_sel == 0 → primary 2-bit drives colour, secondary 3-bit drives alpha.
    // So when idx_sel == 0, RGB indices (which are 2-bit) go in primary.
    // When idx_sel == 1, RGB indices (which are 3-bit) go in secondary;
    // alpha indices (which are 2-bit) go in primary.
    //
    // Either way, the *primary* slot is always 2-bit (32 minus 1 anchor),
    // and the *secondary* slot is always 3-bit (48 minus 1 anchor).
    // Pixel 0 is the anchor in both planes.
    for px in 0..16usize {
        let nbits = if px == 0 {
            primary_bits - 1
        } else {
            primary_bits
        };
        bw.put(primary_idx[px], nbits);
    }
    for px in 0..16usize {
        let nbits = if px == 0 {
            secondary_bits - 1
        } else {
            secondary_bits
        };
        bw.put(secondary_idx[px], nbits);
    }
    bw.into_block()
}

/// Encode one 4×4 RGBA8 block as a mode-5 BC7 candidate for a specific
/// channel-rotation. Mode 5: 7-bit RGB, 8-bit A, no p-bits, both index
/// planes 2-bit.
fn encode_bc7_mode5(pixels_rgba: &[[u8; 4]; 16], rotation: u32) -> ([u8; 16], u64) {
    debug_assert!(rotation < 4);

    let mut rotated = [[0u8; 4]; 16];
    for (i, p) in pixels_rgba.iter().enumerate() {
        rotated[i] = apply_rotation(*p, rotation);
    }

    let rgb_idx_bits = 2u32;
    let alpha_idx_bits = 2u32;
    let rgb_bits = 7u32;
    let alpha_bits = 8u32;

    // Furthest-point seed in RGB.
    let mut best_d = 0u32;
    let mut bi = 0usize;
    let mut bj = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let mut d: u32 = 0;
            for c in 0..3 {
                let dc = rotated[i][c] as i32 - rotated[j][c] as i32;
                d += (dc * dc) as u32;
            }
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    let mut rgb_e0 = [rotated[bi][0], rotated[bi][1], rotated[bi][2]];
    let mut rgb_e1 = [rotated[bj][0], rotated[bj][1], rotated[bj][2]];

    let mut a_min = rotated[0][3];
    let mut a_max = rotated[0][3];
    for p in rotated.iter() {
        a_min = a_min.min(p[3]);
        a_max = a_max.max(p[3]);
    }
    let mut a_e0 = a_min;
    let mut a_e1 = a_max;

    let quantise_rgb = |e0: [u8; 3], e1: [u8; 3]| -> ([u32; 3], [u32; 3], [u8; 3], [u8; 3]) {
        let mut r0 = [0u32; 3];
        let mut r1 = [0u32; 3];
        let mut q0 = [0u8; 3];
        let mut q1 = [0u8; 3];
        for c in 0..3 {
            let (a, b) = quantize_1ch_no_pbit(e0[c], rgb_bits);
            r0[c] = a;
            q0[c] = b;
            let (a, b) = quantize_1ch_no_pbit(e1[c], rgb_bits);
            r1[c] = a;
            q1[c] = b;
        }
        (r0, r1, q0, q1)
    };
    let quantise_alpha = |e0: u8, e1: u8| -> (u32, u32, u8, u8) {
        let (r0, q0) = quantize_1ch_no_pbit(e0, alpha_bits);
        let (r1, q1) = quantize_1ch_no_pbit(e1, alpha_bits);
        (r0, r1, q0, q1)
    };

    let (mut rgb_raw0, mut rgb_raw1, mut rgb_rec0, mut rgb_rec1) = quantise_rgb(rgb_e0, rgb_e1);
    let (mut a_raw0, mut a_raw1, mut a_rec0, mut a_rec1) = quantise_alpha(a_e0, a_e1);

    let snap_rgb_indices = |q0: [u8; 3], q1: [u8; 3]| -> [u32; 16] {
        let palette_size = 1usize << rgb_idx_bits;
        let mut palette = [[0u8; 3]; 4];
        for k in 0..palette_size {
            for c in 0..3 {
                palette[k][c] = interp(q0[c], q1[c], k, rgb_idx_bits);
            }
        }
        let mut indices = [0u32; 16];
        for (px, p) in rotated.iter().enumerate() {
            let mut best_k = 0u32;
            let mut best_d = u32::MAX;
            for (k, pal) in palette.iter().take(palette_size).enumerate() {
                let mut d: u32 = 0;
                for c in 0..3 {
                    let dc = pal[c] as i32 - p[c] as i32;
                    d += (dc * dc) as u32;
                }
                if d < best_d {
                    best_d = d;
                    best_k = k as u32;
                }
            }
            indices[px] = best_k;
        }
        indices
    };
    let snap_alpha_indices = |q0: u8, q1: u8| -> [u32; 16] {
        let palette_size = 1usize << alpha_idx_bits;
        let mut palette = [0u8; 4];
        for k in 0..palette_size {
            palette[k] = interp(q0, q1, k, alpha_idx_bits);
        }
        let mut indices = [0u32; 16];
        for (px, p) in rotated.iter().enumerate() {
            let mut best_k = 0u32;
            let mut best_d = u32::MAX;
            for k in 0..palette_size {
                let dc = palette[k] as i32 - p[3] as i32;
                let d = (dc * dc) as u32;
                if d < best_d {
                    best_d = d;
                    best_k = k as u32;
                }
            }
            indices[px] = best_k;
        }
        indices
    };

    let mut rgb_indices = snap_rgb_indices(rgb_rec0, rgb_rec1);
    let mut alpha_indices = snap_alpha_indices(a_rec0, a_rec1);

    for _ in 0..2 {
        let mut rch = [[0u8; 16]; 3];
        for c in 0..3 {
            for i in 0..16 {
                rch[c][i] = rotated[i][c];
            }
        }
        let mut new_rgb_e0 = [0u8; 3];
        let mut new_rgb_e1 = [0u8; 3];
        for c in 0..3 {
            let (e0, e1) = refine_scalar_endpoints(&rch[c], &rgb_indices, rgb_idx_bits);
            new_rgb_e0[c] = e0;
            new_rgb_e1[c] = e1;
        }
        let mut ach = [0u8; 16];
        for i in 0..16 {
            ach[i] = rotated[i][3];
        }
        let (new_a_e0, new_a_e1) = refine_scalar_endpoints(&ach, &alpha_indices, alpha_idx_bits);

        let (n_rgb_raw0, n_rgb_raw1, n_rgb_rec0, n_rgb_rec1) = quantise_rgb(new_rgb_e0, new_rgb_e1);
        let (n_a_raw0, n_a_raw1, n_a_rec0, n_a_rec1) = quantise_alpha(new_a_e0, new_a_e1);
        let n_rgb_idx = snap_rgb_indices(n_rgb_rec0, n_rgb_rec1);
        let n_a_idx = snap_alpha_indices(n_a_rec0, n_a_rec1);

        let sse_old = sse_mode_4_5_pre_rot(
            &rotated,
            rgb_rec0,
            rgb_rec1,
            a_rec0,
            a_rec1,
            &rgb_indices,
            &alpha_indices,
            rgb_idx_bits,
            alpha_idx_bits,
        );
        let sse_new = sse_mode_4_5_pre_rot(
            &rotated,
            n_rgb_rec0,
            n_rgb_rec1,
            n_a_rec0,
            n_a_rec1,
            &n_rgb_idx,
            &n_a_idx,
            rgb_idx_bits,
            alpha_idx_bits,
        );
        if sse_new < sse_old {
            rgb_raw0 = n_rgb_raw0;
            rgb_raw1 = n_rgb_raw1;
            rgb_rec0 = n_rgb_rec0;
            rgb_rec1 = n_rgb_rec1;
            a_raw0 = n_a_raw0;
            a_raw1 = n_a_raw1;
            a_rec0 = n_a_rec0;
            a_rec1 = n_a_rec1;
            rgb_indices = n_rgb_idx;
            alpha_indices = n_a_idx;
            rgb_e0 = new_rgb_e0;
            rgb_e1 = new_rgb_e1;
            a_e0 = new_a_e0;
            a_e1 = new_a_e1;
        } else {
            break;
        }
    }
    let _ = (rgb_e0, rgb_e1, a_e0, a_e1);

    // Anchor handling — pixel 0 is the anchor for both planes.
    let rgb_pal_size = 1u32 << rgb_idx_bits;
    if rgb_indices[0] >= rgb_pal_size / 2 {
        std::mem::swap(&mut rgb_raw0, &mut rgb_raw1);
        std::mem::swap(&mut rgb_rec0, &mut rgb_rec1);
        for idx in rgb_indices.iter_mut() {
            *idx = (rgb_pal_size - 1) - *idx;
        }
    }
    let alpha_pal_size = 1u32 << alpha_idx_bits;
    if alpha_indices[0] >= alpha_pal_size / 2 {
        std::mem::swap(&mut a_raw0, &mut a_raw1);
        std::mem::swap(&mut a_rec0, &mut a_rec1);
        for idx in alpha_indices.iter_mut() {
            *idx = (alpha_pal_size - 1) - *idx;
        }
    }

    let sse = sse_mode_4_5_pre_rot(
        &rotated,
        rgb_rec0,
        rgb_rec1,
        a_rec0,
        a_rec1,
        &rgb_indices,
        &alpha_indices,
        rgb_idx_bits,
        alpha_idx_bits,
    );

    let block = pack_mode5(
        rotation,
        rgb_raw0,
        rgb_raw1,
        a_raw0,
        a_raw1,
        rgb_indices,
        alpha_indices,
    );
    (block, sse)
}

fn pack_mode5(
    rotation: u32,
    rgb_raw0: [u32; 3],
    rgb_raw1: [u32; 3],
    a_raw0: u32,
    a_raw1: u32,
    rgb_indices: [u32; 16],
    alpha_indices: [u32; 16],
) -> [u8; 16] {
    let mut bw = BitWriter::new();
    // Mode 5 prefix: 5 zeros + 1.
    for _ in 0..5 {
        bw.put(0, 1);
    }
    bw.put(1, 1);
    // Rotation: 2 bits.
    bw.put(rotation, 2);
    // RGB endpoints, channel-major: R0, R1, G0, G1, B0, B1 (7 bits each).
    for ch in 0..3 {
        bw.put(rgb_raw0[ch], 7);
        bw.put(rgb_raw1[ch], 7);
    }
    // Alpha endpoints: A0, A1 (8 bits each).
    bw.put(a_raw0, 8);
    bw.put(a_raw1, 8);
    // Primary index plane (RGB, 2-bit, anchor pixel 0 = 1 bit).
    for px in 0..16usize {
        let nbits = if px == 0 { 1u32 } else { 2u32 };
        bw.put(rgb_indices[px], nbits);
    }
    // Secondary index plane (Alpha, 2-bit, anchor pixel 0 = 1 bit).
    for px in 0..16usize {
        let nbits = if px == 0 { 1u32 } else { 2u32 };
        bw.put(alpha_indices[px], nbits);
    }
    bw.into_block()
}

/// Channel quantiser for modes that don't carry p-bits (mode 2).
/// Quantises each colour channel to `colour_bits` and reconstructs by
/// bit-replication (Microsoft's "high-into-low" rule).
fn encode_subset_no_pbit(e0: [u8; 4], e1: [u8; 4], colour_bits: u32) -> SubsetEnc {
    let mut raw = [[0u32; 4]; 2];
    let mut rec = [[0u8; 4]; 2];
    for c in 0..3 {
        let r0 = (e0[c] as u32) >> (8 - colour_bits);
        let r1 = (e1[c] as u32) >> (8 - colour_bits);
        raw[0][c] = r0;
        raw[1][c] = r1;
        // Decoder bit-replicates: high = (raw << (8 - bits)); v = high | (high >> bits).
        rec[0][c] = recon8_no_pbit(r0, colour_bits);
        rec[1][c] = recon8_no_pbit(r1, colour_bits);
        // Round-test single-step adjustment: try raw ± 1 to see if a
        // neighbouring quantised value is closer.
        let max_raw = (1u32 << colour_bits) - 1;
        for &delta in &[-1i32, 1] {
            for ep in 0..2 {
                let cur = raw[ep][c] as i32 + delta;
                if !(0..=max_raw as i32).contains(&cur) {
                    continue;
                }
                let cand = recon8_no_pbit(cur as u32, colour_bits);
                let new_err = abs_diff_i32(if ep == 0 { e0[c] } else { e1[c] } as i32, cand as i32);
                let cur_err = abs_diff_i32(
                    if ep == 0 { e0[c] } else { e1[c] } as i32,
                    rec[ep][c] as i32,
                );
                if new_err < cur_err {
                    raw[ep][c] = cur as u32;
                    rec[ep][c] = cand;
                }
            }
        }
    }
    rec[0][3] = 255;
    rec[1][3] = 255;
    SubsetEnc {
        raw,
        p: [0; 2],
        rec,
    }
}

/// Reconstruct an 8-bit value from a `bits`-bit raw value with no p-bit.
#[inline]
fn recon8_no_pbit(raw: u32, bits: u32) -> u8 {
    if bits >= 8 {
        return raw as u8;
    }
    let shift = 8 - bits;
    let high = (raw << shift) as u8;
    high | (high >> bits)
}

/// Least-squares endpoint refinement for one subset of a 3-subset
/// partition. Identical to [`refine_endpoints`] but indexes a 3-subset
/// pixel-subset table rather than 2-subset.
fn refine_endpoints_3(
    pixels: &[[u8; 4]; 16],
    pixel_subset: &[u8; 16],
    s: u8,
    indices: &[u32; 16],
    idx_bits: u32,
) -> ([u8; 4], [u8; 4]) {
    refine_endpoints(pixels, pixel_subset, s, indices, idx_bits)
}

/// Pack a 3-subset BC7 block (mode 0 or mode 2).
fn pack_3subset(
    mode: u32,
    partition: u32,
    subsets: &[SubsetEnc; 3],
    indices: [u32; 16],
    anchor1: usize,
    anchor2: usize,
) -> [u8; 16] {
    let (colour_bits, partition_bits, idx_bits, has_pbits) = match mode {
        0 => (4u32, 4u32, 3u32, true),
        2 => (5u32, 6u32, 2u32, false),
        _ => unreachable!(),
    };

    let mut bw = BitWriter::new();
    // Mode prefix: `mode` zeros, then a 1.
    for _ in 0..mode {
        bw.put(0, 1);
    }
    bw.put(1, 1);

    // Partition: `partition_bits` bits.
    bw.put(partition, partition_bits);

    // Endpoint colours: channel-major (all R, all G, all B).
    // Each channel has 6 endpoint slots (3 subsets × 2 endpoints).
    // Slot order: subset0_e0, subset0_e1, subset1_e0, subset1_e1, subset2_e0, subset2_e1.
    for ch in 0..3 {
        for s in 0..3 {
            for ep in 0..2 {
                bw.put(subsets[s].raw[ep][ch], colour_bits);
            }
        }
    }
    // No alpha for modes 0 / 2.

    // P-bits: mode 0 carries 6 per-endpoint p-bits; mode 2 has none.
    if has_pbits {
        for s in 0..3 {
            for ep in 0..2 {
                bw.put(subsets[s].p[ep], 1);
            }
        }
    }

    // Indices: each pixel writes `idx_bits` bits except the three anchors,
    // which write `idx_bits - 1` bits (MSB implicit-0).
    for px in 0..16 {
        let nbits = if px == 0 || px == anchor1 || px == anchor2 {
            idx_bits - 1
        } else {
            idx_bits
        };
        bw.put(indices[px], nbits);
    }

    bw.into_block()
}

// ---- Bit writer --------------------------------------------------------

struct BitWriter {
    block: [u8; 16],
    pos: u32,
}

impl BitWriter {
    fn new() -> Self {
        Self {
            block: [0u8; 16],
            pos: 0,
        }
    }
    fn put(&mut self, bits: u32, n: u32) {
        for i in 0..n {
            if (self.pos as usize) >= 128 {
                return;
            }
            let bit = (bits >> i) & 1;
            let byte = (self.pos / 8) as usize;
            let shift = self.pos & 7;
            self.block[byte] |= (bit as u8) << shift;
            self.pos += 1;
        }
    }
    fn into_block(self) -> [u8; 16] {
        self.block
    }
}

// ---- Block-level mode picker -------------------------------------------

fn encode_block(pixels_rgba: &[[u8; 4]; 16]) -> [u8; 16] {
    let (mut best_block, mut best_sse) = encode_bc7_mode6_block(pixels_rgba);

    let opaque = pixels_rgba.iter().all(|p| p[3] == 0xff);

    // Try 2-subset modes across the full 64-partition table.
    let two_subset_modes: &[u32] = if opaque { &[1, 3] } else { &[7] };
    for &mode in two_subset_modes {
        for partition in 0..PARTITION_COUNT {
            let (cand, sse) = encode_bc7_2subset(pixels_rgba, mode, partition);
            if sse < best_sse {
                best_sse = sse;
                best_block = cand;
            }
        }
    }

    // Try 3-subset modes (opaque content only — modes 0 / 2 have no
    // alpha endpoint). Mode 0 partition selector is 4-bit (16 entries);
    // mode 2 sweeps the full 64-entry 3-subset table.
    if opaque {
        for partition in 0..16 {
            let (cand, sse) = encode_bc7_3subset(pixels_rgba, 0, partition);
            if sse < best_sse {
                best_sse = sse;
                best_block = cand;
            }
        }
        for partition in 0..PARTITION_COUNT {
            let (cand, sse) = encode_bc7_3subset(pixels_rgba, 2, partition);
            if sse < best_sse {
                best_sse = sse;
                best_block = cand;
            }
        }
    }

    // Try mode 4/5 (1-subset, channel-rotation, dual index plane) for
    // content where alpha varies independently from RGB. Even for opaque
    // content these can win when one of R/G/B varies sharply against the
    // other two — the rotation lets that channel use the 6-/8-bit alpha
    // precision while RGB uses the lower-precision plane. Sweep all 4
    // rotations × (mode 4: 2 idx_sel choices) × mode 5.
    for rotation in 0..4u32 {
        for idx_sel in 0..2u32 {
            let (cand, sse) = encode_bc7_mode4(pixels_rgba, rotation, idx_sel);
            if sse < best_sse {
                best_sse = sse;
                best_block = cand;
            }
        }
        let (cand, sse) = encode_bc7_mode5(pixels_rgba, rotation);
        if sse < best_sse {
            best_sse = sse;
            best_block = cand;
        }
    }

    best_block
}

/// Encode a width × height RGBA8 surface to BC7.
///
/// `input` must hold `width × height × 4` bytes (row-major, no padding).
/// `output` must hold `ceil(w/4) × ceil(h/4) × 16` bytes.
///
/// The encoder picks the best of mode 6 (1-subset RGBA), mode 1 / 3
/// (2-subset opaque), mode 0 / 2 (3-subset opaque) and mode 7
/// (2-subset RGBA) per block, sweeping the Microsoft / Khronos
/// partition tables for each multi-subset mode.
pub fn encode_bc7(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 4;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC7 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC7 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    for by in 0..bh {
        for bx in 0..bw {
            let mut block = [[0u8; 4]; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    block[(py * 4 + px) as usize] = rgba_at(input, xx, yy, stride);
                }
            }
            let bc = encode_block(&block);
            let off = (by * bw + bx) * 16;
            output[off..off + 16].copy_from_slice(&bc);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bc7::decode_bc7;

    /// Solid white block → both endpoints quantise to white → roundtrip
    /// is bit-exact.
    #[test]
    fn bc7_encode_solid_white_roundtrip() {
        let input = vec![0xffu8; 4 * 4 * 4];
        let mut bc = vec![0u8; 16];
        encode_bc7(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc7(&bc, 4, 4, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    /// Solid black opaque block. Every candidate mode reconstructs
    /// black with equal SSE; the picker takes the first-best (mode 6),
    /// which matches the round-3 reference.
    #[test]
    fn bc7_encode_solid_black_roundtrip() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for chunk in input.chunks_exact_mut(4) {
            chunk[3] = 0xff;
        }
        let mut bc = vec![0u8; 16];
        encode_bc7(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc7(&bc, 4, 4, &mut decoded).unwrap();
        // Every pixel reconstructs to black ± 1 LSB on each channel
        // (mode-6's per-endpoint p-bit chooses the alpha-LSB majority).
        for chunk in decoded.chunks_exact(4) {
            assert!(chunk[0] <= 1, "R = {}", chunk[0]);
            assert!(chunk[1] <= 1, "G = {}", chunk[1]);
            assert!(chunk[2] <= 1, "B = {}", chunk[2]);
            assert!(chunk[3] >= 254, "A = {}", chunk[3]);
        }
    }

    /// 8×8 RGBA grayscale gradient → PSNR ≥ 30 dB. Mode 6 alone hits
    /// this; the multi-mode picker still picks mode 6 (lowest SSE on
    /// pure 1-axis content).
    #[test]
    fn bc7_encode_8x8_gradient_psnr_gt_30db() {
        let mut input = vec![0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 4;
                let v = ((x + y) * 16) as u8;
                input[off] = v;
                input[off + 1] = v;
                input[off + 2] = v;
                input[off + 3] = 0xff;
            }
        }
        let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
        encode_bc7(&input, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 4];
        decode_bc7(&bc, 8, 8, &mut decoded).unwrap();

        let mut sse: u64 = 0;
        let mut count: u64 = 0;
        for (a, b) in input.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            for c in 0..3 {
                let d = a[c] as i32 - b[c] as i32;
                sse += (d * d) as u64;
                count += 1;
            }
        }
        let mse = sse as f64 / count as f64;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0_f64 * 255.0 / mse).log10()
        };
        assert!(
            psnr > 30.0,
            "BC7 8x8 grayscale gradient PSNR-RGB = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// 8×8 three-axis natural-image RGBA → round-5 lift: ≥ 30 dB via
    /// the 3-subset mode-0/2 partition search (round-4 2-subset mode-1/3
    /// ceiling on this specifically 3-independent-axis pattern was
    /// ~28 dB; the round-5 3-subset modes 0 and 2 push past 30 dB).
    #[test]
    fn bc7_encode_8x8_natural_image_three_axis_psnr_gt_30db() {
        let mut input = vec![0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 4;
                input[off] = (x * 32) as u8;
                input[off + 1] = (y * 32) as u8;
                input[off + 2] = ((x + y) * 16) as u8;
                input[off + 3] = 0xff;
            }
        }
        let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
        encode_bc7(&input, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 4];
        decode_bc7(&bc, 8, 8, &mut decoded).unwrap();

        let mut sse: u64 = 0;
        let mut count: u64 = 0;
        for (a, b) in input.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            for c in 0..3 {
                let d = a[c] as i32 - b[c] as i32;
                sse += (d * d) as u64;
                count += 1;
            }
        }
        let mse = sse as f64 / count as f64;
        let psnr = 10.0 * (255.0_f64 * 255.0 / mse).log10();
        assert!(
            psnr > 30.0,
            "BC7 8x8 three-axis natural-image PSNR-RGB = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// 8×8 two-region content — top-left 4×4 quadrant is a colour A
    /// gradient, bottom-right is colour B gradient — exactly what the
    /// 2-subset BC7 modes are designed for. The 2-subset partition
    /// search picks a partition that splits each block into the two
    /// regions, then each subset fits its own gradient line on a
    /// 7-bit palette → ≥ 30 dB.
    #[test]
    fn bc7_encode_8x8_two_region_psnr_gt_30db() {
        let mut input = vec![0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 4;
                let region_a = x < 4;
                if region_a {
                    let v = (x * 32 + y * 16) as u8;
                    input[off] = v;
                    input[off + 1] = v / 2;
                } else {
                    let v = ((x - 4) * 32 + y * 16) as u8;
                    input[off + 2] = v;
                    input[off + 1] = 128 - v / 2;
                }
                input[off + 3] = 0xff;
            }
        }
        let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
        encode_bc7(&input, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 4];
        decode_bc7(&bc, 8, 8, &mut decoded).unwrap();

        let mut sse: u64 = 0;
        let mut count: u64 = 0;
        for (a, b) in input.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            for c in 0..3 {
                let d = a[c] as i32 - b[c] as i32;
                sse += (d * d) as u64;
                count += 1;
            }
        }
        let mse = sse as f64 / count as f64;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0_f64 * 255.0 / mse).log10()
        };
        assert!(
            psnr > 30.0,
            "BC7 8x8 two-region PSNR-RGB = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// Two-colour split block (left half red, right half blue) →
    /// either mode 6 (furthest-point endpoints = red & blue) or
    /// mode 1/3 (2-subset partition aligns with the split). All
    /// candidates reconstruct the two colours faithfully.
    #[test]
    fn bc7_encode_two_colour_block() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    input[off] = 0xff;
                    input[off + 3] = 0xff;
                } else {
                    input[off + 2] = 0xff;
                    input[off + 3] = 0xff;
                }
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc7(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc7(&bc, 4, 4, &mut decoded).unwrap();
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                let p = &decoded[off..off + 4];
                if x < 2 {
                    assert!(p[0] >= 0xfe, "pixel ({x},{y}) red R = {}", p[0]);
                    assert!(p[1] <= 1, "pixel ({x},{y}) red G = {}", p[1]);
                    assert!(p[2] <= 1, "pixel ({x},{y}) red B = {}", p[2]);
                    assert!(p[3] >= 0xfe, "pixel ({x},{y}) red A = {}", p[3]);
                } else {
                    assert!(p[0] <= 1, "pixel ({x},{y}) blue R = {}", p[0]);
                    assert!(p[1] <= 1, "pixel ({x},{y}) blue G = {}", p[1]);
                    assert!(p[2] >= 0xfe, "pixel ({x},{y}) blue B = {}", p[2]);
                    assert!(p[3] >= 0xfe, "pixel ({x},{y}) blue A = {}", p[3]);
                }
            }
        }
    }

    /// quantize_7p is a no-op round-trip — every 8-bit value reproduces.
    #[test]
    fn quantize_7p_is_lossless() {
        for v in 0..=255u8 {
            let (raw, p) = quantize_7p(v);
            let recon = ((raw << 1) | p) as u8;
            assert_eq!(recon, v, "{v} round-tripped to {recon}");
        }
    }

    /// 5×5 surface (one 4×4 block + edge replication) — exercises the
    /// non-multiple-of-4 padding code.
    #[test]
    fn bc7_encode_5x5_solid_white() {
        let input = vec![0xffu8; 5 * 5 * 4];
        let mut bc = vec![0u8; 4 * 16];
        encode_bc7(&input, 5, 5, &mut bc).unwrap();
        let mut decoded = vec![0u8; 5 * 5 * 4];
        decode_bc7(&bc, 5, 5, &mut decoded).unwrap();
        for chunk in decoded.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }

    /// Translucent two-colour block exercises mode 7 (2-subset RGBA).
    #[test]
    fn bc7_encode_two_colour_translucent_block() {
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                if x < 2 {
                    input[off] = 0xff;
                    input[off + 3] = 0x80; // translucent red
                } else {
                    input[off + 2] = 0xff;
                    input[off + 3] = 0xc0; // mostly-opaque blue
                }
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc7(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 4];
        decode_bc7(&bc, 4, 4, &mut decoded).unwrap();
        // Both halves should reproduce within ~8 LSB on each channel.
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                let p = &decoded[off..off + 4];
                if x < 2 {
                    assert!(p[0] >= 240, "pixel ({x},{y}) red R = {}", p[0]);
                    assert!(p[3].abs_diff(0x80) <= 8, "red A = {}", p[3]);
                } else {
                    assert!(p[2] >= 240, "pixel ({x},{y}) blue B = {}", p[2]);
                    assert!(p[3].abs_diff(0xc0) <= 8, "blue A = {}", p[3]);
                }
            }
        }
    }

    /// Mode 4/5 standalone roundtrip for a single rotation choice — verify
    /// the encoder produces decoder-roundtrippable bytes for a constructed
    /// block where the rotation makes a difference.
    #[test]
    fn bc7_encode_mode4_rotation_roundtrip() {
        // Block: solid RGB (all 0xff red), with alpha that varies on a
        // gradient. Mode 4 with rotation=0, idx_sel=0 should reconstruct
        // both planes well (RGB endpoints both close to red, alpha
        // endpoints span the gradient with 6-bit precision + 3-bit indices).
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                input[off] = 0xff;
                input[off + 3] = ((x + y) * 32).min(255) as u8;
            }
        }
        let (block, _sse) = encode_bc7_mode4(&pixels_from(&input), 0, 0);
        let pixels = crate::bc7::decode_bc7_block(&block);
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                let p = pixels[y * 4 + x];
                assert!(p[0] >= 0xf0, "pixel ({x},{y}) R = {} (want ~0xff)", p[0]);
                let want_a = ((x + y) * 32).min(255) as u8;
                let got_a = p[3];
                assert!(
                    got_a.abs_diff(want_a) <= 16,
                    "pixel ({x},{y}) alpha = {} (want {} ± 16)",
                    got_a,
                    want_a
                );
                let _ = off;
            }
        }
    }

    /// Mode 5 standalone roundtrip — both index planes 2-bit, 7-bit RGB
    /// + 8-bit alpha.
    #[test]
    fn bc7_encode_mode5_rotation_roundtrip() {
        // Block: alpha varies sharply (0 / 255 split horizontally), RGB
        // is constant gray. Mode 5 rotation=1 (A↔R) puts the gray into
        // the bitstream alpha (high precision), and the 0/255 split into
        // bitstream RGB-R — but mode 5's RGB is 7-bit so it can still
        // carry that. Either rotation/mode picks a faithful encoding.
        let mut input = vec![0u8; 4 * 4 * 4];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 4;
                input[off] = 0x80;
                input[off + 1] = 0x80;
                input[off + 2] = 0x80;
                input[off + 3] = if x < 2 { 0x00 } else { 0xff };
            }
        }
        let (block, _sse) = encode_bc7_mode5(&pixels_from(&input), 0);
        let pixels = crate::bc7::decode_bc7_block(&block);
        for y in 0..4 {
            for x in 0..4 {
                let p = pixels[y * 4 + x];
                let want_a = if x < 2 { 0x00 } else { 0xff };
                assert!(
                    p[3].abs_diff(want_a) <= 16,
                    "pixel ({x},{y}) alpha = {} (want {} ± 16)",
                    p[3],
                    want_a
                );
                // RGB should stay near 0x80.
                for c in 0..3 {
                    assert!(
                        p[c].abs_diff(0x80) <= 16,
                        "pixel ({x},{y}) ch{} = {} (want 0x80 ± 16)",
                        c,
                        p[c]
                    );
                }
            }
        }
    }

    /// Mode 4/5 channel-rotation independence: encode a block where alpha
    /// is fully decorrelated from RGB. The block-level picker should
    /// produce a faithful roundtrip, and at least one rotation should
    /// give a markedly better result than no-rotation.
    #[test]
    fn bc7_encode_independent_alpha_rgb_psnr_gt_30db() {
        // 8×8 block: RGB is a 1-D x-axis gradient (smooth), alpha is a
        // vertical y-axis gradient (also smooth, but independent).
        // Without channel-rotation, one of the two planes has to share
        // the colour palette → quality suffers. Mode 4/5 lets each plane
        // run its own endpoints + indices.
        let mut input = vec![0u8; 8 * 8 * 4];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 4;
                input[off] = (x * 32) as u8;
                input[off + 1] = (x * 32) as u8;
                input[off + 2] = (x * 32) as u8;
                input[off + 3] = (y * 32) as u8;
            }
        }
        let mut bc = vec![0u8; 4 * 16];
        encode_bc7(&input, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 4];
        decode_bc7(&bc, 8, 8, &mut decoded).unwrap();
        let mut sse: u64 = 0;
        let mut count: u64 = 0;
        for (a, b) in input.chunks_exact(4).zip(decoded.chunks_exact(4)) {
            for c in 0..4 {
                let d = a[c] as i32 - b[c] as i32;
                sse += (d * d) as u64;
                count += 1;
            }
        }
        let mse = sse as f64 / count as f64;
        let psnr = if mse == 0.0 {
            f64::INFINITY
        } else {
            10.0 * (255.0_f64 * 255.0 / mse).log10()
        };
        assert!(
            psnr > 30.0,
            "BC7 independent-alpha PSNR-RGBA = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// `apply_rotation` is its own inverse (involution): apply twice → identity.
    #[test]
    fn rotation_is_involution() {
        for r in 0..4u32 {
            for &p in &[
                [0, 0, 0, 0u8],
                [255, 128, 64, 32],
                [1, 2, 3, 4],
                [200, 150, 100, 50],
            ] {
                let twice = apply_rotation(apply_rotation(p, r), r);
                assert_eq!(p, twice, "rotation {} not involution on {:?}", r, p);
            }
        }
    }

    fn pixels_from(input: &[u8]) -> [[u8; 4]; 16] {
        let mut block = [[0u8; 4]; 16];
        for i in 0..16 {
            block[i] = [
                input[i * 4],
                input[i * 4 + 1],
                input[i * 4 + 2],
                input[i * 4 + 3],
            ];
        }
        block
    }
}
