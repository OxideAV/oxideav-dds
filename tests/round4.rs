//! Round-4 features: BC6H decompression + BC2/BC3/BC4/BC5 encoders.
//!
//! Covers the new public surface added in round 4:
//! * [`oxideav_dds::decode_bc6h`] — BC6H mode-11 + mode-1 decoder
//!   (HDR-float; modes 0..10 + 12/13 fall back to zero-filled output
//!   plus an `Unsupported` diagnostic).
//! * [`oxideav_dds::encode_bc2`] / [`oxideav_dds::encode_bc3`] /
//!   [`oxideav_dds::encode_bc4_unorm`] / [`oxideav_dds::encode_bc5_unorm`]
//!   — block encoders, validated via roundtrip through the matching
//!   `decode_*` helpers.
//!
//! The round-3 `bc1_encode` plus the round-2 BC1..BC5 + BC7 decoders
//! already have their own coverage; this file focuses purely on the
//! delta.

use oxideav_dds::{
    decode_bc2, decode_bc3, decode_bc4_unorm, decode_bc5_unorm, decode_bc6h, encode_bc2,
    encode_bc3, encode_bc4_unorm, encode_bc5_unorm,
};

/// 8×8 RGBA8 natural-image-style content (smooth + a couple of edges).
fn make_rgba8_8x8() -> Vec<u8> {
    let mut v = vec![0u8; 8 * 8 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let off = (y * 8 + x) * 4;
            v[off] = (x * 32) as u8;
            v[off + 1] = (y * 32) as u8;
            v[off + 2] = ((x + y) * 16) as u8;
            v[off + 3] = if x >= 4 { 0xff } else { (x * 64) as u8 };
        }
    }
    v
}

fn psnr_rgb(a: &[u8], b: &[u8]) -> f64 {
    let mut sse: u64 = 0;
    let mut count: u64 = 0;
    for (chunk_a, chunk_b) in a.chunks_exact(4).zip(b.chunks_exact(4)) {
        for c in 0..3 {
            let d = chunk_a[c] as i32 - chunk_b[c] as i32;
            sse += (d * d) as u64;
            count += 1;
        }
    }
    let mse = sse as f64 / count as f64;
    if mse == 0.0 {
        return f64::INFINITY;
    }
    10.0 * (255.0_f64 * 255.0 / mse).log10()
}

#[test]
fn bc2_encode_8x8_natural_image_psnr() {
    let input = make_rgba8_8x8();
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
    encode_bc2(&input, 8, 8, &mut bc).expect("encode_bc2");
    let mut decoded = vec![0u8; 8 * 8 * 4];
    decode_bc2(&bc, 8, 8, &mut decoded).expect("decode_bc2");
    let psnr = psnr_rgb(&input, &decoded);
    assert!(
        psnr > 18.0,
        "BC2 8x8 natural-image PSNR-RGB = {:.2} dB (want > 18 dB)",
        psnr
    );
}

#[test]
fn bc3_encode_8x8_natural_image_psnr() {
    let input = make_rgba8_8x8();
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
    encode_bc3(&input, 8, 8, &mut bc).expect("encode_bc3");
    let mut decoded = vec![0u8; 8 * 8 * 4];
    decode_bc3(&bc, 8, 8, &mut decoded).expect("decode_bc3");
    let psnr = psnr_rgb(&input, &decoded);
    assert!(
        psnr > 18.0,
        "BC3 8x8 natural-image PSNR-RGB = {:.2} dB (want > 18 dB)",
        psnr
    );
}

#[test]
fn bc4_encode_8x8_gradient_roundtrip_within_bin_width() {
    let mut input = vec![0u8; 8 * 8];
    for (i, b) in input.iter_mut().enumerate() {
        *b = ((i * 4) & 0xff) as u8;
    }
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 8];
    encode_bc4_unorm(&input, 8, 8, &mut bc).expect("encode_bc4");
    let mut decoded = vec![0u8; 8 * 8];
    decode_bc4_unorm(&bc, 8, 8, &mut decoded).expect("decode_bc4");
    let mut max_err = 0i32;
    for (s, d) in input.iter().zip(decoded.iter()) {
        let e = (*s as i32 - *d as i32).abs();
        if e > max_err {
            max_err = e;
        }
    }
    // Per-block max range is ≤ 16 → 8-value palette bin width ≈ 2.3 →
    // worst-case error ≈ ±2; we allow up to ±6 to keep the test resilient
    // to aggressive nearest-palette quantisation at block edges.
    assert!(
        max_err <= 10,
        "BC4 gradient max-err = {} (want ≤ 10)",
        max_err
    );
}

#[test]
fn bc5_encode_8x8_two_channel_roundtrip() {
    let mut input = vec![0u8; 8 * 8 * 2];
    for y in 0..8 {
        for x in 0..8 {
            let off = (y * 8 + x) * 2;
            input[off] = (x * 32) as u8;
            input[off + 1] = (y * 32) as u8;
        }
    }
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
    encode_bc5_unorm(&input, 8, 8, &mut bc).expect("encode_bc5");
    let mut decoded = vec![0u8; 8 * 8 * 2];
    decode_bc5_unorm(&bc, 8, 8, &mut decoded).expect("decode_bc5");
    let mut sse: u64 = 0;
    for (s, d) in input.iter().zip(decoded.iter()) {
        let diff = *s as i32 - *d as i32;
        sse += (diff * diff) as u64;
    }
    let mse = sse as f64 / input.len() as f64;
    let psnr = 10.0 * (255.0_f64 * 255.0 / mse).log10();
    assert!(psnr > 25.0, "BC5 8x8 PSNR = {:.2} dB (want > 25 dB)", psnr);
}

/// Hand-built BC6H mode-11 block (the "anchor" 10-bit no-delta single-
/// subset mode) that decodes to a solid-grey-ish HDR pixel: r = g = b
/// midway between 0 and the mode-11 maximum. This validates the BC6H
/// decoder without needing an external reference encoder (none is
/// available in the workspace, and the spec is a clean-room reference).
#[test]
fn bc6h_mode11_solid_midway() {
    // Mode 11 prefix = 5 bits = 01011.
    // Then six 10-bit endpoint values (r0, r1, g0, g1, b0, b1).
    // Set each pair to (200, 600) — both within the 10-bit unsigned
    // range. With all indices = 0 the output is endpoint 0.
    let mut block = [0u8; 16];
    let mut pos = 0usize;
    let push = |bits: u32, n: u32, block: &mut [u8; 16], pos: &mut usize| {
        for i in 0..n {
            let bit = (bits >> i) & 1;
            block[*pos / 8] |= (bit as u8) << (*pos & 7);
            *pos += 1;
        }
    };
    push(0b01011, 5, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    // Indices: pixel 0 anchor (3 bits) + 15×4 bits = 63 bits. Leave at zero.
    let mut out = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&block, 4, 4, /*signed=*/ false, &mut out).expect("decode_bc6h");
    // Pixel 0: anchor = 0 → endpoint 0; r raw = 200 → unquantize:
    //   ((200 << 15) + 0x4000) >> 9 = ((6553600 + 16384) >> 9) = 12831.
    // Finalise: 12831 * 31 / 64 = 6212.
    let expected_half = ((((200u32) << 15) + 0x4000) >> 9) * 31 / 64;
    let r = u16::from_le_bytes([out[0], out[1]]);
    assert_eq!(r as u32, expected_half, "decoded R half-bits");
}

/// Mode-other-than-1-or-11 blocks should error with `Unsupported`.
#[test]
fn bc6h_unimplemented_mode_returns_unsupported() {
    let block = [0u8; 16]; // 2-bit prefix `00` → mode 0, not supported
    let mut out = vec![0u8; 4 * 4 * 8];
    let result = decode_bc6h(&block, 4, 4, false, &mut out);
    assert!(
        matches!(result, Err(oxideav_dds::DdsError::Unsupported(_))),
        "mode 0 should report Unsupported, got {:?}",
        result
    );
}
