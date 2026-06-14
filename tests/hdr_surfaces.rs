//! Integration tests for the extended high-bit-depth / floating-point
//! uncompressed DDS surfaces.
//!
//! Each test builds a minimal DDS byte stream — either via the legacy
//! numeric `D3DFMT` FourCC code (36 / 110..=116) or via the DX10
//! `DDS_HEADER_DXT10` extension carrying the matching `DXGI_FORMAT` —
//! parses it with [`oxideav_dds::parse_dds`], asserts the resolved
//! [`oxideav_dds::DdsPixelFormat`], the surface byte length, and the
//! decoded channel values.

use oxideav_dds::types::{
    DDPF_ALPHAPIXELS, DDPF_FOURCC, DDPF_RGB, DDSD_PIXELFORMAT, DDSD_REQUIRED, DDS_HEADER_SIZE,
    DDS_MAGIC, DDS_PIXELFORMAT_SIZE, FOURCC_DX10,
};
use oxideav_dds::{
    decode_float_surface, decode_r10g10b10a2_unorm_surface, decode_rgba16_snorm_surface,
    decode_rgba16_unorm_surface, parse_dds, DdsPixelFormat,
};

const CAPS_TEXTURE: u32 = 0x0000_1000;

/// Build a DDS file whose `DDS_PIXELFORMAT.four_cc` is the numeric
/// `D3DFMT` code `numeric_fourcc`, carrying a single `width × height`
/// surface with the provided pixel bytes.
fn build_numeric_fourcc_dds(
    numeric_fourcc: u32,
    width: u32,
    height: u32,
    pixel_data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // pitch_or_linear_size
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    // DDS_PIXELFORMAT — DDPF_FOURCC with the numeric D3DFMT code.
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&numeric_fourcc.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0u32.to_le_bytes()); // r mask
    out.extend_from_slice(&0u32.to_le_bytes()); // g mask
    out.extend_from_slice(&0u32.to_le_bytes()); // b mask
    out.extend_from_slice(&0u32.to_le_bytes()); // a mask
    out.extend_from_slice(&CAPS_TEXTURE.to_le_bytes()); // caps
    out.extend_from_slice(&0u32.to_le_bytes()); // caps2
    out.extend_from_slice(&0u32.to_le_bytes()); // caps3
    out.extend_from_slice(&0u32.to_le_bytes()); // caps4
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    out.extend_from_slice(pixel_data);
    out
}

/// Build a DDS file whose pixel format is the DX10 extension with the
/// supplied `dxgi_format` value, carrying one `width × height` surface.
fn build_dx10_dds(dxgi_format: u32, width: u32, height: u32, pixel_data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&(DDSD_REQUIRED | DDSD_PIXELFORMAT).to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
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
    out.extend_from_slice(&CAPS_TEXTURE.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // DDS_HEADER_DXT10 (20 bytes): dxgi_format, dimension (2D = 3),
    // misc_flag, array_size, misc_flags2.
    out.extend_from_slice(&dxgi_format.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(pixel_data);
    out
}

/// Build a DDS file whose `DDS_PIXELFORMAT` is a masked uncompressed
/// (DDPF_RGB) layout with the supplied flags / bit count / channel masks.
#[allow(clippy::too_many_arguments)]
fn build_masked_dds(
    flags: u32,
    rgb_bit_count: u32,
    r_mask: u32,
    g_mask: u32,
    b_mask: u32,
    a_mask: u32,
    width: u32,
    height: u32,
    pixel_data: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // pitch_or_linear_size
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // four_cc
    out.extend_from_slice(&rgb_bit_count.to_le_bytes());
    out.extend_from_slice(&r_mask.to_le_bytes());
    out.extend_from_slice(&g_mask.to_le_bytes());
    out.extend_from_slice(&b_mask.to_le_bytes());
    out.extend_from_slice(&a_mask.to_le_bytes());
    out.extend_from_slice(&CAPS_TEXTURE.to_le_bytes()); // caps
    out.extend_from_slice(&0u32.to_le_bytes()); // caps2
    out.extend_from_slice(&0u32.to_le_bytes()); // caps3
    out.extend_from_slice(&0u32.to_le_bytes()); // caps4
    out.extend_from_slice(&0u32.to_le_bytes()); // reserved2
    out.extend_from_slice(pixel_data);
    out
}

/// Build one little-endian `R10G10B10A2_UNORM` 32-bit word from raw
/// per-channel integers (R/G/B in 0..=1023, A in 0..=3).
fn pack_r10g10b10a2(r: u32, g: u32, b: u32, a: u32) -> [u8; 4] {
    let word = (r & 0x3ff) | ((g & 0x3ff) << 10) | ((b & 0x3ff) << 20) | ((a & 0x3) << 30);
    word.to_le_bytes()
}

#[test]
fn dx10_r10g10b10a2_unorm() {
    // DXGI_FORMAT_R10G10B10A2_UNORM = 24, 4 bytes/pixel.
    let px = pack_r10g10b10a2(1023, 512, 1, 2);
    let dds = build_dx10_dds(24, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R10G10B10A2Unorm);
    assert!(img.has_dxt10_header);
    assert_eq!(img.surfaces[0].plane.data.len(), 4);
    let out = decode_r10g10b10a2_unorm_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1023, 512, 1, 2]);
}

#[test]
fn legacy_a2b10g10r10_masks_resolve_to_r10g10b10a2() {
    // D3DFMT_A2B10G10R10 masks (canonical 10:10:10:2, R in the LSBs):
    // R=0x000003ff, G=0x000ffc00, B=0x3ff00000, A=0xc0000000.
    let px = pack_r10g10b10a2(7, 1023, 0, 3);
    let dds = build_masked_dds(
        DDPF_RGB | DDPF_ALPHAPIXELS,
        32,
        0x0000_03ff,
        0x000f_fc00,
        0x3ff0_0000,
        0xc000_0000,
        1,
        1,
        &px,
    );
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R10G10B10A2Unorm);
    assert!(!img.has_dxt10_header);
    let out = decode_r10g10b10a2_unorm_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![7, 1023, 0, 3]);
}

#[test]
fn r10g10b10a2_surface_sizing_2x2() {
    // 2x2 R10G10B10A2 = 4 pixels × 4 bytes = 16 bytes.
    let px = vec![0u8; 16];
    let dds = build_dx10_dds(24, 2, 2, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data.len(), 16);
    assert_eq!(img.surfaces[0].plane.stride, 2 * 4);
}

#[test]
fn numeric_fourcc_116_rgba32f_one_pixel() {
    // D3DFMT_A32B32G32R32F = 116 → R32G32B32A32_FLOAT, 16 bytes/pixel.
    let mut px = Vec::new();
    for v in [0.25f32, 0.5, 0.75, 1.0] {
        px.extend_from_slice(&v.to_le_bytes());
    }
    let dds = build_numeric_fourcc_dds(116, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R32G32B32A32Float);
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data.len(), 16);
    let out = decode_float_surface(img.pixel_format, 1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![0.25, 0.5, 0.75, 1.0]);
}

#[test]
fn numeric_fourcc_114_r32f_two_pixels() {
    // D3DFMT_R32F = 114 → R32_FLOAT, 4 bytes/pixel.
    let mut px = Vec::new();
    px.extend_from_slice(&3.5f32.to_le_bytes());
    px.extend_from_slice(&(-1.0f32).to_le_bytes());
    let dds = build_numeric_fourcc_dds(114, 2, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R32Float);
    assert_eq!(img.surfaces[0].plane.data.len(), 8);
    let out = decode_float_surface(img.pixel_format, 2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![3.5, -1.0]);
}

#[test]
fn numeric_fourcc_113_rgba16f() {
    // D3DFMT_A16B16G16R16F = 113 → R16G16B16A16_FLOAT, 8 bytes/pixel.
    let mut px = Vec::new();
    for h in [0x3c00u16, 0x3800, 0x0000, 0x3c00] {
        px.extend_from_slice(&h.to_le_bytes());
    }
    let dds = build_numeric_fourcc_dds(113, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Float);
    assert_eq!(img.surfaces[0].plane.data.len(), 8);
    let out = decode_float_surface(img.pixel_format, 1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1.0, 0.5, 0.0, 1.0]);
}

#[test]
fn numeric_fourcc_36_rgba16_unorm() {
    // D3DFMT_A16B16G16R16 = 36 → R16G16B16A16_UNORM, 8 bytes/pixel.
    let mut px = Vec::new();
    for v in [0u16, 0x8000, 0xffff, 0x4000] {
        px.extend_from_slice(&v.to_le_bytes());
    }
    let dds = build_numeric_fourcc_dds(36, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Unorm);
    assert_eq!(img.surfaces[0].plane.data.len(), 8);
    let out = decode_rgba16_unorm_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![0, 0x8000, 0xffff, 0x4000]);
}

#[test]
fn numeric_fourcc_110_rgba16_snorm() {
    // D3DFMT_Q16W16V16U16 = 110 → R16G16B16A16_SNORM, 8 bytes/pixel.
    let mut px = Vec::new();
    for v in [0x7fffu16, 0x8001, 0x0000, 0xffff] {
        px.extend_from_slice(&v.to_le_bytes());
    }
    let dds = build_numeric_fourcc_dds(110, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Snorm);
    let out = decode_rgba16_snorm_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![32767, -32767, 0, -1]);
}

#[test]
fn dx10_r32g32b32a32_float_matches_numeric() {
    // DXGI_FORMAT_R32G32B32A32_FLOAT = 2; same bytes as FourCC 116.
    let mut px = Vec::new();
    for v in [1.0f32, 2.0, 3.0, 4.0] {
        px.extend_from_slice(&v.to_le_bytes());
    }
    let dds = build_dx10_dds(2, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R32G32B32A32Float);
    assert!(img.has_dxt10_header);
    let out = decode_float_surface(img.pixel_format, 1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1.0, 2.0, 3.0, 4.0]);
}

#[test]
fn dx10_r16g16_float_two_channel() {
    // DXGI_FORMAT_R16G16_FLOAT = 34, 4 bytes/pixel.
    let mut px = Vec::new();
    for h in [0x3c00u16, 0x3800] {
        px.extend_from_slice(&h.to_le_bytes());
    }
    let dds = build_dx10_dds(34, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16Float);
    let out = decode_float_surface(img.pixel_format, 1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1.0, 0.5]);
}

#[test]
fn dx10_r16_float_single_channel() {
    // DXGI_FORMAT_R16_FLOAT = 54, 2 bytes/pixel.
    let px = 0x3c00u16.to_le_bytes();
    let dds = build_dx10_dds(54, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16Float);
    let out = decode_float_surface(img.pixel_format, 1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1.0]);
}

#[test]
fn dx10_r16g16b16a16_unorm() {
    // DXGI_FORMAT_R16G16B16A16_UNORM = 11, 8 bytes/pixel.
    let mut px = Vec::new();
    for v in [0x1000u16, 0x2000, 0x3000, 0xffff] {
        px.extend_from_slice(&v.to_le_bytes());
    }
    let dds = build_dx10_dds(11, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16B16A16Unorm);
    let out = decode_rgba16_unorm_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![0x1000, 0x2000, 0x3000, 0xffff]);
}

#[test]
fn rgba32f_surface_sizing_2x2() {
    // 2x2 R32G32B32A32_FLOAT = 4 pixels × 16 bytes = 64 bytes.
    let px = vec![0u8; 64];
    let dds = build_dx10_dds(2, 2, 2, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data.len(), 64);
    assert_eq!(img.surfaces[0].plane.stride, 2 * 16);
}

#[test]
fn truncated_hdr_surface_rejected() {
    // Claim 2x2 RGBA32F (needs 64 bytes) but supply only 16.
    let px = vec![0u8; 16];
    let dds = build_dx10_dds(2, 2, 2, &px);
    assert!(parse_dds(&dds).is_err());
}
