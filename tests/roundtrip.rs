//! Hard-asserted self-roundtrip tests for every uncompressed
//! [`oxideav_dds::DdsPixelFormat`] the round-1 encoder accepts.
//!
//! Each test builds a tiny synthetic plane (a 4×3 checkerboard with
//! per-channel deterministic byte values), encodes it with
//! [`oxideav_dds::encode_dds_uncompressed`], parses the resulting bytes
//! back with [`oxideav_dds::parse_dds`], and asserts the decoded plane
//! matches the source byte-for-byte.

use oxideav_dds::types::{
    DDPF_ALPHA, DDPF_ALPHAPIXELS, DDPF_FOURCC, DDPF_LUMINANCE, DDPF_RGB, DDSD_REQUIRED,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE, FOURCC_BC4U, FOURCC_BC5U, FOURCC_DX10,
    FOURCC_DXT1, FOURCC_DXT3, FOURCC_DXT5,
};
use oxideav_dds::{
    encode_dds_uncompressed, parse_dds, DdsImage, DdsPixelFormat, DdsPlane, DxgiFormat,
};

fn make_plane(pix: DdsPixelFormat, w: u32, h: u32) -> DdsPlane {
    let bpp = pix.bytes_per_pixel().expect("uncompressed only") as usize;
    let stride = w as usize * bpp;
    let mut data = vec![0u8; stride * h as usize];
    for y in 0..h as usize {
        for x in 0..w as usize {
            for c in 0..bpp {
                // Deterministic per-(x,y,c) byte so any swap or
                // truncation shows up in the diff immediately.
                data[y * stride + x * bpp + c] =
                    (x.wrapping_mul(7) + y.wrapping_mul(31) + c.wrapping_mul(53)) as u8;
            }
        }
    }
    DdsPlane { stride, data }
}

fn roundtrip_format(pix: DdsPixelFormat, w: u32, h: u32) {
    let plane = make_plane(pix, w, h);
    let src = DdsImage {
        width: w,
        height: h,
        pixel_format: pix,
        planes: vec![plane.clone()],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
    };
    let bytes = encode_dds_uncompressed(&src)
        .unwrap_or_else(|e| panic!("encode failed for {}: {e}", pix.name()));
    // Magic + header sanity.
    assert_eq!(
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        DDS_MAGIC,
        "magic mismatch for {}",
        pix.name()
    );
    assert_eq!(
        u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]),
        DDS_HEADER_SIZE as u32,
        "header.size mismatch for {}",
        pix.name()
    );
    let back = parse_dds(&bytes).unwrap_or_else(|e| panic!("parse failed for {}: {e}", pix.name()));
    assert_eq!(back.width, w, "width mismatch for {}", pix.name());
    assert_eq!(back.height, h, "height mismatch for {}", pix.name());
    assert_eq!(
        back.pixel_format,
        pix,
        "pixel_format mismatch for {}",
        pix.name()
    );
    assert_eq!(
        back.planes.len(),
        1,
        "plane count mismatch for {}",
        pix.name()
    );
    assert_eq!(
        back.planes[0].stride,
        plane.stride,
        "stride mismatch for {}",
        pix.name()
    );
    assert_eq!(
        back.planes[0].data,
        plane.data,
        "plane data mismatch for {}",
        pix.name()
    );
    assert!(
        !back.has_dxt10_header,
        "encoder should not emit DX10 header for {}",
        pix.name()
    );
}

#[test]
fn roundtrip_a8r8g8b8() {
    roundtrip_format(DdsPixelFormat::A8R8G8B8, 4, 3);
}

#[test]
fn roundtrip_x8r8g8b8() {
    roundtrip_format(DdsPixelFormat::X8R8G8B8, 4, 3);
}

#[test]
fn roundtrip_a8b8g8r8() {
    roundtrip_format(DdsPixelFormat::A8B8G8R8, 5, 7);
}

#[test]
fn roundtrip_r5g6b5() {
    roundtrip_format(DdsPixelFormat::R5G6B5, 8, 8);
}

#[test]
fn roundtrip_a1r5g5b5() {
    roundtrip_format(DdsPixelFormat::A1R5G5B5, 6, 4);
}

#[test]
fn roundtrip_a4r4g4b4() {
    roundtrip_format(DdsPixelFormat::A4R4G4B4, 6, 4);
}

#[test]
fn roundtrip_r8g8b8() {
    roundtrip_format(DdsPixelFormat::R8G8B8, 4, 4);
}

#[test]
fn roundtrip_a8l8() {
    roundtrip_format(DdsPixelFormat::A8L8, 4, 4);
}

#[test]
fn roundtrip_l8() {
    roundtrip_format(DdsPixelFormat::L8, 8, 4);
}

#[test]
fn roundtrip_a8() {
    roundtrip_format(DdsPixelFormat::A8, 8, 4);
}

#[test]
fn roundtrip_one_pixel() {
    // Smallest legal surface — width = height = 1.
    roundtrip_format(DdsPixelFormat::A8R8G8B8, 1, 1);
}

// --- Block-compressed pass-through tests ---------------------------------

/// Build a minimal legacy DDS file with a `DDPF_FOURCC` pixel format
/// pointing at `four_cc`, an arbitrary-but-well-formed surface size,
/// and `block_payload` bytes for the pixel data.
fn build_fourcc_dds(four_cc: u32, w: u32, h: u32, block_bytes: u32) -> Vec<u8> {
    let bw = (w + 3) / 4;
    let bh = (h + 3) / 4;
    let surface = (bw * bh * block_bytes) as usize;
    let payload: Vec<u8> = (0..surface).map(|i| (i & 0xff) as u8).collect();

    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + surface);
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes()); // size
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes()); // flags
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&surface.to_le_bytes()[..4]); // pitch_or_linear_size (4 bytes)
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    // pixel_format
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&four_cc.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0u32.to_le_bytes()); // r_bit_mask
    out.extend_from_slice(&0u32.to_le_bytes()); // g_bit_mask
    out.extend_from_slice(&0u32.to_le_bytes()); // b_bit_mask
    out.extend_from_slice(&0u32.to_le_bytes()); // a_bit_mask
                                                // caps + caps2..4 + reserved2 (5 × u32)
    out.extend_from_slice(&0x1000u32.to_le_bytes()); // DDSCAPS_TEXTURE
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&payload);
    out
}

#[test]
fn passthrough_dxt1() {
    let bytes = build_fourcc_dds(FOURCC_DXT1, 8, 8, 8);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(img.planes.len(), 1);
    // 2×2 blocks × 8 bytes/block = 32 bytes.
    assert_eq!(img.planes[0].data.len(), 32);
    // Stride is one row of blocks: 2 blocks × 8 bytes/block = 16 bytes.
    assert_eq!(img.planes[0].stride, 16);
    assert!(!img.has_dxt10_header);
}

#[test]
fn passthrough_dxt3() {
    let bytes = build_fourcc_dds(FOURCC_DXT3, 4, 4, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc2);
    assert_eq!(img.planes[0].data.len(), 16);
}

#[test]
fn passthrough_dxt5() {
    let bytes = build_fourcc_dds(FOURCC_DXT5, 4, 4, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc3);
    assert_eq!(img.planes[0].data.len(), 16);
}

#[test]
fn passthrough_bc4u() {
    let bytes = build_fourcc_dds(FOURCC_BC4U, 8, 4, 8);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc4Unorm);
    // 2×1 blocks × 8 bytes/block = 16 bytes.
    assert_eq!(img.planes[0].data.len(), 16);
}

#[test]
fn passthrough_bc5u() {
    let bytes = build_fourcc_dds(FOURCC_BC5U, 4, 4, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc5Unorm);
    assert_eq!(img.planes[0].data.len(), 16);
}

#[test]
fn passthrough_block_compressed_widths_round_up() {
    // 5×5 surface in BC1 must round up to 2×2 blocks (32 bytes), not
    // 1×1 blocks (8 bytes). Tests the `(w+3)/4` rounding in
    // `block_compressed_surface_size`.
    let bytes = build_fourcc_dds(FOURCC_DXT1, 5, 5, 8);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.planes[0].data.len(), 32);
}

// --- DX10 header path tests ---------------------------------------------

/// Build a minimal DDS file with a DX10 extension carrying `dxgi_format`.
fn build_dx10_dds(dxgi_format: u32, w: u32, h: u32, surface_bytes: usize) -> Vec<u8> {
    let payload: Vec<u8> = (0..surface_bytes).map(|i| (i & 0xff) as u8).collect();
    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + 20 + surface_bytes);
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(surface_bytes as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DX10.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0x1000u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // DX10 header
    out.extend_from_slice(&dxgi_format.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes()); // resource_dimension = TEXTURE2D
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flag
    out.extend_from_slice(&1u32.to_le_bytes()); // array_size
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flags2
    out.extend_from_slice(&payload);
    out
}

#[test]
fn dx10_bc7_unorm_passthrough() {
    let bytes = build_dx10_dds(98 /* BC7_UNORM */, 4, 4, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc7Unorm);
    assert!(img.has_dxt10_header);
    assert_eq!(img.dxgi_format, Some(DxgiFormat::Bc7Unorm));
    assert_eq!(img.planes[0].data.len(), 16);
}

#[test]
fn dx10_bc6h_uf16_passthrough() {
    let bytes = build_dx10_dds(95 /* BC6H_UF16 */, 4, 4, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc6hUf16);
    assert_eq!(img.dxgi_format, Some(DxgiFormat::Bc6hUf16));
}

#[test]
fn dx10_r8g8b8a8_unorm_uncompressed() {
    let bytes = build_dx10_dds(28 /* R8G8B8A8_UNORM */, 2, 2, 16);
    let img = parse_dds(&bytes).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::A8B8G8R8);
    assert_eq!(img.dxgi_format, Some(DxgiFormat::R8G8B8A8Unorm));
    assert_eq!(img.planes[0].data.len(), 16);
    assert_eq!(img.planes[0].stride, 8);
}

// --- Negative tests -----------------------------------------------------

#[test]
fn rejects_bad_magic() {
    let mut bytes = vec![0u8; 4 + DDS_HEADER_SIZE];
    bytes[0..4].copy_from_slice(b"XXXX");
    let err = parse_dds(&bytes).unwrap_err();
    let _ = err;
}

#[test]
fn rejects_short_buffer() {
    let bytes = vec![0u8; 16];
    assert!(parse_dds(&bytes).is_err());
}

#[test]
fn rejects_unknown_dxgi_format() {
    let bytes = build_dx10_dds(9999, 4, 4, 16);
    assert!(parse_dds(&bytes).is_err());
}

#[test]
fn rejects_block_compressed_in_uncompressed_encoder() {
    let img = DdsImage {
        width: 4,
        height: 4,
        pixel_format: DdsPixelFormat::Bc1,
        planes: vec![DdsPlane {
            stride: 8,
            data: vec![0u8; 8],
        }],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
    };
    assert!(encode_dds_uncompressed(&img).is_err());
}

#[test]
fn pixel_format_helpers() {
    assert_eq!(DdsPixelFormat::A8R8G8B8.bits_per_pixel(), 32);
    assert_eq!(DdsPixelFormat::A8R8G8B8.bytes_per_pixel(), Some(4));
    assert!(DdsPixelFormat::A8R8G8B8.block_bytes().is_none());
    assert_eq!(DdsPixelFormat::Bc1.block_bytes(), Some(8));
    assert_eq!(DdsPixelFormat::Bc7Unorm.block_bytes(), Some(16));
    assert!(DdsPixelFormat::Bc1.is_block_compressed());
    assert!(!DdsPixelFormat::A8R8G8B8.is_block_compressed());
}

#[test]
fn dxgi_format_round_trip_enum() {
    for raw in [28u32, 29, 49, 61, 71, 72, 80, 95, 98, 115] {
        let f = DxgiFormat::from_u32(raw);
        assert_eq!(f.to_u32(), raw, "DXGI {raw} did not round-trip");
    }
    assert_eq!(DxgiFormat::from_u32(123_456), DxgiFormat::Unknown(123_456));
    assert_eq!(DxgiFormat::Unknown(7).to_u32(), 7);
}

#[test]
fn flags_constants_consistent() {
    // These constants are referenced both inside the crate (decoder /
    // encoder) and exported via `oxideav_dds::types::*`. A simple
    // sanity assert-eq guards against accidental sign-extension or
    // hex-typo regressions.
    assert_eq!(DDPF_RGB, 0x40);
    assert_eq!(DDPF_FOURCC, 0x4);
    assert_eq!(DDPF_LUMINANCE, 0x2_0000);
    assert_eq!(DDPF_ALPHAPIXELS, 0x1);
    assert_eq!(DDPF_ALPHA, 0x2);
}
