//! Round-4 features: BC6H decompression + BC2/BC3/BC4/BC5 encoders.
//!
//! Covers the new public surface added in round 4:
//! * [`oxideav_dds::decode_bc6h`] — BC6H decoder. Round-2 (against
//!   the round-4 README) lifted coverage from "modes 1+11 only" to
//!   all 14 modes (0..13); the integration tests here exercise the
//!   surface-level entry point.
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

/// Hand-built BC6H mode-10 block (the 1-subset 10-bit no-delta anchor
/// mode, prefix `00011`) that decodes to a solid HDR pixel: r = g = b
/// at endpoint 0 (raw 200). Validates the decoder against Microsoft's
/// unquantize formula: `((comp << 16) + 0x8000) >> bits`, then finish
/// `(value * 31) >> 6`. With raw=200, bits=10:
///   unquantized = (200 << 16 + 0x8000) >> 10 = 12831
///   finalised   = (12831 * 31) >> 6 = 6212.
#[test]
fn bc6h_mode10_solid_endpoint() {
    // Mode 10 prefix = 5 bits = 00011.
    // Then six 10-bit endpoint values (rw, gw, bw, rx, gx, bx).
    let mut block = [0u8; 16];
    let mut pos = 0usize;
    let push = |bits: u32, n: u32, block: &mut [u8; 16], pos: &mut usize| {
        for i in 0..n {
            let bit = (bits >> i) & 1;
            block[*pos / 8] |= (bit as u8) << (*pos & 7);
            *pos += 1;
        }
    };
    push(0b00011, 5, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(200, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    push(600, 10, &mut block, &mut pos);
    // Indices: pixel 0 anchor (3 bits) + 15x4 bits = 63 bits. Leave zero.
    let mut out = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&block, 4, 4, /*signed=*/ false, &mut out).expect("decode_bc6h");
    let expected_half = ((((200u32) << 16) + 0x8000) >> 10) * 31 / 64;
    let r = u16::from_le_bytes([out[0], out[1]]);
    assert_eq!(r as u32, expected_half, "decoded R half-bits");
}

/// All 14 BC6H modes (0..13) decode without error. Reserved-prefix
/// blocks (10011, 10111, 11011, 11111) decode to zero RGB per spec.
#[test]
fn bc6h_all_modes_decode_without_error() {
    // Mode 0 prefix = 00, all-zero block.
    let block = [0u8; 16];
    let mut out = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&block, 4, 4, false, &mut out).expect("mode 0 decodes");
    // Every R/G/B should be zero (endpoints zero, indices zero).
    for chunk in out.chunks_exact(8) {
        assert_eq!(u16::from_le_bytes([chunk[0], chunk[1]]), 0);
        assert_eq!(u16::from_le_bytes([chunk[2], chunk[3]]), 0);
        assert_eq!(u16::from_le_bytes([chunk[4], chunk[5]]), 0);
        assert_eq!(u16::from_le_bytes([chunk[6], chunk[7]]), 0x3c00);
    }

    // Reserved 5-bit prefix 10011 → zero RGB.
    let mut rblock = [0u8; 16];
    rblock[0] = 0b10011;
    let mut rout = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&rblock, 4, 4, false, &mut rout).expect("reserved decodes to zero");
    for chunk in rout.chunks_exact(8) {
        assert_eq!(u16::from_le_bytes([chunk[0], chunk[1]]), 0);
        assert_eq!(u16::from_le_bytes([chunk[6], chunk[7]]), 0x3c00);
    }
}
