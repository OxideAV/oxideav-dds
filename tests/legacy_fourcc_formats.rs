//! Round 379: legacy ASCII-FourCC packed RGB / 4:2:2 layouts.
//!
//! Microsoft's "Common DDS File Resource Formats" table lists four
//! packed formats older `.dds` writers carry under an ASCII FourCC tag
//! rather than a DX10 header:
//!
//! * `RGBG` (`D3DFMT_R8G8_B8G8`) and `GRGB` (`D3DFMT_G8R8_G8B8`) — the
//!   two horizontally sub-sampled packed RGB layouts (DXGI values 68 /
//!   69), routed to the existing `R8G8_B8G8_UNORM` / `G8R8_G8B8_UNORM`
//!   byte layouts.
//! * `YUY2` (`D3DFMT_YUY2`) — the 8-bit 4:2:2 packed luma/chroma layout
//!   (DXGI value 107), `[Y0, U, Y1, V]` per pixel pair.
//! * `UYVY` (`D3DFMT_UYVY`) — the byte-swizzled `[U, Y0, V, Y1]` sibling
//!   of `YUY2`, which has no DX10 `DXGI_FORMAT`.
//!
//! All four store their data verbatim on disk; the FourCC routing simply
//! resolves them to the correct `DdsPixelFormat`. Tags + byte layouts are
//! taken solely from Microsoft's public programming-guide table; no
//! external library source was consulted.

use oxideav_dds::types::{
    DDPF_FOURCC, DDSCAPS_TEXTURE, DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
    FOURCC_GRGB, FOURCC_RGBG, FOURCC_UYVY, FOURCC_YUY2,
};
use oxideav_dds::{decode_uyvy_surface, decode_yuy2_surface, parse_dds, DdsPixelFormat, YuvFormat};

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
    for _ in 0..5 {
        out.extend_from_slice(&0u32.to_le_bytes()); // bitcount + 4 masks
    }
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(payload);
    out
}

#[test]
fn rgbg_fourcc_resolves_to_r8g8_b8g8() {
    // 4x2 packed pairs → 2 bytes/pixel.
    let payload: Vec<u8> = (0..(4 * 2 * 2))
        .map(|i| ((i * 41 + 5) % 256) as u8)
        .collect();
    let dds = build_fourcc_dds(FOURCC_RGBG, 4, 2, &payload);
    let img = parse_dds(&dds).expect("parse RGBG");
    assert_eq!(img.pixel_format, DdsPixelFormat::R8G8B8G8Unorm);
    assert_eq!(img.surfaces[0].plane.data, payload);
}

#[test]
fn grgb_fourcc_resolves_to_g8r8_g8b8() {
    let payload = vec![0x42u8; 4 * 2 * 2];
    let dds = build_fourcc_dds(FOURCC_GRGB, 4, 2, &payload);
    let img = parse_dds(&dds).expect("parse GRGB");
    assert_eq!(img.pixel_format, DdsPixelFormat::G8R8G8B8Unorm);
}

#[test]
fn yuy2_fourcc_resolves_and_decodes() {
    // One pixel pair: [Y0=10, U=20, Y1=30, V=40].
    let payload = vec![10u8, 20, 30, 40];
    let dds = build_fourcc_dds(FOURCC_YUY2, 2, 1, &payload);
    let img = parse_dds(&dds).expect("parse YUY2");
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Yuy2));
    let out = decode_yuy2_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    // pixel 0 = [Y0, U, V, 0xff], pixel 1 = [Y1, U, V, 0xff]
    assert_eq!(out, vec![10, 20, 40, 0xff, 30, 20, 40, 0xff]);
}

#[test]
fn uyvy_fourcc_resolves_and_decodes() {
    // One pixel pair: [U=20, Y0=10, V=40, Y1=30] — chroma-first order.
    let payload = vec![20u8, 10, 40, 30];
    let dds = build_fourcc_dds(FOURCC_UYVY, 2, 1, &payload);
    let img = parse_dds(&dds).expect("parse UYVY");
    assert_eq!(img.pixel_format, DdsPixelFormat::Yuv(YuvFormat::Uyvy));
    let out = decode_uyvy_surface(2, 1, &img.surfaces[0].plane.data).unwrap();
    // Same reconstructed pixels as the YUY2 case above (only on-disk
    // byte order differs).
    assert_eq!(out, vec![10, 20, 40, 0xff, 30, 20, 40, 0xff]);
}

#[test]
fn uyvy_rejects_odd_width() {
    assert!(decode_uyvy_surface(3, 1, &[0u8; 8]).is_err());
}

#[test]
fn uyvy_rejects_truncated_payload() {
    // 4x1 needs 8 bytes; give 4.
    assert!(decode_uyvy_surface(4, 1, &[0u8; 4]).is_err());
}
