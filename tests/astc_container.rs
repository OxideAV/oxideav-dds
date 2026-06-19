//! ASTC LDR container recognition + surface-sizing + decode integration
//! tests for `oxideav-dds`.
//!
//! Every byte of the test fixtures below is hand-built from the DDS
//! on-disk layout (`docs/image/dds/`) and the ASTC DXGI mapping
//! (`docs/image/astc/astc-ldr-notes.md` §11). No external DDS / ASTC
//! tooling is consulted.

use oxideav_dds::{
    decode_astc_ldr_surface, parse_dds, DdsPixelFormat, DxgiFormat, DDS_HEADER_SIZE, DDS_MAGIC,
};

/// Build a minimal DX10 DDS file: magic + DDS_HEADER + DDS_HEADER_DXT10
/// + `payload` bytes. `dxgi` is written into the DXT10 extension.
fn build_dx10_dds(width: u32, height: u32, dxgi: u32, payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&DDS_MAGIC.to_le_bytes());

    // DDS_HEADER (124 bytes).
    let mut h = vec![0u8; DDS_HEADER_SIZE];
    let put = |h: &mut [u8], off: usize, val: u32| {
        h[off..off + 4].copy_from_slice(&val.to_le_bytes());
    };
    put(&mut h, 0, DDS_HEADER_SIZE as u32); // dwSize
                                            // flags: caps|height|width|pixelformat|linearsize
    put(&mut h, 4, 0x1 | 0x2 | 0x4 | 0x1000 | 0x0008_0000);
    put(&mut h, 8, height); // dwHeight
    put(&mut h, 12, width); // dwWidth
    put(&mut h, 16, payload.len() as u32); // dwPitchOrLinearSize
                                           // DDS_PIXELFORMAT at offset 72 within the header.
    put(&mut h, 72, 32); // ddspf.dwSize
    put(&mut h, 76, 0x4); // ddspf.flags = DDPF_FOURCC
    put(&mut h, 80, u32::from_le_bytes(*b"DX10")); // four_cc = "DX10"
    put(&mut h, 104, 0x1000); // caps = DDSCAPS_TEXTURE
    v.extend_from_slice(&h);

    // DDS_HEADER_DXT10 (20 bytes).
    let mut dx = vec![0u8; 20];
    dx[0..4].copy_from_slice(&dxgi.to_le_bytes()); // dxgi_format
    dx[4..8].copy_from_slice(&3u32.to_le_bytes()); // TEXTURE2D
    dx[12..16].copy_from_slice(&1u32.to_le_bytes()); // array_size
    v.extend_from_slice(&dx);

    v.extend_from_slice(payload);
    v
}

#[test]
fn dxgi_astc_codes_roundtrip() {
    // Every ASTC code in 133..=187 (skipping the reserved 4th slots)
    // must map to a named variant and back.
    for &code in &[
        133u32, 134, 135, 137, 138, 139, 141, 142, 143, 145, 146, 147, 149, 150, 151, 153, 154,
        155, 157, 158, 159, 161, 162, 163, 165, 166, 167, 169, 170, 171, 173, 174, 175, 177, 178,
        179, 181, 182, 183, 185, 186, 187,
    ] {
        let f = DxgiFormat::from_u32(code);
        assert_eq!(f.to_u32(), code, "round-trip failed for {code}");
        assert!(
            f.astc_footprint().is_some(),
            "code {code} should have a footprint"
        );
    }
    // The reserved 4th slot (136) stays Unknown.
    assert!(matches!(
        DxgiFormat::from_u32(136),
        DxgiFormat::Unknown(136)
    ));
}

#[test]
fn dxgi_astc_footprints_correct() {
    assert_eq!(DxgiFormat::Astc4x4Unorm.astc_footprint(), Some((4, 4)));
    assert_eq!(DxgiFormat::Astc8x5UnormSrgb.astc_footprint(), Some((8, 5)));
    assert_eq!(DxgiFormat::Astc12x12Unorm.astc_footprint(), Some((12, 12)));
    assert_eq!(DxgiFormat::Astc10x6Typeless.astc_footprint(), Some((10, 6)));
    assert_eq!(DxgiFormat::Bc7Unorm.astc_footprint(), None);
}

#[test]
fn parse_astc_4x4_surface_sizing() {
    // 8×8 surface @ 4×4 footprint → 2×2 = 4 blocks × 16 bytes = 64.
    let payload = vec![0u8; 64];
    let dds = build_dx10_dds(8, 8, 134, &payload); // ASTC_4X4_UNORM
    let img = parse_dds(&dds).expect("parse ASTC 4x4");
    assert_eq!(img.pixel_format.astc_footprint(), Some((4, 4)));
    assert!(matches!(
        img.pixel_format,
        DdsPixelFormat::Astc {
            block_w: 4,
            block_h: 4,
            srgb: false
        }
    ));
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data.len(), 64);
    assert_eq!(img.dxgi_format, Some(DxgiFormat::Astc4x4Unorm));
}

#[test]
fn parse_astc_non_square_footprint_sizing() {
    // 10×7 surface @ 8×5 footprint → ceil(10/8)=2, ceil(7/5)=2 → 4
    // blocks × 16 = 64 bytes.
    let payload = vec![0u8; 64];
    let dds = build_dx10_dds(10, 7, 154, &payload); // ASTC_8X5_UNORM
    let img = parse_dds(&dds).expect("parse ASTC 8x5");
    assert_eq!(img.pixel_format.astc_footprint(), Some((8, 5)));
    assert_eq!(img.surfaces[0].plane.data.len(), 64);
}

#[test]
fn parse_astc_srgb_flag_set() {
    let payload = vec![0u8; 16];
    let dds = build_dx10_dds(4, 4, 135, &payload); // ASTC_4X4_UNORM_SRGB
    let img = parse_dds(&dds).expect("parse ASTC 4x4 srgb");
    assert!(matches!(
        img.pixel_format,
        DdsPixelFormat::Astc { srgb: true, .. }
    ));
}

#[test]
fn parse_astc_truncated_payload_rejected() {
    // Claim an 8×8 4×4 surface (needs 64 bytes) but supply only 32.
    let payload = vec![0u8; 32];
    let dds = build_dx10_dds(8, 8, 134, &payload);
    assert!(parse_dds(&dds).is_err());
}

#[test]
fn decode_astc_surface_through_format() {
    // Build a void-extent block (constant magenta-ish colour) and decode
    // a 4×4 surface through the format-aware wrapper.
    let mut block = [0u8; 16];
    // void-extent: bits[8:2]=1111111, bits[1:0]=00, bits[11:10]=11,
    // bit9=0 (LDR). UNORM16 colour with high byte = the output byte.
    let lo: u64 = 0x1FC | (0b11 << 10);
    // R high byte 0x12, G 0x34, B 0x56, A 0xFF.
    let hi: u64 = 0x12_00 | (0x34_00u64 << 16) | (0x56_00u64 << 32) | (0xFF_00u64 << 48);
    block[0..8].copy_from_slice(&lo.to_le_bytes());
    block[8..16].copy_from_slice(&hi.to_le_bytes());

    let pix = DdsPixelFormat::Astc {
        block_w: 4,
        block_h: 4,
        srgb: false,
    };
    let rgba = decode_astc_ldr_surface(pix, &block, 4, 4).expect("astc decode");
    assert_eq!(rgba.len(), 4 * 4 * 4);
    for px in rgba.chunks_exact(4) {
        assert_eq!(px, [0x12, 0x34, 0x56, 0xFF]);
    }
    // A non-ASTC format returns None.
    assert!(decode_astc_ldr_surface(DdsPixelFormat::Bc1, &block, 4, 4).is_none());
}
