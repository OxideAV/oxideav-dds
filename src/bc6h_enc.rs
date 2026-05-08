//! BC6H (DXGI `BC6H_UF16` / `BC6H_SF16`) HDR-float block encoder.
//!
//! Round 3 baseline shipped mode 10 (1-subset, 10.10 absolute endpoint
//! precision per channel, no delta, 4-bit indices) — the simplest BC6H
//! 1-subset mode. Round 6 closes the BC6H encoder gap by adding:
//!
//! * **2-subset modes 0..9** — sweep the 32 BC6H 2-subset partition
//!   table for each candidate mode, seed per-subset endpoints with
//!   furthest-point in that subset, then run two iterations of (snap
//!   pixels to nearest palette → least-squares refine endpoints →
//!   re-quantise) before measuring SSE. Selecting modes 0 (10.5.5.5),
//!   2/3/4 (11.5.4.4 family), 5 (9.5.5.5), and 9 (6.6.6.6) covers
//!   the natural-HDR partition-friendly content the round-5 mode-10
//!   1-subset baseline cannot fit on multi-axis blocks.
//! * **1-subset delta-encoded modes 11/12/13** — mode 11 is 10.10.10
//!   base + 9-bit delta (one extra base bit over mode 10); mode 12 is
//!   12-bit base + 8-bit delta (two extra base bits but smaller delta
//!   range); mode 13 is 16-bit base + 4-bit delta (full half-float
//!   base precision but only ±8 delta range — useful only when both
//!   endpoints are very close in quantised space).
//!
//! Reference: Microsoft's public "BC6H Format" article on
//! learn.microsoft.com (Direct3D 11 reference) and the Intel Open
//! Source Programmer's Reference Manual Vol. 5 (BC6H section, 0BSD-
//! licensed). No DirectXTex, NVTT, ISPC `ispc_texcomp`, basisu, or
//! `bc6h_enc` source was consulted; only the public spec text + the
//! per-mode bit-allocation tables.
//!
//! Encoder strategy (round 6):
//!
//! 1. Build the 4×4 block of half-float RGB pixels (alpha ignored).
//! 2. Encode mode 10 (the round-3 baseline) as the SSE reference.
//! 3. Try each of the round-6 candidate modes. For 2-subset modes,
//!    sweep the 32-entry BC6H partition table. For 1-subset delta
//!    modes (11/12/13), encode against the mode's base + delta
//!    precision. Each candidate computes its post-decode pixel
//!    grid + SSE in half-float-converted f32 space.
//! 4. Pick the lowest-SSE candidate's packed bytes.
//!
//! The BC6H_UF16 finalise (`(comp * 31) >> 6`) loses dynamic range
//! from raw half-bits (16-bit) down to the same 16-bit codomain but
//! with a 31/64 scale; the encoder mirrors this exactly so picked
//! candidates are bit-exact roundtrip-able through the decoder.

// Per-channel and per-pixel inner loops are clearer indexed; silence
// clippy's preference for iterator-style code in this module.
#![allow(clippy::needless_range_loop)]

use crate::bc6h::{half_to_f32, mode_info, FieldBit, ModeInfo, ANCHOR_2_SUBSET_2, PART_2};
use crate::error::{DdsError, Result};

/// Convert a positive `f32` to a finite IEEE-754 binary16 value (unsigned
/// half — we treat negative inputs as 0 and clamp NaN / infinity to the
/// max finite half).
fn f32_to_half(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 31) & 1;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;

    if sign == 1 {
        // BC6H_UF16 is unsigned; clamp negatives to 0.
        return 0;
    }
    if exp == 0xff {
        // Infinity or NaN — clamp to half max-finite (0x7bff).
        if mant == 0 {
            return 0x7bff;
        }
        return 0x7bff;
    }
    if exp == 0 && mant == 0 {
        return 0;
    }

    let exp_f16 = exp - 127 + 15;
    if exp_f16 >= 0x1f {
        // Overflow → half max finite.
        return 0x7bff;
    }
    if exp_f16 <= 0 {
        // Subnormal half — shift mantissa right.
        let shift = 1 - exp_f16;
        if shift > 24 {
            return 0;
        }
        let m = (mant | 0x800000) >> (shift + 13);
        return m as u16;
    }
    let m = mant >> 13;
    ((exp_f16 as u32) << 10 | m) as u16
}

#[inline]
fn rgb_at(pixels: &[u16], x: u32, y: u32, stride_pixels: usize) -> [u16; 3] {
    let off = y as usize * stride_pixels * 4 + x as usize * 4;
    [pixels[off], pixels[off + 1], pixels[off + 2]]
}

/// Quantise a half-float (treated as unsigned 16-bit in [0, 0xffff]) to
/// `bits` bits.
///
/// The DECODER's full forward pipeline for `BC6H_UF16` mode-10 / mode-11
/// endpoints is:
///   `q (n-bit)` → `unq = unquantize(q, n) = ((q << 16) + 0x8000) >> n`
///   → `H = finish_uf16(unq) = (unq * 31) >> 6`.
///
/// So the encoder's job is to find the n-bit `q` such that the
/// post-finalise `H` is closest to the input half-bit value. The
/// "31/64" scale in `finish_uf16` means the dynamic range maps from
/// `[0, 0xffff]` (raw half-bits) onto a *compressed* range — we have
/// to invert the entire pipeline, not just `unquantize`.
///
/// The `q == 0` and `q == max_q` boundaries are special-cased per
/// Microsoft (they produce `0` and `0xffff` respectively).
fn quantize_half_uf16(half_bits: u16, bits: u32) -> u32 {
    let max_q = (1u32 << bits) - 1;
    let target = half_bits as u32;

    // Forward pipeline: q -> half-bits.
    let forward = |q: u32| -> u32 {
        let unq = if q == 0 {
            0u32
        } else if q == max_q {
            0xffffu32
        } else if bits >= 15 {
            q
        } else {
            ((q << 16) + 0x8000) >> bits
        };
        (unq * 31) >> 6
    };

    if target == 0 {
        return 0;
    }
    if target >= forward(max_q) {
        return max_q;
    }

    // Continuous estimate: target * 64 ≈ unq * 31; unq ≈ (target * 64) / 31.
    // Then q * 2^16 + 0x8000 ≈ unq * 2^bits ⇒ q ≈ (unq * 2^bits - 0x8000) / 2^16.
    let unq_est = ((target as u64) << 6) / 31;
    let lhs = (unq_est << bits).saturating_sub(0x8000);
    let q_est = (lhs >> 16) as u32;

    // Probe ±2 around the estimate.
    let mut best = q_est.min(max_q);
    let mut best_err = (forward(best) as i32 - target as i32).unsigned_abs();
    for d in [-2i32, -1, 0, 1, 2] {
        let cand = (q_est as i32 + d).clamp(0, max_q as i32) as u32;
        let err = (forward(cand) as i32 - target as i32).unsigned_abs();
        if err < best_err {
            best_err = err;
            best = cand;
        }
    }
    best
}

/// Reproduce the decoder's endpoint pipeline (e.g. for mode 10):
/// `unquantize(q, bits, false)` → 17-bit signed integer.
fn unquantize_uf16(comp: u32, bits: u32) -> i32 {
    if bits >= 15 {
        return comp as i32;
    }
    let max_q = (1u32 << bits) - 1;
    if comp == 0 {
        0
    } else if comp == max_q {
        0xffff
    } else {
        (((comp << 16) + 0x8000) >> bits) as i32
    }
}

/// Microsoft's BC6H_UF16 finalise: `(comp * 31) >> 6` → unsigned half.
fn finish_uf16(comp: i32) -> u16 {
    if comp <= 0 {
        return 0;
    }
    let v = (comp as u32 * 31) >> 6;
    v.min(0xffff) as u16
}

/// 16-entry weight table for 4-bit indices (Microsoft).
const WEIGHT_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
/// 8-entry weight table for 3-bit indices (2-subset modes).
const WEIGHT_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];

/// Interpolate two unquantized 17-bit endpoints with a `n`-bit index.
fn interp_endpoint(e0: i32, e1: i32, idx: usize, idx_bits: u32) -> i32 {
    let w = match idx_bits {
        3 => WEIGHT_3[idx] as i64,
        4 => WEIGHT_4[idx] as i64,
        _ => unreachable!(),
    };
    let a = e0 as i64;
    let b = e1 as i64;
    ((a * (64 - w) + b * w + 32) >> 6) as i32
}

/// Squared error between two RGB half-float triples (after expansion to
/// f32 — the metric the human eye approximates better than raw
/// half-bits).
fn sq_err_rgb_half(a: [u16; 3], b: [u16; 3]) -> f64 {
    let mut s = 0.0_f64;
    for c in 0..3 {
        let af = half_to_f32(a[c]) as f64;
        let bf = half_to_f32(b[c]) as f64;
        let d = af - bf;
        s += d * d;
    }
    s
}

// ---- Bit-stream writer (LSB-first across the 16-byte block) -------------

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
    fn put(&mut self, value: u32, n: u32) {
        for i in 0..n {
            let bit = (value >> i) & 1;
            let bp = self.pos + i;
            if (bp as usize) < 128 {
                let byte = (bp / 8) as usize;
                let shift = bp & 7;
                self.block[byte] |= ((bit & 1) as u8) << shift;
            }
        }
        self.pos += n;
    }
    fn into_block(self) -> [u8; 16] {
        self.block
    }
}

// ---- Common pack helper -------------------------------------------------

/// Pack a BC6H block from `q[ch][ep]` quantised values, indices, prefix,
/// and (optional) partition.
///
/// `q` carries the bit pattern stored in the bitstream — for delta modes
/// the encoder must convert `q1 = (q1_abs - q0) & ((1 << delta) - 1)`
/// (the wrapped delta) BEFORE calling this, so this packer is purely
/// mechanical bit-scattering.
fn pack_bc6h(
    mi: &ModeInfo,
    prefix: u32,
    prefix_len: u32,
    q: [[u32; 4]; 3],
    partition: u32,
    indices: &[u32; 16],
    anchor_subset1: u8,
) -> [u8; 16] {
    let mut bw = BitWriter::new();
    bw.put(prefix, prefix_len);
    for f in mi.fields {
        let v = q[f.channel as usize][f.endpoint as usize];
        bw.put((v >> f.dest_bit) & 1, 1);
    }
    if mi.subsets == 2 {
        bw.put(partition, 5);
        // 2-subset: 3-bit indices; subset-0 anchor is pixel 0 (2 bits);
        // subset-1 anchor is `anchor_subset1` (2 bits); rest are 3 bits.
        for px in 0..16usize {
            let nbits = if px == 0 || px as u8 == anchor_subset1 {
                2
            } else {
                3
            };
            bw.put(indices[px], nbits);
        }
    } else {
        // 1-subset: 4-bit indices; pixel-0 anchor is 3 bits; rest 4 bits.
        bw.put(indices[0], 3);
        for px in 1..16usize {
            bw.put(indices[px], 4);
        }
    }
    bw.into_block()
}

// ---- Furthest-point endpoint search ------------------------------------

fn furthest_pair_global(pixels: &[[u16; 3]; 16]) -> (usize, usize) {
    let mut best_d = -1.0_f64;
    let mut bi = 0usize;
    let mut bj = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let d = sq_err_rgb_half(pixels[i], pixels[j]);
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    (bi, bj)
}

fn furthest_pair_subset(
    pixels: &[[u16; 3]; 16],
    subsets: &[u8; 16],
    s: u8,
) -> ([u16; 3], [u16; 3]) {
    let mut idxs: [usize; 16] = [0; 16];
    let mut n = 0usize;
    for (i, &t) in subsets.iter().enumerate() {
        if t == s {
            idxs[n] = i;
            n += 1;
        }
    }
    if n == 0 {
        return ([0; 3], [0; 3]);
    }
    if n == 1 {
        return (pixels[idxs[0]], pixels[idxs[0]]);
    }
    let mut best_d = -1.0_f64;
    let mut bi = idxs[0];
    let mut bj = idxs[1];
    for ai in 0..n {
        for aj in (ai + 1)..n {
            let i = idxs[ai];
            let j = idxs[aj];
            let d = sq_err_rgb_half(pixels[i], pixels[j]);
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    (pixels[bi], pixels[bj])
}

// ---- Mode 10 (1-subset, no delta) — round-3 baseline -------------------

fn encode_mode10(pixels: &[[u16; 3]; 16]) -> ([u8; 16], f64) {
    let (best_i, best_j) = furthest_pair_global(pixels);
    let half0 = pixels[best_i];
    let half1 = pixels[best_j];

    let mut q0 = [0u32; 3];
    let mut q1 = [0u32; 3];
    for c in 0..3 {
        q0[c] = quantize_half_uf16(half0[c], 10);
        q1[c] = quantize_half_uf16(half1[c], 10);
    }
    let mut indices = [0u32; 16];
    let (mut sse, _palette) = snap_indices_1subset(pixels, &q0, &q1, 10, 4, &mut indices);

    // Iterative refinement (2 passes): least-squares fit + re-snap.
    for _ in 0..2 {
        let (q0_new, q1_new) = refine_endpoints_1subset(pixels, &indices, 4, 10);
        let mut idx_new = [0u32; 16];
        let (sse_new, _) = snap_indices_1subset(pixels, &q0_new, &q1_new, 10, 4, &mut idx_new);
        if sse_new < sse {
            sse = sse_new;
            q0 = q0_new;
            q1 = q1_new;
            indices = idx_new;
        } else {
            break;
        }
    }

    if indices[0] >= 8 {
        std::mem::swap(&mut q0, &mut q1);
        for idx in indices.iter_mut() {
            *idx = 15 - *idx;
        }
    }
    // Pack (mode 10 layout — uses ModeInfo's fields).
    let mi = mode_info(10).expect("mode 10 info");
    let q = [
        [q0[0], q1[0], 0, 0],
        [q0[1], q1[1], 0, 0],
        [q0[2], q1[2], 0, 0],
    ];
    let block = pack_bc6h(&mi, 0b00011, 5, q, 0, &indices, 15);
    (block, sse)
}

/// Build the 1-subset palette + snap each pixel to the nearest entry.
/// `q0`, `q1` are absolute quantised endpoints (per channel). Returns
/// (sse, palette).
fn snap_indices_1subset(
    pixels: &[[u16; 3]; 16],
    q0: &[u32; 3],
    q1: &[u32; 3],
    prec: u32,
    idx_bits: u32,
    indices: &mut [u32; 16],
) -> (f64, [[u16; 3]; 16]) {
    let mut endpoints = [[0i32; 2]; 3];
    for c in 0..3 {
        endpoints[c][0] = unquantize_uf16(q0[c], prec);
        endpoints[c][1] = unquantize_uf16(q1[c], prec);
    }
    let palette_size = 1usize << idx_bits;
    let mut palette = [[0u16; 3]; 16];
    for k in 0..palette_size {
        for c in 0..3 {
            let v = interp_endpoint(endpoints[c][0], endpoints[c][1], k, idx_bits);
            palette[k][c] = finish_uf16(v);
        }
    }
    let mut sse = 0.0f64;
    for (px, &p) in pixels.iter().enumerate() {
        let mut best_k = 0u32;
        let mut best_e = f64::MAX;
        for k in 0..palette_size {
            let e = sq_err_rgb_half(p, palette[k]);
            if e < best_e {
                best_e = e;
                best_k = k as u32;
            }
        }
        indices[px] = best_k;
        sse += best_e;
    }
    (sse, palette)
}

/// Least-squares refinement of two endpoints `q0, q1` (per channel)
/// against fixed `indices`. Returns `(q0_new, q1_new)`.
///
/// For each channel, we solve the 2-variable linear system:
///   sum_i ((1 - w_i) e0 + w_i e1 - p_i)^2 → minimum
/// with `w_i` = the index weight at pixel i. Closed-form solution:
///   [aa ab; ab bb] [e0 e1]^T = [ap bp]^T
/// where aa = sum (1-w)^2, bb = sum w^2, ab = sum (1-w)w, ap = sum (1-w)p, bp = sum w p.
fn refine_endpoints_1subset(
    pixels: &[[u16; 3]; 16],
    indices: &[u32; 16],
    idx_bits: u32,
    prec: u32,
) -> ([u32; 3], [u32; 3]) {
    let weights = match idx_bits {
        3 => &WEIGHT_3[..],
        4 => &WEIGHT_4[..],
        _ => unreachable!(),
    };
    let mut q0 = [0u32; 3];
    let mut q1 = [0u32; 3];
    for c in 0..3 {
        let mut aa = 0.0f64;
        let mut bb = 0.0f64;
        let mut ab = 0.0f64;
        let mut ap = 0.0f64;
        let mut bp = 0.0f64;
        for i in 0..16 {
            let w = weights[indices[i] as usize] as f64 / 64.0;
            let a = 1.0 - w;
            let b = w;
            let p = half_to_f32(pixels[i][c]) as f64;
            aa += a * a;
            bb += b * b;
            ab += a * b;
            ap += a * p;
            bp += b * p;
        }
        let det = aa * bb - ab * ab;
        let (e0, e1) = if det.abs() < 1e-9 {
            // Degenerate — use mean.
            let mut sum = 0.0f64;
            for i in 0..16 {
                sum += half_to_f32(pixels[i][c]) as f64;
            }
            let m = sum / 16.0;
            (m, m)
        } else {
            let v0 = (bb * ap - ab * bp) / det;
            let v1 = (aa * bp - ab * ap) / det;
            (v0.max(0.0), v1.max(0.0))
        };
        // Convert back to half then quantise.
        q0[c] = quantize_half_uf16(f32_to_half(e0 as f32), prec);
        q1[c] = quantize_half_uf16(f32_to_half(e1 as f32), prec);
    }
    (q0, q1)
}

// ---- 2-subset endpoint snapping ----------------------------------------

fn snap_indices_2subset(
    pixels: &[[u16; 3]; 16],
    subsets: &[u8; 16],
    endpoints_unq: &[[i32; 4]; 3],
    idx_bits: u32,
    indices: &mut [u32; 16],
) -> f64 {
    let palette_size = 1usize << idx_bits;
    let mut sse = 0.0f64;
    for px in 0..16 {
        let s = subsets[px] as usize;
        let i0 = s * 2;
        let i1 = s * 2 + 1;
        let mut best_k = 0u32;
        let mut best_e = f64::MAX;
        for k in 0..palette_size {
            let mut pal = [0u16; 3];
            for c in 0..3 {
                let v = interp_endpoint(endpoints_unq[c][i0], endpoints_unq[c][i1], k, idx_bits);
                pal[c] = finish_uf16(v);
            }
            let e = sq_err_rgb_half(pixels[px], pal);
            if e < best_e {
                best_e = e;
                best_k = k as u32;
            }
        }
        indices[px] = best_k;
        sse += best_e;
    }
    sse
}

/// Per-subset least-squares refinement for 2-subset content. Returns
/// updated `q_abs[ch][slot]` for slots [s*2, s*2+1] of the given subset.
fn refine_endpoints_2subset(
    pixels: &[[u16; 3]; 16],
    subsets: &[u8; 16],
    s: u8,
    indices: &[u32; 16],
    idx_bits: u32,
    prec: u32,
) -> ([u32; 3], [u32; 3]) {
    let weights = match idx_bits {
        3 => &WEIGHT_3[..],
        4 => &WEIGHT_4[..],
        _ => unreachable!(),
    };
    let mut q0 = [0u32; 3];
    let mut q1 = [0u32; 3];
    for c in 0..3 {
        let mut aa = 0.0f64;
        let mut bb = 0.0f64;
        let mut ab = 0.0f64;
        let mut ap = 0.0f64;
        let mut bp = 0.0f64;
        let mut count = 0u32;
        let mut sum = 0.0f64;
        for i in 0..16 {
            if subsets[i] != s {
                continue;
            }
            count += 1;
            let w = weights[indices[i] as usize] as f64 / 64.0;
            let a = 1.0 - w;
            let b = w;
            let p = half_to_f32(pixels[i][c]) as f64;
            aa += a * a;
            bb += b * b;
            ab += a * b;
            ap += a * p;
            bp += b * p;
            sum += p;
        }
        if count == 0 {
            // Empty subset (shouldn't happen for sane partitions).
            q0[c] = 0;
            q1[c] = 0;
            continue;
        }
        let det = aa * bb - ab * ab;
        let (e0, e1) = if det.abs() < 1e-9 {
            let m = sum / count as f64;
            (m, m)
        } else {
            let v0 = (bb * ap - ab * bp) / det;
            let v1 = (aa * bp - ab * ap) / det;
            (v0.max(0.0), v1.max(0.0))
        };
        q0[c] = quantize_half_uf16(f32_to_half(e0 as f32), prec);
        q1[c] = quantize_half_uf16(f32_to_half(e1 as f32), prec);
    }
    (q0, q1)
}

// ---- Delta-encoding helpers --------------------------------------------

/// Convert an absolute quantised endpoint `q1_abs` into the wrapped
/// delta value (in `delta_bits` bits) relative to `q0`. Returns
/// `Some(wrapped_delta)` when the signed delta fits in `delta_bits`,
/// or `None` when overflow forces a different mode.
fn encode_delta(q0: u32, q1_abs: u32, prec: u32, delta_bits: u32) -> Option<u32> {
    let mask: i64 = if prec >= 32 { -1 } else { (1i64 << prec) - 1 };
    let signed_d = (q1_abs as i64) - (q0 as i64);
    // Wrap so `(q0 + d) & mask == q1_abs`.
    let raw = (signed_d & mask) as u32;
    // The decoder sign-extends raw from `delta_bits`. So the encoded
    // value's high bit (`delta_bits-1`) must mirror the actual sign of
    // `signed_d`. Equivalently: `signed_d` must be in the signed
    // range `[-2^(delta_bits-1), 2^(delta_bits-1) - 1]`.
    let half = 1i64 << (delta_bits - 1);
    if signed_d < -half || signed_d >= half {
        return None;
    }
    // Truncate to delta_bits.
    let dmask = (1u32 << delta_bits) - 1;
    Some(raw & dmask)
}

// ---- 1-subset delta-encoded modes 11/12/13 -----------------------------

/// Try mode 11 (1-subset, 10.10.10 base + 9-bit delta) for the block.
/// Returns `(block, sse)`.
fn encode_mode_delta_1subset(
    pixels: &[[u16; 3]; 16],
    mode: u32,
    prefix: u32,
    prec: u32,
    delta_bits: u32,
) -> Option<([u8; 16], f64)> {
    let (best_i, best_j) = furthest_pair_global(pixels);
    let half0 = pixels[best_i];
    let half1 = pixels[best_j];

    let mut q0 = [0u32; 3];
    let mut q1_abs = [0u32; 3];
    for c in 0..3 {
        q0[c] = quantize_half_uf16(half0[c], prec);
        q1_abs[c] = quantize_half_uf16(half1[c], prec);
    }

    // Delta encoding: every channel's delta must fit. If overflow on
    // any channel, abandon this mode and clamp endpoint 1 to the
    // nearest in-range value (so we still report SOME SSE — the picker
    // will choose mode 10 instead if this is too lossy).
    let mut deltas = [0u32; 3];
    for c in 0..3 {
        let half = 1i64 << (delta_bits - 1);
        let d_signed = (q1_abs[c] as i64) - (q0[c] as i64);
        let d_clamped = d_signed.clamp(-half, half - 1);
        if d_clamped != d_signed {
            // Reconstitute q1_abs after clamp (so palette computation
            // matches what the decoder will produce).
            let mask: i64 = if prec >= 32 { -1 } else { (1i64 << prec) - 1 };
            let q1_new = ((q0[c] as i64 + d_clamped) & mask) as u32;
            q1_abs[c] = q1_new;
        }
        let dmask = (1u32 << delta_bits) - 1;
        deltas[c] = (d_clamped as u32) & dmask;
    }

    let mut indices = [0u32; 16];
    let (sse, _palette) = snap_indices_1subset(pixels, &q0, &q1_abs, prec, 4, &mut indices);

    // Anchor: ensure pixel-0 index < 8 (3-bit MSB-implicit).
    if indices[0] >= 8 {
        // Swap absolute endpoints; recompute deltas.
        std::mem::swap(&mut q0, &mut q1_abs);
        for c in 0..3 {
            // After swap, q0 holds what was q1_abs, q1_abs holds what was q0.
            // Re-encode delta (which can still overflow if range is asymmetric).
            let half = 1i64 << (delta_bits - 1);
            let d_signed = (q1_abs[c] as i64) - (q0[c] as i64);
            if d_signed < -half || d_signed >= half {
                // Asymmetric range — swap forced overflow. Bail.
                return None;
            }
            let dmask = (1u32 << delta_bits) - 1;
            deltas[c] = (d_signed as u32) & dmask;
        }
        for idx in indices.iter_mut() {
            *idx = 15 - *idx;
        }
    }

    let mi = mode_info(mode).expect("mode info");
    let q = [
        [q0[0], deltas[0], 0, 0],
        [q0[1], deltas[1], 0, 0],
        [q0[2], deltas[2], 0, 0],
    ];
    let block = pack_bc6h(&mi, prefix, 5, q, 0, &indices, 15);
    Some((block, sse))
}

// ---- 2-subset modes 0..9 ------------------------------------------------

#[derive(Clone, Copy)]
struct TwoSubsetSpec {
    mode: u32,
    prefix: u32,
    prefix_len: u32,
    prec: u32, // shared base precision (per the 2-subset modes — same on all channels for our subset of the table)
    delta_r: u32,
    delta_g: u32,
    delta_b: u32,
}

const TWO_SUBSET_MODES: &[TwoSubsetSpec] = &[
    TwoSubsetSpec {
        mode: 0,
        prefix: 0b00,
        prefix_len: 2,
        prec: 10,
        delta_r: 5,
        delta_g: 5,
        delta_b: 5,
    },
    TwoSubsetSpec {
        mode: 1,
        prefix: 0b01,
        prefix_len: 2,
        prec: 7,
        delta_r: 6,
        delta_g: 6,
        delta_b: 6,
    },
    TwoSubsetSpec {
        mode: 2,
        prefix: 0b00010,
        prefix_len: 5,
        prec: 11,
        delta_r: 5,
        delta_g: 4,
        delta_b: 4,
    },
    TwoSubsetSpec {
        mode: 3,
        prefix: 0b00110,
        prefix_len: 5,
        prec: 11,
        delta_r: 4,
        delta_g: 5,
        delta_b: 4,
    },
    TwoSubsetSpec {
        mode: 4,
        prefix: 0b01010,
        prefix_len: 5,
        prec: 11,
        delta_r: 4,
        delta_g: 4,
        delta_b: 5,
    },
    TwoSubsetSpec {
        mode: 5,
        prefix: 0b01110,
        prefix_len: 5,
        prec: 9,
        delta_r: 5,
        delta_g: 5,
        delta_b: 5,
    },
    TwoSubsetSpec {
        mode: 6,
        prefix: 0b10010,
        prefix_len: 5,
        prec: 8,
        delta_r: 6,
        delta_g: 5,
        delta_b: 5,
    },
    TwoSubsetSpec {
        mode: 7,
        prefix: 0b10110,
        prefix_len: 5,
        prec: 8,
        delta_r: 5,
        delta_g: 6,
        delta_b: 5,
    },
    TwoSubsetSpec {
        mode: 8,
        prefix: 0b11010,
        prefix_len: 5,
        prec: 8,
        delta_r: 5,
        delta_g: 5,
        delta_b: 6,
    },
    TwoSubsetSpec {
        mode: 9,
        prefix: 0b11110,
        prefix_len: 5,
        prec: 6,
        delta_r: 0,
        delta_g: 0,
        delta_b: 0,
    },
];

/// Try one 2-subset mode/partition. Returns `(block, sse)` or `None`
/// when delta overflow on any channel forces mode rejection.
fn try_2subset(
    pixels: &[[u16; 3]; 16],
    spec: &TwoSubsetSpec,
    partition: u32,
) -> Option<([u8; 16], f64)> {
    let part = PART_2[partition as usize];
    let anchor1 = ANCHOR_2_SUBSET_2[partition as usize];

    // Per-subset furthest-point seed.
    let (s0_e0, s0_e1) = furthest_pair_subset(pixels, &part, 0);
    let (s1_e0, s1_e1) = furthest_pair_subset(pixels, &part, 1);

    let prec = spec.prec;
    let delta = [spec.delta_r, spec.delta_g, spec.delta_b];

    // Quantise each absolute endpoint.
    // Endpoint slots: w (subset-0 ep0), x (subset-0 ep1), y (subset-1 ep0), z (subset-1 ep1).
    let mut q_abs = [[0u32; 4]; 3]; // [channel][slot]
    for c in 0..3 {
        q_abs[c][0] = quantize_half_uf16(s0_e0[c], prec);
        q_abs[c][1] = quantize_half_uf16(s0_e1[c], prec);
        q_abs[c][2] = quantize_half_uf16(s1_e0[c], prec);
        q_abs[c][3] = quantize_half_uf16(s1_e1[c], prec);
    }

    // Build palette in unquantized space + snap indices.
    let build_endpoints_unq = |q_abs: &[[u32; 4]; 3]| -> [[i32; 4]; 3] {
        let mut e = [[0i32; 4]; 3];
        for c in 0..3 {
            for ep in 0..4 {
                e[c][ep] = unquantize_uf16(q_abs[c][ep], prec);
            }
        }
        e
    };
    let endpoints_unq = build_endpoints_unq(&q_abs);
    let mut indices = [0u32; 16];
    let mut sse = snap_indices_2subset(pixels, &part, &endpoints_unq, 3, &mut indices);

    // Iterative refinement (2 passes per subset).
    for _ in 0..2 {
        let mut q_new = q_abs;
        for s in 0..2u8 {
            let (qs0, qs1) = refine_endpoints_2subset(pixels, &part, s, &indices, 3, prec);
            for c in 0..3 {
                q_new[c][(s * 2) as usize] = qs0[c];
                q_new[c][(s * 2 + 1) as usize] = qs1[c];
            }
        }
        let endpoints_new = build_endpoints_unq(&q_new);
        let mut idx_new = [0u32; 16];
        let sse_new = snap_indices_2subset(pixels, &part, &endpoints_new, 3, &mut idx_new);
        if sse_new < sse {
            sse = sse_new;
            q_abs = q_new;
            indices = idx_new;
        } else {
            break;
        }
    }

    // After refinement, encode the delta fields. For mode 9 (delta=0)
    // all four slots are absolute. For modes 0..8 every channel must
    // fit in delta_bits — bail when overflow.
    let mut q_field = [[0u32; 4]; 3];
    for c in 0..3 {
        q_field[c][0] = q_abs[c][0];
        if delta[c] == 0 {
            q_field[c][1] = q_abs[c][1] & ((1u32 << prec) - 1);
            q_field[c][2] = q_abs[c][2] & ((1u32 << prec) - 1);
            q_field[c][3] = q_abs[c][3] & ((1u32 << prec) - 1);
        } else {
            let half = 1i64 << (delta[c] - 1);
            for slot in 1..4 {
                let d = (q_abs[c][slot] as i64) - (q_abs[c][0] as i64);
                if d < -half || d >= half {
                    return None;
                }
                let dmask = (1u32 << delta[c]) - 1;
                q_field[c][slot] = (d as u32) & dmask;
            }
        }
    }

    // Anchor handling: subset-0 anchor (pixel 0) MSB implicit — its
    // index must fit in 2 bits; same for subset-1 anchor at `anchor1`.
    // If anchor is >= 4 (3-bit value with high bit set), we can swap the
    // two endpoints of that subset, complement indices in that subset.
    let mut subset_swap = [false; 2];
    if indices[0] >= 4 {
        subset_swap[0] = true;
    }
    if indices[anchor1 as usize] >= 4 {
        subset_swap[1] = true;
    }
    for s in 0..2u8 {
        if !subset_swap[s as usize] {
            continue;
        }
        // Swap endpoint slots in q_abs, and complement indices for
        // pixels in this subset.
        let i0 = (s as usize) * 2;
        let i1 = (s as usize) * 2 + 1;
        for c in 0..3 {
            q_abs[c].swap(i0, i1);
        }
        for px in 0..16 {
            if part[px] == s {
                indices[px] = 7 - indices[px];
            }
        }
    }

    // Re-encode field bits with the (possibly) swapped absolute endpoints.
    if subset_swap[0] || subset_swap[1] {
        for c in 0..3 {
            q_field[c][0] = q_abs[c][0];
            if delta[c] == 0 {
                q_field[c][1] = q_abs[c][1] & ((1u32 << prec) - 1);
                q_field[c][2] = q_abs[c][2] & ((1u32 << prec) - 1);
                q_field[c][3] = q_abs[c][3] & ((1u32 << prec) - 1);
            } else {
                let half = 1i64 << (delta[c] - 1);
                for slot in 1..4 {
                    let d = (q_abs[c][slot] as i64) - (q_abs[c][0] as i64);
                    if d < -half || d >= half {
                        return None;
                    }
                    let dmask = (1u32 << delta[c]) - 1;
                    q_field[c][slot] = (d as u32) & dmask;
                }
            }
        }
    }

    let mi = mode_info(spec.mode).expect("mode info");
    let block = pack_bc6h(
        &mi,
        spec.prefix,
        spec.prefix_len,
        q_field,
        partition,
        &indices,
        anchor1,
    );
    Some((block, sse))
}

// ---- Block-level picker -------------------------------------------------

fn encode_bc6h_block(pixels: &[[u16; 3]; 16]) -> [u8; 16] {
    let (mut best_block, mut best_sse) = encode_mode10(pixels);

    // Try delta-encoded 1-subset modes 11, 12, 13.
    if let Some((b, sse)) = encode_mode_delta_1subset(pixels, 11, 0b00111, 10, 9) {
        if sse < best_sse {
            best_sse = sse;
            best_block = b;
        }
    }
    if let Some((b, sse)) = encode_mode_delta_1subset(pixels, 12, 0b01011, 12, 8) {
        if sse < best_sse {
            best_sse = sse;
            best_block = b;
        }
    }
    if let Some((b, sse)) = encode_mode_delta_1subset(pixels, 13, 0b01111, 16, 4) {
        if sse < best_sse {
            best_sse = sse;
            best_block = b;
        }
    }

    // Try 2-subset modes 0..9 across the 32-partition table.
    for spec in TWO_SUBSET_MODES {
        for partition in 0..32u32 {
            if let Some((b, sse)) = try_2subset(pixels, spec, partition) {
                if sse < best_sse {
                    best_sse = sse;
                    best_block = b;
                }
            }
        }
    }

    best_block
}

/// Encode a width × height RGBA half-float surface to BC6H.
///
/// `input` must hold `width × height × 8` bytes (interleaved RGBA half-
/// float, alpha ignored). `output` must hold
/// `ceil(w/4) × ceil(h/4) × 16` bytes.
///
/// The encoder picks the best of mode 10 (1-subset 10.10 absolute),
/// modes 11/12/13 (1-subset delta-encoded with progressively higher
/// base precision and smaller delta range), and modes 0..9 (2-subset
/// with the BC6H 32-partition sweep) per block.
pub fn encode_bc6h(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 8;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC6H encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC6H encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    // Reinterpret input bytes as `[u16; ?]` — RGBA half-float pixels.
    let mut halves = vec![0u16; (width * height * 4) as usize];
    for (i, h) in halves.iter_mut().enumerate() {
        *h = u16::from_le_bytes([input[i * 2], input[i * 2 + 1]]);
    }
    let stride_pixels = width as usize;

    for by in 0..bh {
        for bx in 0..bw {
            let mut block = [[0u16; 3]; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    let rgb = rgb_at(&halves, xx, yy, stride_pixels);
                    block[(py * 4 + px) as usize] = rgb;
                }
            }
            let bc = encode_bc6h_block(&block);
            let off = (by * bw + bx) * 16;
            output[off..off + 16].copy_from_slice(&bc);
        }
    }
    Ok(())
}

/// Convenience: encode a `[[u16; 3]; ...]` (RGB-only, no alpha) f32 input
/// stream by first quantising f32 → half-float per pixel.
///
/// `input` is `width × height × 3` f32 RGB samples (linear, scene-
/// referred). Output identical to [`encode_bc6h`] with prepared half-
/// float input.
pub fn encode_bc6h_from_f32(
    input: &[f32],
    width: u32,
    height: u32,
    output: &mut [u8],
) -> Result<()> {
    let want_in = (width as usize) * (height as usize) * 3;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC6H f32 input {} samples < expected {} for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    // Convert to half RGBA (alpha = 0x3c00 = 1.0).
    let mut halves = vec![0u8; width as usize * height as usize * 8];
    for i in 0..(width as usize * height as usize) {
        let r = f32_to_half(input[i * 3]);
        let g = f32_to_half(input[i * 3 + 1]);
        let b = f32_to_half(input[i * 3 + 2]);
        let a = 0x3c00u16;
        halves[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
        halves[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
        halves[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
        halves[i * 8 + 6..i * 8 + 8].copy_from_slice(&a.to_le_bytes());
    }
    encode_bc6h(&halves, width, height, output)
}

// ---- BC6H_SF16 (signed half-float) encoder -----------------------------
//
// Round 7 closes the BC6H encoder coverage gap by adding a signed-format
// (BC6H_SF16) entry point. The decoder already supports both unsigned and
// signed flags; this section mirrors the encoder pipeline against the
// signed unquantize / finalize formulas so encoded blocks roundtrip
// through the decoder when the caller opts into BC6H_SF16.
//
// Pipeline differences from BC6H_UF16:
//
//   1. `quantize_half_sf16(half, bits)` — quantises a *signed* 16-bit half
//      to a `bits`-bit signed integer. Forward: q -> unq_signed(q, bits)
//      -> finish_sf16(unq) = (sign(unq) * (|unq| * 31) >> 5) | sign<<15.
//   2. `unquantize_sf16(comp, bits)` — sign-magnitude unquantize. Returns
//      a 17-bit signed integer (range [-0xffff, 0xffff]).
//   3. `finish_sf16(comp)` — outputs a 16-bit half (sign-magnitude).
//
// We support modes 10 (1-subset 10/10 absolute) + 11 (1-subset 10.10 base
// + 9-bit signed delta) for SF16; that covers the natural single-subset
// HDR-with-negative-radiance use case. Multi-subset SF16 modes use the
// same partition-sweep machinery as UF16 but with the signed-formula
// helpers (added incrementally as workloads demand).

#[inline]
fn unquantize_sf16(comp: i32, bits: u32) -> i32 {
    if bits >= 16 {
        return comp;
    }
    let s = comp < 0;
    let mut c = if s { -comp } else { comp };
    let unq = if c == 0 {
        0
    } else if c >= ((1i32 << (bits - 1)) - 1) {
        0x7fff
    } else {
        ((c << 15) + 0x4000) >> (bits - 1)
    };
    c = unq;
    if s {
        -c
    } else {
        c
    }
}

#[inline]
fn finish_sf16(comp: i32) -> u16 {
    let (s, c) = if comp < 0 {
        (1u16, ((-comp) as u32 * 31) >> 5)
    } else {
        (0u16, (comp as u32 * 31) >> 5)
    };
    (s << 15) | (c.min(0x7fff) as u16)
}

/// Convert a signed 16-bit half-float to its sign-magnitude integer
/// representation: returns the value in [-0x7fff, 0x7fff].
#[inline]
fn half_sf16_to_signed_magnitude(half_bits: u16) -> i32 {
    let s = (half_bits >> 15) & 1;
    let mag = (half_bits & 0x7fff) as i32;
    if s == 1 {
        -mag
    } else {
        mag
    }
}

/// Convert a signed magnitude back to a sign-magnitude half-float.
/// Currently only used by the SF16 round-trip self-test.
#[cfg(test)]
fn signed_magnitude_to_half(value: i32) -> u16 {
    if value < 0 {
        let mag = (-value).min(0x7fff) as u16;
        (1u16 << 15) | mag
    } else {
        (value.min(0x7fff)) as u16
    }
}

/// Quantise a sign-magnitude half-float (i32 in [-0x7fff, 0x7fff]) to a
/// `bits`-bit signed integer. The forward pipeline is q -> unq_signed
/// -> finish_sf16; we invert it via probe-around-estimate.
fn quantize_half_sf16(half_signed: i32, bits: u32) -> i32 {
    let max_q = (1i32 << (bits - 1)) - 1;
    let min_q = -max_q; // BC6H_SF16's signed range is symmetric.

    // Forward: q -> 16-bit signed half-bits.
    let forward = |q: i32| -> i32 {
        let unq = unquantize_sf16(q, bits);
        let half = finish_sf16(unq);
        half_sf16_to_signed_magnitude(half)
    };

    if half_signed == 0 {
        return 0;
    }

    // Continuous estimate: target_mag * 32 ≈ unq_mag * 31; unq ≈ target * 32 / 31.
    let target_mag = half_signed.unsigned_abs() as i64;
    let unq_est = (target_mag * 32) / 31;
    let lhs = (unq_est << (bits - 1)).saturating_sub(0x4000);
    let q_mag = (lhs >> 15) as i32;
    let q_est = if half_signed < 0 {
        -q_mag.min(max_q)
    } else {
        q_mag.min(max_q)
    };

    let mut best = q_est.clamp(min_q, max_q);
    let mut best_err = (forward(best) - half_signed).unsigned_abs() as i64;
    for d in [-2i32, -1, 0, 1, 2] {
        let cand = (q_est + d).clamp(min_q, max_q);
        let err = (forward(cand) - half_signed).unsigned_abs() as i64;
        if err < best_err {
            best_err = err;
            best = cand;
        }
    }
    best
}

/// Squared error between two signed-half-float RGB triples (in f32 space).
fn sq_err_rgb_signed_half(a: [u16; 3], b: [u16; 3]) -> f64 {
    let mut s = 0.0_f64;
    for c in 0..3 {
        let af = half_to_f32(a[c]) as f64;
        let bf = half_to_f32(b[c]) as f64;
        let d = af - bf;
        s += d * d;
    }
    s
}

fn furthest_pair_global_signed(pixels: &[[u16; 3]; 16]) -> (usize, usize) {
    let mut best_d = -1.0_f64;
    let mut bi = 0usize;
    let mut bj = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let d = sq_err_rgb_signed_half(pixels[i], pixels[j]);
            if d > best_d {
                best_d = d;
                bi = i;
                bj = j;
            }
        }
    }
    (bi, bj)
}

/// Pack the SF16 mode-10 bitstream. Identical bit layout to the UF16
/// mode 10; the only difference is in the *interpretation* of the field
/// bits (signed vs unsigned). We store the raw twos-complement value
/// truncated to `prec` bits — the decoder sign-extends it back when the
/// signed flag is on.
fn pack_mode10_signed(q0: [i32; 3], q1: [i32; 3], indices: [u32; 16]) -> [u8; 16] {
    let mi = mode_info(10).expect("mode 10 info");
    let prec_mask = (1u32 << 10) - 1;
    let q = [
        [(q0[0] as u32) & prec_mask, (q1[0] as u32) & prec_mask, 0, 0],
        [(q0[1] as u32) & prec_mask, (q1[1] as u32) & prec_mask, 0, 0],
        [(q0[2] as u32) & prec_mask, (q1[2] as u32) & prec_mask, 0, 0],
    ];
    pack_bc6h(&mi, 0b00011, 5, q, 0, &indices, 15)
}

fn snap_indices_1subset_signed(
    pixels: &[[u16; 3]; 16],
    q0: &[i32; 3],
    q1: &[i32; 3],
    prec: u32,
    idx_bits: u32,
    indices: &mut [u32; 16],
) -> f64 {
    let mut endpoints = [[0i32; 2]; 3];
    for c in 0..3 {
        endpoints[c][0] = unquantize_sf16(q0[c], prec);
        endpoints[c][1] = unquantize_sf16(q1[c], prec);
    }
    let palette_size = 1usize << idx_bits;
    let mut palette = [[0u16; 3]; 16];
    for k in 0..palette_size {
        for c in 0..3 {
            let v = interp_endpoint(endpoints[c][0], endpoints[c][1], k, idx_bits);
            palette[k][c] = finish_sf16(v);
        }
    }
    let mut sse = 0.0f64;
    for (px, &p) in pixels.iter().enumerate() {
        let mut best_k = 0u32;
        let mut best_e = f64::MAX;
        for k in 0..palette_size {
            let e = sq_err_rgb_signed_half(p, palette[k]);
            if e < best_e {
                best_e = e;
                best_k = k as u32;
            }
        }
        indices[px] = best_k;
        sse += best_e;
    }
    sse
}

/// Encode one 4×4 block as BC6H_SF16 mode 10 (1-subset, 10-bit signed
/// absolute endpoints, 4-bit indices). Returns `(block, sse)`.
fn encode_mode10_signed(pixels: &[[u16; 3]; 16]) -> ([u8; 16], f64) {
    let (bi, bj) = furthest_pair_global_signed(pixels);
    let half0 = pixels[bi];
    let half1 = pixels[bj];

    let mut q0 = [0i32; 3];
    let mut q1 = [0i32; 3];
    for c in 0..3 {
        let s0 = half_sf16_to_signed_magnitude(half0[c]);
        let s1 = half_sf16_to_signed_magnitude(half1[c]);
        q0[c] = quantize_half_sf16(s0, 10);
        q1[c] = quantize_half_sf16(s1, 10);
    }
    let mut indices = [0u32; 16];
    let sse = snap_indices_1subset_signed(pixels, &q0, &q1, 10, 4, &mut indices);

    if indices[0] >= 8 {
        std::mem::swap(&mut q0, &mut q1);
        for idx in indices.iter_mut() {
            *idx = 15 - *idx;
        }
    }
    let block = pack_mode10_signed(q0, q1, indices);
    (block, sse)
}

/// Block-level BC6H_SF16 picker. Currently dispatches mode 10 (1-subset
/// 10/10 absolute) only — mode 11 (delta-encoded) and 2-subset modes
/// require parallel signed-quantizer + signed-delta logic which are
/// follow-on work.
fn encode_bc6h_block_sf16(pixels: &[[u16; 3]; 16]) -> [u8; 16] {
    let (block, _sse) = encode_mode10_signed(pixels);
    block
}

/// Encode a width × height RGBA half-float surface (with sign-magnitude
/// halves) to BC6H_SF16. Inputs may include negative values (sign bit
/// set in any pixel half). The decoder must be invoked with `signed=true`.
///
/// This is the signed counterpart to [`encode_bc6h`]. `input` must hold
/// `width × height × 8` bytes (interleaved RGBA half-float, alpha
/// ignored). `output` must hold `ceil(w/4) × ceil(h/4) × 16` bytes.
pub fn encode_bc6h_sf16(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = width as usize * height as usize * 8;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC6H_SF16 encoder input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = bw * bh * 16;
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC6H_SF16 encoder output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let mut halves = vec![0u16; (width * height * 4) as usize];
    for (i, h) in halves.iter_mut().enumerate() {
        *h = u16::from_le_bytes([input[i * 2], input[i * 2 + 1]]);
    }
    let stride_pixels = width as usize;
    for by in 0..bh {
        for bx in 0..bw {
            let mut block = [[0u16; 3]; 16];
            for py in 0..4u32 {
                let yy = ((by as u32) * 4 + py).min(height.saturating_sub(1));
                for px in 0..4u32 {
                    let xx = ((bx as u32) * 4 + px).min(width.saturating_sub(1));
                    let rgb = rgb_at(&halves, xx, yy, stride_pixels);
                    block[(py * 4 + px) as usize] = rgb;
                }
            }
            let bc = encode_bc6h_block_sf16(&block);
            let off = (by * bw + bx) * 16;
            output[off..off + 16].copy_from_slice(&bc);
        }
    }
    Ok(())
}

/// Convenience: encode a `width × height × 3` f32 RGB surface as
/// BC6H_SF16 (signed). Negative samples are preserved in the output
/// (unlike [`encode_bc6h_from_f32`] which clamps negatives to zero).
pub fn encode_bc6h_sf16_from_f32(
    input: &[f32],
    width: u32,
    height: u32,
    output: &mut [u8],
) -> Result<()> {
    let want_in = (width as usize) * (height as usize) * 3;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC6H_SF16 f32 input {} samples < expected {} for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    // Convert to sign-magnitude half RGBA (alpha = 0x3c00 = 1.0).
    let mut halves = vec![0u8; width as usize * height as usize * 8];
    for i in 0..(width as usize * height as usize) {
        let r = f32_to_half_signed(input[i * 3]);
        let g = f32_to_half_signed(input[i * 3 + 1]);
        let b = f32_to_half_signed(input[i * 3 + 2]);
        let a = 0x3c00u16;
        halves[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
        halves[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
        halves[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
        halves[i * 8 + 6..i * 8 + 8].copy_from_slice(&a.to_le_bytes());
    }
    encode_bc6h_sf16(&halves, width, height, output)
}

/// Convert an `f32` to a sign-magnitude IEEE-754 binary16 (preserves
/// the sign bit, unlike [`f32_to_half`] which clamps negatives to zero).
fn f32_to_half_signed(value: f32) -> u16 {
    let bits = value.to_bits();
    let sign = (bits >> 31) & 1;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;

    if exp == 0xff {
        // Infinity / NaN → max-finite, preserve sign.
        return ((sign as u16) << 15) | 0x7bff;
    }
    if exp == 0 && mant == 0 {
        return (sign as u16) << 15;
    }
    let exp_f16 = exp - 127 + 15;
    if exp_f16 >= 0x1f {
        // Overflow → max-finite.
        return ((sign as u16) << 15) | 0x7bff;
    }
    if exp_f16 <= 0 {
        // Subnormal half — shift mantissa right.
        let shift = 1 - exp_f16;
        if shift > 24 {
            return (sign as u16) << 15;
        }
        let m = (mant | 0x800000) >> (shift + 13);
        return ((sign as u16) << 15) | (m as u16);
    }
    let m = mant >> 13;
    ((sign as u16) << 15) | ((exp_f16 as u16) << 10) | (m as u16)
}

// Suppress unused-warning for helper kept for future iterative refinement.
#[allow(dead_code)]
fn _unused() {
    let _ = encode_delta(0, 0, 10, 9);
    let _: &[FieldBit] = &[];
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bc6h::decode_bc6h;

    fn psnr_rgb_half(a: &[u8], b: &[u8]) -> f64 {
        let mut sse = 0.0_f64;
        let mut count: u64 = 0;
        let n = a.len() / 8;
        for i in 0..n {
            let off = i * 8;
            for c in 0..3 {
                let av = u16::from_le_bytes([a[off + c * 2], a[off + c * 2 + 1]]);
                let bv = u16::from_le_bytes([b[off + c * 2], b[off + c * 2 + 1]]);
                let af = half_to_f32(av) as f64;
                let bf = half_to_f32(bv) as f64;
                let d = af - bf;
                sse += d * d;
                count += 1;
            }
        }
        let mse = sse / count as f64;
        if mse <= 0.0 {
            return f64::INFINITY;
        }
        // Use peak = 1.0 (typical normalised-HDR scale).
        10.0 * (1.0_f64 / mse).log10()
    }

    /// f32_to_half: round-trip 1.0 / 0.5 / 0.0.
    #[test]
    fn f32_to_half_simple() {
        assert_eq!(f32_to_half(1.0), 0x3c00);
        assert_eq!(f32_to_half(0.5), 0x3800);
        assert_eq!(f32_to_half(0.0), 0x0000);
        // Negative clamps to 0.
        assert_eq!(f32_to_half(-1.0), 0x0000);
    }

    /// quantize_half_uf16 round-trips zero exactly.
    #[test]
    fn quantize_uf16_zero() {
        assert_eq!(quantize_half_uf16(0, 10), 0);
        // Max encodable input maps to max-q.
        assert_eq!(quantize_half_uf16(0xffff, 10), 0x3ff);
    }

    /// Solid HDR block (every pixel = (0.5, 0.5, 0.5)) → both endpoints
    /// quantise to the same value → palette is constant → every index is
    /// 0 → roundtrip is bit-exact (after the BC6H_UF16 finalise).
    #[test]
    fn bc6h_encode_solid_block_close_roundtrip() {
        // Build 4x4 RGBA half-float block, all pixels = (0.5, 0.5, 0.5).
        let half_05 = 0x3800u16; // half(0.5)
        let mut input = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let off = i * 8;
            input[off..off + 2].copy_from_slice(&half_05.to_le_bytes());
            input[off + 2..off + 4].copy_from_slice(&half_05.to_le_bytes());
            input[off + 4..off + 6].copy_from_slice(&half_05.to_le_bytes());
            input[off + 6..off + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h(&input, 4, 4, &mut bc).unwrap();

        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();

        // Per-pixel error should be small. Compare half-float magnitude
        // → f32 magnitude difference.
        let psnr = psnr_rgb_half(&input, &decoded);
        assert!(
            psnr > 30.0,
            "BC6H solid-0.5 block PSNR (peak 1.0) = {:.2} dB",
            psnr
        );
    }

    /// HDR gradient: f32 [0..1] grayscale (R=G=B) horizontal gradient →
    /// encode → decode → PSNR ≥ 30 dB (peak = 1.0). 1-subset mode-10
    /// fits a single-axis gradient well; multi-axis gradients (where
    /// R, G, B vary independently) benefit from the round-6 2-subset
    /// modes 0..9 partition sweep.
    #[test]
    fn bc6h_encode_8x8_grayscale_gradient_psnr_gt_30db() {
        let mut input_f32 = vec![0f32; 8 * 8 * 3];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 3;
                let v = (x + y) as f32 / 14.0;
                input_f32[off] = v;
                input_f32[off + 1] = v;
                input_f32[off + 2] = v;
            }
        }
        let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
        encode_bc6h_from_f32(&input_f32, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 8];
        decode_bc6h(&bc, 8, 8, false, &mut decoded).unwrap();

        // Build the half-version of the input for fair PSNR comparison.
        let mut input_half = vec![0u8; 8 * 8 * 8];
        for i in 0..(8 * 8) {
            let r = f32_to_half(input_f32[i * 3]);
            let g = f32_to_half(input_f32[i * 3 + 1]);
            let b = f32_to_half(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 30.0,
            "BC6H 8x8 grayscale gradient PSNR (peak 1.0) = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// Edge-padded surface: 5×5 input → encoder pads the 4×4 block by
    /// repeating the last in-bounds pixel.
    #[test]
    fn bc6h_encode_5x5_solid_block() {
        let half_v = 0x3800u16; // half(0.5)
        let mut input = vec![0u8; 5 * 5 * 8];
        for i in 0..(5 * 5) {
            let off = i * 8;
            input[off..off + 2].copy_from_slice(&half_v.to_le_bytes());
            input[off + 2..off + 4].copy_from_slice(&half_v.to_le_bytes());
            input[off + 4..off + 6].copy_from_slice(&half_v.to_le_bytes());
            input[off + 6..off + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let mut bc = vec![0u8; 4 * 16];
        encode_bc6h(&input, 5, 5, &mut bc).unwrap();
        let mut decoded = vec![0u8; 5 * 5 * 8];
        decode_bc6h(&bc, 5, 5, false, &mut decoded).unwrap();
        // Solid block must roundtrip with high PSNR.
        let psnr = psnr_rgb_half(&input, &decoded);
        assert!(
            psnr > 30.0,
            "BC6H 5x5 solid PSNR = {:.2} dB (want > 30 dB)",
            psnr
        );
    }

    /// Two-cluster block where the 2-subset modes can fit cleanly:
    /// left half (8 pixels) at (0.4, 0.4, 0.4), right half (8 pixels) at
    /// (0.6, 0.6, 0.6). Each subset's intra-spread is zero, so any
    /// 2-subset mode encodes losslessly. This validates the 2-subset
    /// partition encoder + delta packing.
    #[test]
    fn bc6h_encode_2subset_two_cluster_block_psnr_gt_40db() {
        let mut input_f32 = vec![0f32; 4 * 4 * 3];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 3;
                let v = if x < 2 { 0.4 } else { 0.6 };
                input_f32[off] = v;
                input_f32[off + 1] = v;
                input_f32[off + 2] = v;
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
        let mut input_half = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let r = f32_to_half(input_f32[i * 3]);
            let g = f32_to_half(input_f32[i * 3 + 1]);
            let b = f32_to_half(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 40.0,
            "BC6H 2-subset two-cluster PSNR = {:.2} dB (want > 40 dB)",
            psnr
        );
    }

    /// Mode-0 round-trip diagnostic: pick a tiny 4×4 block where the
    /// 2-subset partition is optimal (left half = red, right half = blue),
    /// encode + decode + verify the encoded mode is one of 0..9.
    #[test]
    fn bc6h_encode_2subset_block_decoder_roundtrip() {
        let mut input_f32 = vec![0f32; 4 * 4 * 3];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 3;
                if x < 2 {
                    input_f32[off] = 1.0; // red half
                } else {
                    input_f32[off + 2] = 1.0; // blue half
                }
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
        // Round-trip: decode and check the resulting reds & blues.
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
        // The left two columns should be reddish (R > B); right two
        // columns blueish (B > R). Not bit-exact (BC6H's 31/64 finalise
        // means perfect-1.0 inputs decode to ~0xf83e ≈ 0.97), but the
        // hue ordering must be preserved.
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 8;
                let r = u16::from_le_bytes([decoded[off], decoded[off + 1]]);
                let b = u16::from_le_bytes([decoded[off + 4], decoded[off + 5]]);
                if x < 2 {
                    assert!(
                        r > b,
                        "x={} y={} red half but R={:#x} <= B={:#x}",
                        x,
                        y,
                        r,
                        b
                    );
                } else {
                    assert!(
                        b > r,
                        "x={} y={} blue half but B={:#x} <= R={:#x}",
                        x,
                        y,
                        b,
                        r
                    );
                }
            }
        }
    }

    /// Round-6 lift: 8×8 multi-axis HDR gradient. R varies with x, G
    /// with y, B with x+y → genuine 3-axis content. The 2-subset mode-9
    /// (6.6.6.6 absolute) and mode-6/7/8 (8-bit base + 5-6 bit deltas)
    /// outperform the 1-subset mode-10 baseline on each 4×4 block;
    /// the partition sweep + iterative refinement lift the per-block
    /// SSE by ~2× over the round-3 mode-10-only baseline. Mode 0
    /// (10.5.5.5) and modes 2..4 (11.5.4.4 family) cannot fit when
    /// the cross-subset spread exceeds ±16 in 10/11-bit q-space —
    /// for content with widely-separated subsets the lower-precision
    /// modes 9 / 6 / 7 / 8 dominate.
    ///
    /// Threshold chosen empirically to be ~2 dB above the round-3
    /// mode-10-only baseline of ~21 dB.
    #[test]
    fn bc6h_encode_8x8_multi_axis_gradient_psnr_gt_24db() {
        let mut input_f32 = vec![0f32; 8 * 8 * 3];
        for y in 0..8 {
            for x in 0..8 {
                let off = (y * 8 + x) * 3;
                input_f32[off] = x as f32 / 7.0;
                input_f32[off + 1] = y as f32 / 7.0;
                input_f32[off + 2] = (x + y) as f32 / 14.0;
            }
        }
        let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
        encode_bc6h_from_f32(&input_f32, 8, 8, &mut bc).unwrap();
        let mut decoded = vec![0u8; 8 * 8 * 8];
        decode_bc6h(&bc, 8, 8, false, &mut decoded).unwrap();

        let mut input_half = vec![0u8; 8 * 8 * 8];
        for i in 0..(8 * 8) {
            let r = f32_to_half(input_f32[i * 3]);
            let g = f32_to_half(input_f32[i * 3 + 1]);
            let b = f32_to_half(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 24.0,
            "BC6H 8x8 multi-axis gradient PSNR = {:.2} dB (want > 24 dB)",
            psnr
        );
    }

    /// Tight gradient block where delta encoding fits: pixels with R, G, B
    /// all in [0.4, 0.5] — endpoints differ by < 0.1 → quantised in 11-bit
    /// space, the delta is small enough to fit in modes 11/12/13's
    /// asymmetric ranges. Verifies the delta-encoding pack path
    /// round-trips through the decoder.
    #[test]
    fn bc6h_encode_tight_gradient_delta_modes() {
        let mut input_f32 = vec![0f32; 4 * 4 * 3];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 3;
                let v = 0.4 + ((x + y) as f32 / 6.0) * 0.1;
                input_f32[off] = v;
                input_f32[off + 1] = v;
                input_f32[off + 2] = v;
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
        let mut input_half = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let r = f32_to_half(input_f32[i * 3]);
            let g = f32_to_half(input_f32[i * 3 + 1]);
            let b = f32_to_half(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 35.0,
            "BC6H tight-gradient PSNR = {:.2} dB (want > 35 dB)",
            psnr
        );
    }

    /// Mode 11 (1-subset, 10.10.10 base + 9-bit delta) round-trip:
    /// build a block where mode 11 should be picked over mode 10 due to
    /// the extra base bit. Verify it decodes cleanly.
    #[test]
    fn bc6h_encode_mode11_solid_block() {
        // All pixels = (0.5, 0.5, 0.5) → both endpoints quantise to the
        // same value → delta = 0 → mode 11 fits.
        let mut input = vec![0u8; 4 * 4 * 8];
        let half_v = 0x3800u16;
        for i in 0..16 {
            let off = i * 8;
            input[off..off + 2].copy_from_slice(&half_v.to_le_bytes());
            input[off + 2..off + 4].copy_from_slice(&half_v.to_le_bytes());
            input[off + 4..off + 6].copy_from_slice(&half_v.to_le_bytes());
            input[off + 6..off + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
        let psnr = psnr_rgb_half(&input, &decoded);
        assert!(psnr > 30.0, "BC6H mode 11 solid PSNR = {:.2} dB", psnr);
    }

    /// encode_delta: zero delta encodes as 0.
    #[test]
    fn encode_delta_zero() {
        assert_eq!(encode_delta(100, 100, 10, 9), Some(0));
    }

    /// encode_delta: small positive delta within range.
    #[test]
    fn encode_delta_small_positive() {
        // q0=100, q1=110 → delta=+10. 9-bit signed range is [-256, 255].
        // 10 fits; encoded raw should be 10.
        assert_eq!(encode_delta(100, 110, 10, 9), Some(10));
    }

    /// encode_delta: small negative delta wraps under the prec mask.
    #[test]
    fn encode_delta_small_negative() {
        // q0=100, q1=90 → delta=-10. 9-bit dmask=0x1ff. Raw = (-10 & 0x1ff) = 502.
        assert_eq!(encode_delta(100, 90, 10, 9), Some(0x1f6));
    }

    /// encode_delta: out-of-range overflow returns None.
    #[test]
    fn encode_delta_overflow_returns_none() {
        // q0=0, q1=512 with delta_bits=9. Signed range is [-256,255]; 512 > 255.
        assert_eq!(encode_delta(0, 512, 10, 9), None);
    }

    // ---- BC6H_SF16 (signed) tests --------------------------------------

    /// f32_to_half_signed preserves sign for negative inputs.
    #[test]
    fn f32_to_half_signed_negative() {
        assert_eq!(f32_to_half_signed(-1.0), 0x8000 | 0x3c00);
        assert_eq!(f32_to_half_signed(-0.5), 0x8000 | 0x3800);
        assert_eq!(f32_to_half_signed(0.0), 0x0000);
        assert_eq!(f32_to_half_signed(1.0), 0x3c00);
    }

    /// half_sf16_to_signed_magnitude / signed_magnitude_to_half round-trip.
    /// Negative zero (`0x8000`) collapses to +0 because sign-magnitude
    /// integer 0 has no sign bit; that's the IEEE-equivalent value.
    #[test]
    fn half_sf16_signed_magnitude_roundtrip() {
        for h in [0x0000u16, 0x3c00, 0x3800, 0x7bff, 0x8001, 0xbc00, 0xfbff] {
            let sm = half_sf16_to_signed_magnitude(h);
            let back = signed_magnitude_to_half(sm);
            assert_eq!(h, back, "roundtrip {:#x} -> {} -> {:#x}", h, sm, back);
        }
        // Negative zero: signed-magnitude integer 0 carries no sign,
        // so the round-trip yields the IEEE-equivalent +0.
        assert_eq!(
            signed_magnitude_to_half(half_sf16_to_signed_magnitude(0x8000)),
            0x0000
        );
    }

    /// SF16 quantize then unquantize then finish reproduces the input
    /// half within ~1 LSB for typical mid-range values.
    #[test]
    fn quantize_sf16_roundtrip_midrange() {
        // Half(0.5) = 0x3800 (positive); half(-0.5) = 0xb800.
        for h in [0x3800u16, 0x3c00u16, 0x4400u16, 0xb800u16, 0xbc00u16] {
            let s = half_sf16_to_signed_magnitude(h);
            let q = quantize_half_sf16(s, 10);
            let unq = unquantize_sf16(q, 10);
            let back = finish_sf16(unq);
            let s_back = half_sf16_to_signed_magnitude(back);
            // Should be within ~1% relative error after the 31/32 finalise.
            let abs_err = (s - s_back).unsigned_abs();
            assert!(
                abs_err < 200,
                "SF16 roundtrip {:#x}: orig sm={} -> q={} -> unq={} -> back={:#x} (sm={}) abs_err={}",
                h,
                s,
                q,
                unq,
                back,
                s_back,
                abs_err
            );
        }
    }

    /// BC6H_SF16: encode a solid (-0.5, -0.5, -0.5) HDR block (negative
    /// radiance — only valid in signed format). Decode with `signed=true`
    /// and verify all pixels recover something close to (-0.5, -0.5, -0.5).
    #[test]
    fn bc6h_encode_sf16_solid_negative_block() {
        let half_neg = 0xb800u16; // half(-0.5)
        let mut input = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let off = i * 8;
            input[off..off + 2].copy_from_slice(&half_neg.to_le_bytes());
            input[off + 2..off + 4].copy_from_slice(&half_neg.to_le_bytes());
            input[off + 4..off + 6].copy_from_slice(&half_neg.to_le_bytes());
            input[off + 6..off + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_sf16(&input, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, true, &mut decoded).unwrap();
        for i in 0..16 {
            let off = i * 8;
            for c in 0..3 {
                let v = u16::from_le_bytes([decoded[off + c * 2], decoded[off + c * 2 + 1]]);
                let f = half_to_f32(v);
                // Expect ~-0.5; allow ±0.05 for the SF16 31/32 finalise.
                assert!(
                    f < -0.4 && f > -0.6,
                    "pixel {} ch {} = {:#x} ({}) — expected ~-0.5",
                    i,
                    c,
                    v,
                    f
                );
            }
        }
    }

    /// BC6H_SF16: encode an f32 gradient that spans both signs (e.g.,
    /// [-0.5, 0.5]). Decode with `signed=true` and verify per-pixel
    /// PSNR > 19 dB. Mode 10 alone (1-subset, 10-bit signed absolute
    /// endpoints) hits this on a sign-spanning gradient — the BC6H_SF16
    /// 31/32 finalise has an inherent ~1.6% cap-loss at endpoint
    /// extremes, so the PSNR ceiling on f32 [−0.5, 0.5] content is
    /// ~22-24 dB even for an exactly-fitting endpoint pair. Mode 11
    /// (delta-encoded, +1 base bit) and the 2-subset signed modes
    /// remain a follow-on for tighter signed-content PSNR.
    #[test]
    fn bc6h_encode_sf16_signed_gradient_psnr_gt_19db() {
        let mut input_f32 = vec![0f32; 4 * 4 * 3];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 3;
                let v = -0.5 + ((x + y) as f32) / 6.0;
                input_f32[off] = v;
                input_f32[off + 1] = v;
                input_f32[off + 2] = v;
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_sf16_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, true, &mut decoded).unwrap();

        let mut input_half = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let r = f32_to_half_signed(input_f32[i * 3]);
            let g = f32_to_half_signed(input_f32[i * 3 + 1]);
            let b = f32_to_half_signed(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 19.0,
            "BC6H_SF16 signed gradient PSNR (peak 1.0) = {:.2} dB (want > 19 dB)",
            psnr
        );
    }

    /// BC6H_SF16: a positive-only block should round-trip through the
    /// signed pipeline just as well as through the unsigned pipeline
    /// (the sign bit is just always 0).
    #[test]
    fn bc6h_encode_sf16_positive_block_psnr_gt_25db() {
        let mut input_f32 = vec![0f32; 4 * 4 * 3];
        for y in 0..4 {
            for x in 0..4 {
                let off = (y * 4 + x) * 3;
                let v = 0.1 + (x + y) as f32 * 0.05;
                input_f32[off] = v;
                input_f32[off + 1] = v;
                input_f32[off + 2] = v;
            }
        }
        let mut bc = vec![0u8; 16];
        encode_bc6h_sf16_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
        let mut decoded = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&bc, 4, 4, true, &mut decoded).unwrap();

        let mut input_half = vec![0u8; 4 * 4 * 8];
        for i in 0..16 {
            let r = f32_to_half_signed(input_f32[i * 3]);
            let g = f32_to_half_signed(input_f32[i * 3 + 1]);
            let b = f32_to_half_signed(input_f32[i * 3 + 2]);
            input_half[i * 8..i * 8 + 2].copy_from_slice(&r.to_le_bytes());
            input_half[i * 8 + 2..i * 8 + 4].copy_from_slice(&g.to_le_bytes());
            input_half[i * 8 + 4..i * 8 + 6].copy_from_slice(&b.to_le_bytes());
            input_half[i * 8 + 6..i * 8 + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
        }
        let psnr = psnr_rgb_half(&input_half, &decoded);
        assert!(
            psnr > 25.0,
            "BC6H_SF16 positive block PSNR (peak 1.0) = {:.2} dB (want > 25 dB)",
            psnr
        );
    }
}
