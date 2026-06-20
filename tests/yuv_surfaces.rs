//! Integration tests for the YUV (video) DXGI surface formats.
//!
//! Each test builds a minimal DX10 DDS byte stream carrying one of the
//! eleven YUV `DXGI_FORMAT` values, parses it with
//! [`oxideav_dds::parse_dds`], asserts the resolved
//! [`oxideav_dds::DdsPixelFormat::Yuv`] variant and the carried surface
//! byte length, then expands the raw bytes with the matching
//! `decode_*_surface` helper and checks the interleaved `[Y, U, V, A]`
//! output.
//!
//! All layout facts (channel mapping, plane structure, subsampling, byte
//! sizing, dimension constraints) come from Microsoft's public
//! `DXGI_FORMAT` enumeration page staged under `docs/image/dds/`.

use oxideav_dds::types::{
    DDPF_FOURCC, DDSD_PIXELFORMAT, DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
    FOURCC_DX10,
};
use oxideav_dds::yuv::YuvFormat;
use oxideav_dds::{
    decode_420_opaque_surface, decode_ayuv_surface, decode_nv11_surface, decode_nv12_surface,
    decode_p010_surface, decode_p016_surface, decode_y210_surface, decode_y216_surface,
    decode_y410_surface, decode_y416_surface, decode_yuy2_surface, parse_dds, DdsPixelFormat,
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
                                                // DDS_HEADER_DXT10: dxgi_format, dimension (2D = 3), misc, array=1, misc2
    out.extend_from_slice(&dxgi_format.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(pixel_data);
    out
}

fn le16(words: &[u16]) -> Vec<u8> {
    let mut v = Vec::with_capacity(words.len() * 2);
    for w in words {
        v.extend_from_slice(&w.to_le_bytes());
    }
    v
}

// --- 4:4:4 packed --------------------------------------------------------

#[test]
fn dx10_ayuv_end_to_end() {
    // DXGI_FORMAT_AYUV = 100. One pixel: on-disk [V, U, Y, A].
    let dds = build_dx10_dds(100, 1, 1, &[5u8, 6, 7, 8]);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Ayuv));
    assert_eq!(img.pixel_format.name(), "AYUV");
    assert_eq!(img.surfaces[0].plane.data.len(), 4);
    let out = decode_ayuv_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![7, 6, 5, 8]); // [Y, U, V, A]
}

#[test]
fn dx10_y410_end_to_end() {
    // DXGI_FORMAT_Y410 = 101. U=3, Y=5, V=7, A=2 packed R10G10B10A2-style.
    let word: u32 = 3 | (5 << 10) | (7 << 20) | (2 << 30);
    let dds = build_dx10_dds(101, 2, 1, &word.to_le_bytes().repeat(2));
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Y410));
    assert_eq!(img.surfaces[0].plane.data.len(), 8); // 2px × 4 bytes
    let out = decode_y410_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![5, 3, 7, 2, 5, 3, 7, 2]);
}

#[test]
fn dx10_y416_end_to_end() {
    // DXGI_FORMAT_Y416 = 102. On-disk [U, Y, V, A] u16.
    let px = le16(&[100, 200, 300, 400]);
    let dds = build_dx10_dds(102, 1, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Y416));
    assert_eq!(img.surfaces[0].plane.data.len(), 8);
    let out = decode_y416_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![200, 100, 300, 400]);
}

// --- 4:2:2 packed --------------------------------------------------------

#[test]
fn dx10_yuy2_end_to_end() {
    // DXGI_FORMAT_YUY2 = 107. Pixel pair [Y0, U, Y1, V].
    let dds = build_dx10_dds(107, 2, 1, &[10u8, 20, 30, 40]);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Yuy2));
    assert_eq!(img.surfaces[0].plane.data.len(), 4); // 2px → 4 bytes
    let out = decode_yuy2_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![10, 20, 40, 0xff, 30, 20, 40, 0xff]);
}

#[test]
fn dx10_y210_end_to_end() {
    // DXGI_FORMAT_Y210 = 108. Pixel pair [Y0, U, Y1, V] u16.
    let px = le16(&[1000, 2000, 3000, 4000]);
    let dds = build_dx10_dds(108, 2, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Y210));
    assert_eq!(img.surfaces[0].plane.data.len(), 8); // 2px → 8 bytes
    let out = decode_y210_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(
        out,
        vec![1000, 2000, 4000, 0xffff, 3000, 2000, 4000, 0xffff]
    );
}

#[test]
fn dx10_y216_end_to_end() {
    let px = le16(&[1, 2, 3, 4]);
    let dds = build_dx10_dds(109, 2, 1, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Y216));
    let out = decode_y216_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![1, 2, 4, 0xffff, 3, 2, 4, 0xffff]);
}

// --- 4:2:0 planar --------------------------------------------------------

#[test]
fn dx10_nv12_end_to_end() {
    // DXGI_FORMAT_NV12 = 103. 2x2: Y plane [1,2,3,4] + UV [200,201].
    let dds = build_dx10_dds(103, 2, 2, &[1u8, 2, 3, 4, 200, 201]);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Nv12));
    // 2x2 4:2:0: Y(4) + UV(2) = 6 bytes.
    assert_eq!(img.surfaces[0].plane.data.len(), 6);
    let out = decode_nv12_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(
        out,
        vec![1, 200, 201, 0xff, 2, 200, 201, 0xff, 3, 200, 201, 0xff, 4, 200, 201, 0xff]
    );
}

#[test]
fn dx10_420_opaque_end_to_end() {
    let dds = build_dx10_dds(106, 2, 2, &[1u8, 2, 3, 4, 200, 201]);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Opaque420));
    assert_eq!(img.pixel_format.name(), "420_OPAQUE");
    let out = decode_420_opaque_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    let nv12 = decode_nv12_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, nv12);
}

#[test]
fn dx10_p010_end_to_end() {
    // DXGI_FORMAT_P010 = 104. 2x2 u16 Y plane [1,2,3,4] + UV [500,600].
    let px = le16(&[1, 2, 3, 4, 500, 600]);
    let dds = build_dx10_dds(104, 2, 2, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::P010));
    // 2x2 u16 4:2:0: Y(8) + UV(4) = 12 bytes.
    assert_eq!(img.surfaces[0].plane.data.len(), 12);
    let out = decode_p010_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(
        out,
        vec![1, 500, 600, 0xffff, 2, 500, 600, 0xffff, 3, 500, 600, 0xffff, 4, 500, 600, 0xffff]
    );
}

#[test]
fn dx10_p016_end_to_end() {
    let px = le16(&[1, 2, 3, 4, 5, 6]);
    let dds = build_dx10_dds(105, 2, 2, &px);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::P016));
    let out = decode_p016_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    let p010 = decode_p010_surface(2, 2, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, p010);
}

// --- 4:1:1 planar --------------------------------------------------------

#[test]
fn dx10_nv11_end_to_end() {
    // DXGI_FORMAT_NV11 = 110. 4x1: Y [1,2,3,4] + UV [50,51], padded to 8.
    let dds = build_dx10_dds(110, 4, 1, &[1u8, 2, 3, 4, 50, 51, 0, 0]);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Nv11));
    // Staging size is padded to 2*w*h = 8 bytes.
    assert_eq!(img.surfaces[0].plane.data.len(), 8);
    let out = decode_nv11_surface(4, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(
        out,
        vec![1, 50, 51, 0xff, 2, 50, 51, 0xff, 3, 50, 51, 0xff, 4, 50, 51, 0xff]
    );
}

// --- Truncation / dimension robustness through parse_dds -----------------

#[test]
fn truncated_yuv_payload_is_rejected() {
    // NV12 2x2 needs 6 bytes; supply only 4.
    let dds = build_dx10_dds(103, 2, 2, &[1u8, 2, 3, 4]);
    assert!(parse_dds(&dds).is_err());
}

#[test]
fn odd_width_yuy2_is_rejected() {
    // YUY2 requires even width; 3x1 must fail the dimension check.
    let dds = build_dx10_dds(107, 3, 1, &[0u8; 6]);
    assert!(parse_dds(&dds).is_err());
}
