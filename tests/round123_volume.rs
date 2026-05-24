//! Round 123: volume (3D) texture support.
//!
//! Exercises the new volume-texture decode path (legacy `DDSCAPS2_VOLUME`
//! with `DDSD_DEPTH`, plus DX10 `DDS_DIMENSION_TEXTURE3D` headers) and
//! the `encode_dds_volume` writer, including the per-mip depth-halving
//! rule Microsoft mandates for 3D textures.
//!
//! Reference: Microsoft's public "DDS file layout for textures" and
//! "Volume textures" guidance on learn.microsoft.com. No DirectXTex,
//! D3DX, NVTT, or squish source consulted.

use oxideav_dds::{
    encode_dds_volume, parse_dds, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface, DxgiFormat,
};

// Header field constants (kept local so the test asserts the on-disk
// layout independently of the crate's `types` module).
const DDS_MAGIC: u32 = 0x2053_4444;
const DDSD_CAPS: u32 = 0x0000_0001;
const DDSD_HEIGHT: u32 = 0x0000_0002;
const DDSD_WIDTH: u32 = 0x0000_0004;
const DDSD_PIXELFORMAT: u32 = 0x0000_1000;
const DDSD_DEPTH: u32 = 0x0080_0000;
const DDSD_MIPMAPCOUNT: u32 = 0x0002_0000;
const DDPF_RGB: u32 = 0x0000_0040;
const DDPF_ALPHAPIXELS: u32 = 0x0000_0001;
const DDPF_FOURCC: u32 = 0x0000_0004;
const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
const DDSCAPS_COMPLEX: u32 = 0x0000_0008;
const DDSCAPS2_VOLUME: u32 = 0x0020_0000;
const FOURCC_DX10: u32 = u32::from_le_bytes(*b"DX10");
const DDS_DIMENSION_TEXTURE3D: u32 = 4;

/// Build a synthetic legacy A8B8G8R8 (RGBA-on-disk) volume DDS by hand.
/// `mip_depths` is the per-mip slice count; each slice is filled with a
/// distinct constant byte so the decode test can verify slice ordering.
fn build_legacy_volume(width: u32, height: u32, depth: u32, mip_count: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());

    push(&mut out, DDS_MAGIC);
    push(&mut out, 124); // DDS_HEADER.size
    let with_mips = mip_count > 1;
    let flags = DDSD_CAPS
        | DDSD_HEIGHT
        | DDSD_WIDTH
        | DDSD_PIXELFORMAT
        | DDSD_DEPTH
        | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    push(&mut out, flags);
    push(&mut out, height);
    push(&mut out, width);
    push(&mut out, width * 4); // pitch
    push(&mut out, depth);
    push(&mut out, mip_count);
    for _ in 0..11 {
        push(&mut out, 0); // reserved1
    }
    // DDS_PIXELFORMAT — A8B8G8R8 (RGBA on disk).
    push(&mut out, 32); // pixel format size
    push(&mut out, DDPF_RGB | DDPF_ALPHAPIXELS);
    push(&mut out, 0); // four_cc
    push(&mut out, 32); // rgb bit count
    push(&mut out, 0x0000_00ff); // R
    push(&mut out, 0x0000_ff00); // G
    push(&mut out, 0x00ff_0000); // B
    push(&mut out, 0xff00_0000); // A
    let mut caps = DDSCAPS_TEXTURE | DDSCAPS_COMPLEX;
    if with_mips {
        caps |= 0x0040_0000; // DDSCAPS_MIPMAP
    }
    push(&mut out, caps);
    push(&mut out, DDSCAPS2_VOLUME);
    push(&mut out, 0); // caps3
    push(&mut out, 0); // caps4
    push(&mut out, 0); // reserved2

    // Pixel payload: mip-major, depth-shrinking. Each 2D slice is filled
    // with a unique running counter so the test can verify ordering.
    let mut tag: u8 = 0;
    for m in 0..mip_count {
        let mw = (width >> m).max(1);
        let mh = (height >> m).max(1);
        let md = (depth >> m).max(1);
        for _z in 0..md {
            out.extend(vec![tag; (mw * mh * 4) as usize]);
            tag = tag.wrapping_add(1);
        }
    }
    out
}

/// Build the same shape, but with a DX10 extension header declaring a
/// 3D resource dimension and DXGI `R8G8B8A8_UNORM`.
fn build_dx10_volume(width: u32, height: u32, depth: u32) -> Vec<u8> {
    let mut out = Vec::new();
    let push = |out: &mut Vec<u8>, v: u32| out.extend_from_slice(&v.to_le_bytes());

    push(&mut out, DDS_MAGIC);
    push(&mut out, 124);
    push(
        &mut out,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_DEPTH,
    );
    push(&mut out, height);
    push(&mut out, width);
    push(&mut out, width * 4);
    push(&mut out, depth);
    push(&mut out, 1); // mip count
    for _ in 0..11 {
        push(&mut out, 0);
    }
    // DDS_PIXELFORMAT → DX10 extension.
    push(&mut out, 32);
    push(&mut out, DDPF_FOURCC);
    push(&mut out, FOURCC_DX10);
    push(&mut out, 0);
    push(&mut out, 0);
    push(&mut out, 0);
    push(&mut out, 0);
    push(&mut out, 0);
    push(&mut out, DDSCAPS_TEXTURE | DDSCAPS_COMPLEX);
    push(&mut out, DDSCAPS2_VOLUME);
    push(&mut out, 0);
    push(&mut out, 0);
    push(&mut out, 0);
    // DDS_HEADER_DXT10.
    push(&mut out, DxgiFormat::R8G8B8A8Unorm.to_u32());
    push(&mut out, DDS_DIMENSION_TEXTURE3D);
    push(&mut out, 0); // misc_flag
    push(&mut out, 1); // array_size
    push(&mut out, 0); // misc_flags2

    let mut tag: u8 = 10;
    for _z in 0..depth {
        out.extend(vec![tag; (width * height * 4) as usize]);
        tag = tag.wrapping_add(1);
    }
    out
}

#[test]
fn decode_legacy_volume_single_mip() {
    // 4×4×4 volume, no mips → 4 depth slices.
    let bytes = build_legacy_volume(4, 4, 4, 1);
    let img = parse_dds(&bytes).expect("parse legacy volume");

    assert_eq!(img.width, 4);
    assert_eq!(img.height, 4);
    assert_eq!(img.depth, 4);
    assert_eq!(img.mip_map_count, 1);
    assert_eq!(img.array_size, 1);
    assert!(!img.is_cubemap);
    assert_eq!(img.pixel_format, DdsPixelFormat::A8B8G8R8);
    assert_eq!(img.surfaces.len(), 4);

    // Slice indices run 0..4 at mip 0, each filled with its own tag.
    for (z, s) in img.surfaces.iter().enumerate() {
        assert_eq!(s.mip_level, 0);
        assert_eq!(s.depth_slice, z as u32);
        assert_eq!(s.width, 4);
        assert_eq!(s.height, 4);
        assert_eq!(s.face, None);
        assert!(s.plane.data.iter().all(|&b| b == z as u8));
    }
}

#[test]
fn decode_legacy_volume_with_mips_depth_halving() {
    // 4×4×4 with 3 mips → mip0: 4 slices (4×4), mip1: 2 slices (2×2),
    // mip2: 1 slice (1×1). Total 7 surfaces, mip-major order.
    let bytes = build_legacy_volume(4, 4, 4, 3);
    let img = parse_dds(&bytes).expect("parse mipped volume");

    assert_eq!(img.depth, 4);
    assert_eq!(img.mip_map_count, 3);
    assert_eq!(img.surfaces.len(), 4 + 2 + 1);

    // Verify the (mip, depth_slice, dims) progression.
    let expected: &[(u32, u32, u32, u32)] = &[
        (0, 0, 4, 4),
        (0, 1, 4, 4),
        (0, 2, 4, 4),
        (0, 3, 4, 4),
        (1, 0, 2, 2),
        (1, 1, 2, 2),
        (2, 0, 1, 1),
    ];
    assert_eq!(img.surfaces.len(), expected.len());
    for (s, &(ml, ds, w, h)) in img.surfaces.iter().zip(expected) {
        assert_eq!(s.mip_level, ml, "mip mismatch");
        assert_eq!(s.depth_slice, ds, "slice mismatch");
        assert_eq!((s.width, s.height), (w, h), "dim mismatch");
    }

    // Tags run 0..7 in the same on-disk order.
    for (i, s) in img.surfaces.iter().enumerate() {
        assert!(
            s.plane.data.iter().all(|&b| b == i as u8),
            "surface {i} not uniformly tagged"
        );
    }
}

#[test]
fn decode_dx10_volume() {
    let bytes = build_dx10_volume(2, 2, 3);
    let img = parse_dds(&bytes).expect("parse dx10 volume");

    assert!(img.has_dxt10_header);
    assert_eq!(img.depth, 3);
    assert_eq!(img.dxgi_format, Some(DxgiFormat::R8G8B8A8Unorm));
    // R8G8B8A8_UNORM maps to crate-local A8B8G8R8.
    assert_eq!(img.pixel_format, DdsPixelFormat::A8B8G8R8);
    assert_eq!(img.surfaces.len(), 3);
    for (z, s) in img.surfaces.iter().enumerate() {
        assert_eq!(s.depth_slice, z as u32);
        assert!(s.plane.data.iter().all(|&b| b == (10 + z) as u8));
    }
}

#[test]
fn decode_volume_truncated_payload_errors() {
    // Lop the last slice off a 4×4×4 single-mip volume.
    let mut bytes = build_legacy_volume(4, 4, 4, 1);
    bytes.truncate(bytes.len() - 4 * 4 * 4);
    let err = parse_dds(&bytes).expect_err("truncated volume must error");
    let msg = format!("{err}");
    assert!(msg.contains("truncated"), "unexpected error: {msg}");
}

/// Helper: synthesise per-(mip, slice) uncompressed surfaces for a
/// volume, each slice filled with a distinct tag byte.
fn make_volume_surfaces(
    width: u32,
    height: u32,
    depth: u32,
    mip_count: u32,
    pix: DdsPixelFormat,
) -> Vec<DdsSurface> {
    let bpp = pix.bytes_per_pixel().unwrap();
    let mut surfaces = Vec::new();
    let mut tag: u8 = 0;
    for m in 0..mip_count {
        let mw = (width >> m).max(1);
        let mh = (height >> m).max(1);
        let md = (depth >> m).max(1);
        for z in 0..md {
            let data = vec![tag; (mw * mh * bpp) as usize];
            surfaces.push(DdsSurface {
                width: mw,
                height: mh,
                mip_level: m,
                array_slice: 0,
                face: None,
                depth_slice: z,
                plane: DdsPlane {
                    stride: (mw * bpp) as usize,
                    data,
                },
            });
            tag = tag.wrapping_add(1);
        }
    }
    surfaces
}

#[test]
fn roundtrip_volume_single_mip() {
    let pix = DdsPixelFormat::A8B8G8R8;
    let surfaces = make_volume_surfaces(4, 4, 4, 1, pix);
    let img = DdsImage {
        width: 4,
        height: 4,
        pixel_format: pix,
        planes: vec![surfaces[0].plane.clone()],
        surfaces: surfaces.clone(),
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 4,
    };

    let bytes = encode_dds_volume(&img).expect("encode volume");
    let decoded = parse_dds(&bytes).expect("re-parse volume");

    assert_eq!(decoded.width, 4);
    assert_eq!(decoded.height, 4);
    assert_eq!(decoded.depth, 4);
    assert_eq!(decoded.surfaces.len(), surfaces.len());
    for (orig, got) in surfaces.iter().zip(&decoded.surfaces) {
        assert_eq!(orig.mip_level, got.mip_level);
        assert_eq!(orig.depth_slice, got.depth_slice);
        assert_eq!(orig.width, got.width);
        assert_eq!(orig.height, got.height);
        assert_eq!(orig.plane.data, got.plane.data);
    }
}

#[test]
fn roundtrip_volume_with_mips() {
    // 8×8×8 with 4 mips: depths 8,4,2,1; dims 8,4,2,1.
    let pix = DdsPixelFormat::A8B8G8R8;
    let surfaces = make_volume_surfaces(8, 8, 8, 4, pix);
    // Sanity: total slices = 8 + 4 + 2 + 1 = 15.
    assert_eq!(surfaces.len(), 15);

    let img = DdsImage {
        width: 8,
        height: 8,
        pixel_format: pix,
        planes: vec![surfaces[0].plane.clone()],
        surfaces: surfaces.clone(),
        pts: None,
        mip_map_count: 4,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 8,
    };

    let bytes = encode_dds_volume(&img).expect("encode mipped volume");
    let decoded = parse_dds(&bytes).expect("re-parse mipped volume");

    assert_eq!(decoded.mip_map_count, 4);
    assert_eq!(decoded.depth, 8);
    assert_eq!(decoded.surfaces.len(), 15);
    for (orig, got) in surfaces.iter().zip(&decoded.surfaces) {
        assert_eq!(
            (orig.mip_level, orig.depth_slice, orig.width, orig.height),
            (got.mip_level, got.depth_slice, got.width, got.height)
        );
        assert_eq!(orig.plane.data, got.plane.data);
    }
}

#[test]
fn encode_volume_rejects_depth_one() {
    let pix = DdsPixelFormat::A8B8G8R8;
    let surfaces = make_volume_surfaces(4, 4, 1, 1, pix);
    let img = DdsImage {
        width: 4,
        height: 4,
        pixel_format: pix,
        planes: vec![surfaces[0].plane.clone()],
        surfaces,
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    assert!(
        encode_dds_volume(&img).is_err(),
        "depth==1 should be rejected by encode_dds_volume"
    );
}
