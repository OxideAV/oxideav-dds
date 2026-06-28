//! Round 379: legacy `DDS_PIXELFORMAT` mask layouts beyond the round-1
//! common set.
//!
//! Microsoft's "Common DDS File Resource Formats" table (the
//! programming-guide pixel-format section) lists several legacy
//! uncompressed layouts that older `.dds` writers emit with explicit
//! channel masks rather than a DX10 header:
//!
//! * `D3DFMT_G16R16` — a two-channel 16:16 layout (red low 16 bits,
//!   green high 16 bits) sharing the bytes of `DXGI_FORMAT_R16G16_UNORM`.
//! * `D3DFMT_A2R10G10B10` — the BGR-ordered sibling of the
//!   already-supported `A2B10G10R10`: red in the most-significant 10
//!   colour bits, blue in the least, alpha in the top two bits.
//!
//! Both store their channels verbatim on disk, so a byte-identical
//! round-trip through `parse_dds` is the correctness contract. The masks
//! are taken solely from Microsoft's public programming-guide table; no
//! external library source was consulted.

use oxideav_dds::types::{
    DDPF_ALPHAPIXELS, DDPF_RGB, DDSCAPS_TEXTURE, DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC,
    DDS_PIXELFORMAT_SIZE,
};
use oxideav_dds::{
    decode_a2r10g10b10_surface, encode_dds_uncompressed, parse_dds, DdsImage, DdsPixelFormat,
    DdsPlane,
};

/// Build a legacy (non-DX10) uncompressed DDS file from explicit
/// pixel-format masks and a raw payload.
#[allow(clippy::too_many_arguments)]
fn build_mask_dds(
    flags: u32,
    rgb_bit_count: u32,
    r: u32,
    g: u32,
    b: u32,
    a: u32,
    w: u32,
    h: u32,
    payload: &[u8],
) -> Vec<u8> {
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
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // four_cc
    out.extend_from_slice(&rgb_bit_count.to_le_bytes());
    out.extend_from_slice(&r.to_le_bytes());
    out.extend_from_slice(&g.to_le_bytes());
    out.extend_from_slice(&b.to_le_bytes());
    out.extend_from_slice(&a.to_le_bytes());
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(payload);
    out
}

#[test]
fn g16r16_legacy_mask_resolves_to_r16g16_unorm() {
    // 4x2, four bytes per pixel.
    let payload: Vec<u8> = (0..(4 * 2 * 4))
        .map(|i| ((i * 53 + 7) % 256) as u8)
        .collect();
    // DDS_RGB flavour (no alpha): R=0x0000ffff, G=0xffff0000.
    let dds = build_mask_dds(DDPF_RGB, 32, 0x0000_ffff, 0xffff_0000, 0, 0, 4, 2, &payload);
    let img = parse_dds(&dds).expect("parse G16R16");
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16Unorm);
    assert_eq!(img.width, 4);
    assert_eq!(img.height, 2);
    assert_eq!(img.surfaces[0].plane.data, payload);
}

#[test]
fn g16r16_legacy_mask_rgba_flag_flavour_also_resolves() {
    // The programming-guide table lists G16R16 under both DDS_RGBA and
    // DDS_RGB rows; the RGBA row carries the spurious alpha flag but no
    // alpha mask. Accept it too.
    let payload = vec![0x11u8; 2 * 2 * 4];
    let dds = build_mask_dds(
        DDPF_RGB | DDPF_ALPHAPIXELS,
        32,
        0x0000_ffff,
        0xffff_0000,
        0,
        0,
        2,
        2,
        &payload,
    );
    let img = parse_dds(&dds).expect("parse G16R16 (RGBA flag)");
    assert_eq!(img.pixel_format, DdsPixelFormat::R16G16Unorm);
}

#[test]
fn a2r10g10b10_legacy_mask_resolves_and_decodes() {
    // One pixel, channels chosen to be distinguishable: R=0x2aa (682),
    // G=0x155 (341), B=0x0ff (255), A=0x2. Pack BGR order:
    // word = B | (G<<10) | (R<<20) | (A<<30).
    let r: u32 = 0x2aa;
    let g: u32 = 0x155;
    let b: u32 = 0x0ff;
    let a: u32 = 0x2;
    let word = b | (g << 10) | (r << 20) | (a << 30);
    let payload = word.to_le_bytes().to_vec();
    let dds = build_mask_dds(
        DDPF_RGB | DDPF_ALPHAPIXELS,
        32,
        0x3ff0_0000,
        0x000f_fc00,
        0x0000_03ff,
        0xc000_0000,
        1,
        1,
        &payload,
    );
    let img = parse_dds(&dds).expect("parse A2R10G10B10");
    assert_eq!(img.pixel_format, DdsPixelFormat::A2R10G10B10);
    assert_eq!(img.surfaces[0].plane.data, payload);

    // Decode the stored channels back out in [R, G, B, A] order.
    let out = decode_a2r10g10b10_surface(1, 1, &img.surfaces[0].plane.data).unwrap();
    assert_eq!(out, vec![r as u16, g as u16, b as u16, a as u16]);
}

#[test]
fn a2r10g10b10_roundtrips_through_encoder() {
    // The legacy encoder writes the A2R10G10B10 mask, so a hand-built
    // image round-trips byte-for-byte.
    let payload: Vec<u8> = (0..(2 * 2 * 4))
        .map(|i| ((i * 91 + 3) % 256) as u8)
        .collect();
    let img = DdsImage {
        width: 2,
        height: 2,
        pixel_format: DdsPixelFormat::A2R10G10B10,
        planes: vec![DdsPlane {
            stride: 2 * 4,
            data: payload.clone(),
        }],
        surfaces: Vec::new(),
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).expect("encode A2R10G10B10");
    let decoded = parse_dds(&bytes).expect("re-parse A2R10G10B10");
    assert_eq!(decoded.pixel_format, DdsPixelFormat::A2R10G10B10);
    assert!(!decoded.has_dxt10_header);
    assert_eq!(decoded.surfaces[0].plane.data, payload);
}

#[test]
fn a2r10g10b10_decode_short_buffer_errors() {
    let err = decode_a2r10g10b10_surface(2, 2, &[0u8; 4]);
    assert!(err.is_err());
}
