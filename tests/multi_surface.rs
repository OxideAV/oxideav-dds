//! Mipmap chain + cubemap face + DX10 texture array tests.
//!
//! Each test builds a minimal DDS file with the relevant
//! `caps2 / mip_map_count / array_size` combination, parses it via
//! [`oxideav_dds::parse_dds`], and asserts the resulting
//! [`oxideav_dds::DdsImage::surfaces`] vector has the right shape
//! (surface count, per-surface (mip_level, array_slice, face) tag,
//! per-surface dimensions).

use oxideav_dds::types::{
    DDPF_FOURCC, DDPF_RGB, DDSCAPS2_CUBEMAP, DDSCAPS2_CUBEMAP_ALL_FACES, DDSD_MIPMAPCOUNT,
    DDSD_REQUIRED, DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE, DDS_RESOURCE_MISC_TEXTURECUBE,
    FOURCC_DX10, FOURCC_DXT1,
};
use oxideav_dds::{parse_dds, CubemapFace, DdsPixelFormat};

fn push_pixel_format_a8r8g8b8(out: &mut Vec<u8>) {
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&(DDPF_RGB | 0x1).to_le_bytes()); // RGB | ALPHAPIXELS
    out.extend_from_slice(&0u32.to_le_bytes()); // four_cc
    out.extend_from_slice(&32u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0x00ff_0000u32.to_le_bytes());
    out.extend_from_slice(&0x0000_ff00u32.to_le_bytes());
    out.extend_from_slice(&0x0000_00ffu32.to_le_bytes());
    out.extend_from_slice(&0xff00_0000u32.to_le_bytes());
}

fn push_pixel_format_dxt1(out: &mut Vec<u8>) {
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DXT1.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

fn push_pixel_format_dx10(out: &mut Vec<u8>) {
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DX10.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
}

#[test]
fn mipmap_chain_uncompressed_8x8_a8r8g8b8_4_levels() {
    // 8x8 + 4 mip levels = 8x8, 4x4, 2x2, 1x1 surfaces.
    // A8R8G8B8 = 4 bytes/pixel.
    // Total = 64*4 + 16*4 + 4*4 + 1*4 = 256 + 64 + 16 + 4 = 340 bytes.
    let mut data = Vec::new();
    // mip 0: 64 pixels of 0xaa
    data.extend(std::iter::repeat(0xaa).take(64 * 4));
    // mip 1: 16 pixels of 0xbb
    data.extend(std::iter::repeat(0xbb).take(16 * 4));
    // mip 2: 4 pixels of 0xcc
    data.extend(std::iter::repeat(0xcc).take(4 * 4));
    // mip 3: 1 pixel of 0xdd
    data.extend(std::iter::repeat_n(0xdd, 4));

    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&(DDSD_REQUIRED | DDSD_MIPMAPCOUNT).to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes()); // height
    out.extend_from_slice(&8u32.to_le_bytes()); // width
    out.extend_from_slice(&(8u32 * 4).to_le_bytes()); // pitch
    out.extend_from_slice(&0u32.to_le_bytes()); // depth
    out.extend_from_slice(&4u32.to_le_bytes()); // mip_map_count
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_a8r8g8b8(&mut out);
    out.extend_from_slice(&0x401008u32.to_le_bytes()); // caps: TEXTURE | COMPLEX | MIPMAP
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 8);
    assert_eq!(img.mip_map_count, 4);
    assert_eq!(img.array_size, 1);
    assert!(!img.is_cubemap);
    assert_eq!(img.surfaces.len(), 4);

    let dims: Vec<(u32, u32)> = img.surfaces.iter().map(|s| (s.width, s.height)).collect();
    assert_eq!(dims, vec![(8, 8), (4, 4), (2, 2), (1, 1)]);

    // Check mip-0 fill.
    assert!(img.surfaces[0].plane.data.iter().all(|&b| b == 0xaa));
    // Check mip-3 fill.
    assert!(img.surfaces[3].plane.data.iter().all(|&b| b == 0xdd));

    // mip_level / array_slice / face tags.
    for (i, s) in img.surfaces.iter().enumerate() {
        assert_eq!(s.mip_level, i as u32);
        assert_eq!(s.array_slice, 0);
        assert!(s.face.is_none());
    }

    // The legacy `planes[0]` field must mirror surface[0].
    assert_eq!(img.planes[0].data, img.surfaces[0].plane.data);
}

#[test]
fn cubemap_legacy_dxt1_4x4_six_faces() {
    // Legacy cubemap: 6 × DXT1 4x4 surfaces (8 bytes each = 48 bytes).
    // Each face filled with a different repeating byte so we can
    // verify per-face ordering.
    let mut data = Vec::new();
    for face_byte in [0x10u8, 0x20, 0x30, 0x40, 0x50, 0x60] {
        data.extend(std::iter::repeat(face_byte).take(8));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes()); // linear_size = one face
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count = 1 (none)
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_dxt1(&mut out);
    out.extend_from_slice(&0x1008u32.to_le_bytes()); // caps: TEXTURE | COMPLEX
    out.extend_from_slice(&(DDSCAPS2_CUBEMAP | DDSCAPS2_CUBEMAP_ALL_FACES).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert!(img.is_cubemap);
    assert_eq!(img.array_size, 1);
    assert_eq!(img.mip_map_count, 1);
    assert_eq!(img.surfaces.len(), 6);

    let face_order: Vec<CubemapFace> = img.surfaces.iter().map(|s| s.face.unwrap()).collect();
    assert_eq!(face_order, CubemapFace::ALL.to_vec());

    // Verify each face's data matches its byte fill.
    for (i, fb) in [0x10u8, 0x20, 0x30, 0x40, 0x50, 0x60].iter().enumerate() {
        assert!(
            img.surfaces[i].plane.data.iter().all(|b| b == fb),
            "face {} should be filled with {:02x}, got {:02x?}",
            img.surfaces[i].face.unwrap().short_name(),
            fb,
            &img.surfaces[i].plane.data[..4]
        );
    }
}

#[test]
fn dx10_texture_array_three_slices() {
    // DX10 array_size = 3; each slice is one A8R8G8B8 surface 2x2 = 16 bytes.
    let mut data = Vec::new();
    for slice_byte in [0xa0u8, 0xb0, 0xc0] {
        data.extend(std::iter::repeat(slice_byte).take(2 * 2 * 4));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes()); // pitch = 2*4
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_dx10(&mut out);
    out.extend_from_slice(&0x1000u32.to_le_bytes()); // caps: TEXTURE
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    // DX10 header
    out.extend_from_slice(&87u32.to_le_bytes()); // dxgi = B8G8R8A8_UNORM
    out.extend_from_slice(&3u32.to_le_bytes()); // resource_dimension = TEXTURE2D
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flag
    out.extend_from_slice(&3u32.to_le_bytes()); // array_size = 3
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flags2
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert_eq!(img.array_size, 3);
    assert_eq!(img.surfaces.len(), 3);
    assert!(!img.is_cubemap);
    for (i, sb) in [0xa0u8, 0xb0, 0xc0].iter().enumerate() {
        assert_eq!(img.surfaces[i].array_slice, i as u32);
        assert_eq!(img.surfaces[i].mip_level, 0);
        assert!(img.surfaces[i].face.is_none());
        assert!(img.surfaces[i].plane.data.iter().all(|b| b == sb));
    }
}

#[test]
fn dx10_cubemap_misc_flag() {
    // DX10 cubemap: misc_flag = TEXTURECUBE. array_size in DX10 is
    // the number of CUBE arrays — for a single cubemap the on-disk
    // surface count is 6 × 1 = 6.
    let mut data = Vec::new();
    for face_byte in [0x11u8, 0x22, 0x33, 0x44, 0x55, 0x66] {
        data.extend(std::iter::repeat(face_byte).take(2 * 2 * 4));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&2u32.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_dx10(&mut out);
    out.extend_from_slice(&0x1000u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&87u32.to_le_bytes()); // B8G8R8A8_UNORM
    out.extend_from_slice(&3u32.to_le_bytes());
    out.extend_from_slice(&DDS_RESOURCE_MISC_TEXTURECUBE.to_le_bytes());
    out.extend_from_slice(&1u32.to_le_bytes()); // 1 cube array
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert!(img.is_cubemap);
    assert_eq!(img.surfaces.len(), 6);
    let order: Vec<CubemapFace> = img.surfaces.iter().map(|s| s.face.unwrap()).collect();
    assert_eq!(order, CubemapFace::ALL.to_vec());
}

#[test]
fn cubemap_with_mipmaps_legacy() {
    // Legacy cubemap, 4×4 with 3 mip levels. Each face: 4x4, 2x2,
    // 1x1 DXT1 = 8 + 8 + 8 = 24 bytes (every BC1 surface rounds up
    // to 4×4 → 8 bytes). Total = 6 × 24 = 144 bytes.
    let face_bytes = 8 + 8 + 8;
    let mut data = Vec::new();
    for face_byte in [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06] {
        data.extend(std::iter::repeat(face_byte).take(face_bytes));
    }
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&(DDSD_REQUIRED | DDSD_MIPMAPCOUNT).to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&8u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&3u32.to_le_bytes()); // mip_map_count = 3
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_dxt1(&mut out);
    out.extend_from_slice(&0x401008u32.to_le_bytes());
    out.extend_from_slice(&(DDSCAPS2_CUBEMAP | DDSCAPS2_CUBEMAP_ALL_FACES).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert_eq!(img.surfaces.len(), 6 * 3);

    // Outer loop = face, inner loop = mip.
    for (face_idx, fb) in [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06].iter().enumerate() {
        for mi in 0..3 {
            let s = &img.surfaces[face_idx * 3 + mi];
            assert_eq!(s.face, Some(CubemapFace::ALL[face_idx]));
            assert_eq!(s.mip_level, mi as u32);
            assert!(s.plane.data.iter().all(|b| b == fb));
        }
    }
}

#[test]
fn no_mipmaps_no_cubemap_yields_single_surface() {
    // Build a 4x4 A8R8G8B8 file with no mips and no cubemap.
    let data = vec![0x11u8; 4 * 4 * 4];
    let mut out = Vec::new();
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&4u32.to_le_bytes());
    out.extend_from_slice(&16u32.to_le_bytes()); // pitch
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    push_pixel_format_a8r8g8b8(&mut out);
    out.extend_from_slice(&0x1000u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&data);

    let img = parse_dds(&out).unwrap();
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].mip_level, 0);
    assert_eq!(img.surfaces[0].array_slice, 0);
    assert!(img.surfaces[0].face.is_none());
    assert_eq!(img.surfaces[0].plane.data, img.planes[0].data);
    assert_eq!(img.pixel_format, DdsPixelFormat::A8R8G8B8);
}
