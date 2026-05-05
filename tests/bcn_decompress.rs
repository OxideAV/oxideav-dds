//! End-to-end BC1..BC5 decompression tests.
//!
//! Each test builds a small synthetic BC* DDS file in memory, parses
//! it through [`oxideav_dds::parse_dds`], then runs the matching
//! [`oxideav_dds::decode_bc*`] entry point against the raw block
//! plane and asserts the decoded pixels match a hand-computed
//! reference for that block.
//!
//! Reference values come from the public Microsoft "BC1, BC2 and
//! BC3" / "BC4" / "BC5" articles on learn.microsoft.com — no
//! external decoder source was consulted.

use oxideav_dds::types::{
    DDPF_FOURCC, DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE, FOURCC_BC4U,
    FOURCC_BC5U, FOURCC_DXT1, FOURCC_DXT3, FOURCC_DXT5,
};
use oxideav_dds::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_unorm, decode_bc5_unorm, parse_dds,
    DdsPixelFormat,
};

/// Build a minimal legacy DDS file with a `DDPF_FOURCC` pixel format
/// pointing at `four_cc`, an arbitrary-but-well-formed surface size,
/// and `payload` bytes for the pixel data.
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
    out.extend_from_slice(&0x1000u32.to_le_bytes()); // DDSCAPS_TEXTURE
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[test]
fn bc1_decompress_solid_white_4x4() {
    // c0 = 0xffff (white in RGB565), c1 = 0x0000, indices = 0 → all
    // pixels = c0 = (255, 255, 255).
    let block: [u8; 8] = [0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0];
    let bytes = build_fourcc_dds(FOURCC_DXT1, 4, 4, &block);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(img.surfaces.len(), 1);

    let mut rgba = vec![0u8; 4 * 4 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 4, 4, &mut rgba).unwrap();
    for chunk in rgba.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

#[test]
fn bc1_decompress_two_colour_8x4() {
    // Two side-by-side blocks. Block 0: solid red. Block 1: solid blue.
    // RGB565: red = 0xf800, blue = 0x001f.
    let red = 0xf800u16.to_le_bytes();
    let blue = 0x001fu16.to_le_bytes();
    let mut payload = Vec::new();
    // Block 0: c0 = red, c1 = black, all idx = 0 → all red.
    payload.extend_from_slice(&red);
    payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    // Block 1: c0 = blue, c1 = black, all idx = 0 → all blue.
    payload.extend_from_slice(&blue);
    payload.extend_from_slice(&[0, 0, 0, 0, 0, 0]);

    let bytes = build_fourcc_dds(FOURCC_DXT1, 8, 4, &payload);
    let img = parse_dds(&bytes).unwrap();
    let mut rgba = vec![0u8; 8 * 4 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 8, 4, &mut rgba).unwrap();

    // Pixel (0,0) = red (255, 0, 0, 255).
    assert_eq!(&rgba[0..4], &[255, 0, 0, 255]);
    // Pixel (4,0) = blue (0, 0, 255, 255). Row 0 starts at offset 0,
    // pixel x=4 at offset 4*4 = 16.
    assert_eq!(&rgba[16..20], &[0, 0, 255, 255]);
    // Pixel (3,0) still in red block.
    assert_eq!(&rgba[12..16], &[255, 0, 0, 255]);
}

#[test]
fn bc2_decompress_explicit_alpha_block() {
    // 4×4 block. Alpha block: each nibble carries the pixel index
    // (0..15) so pixel i has alpha = (i << 4) | i. Colour block:
    // solid white.
    let mut payload = Vec::new();
    // Alpha block: 16 nibbles, nibble i = i; bit-replicated to 8-bit
    // produces alphas 0x00, 0x11, 0x22, ..., 0xff.
    for i in 0..8 {
        let lo = (2 * i) as u8 & 0x0f;
        let hi = (2 * i + 1) as u8 & 0x0f;
        payload.push(lo | (hi << 4));
    }
    payload.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0]); // colour: white

    let bytes = build_fourcc_dds(FOURCC_DXT3, 4, 4, &payload);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc2);
    let mut rgba = vec![0u8; 4 * 4 * 4];
    decode_bc2(&img.surfaces[0].plane.data, 4, 4, &mut rgba).unwrap();
    for (i, chunk) in rgba.chunks_exact(4).enumerate() {
        let want_a = ((i as u8) << 4) | (i as u8);
        assert_eq!(chunk, &[255, 255, 255, want_a], "pixel {i}");
    }
}

#[test]
fn bc3_decompress_interpolated_alpha_block() {
    // a0 = 255, a1 = 0; indices all 0 → every pixel alpha = 255.
    // Colour block: solid white.
    let mut payload = vec![255u8, 0, 0, 0, 0, 0, 0, 0];
    payload.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0]);

    let bytes = build_fourcc_dds(FOURCC_DXT5, 4, 4, &payload);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc3);
    let mut rgba = vec![0u8; 4 * 4 * 4];
    decode_bc3(&img.surfaces[0].plane.data, 4, 4, &mut rgba).unwrap();
    for chunk in rgba.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

#[test]
fn bc4_decompress_solid_red_4x4() {
    // a0 = 200, a1 = 100; indices all 0 → all pixels = 200.
    let payload: [u8; 8] = [200, 100, 0, 0, 0, 0, 0, 0];
    let bytes = build_fourcc_dds(FOURCC_BC4U, 4, 4, &payload);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc4Unorm);
    let mut r = vec![0u8; 4 * 4];
    decode_bc4_unorm(&img.surfaces[0].plane.data, 4, 4, &mut r).unwrap();
    for &v in r.iter() {
        assert_eq!(v, 200);
    }
}

#[test]
fn bc5_decompress_two_channel_4x4() {
    // R block: a0 = 200, indices 0; G block: a0 = 50, indices 0.
    let mut payload = Vec::new();
    payload.extend_from_slice(&[200, 100, 0, 0, 0, 0, 0, 0]); // R
    payload.extend_from_slice(&[50, 25, 0, 0, 0, 0, 0, 0]); // G
    let bytes = build_fourcc_dds(FOURCC_BC5U, 4, 4, &payload);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc5Unorm);
    let mut rg = vec![0u8; 4 * 4 * 2];
    decode_bc5_unorm(&img.surfaces[0].plane.data, 4, 4, &mut rg).unwrap();
    for pair in rg.chunks_exact(2) {
        assert_eq!(pair, &[200, 50]);
    }
}

#[test]
fn bc1_handles_5x5_surface() {
    // 5×5 → 2×2 block grid. All blocks white.
    let mut payload = Vec::new();
    for _ in 0..4 {
        payload.extend_from_slice(&[0xff, 0xff, 0x00, 0x00, 0, 0, 0, 0]);
    }
    let bytes = build_fourcc_dds(FOURCC_DXT1, 5, 5, &payload);
    let img = parse_dds(&bytes).unwrap();
    let mut rgba = vec![0u8; 5 * 5 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 5, 5, &mut rgba).unwrap();
    for chunk in rgba.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}
