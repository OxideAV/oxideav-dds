//! Integration tests for the depth / depth-stencil DXGI surface formats.
//!
//! Each test builds a minimal DX10 DDS byte stream carrying one of the
//! four documented depth `DXGI_FORMAT` values, parses it with
//! [`oxideav_dds::parse_dds`], asserts the resolved
//! [`oxideav_dds::DdsPixelFormat`] variant and the carried surface byte
//! length, then expands the raw bytes with the matching
//! `decode_depth_*_surface` helper and checks the decoded depth /
//! stencil values.
//!
//! All layout facts (bit fields, channel sizing, packing) come from
//! Microsoft's public `DXGI_FORMAT` enumeration page staged under
//! `docs/image/dds/`.

use oxideav_dds::types::{
    DDPF_FOURCC, DDSD_PIXELFORMAT, DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
    FOURCC_DX10,
};
use oxideav_dds::{
    decode_depth_d16_surface, decode_depth_d24s8_surface, decode_depth_d32_surface,
    decode_depth_d32s8_surface, parse_dds, DdsPixelFormat,
};

const CAPS_TEXTURE: u32 = 0x0000_1000;

/// Build a minimal DX10 DDS file carrying one `width × height` surface
/// of the given `dxgi_format` value.
fn build_dx10_dds(dxgi_format: u32, width: u32, height: u32, pixel_data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&(DDSD_REQUIRED | DDSD_PIXELFORMAT).to_le_bytes());
    out.extend_from_slice(&height.to_le_bytes());
    out.extend_from_slice(&width.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // pitch_or_linear_size
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DX10.to_le_bytes());
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
                                                // DDS_HEADER_DXT10
    out.extend_from_slice(&dxgi_format.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes()); // dimension = 2D
    out.extend_from_slice(&0u32.to_le_bytes()); // misc
    out.extend_from_slice(&1u32.to_le_bytes()); // array_size
    out.extend_from_slice(&0u32.to_le_bytes()); // misc2
    out.extend_from_slice(pixel_data);
    out
}

// --- D16_UNORM (value 55) -----------------------------------------------

#[test]
fn dx10_d16_unorm_end_to_end() {
    // Two texels: 0x0000 -> 0.0, 0xffff -> 1.0.
    let mut px = Vec::new();
    px.extend_from_slice(&0x0000u16.to_le_bytes());
    px.extend_from_slice(&0xffffu16.to_le_bytes());
    let dds = build_dx10_dds(55, 2, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D16Unorm);
    assert_eq!(img.pixel_format.name(), "D16_UNORM");
    assert_eq!(img.surfaces[0].plane.data.len(), 4); // 2px × 2 bytes
    let out = decode_depth_d16_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![0.0, 1.0]);
}

// --- D32_FLOAT (value 40) -----------------------------------------------

#[test]
fn dx10_d32_float_end_to_end() {
    let mut px = Vec::new();
    px.extend_from_slice(&0.0f32.to_le_bytes());
    px.extend_from_slice(&0.625f32.to_le_bytes());
    let dds = build_dx10_dds(40, 2, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D32Float);
    assert_eq!(img.pixel_format.name(), "D32_FLOAT");
    assert_eq!(img.surfaces[0].plane.data.len(), 8); // 2px × 4 bytes
    let out = decode_depth_d32_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![0.0, 0.625]);
}

// --- D24_UNORM_S8_UINT (value 45) + typeless view R24G8 (value 44) -------

#[test]
fn dx10_d24s8_end_to_end() {
    // depth = full (0xffffff -> 1.0), stencil = 0x42.
    let word: u32 = (0x42u32 << 24) | 0x00ff_ffff;
    let dds = build_dx10_dds(45, 1, 1, &word.to_le_bytes());
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D24UnormS8Uint);
    assert_eq!(img.pixel_format.name(), "D24_UNORM_S8_UINT");
    assert_eq!(img.surfaces[0].plane.data.len(), 4);
    let out = decode_depth_d24s8_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out[0].depth, 1.0);
    assert_eq!(out[0].stencil, 0x42);
}

#[test]
fn dx10_r24g8_typeless_routes_to_d24s8() {
    // The typeless view (value 44) over the same memory resolves to the
    // depth-stencil variant.
    let word: u32 = 0x10u32 << 24; // stencil 0x10, depth 0.
    let dds = build_dx10_dds(44, 1, 1, &word.to_le_bytes());
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D24UnormS8Uint);
    let out = decode_depth_d24s8_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out[0].depth, 0.0);
    assert_eq!(out[0].stencil, 0x10);
}

// --- D32_FLOAT_S8X24_UINT (value 20) + typeless R32G8X24 (value 19) ------

#[test]
fn dx10_d32s8_end_to_end() {
    let mut px = Vec::new();
    px.extend_from_slice(&0.75f32.to_le_bytes());
    // stencil 0x09 in low byte; upper 24 bits unused (set non-zero to
    // confirm they are ignored).
    px.extend_from_slice(&0x00ab_cd09u32.to_le_bytes());
    let dds = build_dx10_dds(20, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D32FloatS8X24Uint);
    assert_eq!(img.pixel_format.name(), "D32_FLOAT_S8X24_UINT");
    assert_eq!(img.surfaces[0].plane.data.len(), 8); // 1px × 8 bytes
    let out = decode_depth_d32s8_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out[0].depth, 0.75);
    assert_eq!(out[0].stencil, 0x09);
}

#[test]
fn dx10_r32g8x24_typeless_routes_to_d32s8() {
    let mut px = Vec::new();
    px.extend_from_slice(&0.125f32.to_le_bytes());
    px.extend_from_slice(&0x0000_0080u32.to_le_bytes());
    let dds = build_dx10_dds(19, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::D32FloatS8X24Uint);
    let out = decode_depth_d32s8_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out[0].depth, 0.125);
    assert_eq!(out[0].stencil, 0x80);
}

// --- sizing / mip chain --------------------------------------------------

#[test]
fn d24s8_surface_size_and_stride() {
    // 4x4 D24S8 surface: 16 texels × 4 bytes = 64 bytes.
    let px = vec![0u8; 4 * 4 * 4];
    let dds = build_dx10_dds(45, 4, 4, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.surfaces[0].plane.data.len(), 64);
    assert_eq!(img.surfaces[0].plane.stride, 4 * 4); // width × 4 bytes
}

#[test]
fn d32s8_surface_size_is_8_bytes_per_texel() {
    let px = vec![0u8; 2 * 2 * 8];
    let dds = build_dx10_dds(20, 2, 2, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.surfaces[0].plane.data.len(), 32);
    assert_eq!(img.surfaces[0].plane.stride, 2 * 8);
}
