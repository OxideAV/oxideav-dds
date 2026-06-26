//! End-to-end ASTC LDR encode tests for `oxideav-dds`.
//!
//! Each test encodes an RGBA8 surface to a complete `.dds` file with
//! [`encode_dds_astc`], parses it back through [`parse_dds`], decodes the
//! ASTC payload, and checks the round-trip against the source within the
//! single-partition encoder's documented tolerance. Every byte comes
//! from this crate's own encode + decode path and the DDS on-disk layout
//! (`docs/image/dds/`); no external DDS / ASTC tooling is consulted.

use oxideav_dds::{
    decode_astc_ldr_surface, encode_dds_astc, parse_dds, DdsPixelFormat, DxgiFormat,
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
