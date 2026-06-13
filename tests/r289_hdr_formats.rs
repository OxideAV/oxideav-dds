//! Round 289: extended high-bit-depth / floating-point uncompressed DDS
//! surfaces — legacy D3DFMT numeric FourCC codes (36, 110..=116) and the
//! matching DX10 `DXGI_FORMAT` values, end-to-end through `parse_dds` and
//! `decode_hdr_to_f32`.

use oxideav_dds::types::{
    DDPF_FOURCC, DDSCAPS_TEXTURE, DDSD_REQUIRED, DDS_DIMENSION_TEXTURE2D, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE, FOURCC_DX10,
};
use oxideav_dds::{decode_hdr_to_f32, parse_dds, DdsPixelFormat};

/// Wrap a single-surface payload in a DDS file with a legacy
/// `DDS_PIXELFORMAT` FourCC field (which for these formats carries a
/// small integer rather than four ASCII bytes).
fn build_fourcc_dds(four_cc: u32, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + payload.len());
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&four_cc.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(payload);
    out
}

/// Wrap a single-surface payload in a DX10-extension DDS file.
fn build_dx10_dds(dxgi: u32, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + DDS_HEADER_DXT10_SIZE + payload.len());
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DX10.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&dxgi.to_le_bytes());
    out.extend_from_slice(&DDS_DIMENSION_TEXTURE2D.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flag
    out.extend_from_slice(&1u32.to_le_bytes()); // array_size
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flags2
    out.extend_from_slice(payload);
    out
}

/// One 2x1 R16G16B16A16_FLOAT surface: pixel0 = (1.0, 0.5, 2.0, 0.0),
/// pixel1 = (0.0, 0.25, 0.0, 1.0). Returned as raw little-endian bytes.
fn rgba16f_2x1() -> Vec<u8> {
    let mut p = Vec::new();
    // pixel 0: 1.0=0x3c00, 0.5=0x3800, 2.0=0x4000, 0.0=0x0000
    for h in [0x3c00u16, 0x3800, 0x4000, 0x0000] {
        p.extend_from_slice(&h.to_le_bytes());
    }
    // pixel 1: 0.0, 0.25=0x3400, 0.0, 1.0
    for h in [0x0000u16, 0x3400, 0x0000, 0x3c00] {
        p.extend_from_slice(&h.to_le_bytes());
    }
    p
}

#[test]
fn legacy_fourcc113_rgba16f_parses_and_decodes() {
    let payload = rgba16f_2x1();
    let dds = build_fourcc_dds(113, 2, 1, &payload);
    let img = parse_dds(&dds).expect("parse legacy FourCC 113");
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Float);
    assert_eq!(img.width, 2);
    assert_eq!(img.height, 1);
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data, payload);

    let mut out = [0.0f32; 8];
    decode_hdr_to_f32(
        &img.surfaces[0].plane.data,
        img.pixel_format,
        img.width,
        img.height,
        &mut out,
    )
    .unwrap();
    assert_eq!(&out[..4], &[1.0, 0.5, 2.0, 0.0]);
    assert_eq!(&out[4..], &[0.0, 0.25, 0.0, 1.0]);
}

#[test]
fn dx10_dxgi10_rgba16f_parses_and_decodes() {
    let payload = rgba16f_2x1();
    // DXGI_FORMAT_R16G16B16A16_FLOAT = 10.
    let dds = build_dx10_dds(10, 2, 1, &payload);
    let img = parse_dds(&dds).expect("parse DX10 dxgi 10");
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Float);
    assert!(img.has_dxt10_header);

    let mut out = [0.0f32; 8];
    decode_hdr_to_f32(
        &img.surfaces[0].plane.data,
        img.pixel_format,
        2,
        1,
        &mut out,
    )
    .unwrap();
    assert_eq!(&out[..4], &[1.0, 0.5, 2.0, 0.0]);
}

#[test]
fn legacy_fourcc_code_to_format_table() {
    // Each robust-reader-mandated legacy numeric FourCC maps to the
    // expected extended uncompressed format. One 1x1 surface each
    // (sized by the format's bytes-per-pixel).
    let cases: &[(u32, DdsPixelFormat, usize)] = &[
        (36, DdsPixelFormat::R16G16B16A16Unorm, 8),
        (110, DdsPixelFormat::R16G16B16A16Snorm, 8),
        (111, DdsPixelFormat::R16Float, 2),
        (112, DdsPixelFormat::R16G16Float, 4),
        (113, DdsPixelFormat::R16G16B16A16Float, 8),
        (114, DdsPixelFormat::R32Float, 4),
        (115, DdsPixelFormat::R32G32Float, 8),
        (116, DdsPixelFormat::R32G32B32A32Float, 16),
    ];
    for &(code, fmt, bpp) in cases {
        let payload = vec![0u8; bpp];
        let dds = build_fourcc_dds(code, 1, 1, &payload);
        let img = parse_dds(&dds).unwrap_or_else(|e| panic!("FourCC {code}: {e}"));
        assert_eq!(img.pixel_format, fmt, "FourCC {code}");
    }
}

#[test]
fn r32_float_dx10_decodes_full_precision() {
    // DXGI_FORMAT_R32_FLOAT = 41; single pixel = pi.
    let payload = std::f32::consts::PI.to_le_bytes();
    let dds = build_dx10_dds(41, 1, 1, &payload);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R32Float);

    let mut out = [0.0f32; 4];
    decode_hdr_to_f32(
        &img.surfaces[0].plane.data,
        img.pixel_format,
        1,
        1,
        &mut out,
    )
    .unwrap();
    assert_eq!(out, [std::f32::consts::PI, 0.0, 0.0, 1.0]);
}

#[test]
fn rgba16_unorm_full_white_normalises_to_one() {
    // DXGI_FORMAT_R16G16B16A16_UNORM = 11; all channels max → 1.0.
    let mut payload = Vec::new();
    for _ in 0..4 {
        payload.extend_from_slice(&65535u16.to_le_bytes());
    }
    let dds = build_dx10_dds(11, 1, 1, &payload);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Unorm);

    let mut out = [0.0f32; 4];
    decode_hdr_to_f32(
        &img.surfaces[0].plane.data,
        img.pixel_format,
        1,
        1,
        &mut out,
    )
    .unwrap();
    assert_eq!(out, [1.0, 1.0, 1.0, 1.0]);
}

#[test]
fn truncated_hdr_surface_is_rejected_not_panicked() {
    // Declare a 4x4 RGBA16F (128 bytes) but supply only 16 bytes.
    let dds = build_fourcc_dds(113, 4, 4, &[0u8; 16]);
    let r = parse_dds(&dds);
    assert!(r.is_err(), "truncated HDR payload must reject");
}
