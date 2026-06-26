//! End-to-end ASTC LDR encode tests for `oxideav-dds`.
//!
//! Each test encodes an RGBA8 surface to a complete `.dds` file with
//! [`encode_dds_astc`], parses it back through [`parse_dds`], decodes the
//! ASTC payload, and checks the round-trip against the source within the
//! single-partition encoder's documented tolerance. Every byte comes
//! from this crate's own encode + decode path and the DDS on-disk layout
//! (`docs/image/dds/`); no external DDS / ASTC tooling is consulted.

use oxideav_dds::{
    decode_astc_ldr_block, decode_astc_ldr_surface, encode_astc_ldr_block, encode_dds_astc,
    parse_dds, DdsPixelFormat, DxgiFormat,
};

fn astc_fmt(bw: u32, bh: u32) -> DdsPixelFormat {
    DdsPixelFormat::Astc {
        block_w: bw,
        block_h: bh,
        srgb: false,
    }
}

/// Solid-colour surfaces must round-trip byte-exact (void extent) at
/// every footprint and arbitrary dimensions.
#[test]
fn solid_surface_roundtrips_exact_all_footprints() {
    let footprints = [
        (4u32, 4u32),
        (5, 4),
        (5, 5),
        (6, 5),
        (6, 6),
        (8, 5),
        (8, 6),
        (8, 8),
        (10, 5),
        (10, 6),
        (10, 8),
        (10, 10),
        (12, 10),
        (12, 12),
    ];
    let color = [12u8, 240, 77, 255];
    let (w, h) = (37u32, 19u32); // deliberately not block-aligned
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for px in rgba.chunks_exact_mut(4) {
        px.copy_from_slice(&color);
    }
    for (bw, bh) in footprints {
        let file = encode_dds_astc(&rgba, w, h, astc_fmt(bw, bh), 1).unwrap();
        let img = parse_dds(&file).unwrap();
        assert_eq!(img.width, w);
        assert_eq!(img.height, h);
        assert_eq!(img.pixel_format.astc_footprint(), Some((bw, bh)));
        let dec =
            decode_astc_ldr_surface(img.pixel_format, &img.surfaces[0].plane.data, w, h).unwrap();
        for px in dec.chunks_exact(4) {
            assert_eq!(px, color, "{bw}x{bh} solid not exact");
        }
    }
}

/// A collinear (luminance) ramp round-trips with a small mean error.
#[test]
fn luma_ramp_roundtrips_within_tolerance() {
    let (w, h) = (32u32, 32u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let o = ((y * w + x) * 4) as usize;
            let v = (((x + y) * 255) / (w + h - 2)) as u8;
            rgba[o] = v;
            rgba[o + 1] = v;
            rgba[o + 2] = v;
            rgba[o + 3] = 255;
        }
    }
    for (bw, bh) in [(4u32, 4u32), (8, 8), (6, 6)] {
        let file = encode_dds_astc(&rgba, w, h, astc_fmt(bw, bh), 1).unwrap();
        let img = parse_dds(&file).unwrap();
        let dec =
            decode_astc_ldr_surface(img.pixel_format, &img.surfaces[0].plane.data, w, h).unwrap();
        let mut sum: u64 = 0;
        for i in 0..(w * h) as usize {
            for c in 0..3 {
                sum += (rgba[i * 4 + c] as i64 - dec[i * 4 + c] as i64).unsigned_abs();
            }
        }
        let mae = sum as f64 / ((w * h * 3) as f64);
        assert!(mae < 16.0, "{bw}x{bh} MAE {mae}");
    }
}

/// The DX10 header must carry the matching `DXGI_FORMAT_ASTC_*` code and
/// the mipmap chain must parse into the right number of surfaces.
#[test]
fn dx10_header_and_mip_chain() {
    let (w, h) = (16u32, 16u32);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for (i, px) in rgba.chunks_exact_mut(4).enumerate() {
        px.copy_from_slice(&[(i as u8), 100, 200, 255]);
    }
    let file = encode_dds_astc(&rgba, w, h, astc_fmt(4, 4), 5).unwrap();
    let img = parse_dds(&file).unwrap();
    // DXGI code = ASTC_4X4_UNORM.
    assert_eq!(
        img.dxgi_format,
        Some(DxgiFormat::astc_unorm(4, 4, false).unwrap())
    );
    // 16x16 → 5 mip levels (16,8,4,2,1).
    assert_eq!(img.mip_map_count, 5);
    assert_eq!(img.surfaces.len(), 5);
    assert_eq!((img.surfaces[0].width, img.surfaces[0].height), (16, 16));
    assert_eq!((img.surfaces[4].width, img.surfaces[4].height), (1, 1));
}

/// sRGB ASTC encode picks the `_UNORM_SRGB` DXGI code.
#[test]
fn srgb_format_roundtrips() {
    let (w, h) = (8u32, 8u32);
    let rgba = vec![128u8; (w * h * 4) as usize];
    let fmt = DdsPixelFormat::Astc {
        block_w: 4,
        block_h: 4,
        srgb: true,
    };
    let file = encode_dds_astc(&rgba, w, h, fmt, 1).unwrap();
    let img = parse_dds(&file).unwrap();
    assert_eq!(
        img.dxgi_format,
        Some(DxgiFormat::astc_unorm(4, 4, true).unwrap())
    );
    assert!(matches!(
        img.pixel_format,
        DdsPixelFormat::Astc { srgb: true, .. }
    ));
}

/// Bad inputs are rejected, not panicked.
#[test]
fn rejects_bad_inputs() {
    // Non-ASTC format.
    assert!(encode_dds_astc(&[0u8; 16], 2, 2, DdsPixelFormat::A8R8G8B8, 1).is_err());
    // Zero size.
    assert!(encode_dds_astc(&[], 0, 0, astc_fmt(4, 4), 1).is_err());
    // Short input.
    assert!(encode_dds_astc(&[0u8; 4], 4, 4, astc_fmt(4, 4), 1).is_err());
}

/// A block with three sharply distinct opaque colour regions is
/// reconstructed by the encoder no worse than the chooser's best
/// non-three-subset candidate: enabling the three-subset path must never
/// regress a block (the error-driven chooser only keeps a candidate that
/// decodes strictly closer). We verify this by comparing the full
/// encoder's output against a baseline that excludes the three-subset
/// path — re-encoding the SAME texels but with all alpha forced to a
/// non-opaque value, which disables the opaque-only three-subset branch
/// while leaving the single-/two-subset/dual-plane search intact. The
/// real (opaque) block's RGB error must be ≤ the baseline's RGB error.
#[test]
fn three_subset_never_regresses_three_region_block() {
    let (bw, bh) = (8u32, 8u32);
    let red = [220u8, 20, 20, 255];
    let green = [20u8, 220, 20, 255];
    let blue = [20u8, 20, 220, 255];
    let mut texels = Vec::with_capacity((bw * bh) as usize);
    for _y in 0..bh {
        for x in 0..bw {
            let t = match (x * 3) / bw {
                0 => red,
                1 => green,
                _ => blue,
            };
            texels.push(t);
        }
    }

    // RGB-only SAD helper (the baseline forces alpha, so compare RGB).
    let rgb_sad = |dec: &[[u8; 4]], src: &[[u8; 4]]| -> u64 {
        dec.iter()
            .zip(src.iter())
            .map(|(d, s)| {
                (0..3)
                    .map(|c| (d[c] as i64 - s[c] as i64).unsigned_abs())
                    .sum::<u64>()
            })
            .sum()
    };

    // Full encoder (opaque → three-subset path active).
    let block = encode_astc_ldr_block(&texels, bw, bh);
    let decoded = decode_astc_ldr_block(&block, bw, bh);
    assert!(
        decoded.iter().all(|t| t[3] == 255),
        "alpha must stay opaque"
    );
    let real_rgb_sad = rgb_sad(&decoded, &texels);

    // Baseline: same RGB, alpha = 200 everywhere (uniform but non-opaque)
    // disables the opaque-only three-subset branch.
    let mut baseline_src = texels.clone();
    for t in &mut baseline_src {
        t[3] = 200;
    }
    let base_block = encode_astc_ldr_block(&baseline_src, bw, bh);
    let base_decoded = decode_astc_ldr_block(&base_block, bw, bh);
    let base_rgb_sad = rgb_sad(&base_decoded, &baseline_src);

    assert!(
        real_rgb_sad <= base_rgb_sad,
        "three-subset path regressed RGB: opaque {real_rgb_sad} > baseline {base_rgb_sad}"
    );
}

/// A solid opaque block still round-trips exactly even though the
/// three-subset candidate is now also evaluated (the constant fast path
/// short-circuits to a void extent before any subset search).
#[test]
fn solid_opaque_block_unaffected_by_three_subset() {
    let (bw, bh) = (6u32, 6u32);
    let col = [80u8, 160, 240, 255];
    let texels = vec![col; (bw * bh) as usize];
    let block = encode_astc_ldr_block(&texels, bw, bh);
    let decoded = decode_astc_ldr_block(&block, bw, bh);
    assert!(decoded.iter().all(|&t| t == col), "solid block not exact");
}

/// Every footprint encodes a three-region opaque block to a decodable,
/// opaque-alpha block (no panic, correct texel count).
#[test]
fn three_region_block_all_footprints_panic_free() {
    let footprints = [
        (4u32, 4u32),
        (5, 4),
        (5, 5),
        (6, 5),
        (6, 6),
        (8, 5),
        (8, 6),
        (8, 8),
        (10, 5),
        (10, 6),
        (10, 8),
        (10, 10),
        (12, 10),
        (12, 12),
    ];
    for (bw, bh) in footprints {
        let mut texels = Vec::with_capacity((bw * bh) as usize);
        for _y in 0..bh {
            for x in 0..bw {
                let t = match (x * 3) / bw {
                    0 => [200u8, 30, 30, 255],
                    1 => [30u8, 200, 30, 255],
                    _ => [30u8, 30, 200, 255],
                };
                texels.push(t);
            }
        }
        let block = encode_astc_ldr_block(&texels, bw, bh);
        let decoded = decode_astc_ldr_block(&block, bw, bh);
        assert_eq!(decoded.len(), (bw * bh) as usize, "{bw}x{bh} texel count");
        assert!(
            decoded.iter().all(|t| t[3] == 255),
            "{bw}x{bh} alpha not opaque"
        );
    }
}
