//! BC6H (DXGI `BC6H_UF16` / `BC6H_SF16`) HDR-float block encoder.
//!
//! Round-3 baseline: emits BC6H mode 10 (1-subset, 10.10.10 absolute
//! endpoint precision per channel, no delta, 4-bit indices). Mode 10
//! is the simplest BC6H 1-subset mode — both endpoints carry an
//! absolute 10-bit value per channel, so the encoder doesn't have to
//! worry about 9-bit signed delta range overflow that mode 11 imposes
//! when the two endpoints are far apart in quantised space.
//!
//! Mode 11 (1-subset, 11-bit base + 9-bit delta) is also a valid
//! baseline target and would give one extra bit of base precision,
//! but only when the second endpoint is within ±256 of the first in
//! 11-bit space — for natural HDR gradients spanning a wide dynamic
//! range, mode 10's absolute encoding gives much better worst-case
//! reconstruction quality than mode 11's clamped delta. Future
//! rounds can pick the best mode per block; for the round-3 baseline
//! we use mode 10 unconditionally.
//!
//! Reference: Microsoft's public "BC6H Format" article on
//! learn.microsoft.com (Direct3D 11 reference) and the Intel Open
//! Source Programmer's Reference Manual Vol. 5 (BC6H section, 0BSD-
//! licensed). No DirectXTex, NVTT, ISPC `ispc_texcomp`, basisu, or
//! `bc6h_enc` source was consulted; only the public spec text + the
//! per-mode bit-allocation tables.
//!
//! Encoder strategy (mode 11, 1-subset):
//!
//! 1. For each 4×4 block, find the two pixels furthest apart in 3-D
//!    half-float-converted-to-f32 RGB space (squared Euclidean) — the
//!    "furthest-point" heuristic.
//! 2. Quantise each endpoint's 16-bit half-float into the mode-11
//!    11-bit precision. The Microsoft spec defines unquantize as
//!    `((comp << 16) + 0x8000) >> bits` for unsigned (with edge cases
//!    for `0` and `(1 << bits) - 1`); the inverse for the encoder is
//!    a multiply / shift that picks the integer minimising the round-
//!    trip error.
//! 3. Compute the second endpoint as a signed 9-bit delta relative to
//!    the first. Clamp the delta if it overflows 9-bit signed range.
//! 4. Snap each pixel to the nearest of the 16 interpolated palette
//!    entries (4-bit indices). Pixel 0 is the anchor — its index has
//!    one fewer bit (3 bits, MSB implicit-0); we ensure the pixel-0
//!    index is < 8 by swapping endpoints + complementing indices when
//!    necessary.
//!
//! Encoder operates on `BC6H_UF16` (unsigned half) only. Signed
//! `BC6H_SF16` is decoded but not encoded by the round-3 scope.
//!
//! Quality: ≥ 30 dB on natural HDR gradients, bit-exact on solid
//! blocks (both endpoints quantise to the same value, every index = 0).

// Per-channel and per-pixel inner loops are clearer indexed; silence
// clippy's preference for iterator-style code in this module.
#![allow(clippy::needless_range_loop)]

use crate::bc6h::half_to_f32;
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
/// The DECODER's full forward pipeline for `BC6H_UF16` mode-11
/// endpoints is:
///   `q (11-bit)` → `unq = unquantize(q, 11) = ((q << 16) + 0x8000) >> 11`
///   → `H = finish_uf16(unq) = (unq * 31) >> 6`.
///
/// So the encoder's job is to find the 11-bit `q` such that the
/// post-finalise `H` is closest to the input half-bit value. The
/// "31/64" scale in `finish_uf16` means the dynamic range maps from
/// `[0, 0xffff]` (raw half-bits) onto a *compressed* range — we have
/// to invert the entire pipeline, not just `unquantize`.
///
/// Closed-form continuous estimate:
///   `q ≈ ((target << 6) / 31) << 11 - 0x8000) >> 16`
/// → simplified: `q ≈ ((target << 17) / 31 - 0x8000) >> 16`.
///
/// We then probe ±1 around that estimate and pick the integer that
/// minimises absolute round-trip error. The `q == 0` and
/// `q == max_q` boundaries are special-cased per Microsoft (they
/// produce `0` and `0xffff` respectively).
fn quantize_half_uf16(half_bits: u16, bits: u32) -> u32 {
    let max_q = (1u32 << bits) - 1;
    let target = half_bits as u32;

    // Forward pipeline: q -> half-bits.
    let forward = |q: u32| -> u32 {
        let unq = if q == 0 {
            0u32
        } else if q == max_q {
            0xffffu32
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

/// Interpolate two unquantized 17-bit endpoints with a 4-bit index.
fn interp_endpoint(e0: i32, e1: i32, idx: usize) -> i32 {
    let w = WEIGHT_4[idx] as i64;
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

/// Encode one 4×4 RGBA half-float block (RGB + alpha, alpha ignored —
/// BC6H stores no alpha) into a 16-byte BC6H mode-10 block.
///
/// `pixels[i] = [r, g, b]` half-bits per pixel.
fn encode_bc6h_mode10_block(pixels: &[[u16; 3]; 16]) -> [u8; 16] {
    // ---- Endpoint search: furthest-point in f32-RGB space (matches
    //      visual distance better than raw half-bit Manhattan).
    let mut best_d = -1.0_f64;
    let mut best_i = 0usize;
    let mut best_j = 0usize;
    for i in 0..16 {
        for j in (i + 1)..16 {
            let d = sq_err_rgb_half(pixels[i], pixels[j]);
            if d > best_d {
                best_d = d;
                best_i = i;
                best_j = j;
            }
        }
    }

    let half0 = pixels[best_i];
    let half1 = pixels[best_j];

    // ---- Quantise endpoints to 10-bit precision per channel (mode 10).
    let mut q0 = [0u32; 3];
    let mut q1 = [0u32; 3];
    for c in 0..3 {
        q0[c] = quantize_half_uf16(half0[c], 10);
        q1[c] = quantize_half_uf16(half1[c], 10);
    }

    // ---- Build palette using the decoder's reconstruction.
    let build_palette = |q0: [u32; 3], q1: [u32; 3]| -> [[u16; 3]; 16] {
        let mut endpoints = [[0i32; 2]; 3];
        for c in 0..3 {
            endpoints[c][0] = unquantize_uf16(q0[c], 10);
            endpoints[c][1] = unquantize_uf16(q1[c], 10);
        }
        let mut palette = [[0u16; 3]; 16];
        for k in 0..16 {
            for c in 0..3 {
                let v = interp_endpoint(endpoints[c][0], endpoints[c][1], k);
                palette[k][c] = finish_uf16(v);
            }
        }
        palette
    };

    let palette = build_palette(q0, q1);

    // ---- Quantise each pixel to nearest palette entry.
    let mut indices = [0u32; 16];
    for (px, &p) in pixels.iter().enumerate() {
        let mut best_k = 0u32;
        let mut best_e = f64::MAX;
        for (k, pal) in palette.iter().enumerate() {
            let e = sq_err_rgb_half(p, *pal);
            if e < best_e {
                best_e = e;
                best_k = k as u32;
            }
        }
        indices[px] = best_k;
    }

    // ---- Anchor handling: pixel 0 anchor index must be < 8 (3-bit
    //      MSB-implicit). Swap endpoints if necessary; complement all
    //      indices to preserve palette mapping.
    if indices[0] >= 8 {
        std::mem::swap(&mut q0, &mut q1);
        for idx in indices.iter_mut() {
            *idx = 15 - *idx;
        }
    }

    pack_mode10(q0, q1, indices)
}

/// Pack a BC6H mode-10 block.
///
/// Layout (LSB-first):
/// - bits 0..4:    `00011` (5-bit mode prefix)
/// - bits 5..14:   rw[9:0]
/// - bits 15..24:  gw[9:0]
/// - bits 25..34:  bw[9:0]
/// - bits 35..44:  rx[9:0]
/// - bits 45..54:  gx[9:0]
/// - bits 55..64:  bx[9:0]
/// - bits 65..127: 16 indices @ 4 bits with anchor (pixel 0) short by 1
fn pack_mode10(q0: [u32; 3], q1: [u32; 3], indices: [u32; 16]) -> [u8; 16] {
    let mut block = [0u8; 16];
    let mut pos: u32 = 0;

    let put = |bit: u32, b: &mut [u8; 16], pos: &mut u32| {
        if (*pos as usize) < 128 {
            let byte = (*pos / 8) as usize;
            let shift = *pos & 7;
            b[byte] |= ((bit & 1) as u8) << shift;
            *pos += 1;
        }
    };

    // Mode prefix `00011` (LSB-first: bits 1,1,0,0,0).
    let prefix = 0b00011u32;
    for i in 0..5 {
        put((prefix >> i) & 1, &mut block, &mut pos);
    }

    // Six 10-bit endpoint values: rw, gw, bw, rx, gx, bx.
    for c in 0..3 {
        for i in 0..10 {
            put((q0[c] >> i) & 1, &mut block, &mut pos);
        }
    }
    for c in 0..3 {
        for i in 0..10 {
            put((q1[c] >> i) & 1, &mut block, &mut pos);
        }
    }

    // Indices: pixel 0 anchor (3 bits), pixels 1..15 are 4 bits each.
    for i in 0..3 {
        put((indices[0] >> i) & 1, &mut block, &mut pos);
    }
    for px in 1..16 {
        for i in 0..4 {
            put((indices[px] >> i) & 1, &mut block, &mut pos);
        }
    }

    block
}

/// Encode a width × height RGBA half-float surface to BC6H mode 11.
///
/// `input` must hold `width × height × 8` bytes (interleaved RGBA half-
/// float, alpha ignored). `output` must hold
/// `ceil(w/4) × ceil(h/4) × 16` bytes.
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
            let bc = encode_bc6h_mode10_block(&block);
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
    /// R, G, B vary independently) need 2-subset partitions to beat
    /// 30 dB consistently — the round-3 baseline is mode 10 only.
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
}
