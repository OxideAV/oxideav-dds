//! Round 375: DX10-header uncompressed encode round-trips.
//!
//! `encode_dds_uncompressed_dx10` writes the high-bit-depth /
//! floating-point / packed-HDR / plain-integer / normalised
//! single-/dual-channel / depth uncompressed formats that have no legacy
//! `DDS_PIXELFORMAT` mask layout. Each of these formats stores its
//! little-endian channels verbatim on disk, so a byte-identical
//! round-trip through `parse_dds` is the correctness contract.
//!
//! Reference: Microsoft's public "DDS header (DXT10 extension)" and the
//! `DXGI_FORMAT` enumeration pages on learn.microsoft.com. No external
//! library source consulted.

use oxideav_dds::{
    encode_dds_uncompressed_dx10, parse_dds, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface,
    DxgiFormat,
};

/// Build a single-plane DX10 image with a pseudo-random byte payload.
fn make_image(width: u32, height: u32, pix: DdsPixelFormat, mip_map_count: u32) -> DdsImage {
    let bpp = pix.bytes_per_pixel().unwrap();
    let n = (width * height * bpp) as usize;
    // Deterministic non-trivial byte fill (every byte distinct mod 251).
    let data: Vec<u8> = (0..n).map(|i| ((i * 37 + 11) % 251) as u8).collect();
    DdsImage {
        width,
        height,
        pixel_format: pix,
        planes: vec![DdsPlane {
            stride: (width * bpp) as usize,
            data,
        }],
        surfaces: Vec::new(),
        pts: None,
        mip_map_count,
        has_dxt10_header: true,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    }
}

/// Round-trip a single-mip DX10 uncompressed surface and assert the
/// payload bytes survive verbatim and the format / DXGI code match.
fn roundtrip_one(pix: DdsPixelFormat, expect_dxgi: DxgiFormat) {
    let img = make_image(6, 4, pix, 1);
    let orig = img.planes[0].data.clone();

    let bytes = encode_dds_uncompressed_dx10(&img).expect("encode dx10 uncompressed");
    let decoded = parse_dds(&bytes).expect("re-parse dx10 uncompressed");

    assert_eq!(decoded.width, 6);
    assert_eq!(decoded.height, 4);
    assert!(decoded.has_dxt10_header, "{} should be DX10", pix.name());
    assert_eq!(
        decoded.dxgi_format,
        Some(expect_dxgi),
        "{} DXGI mismatch",
        pix.name()
    );
    assert_eq!(decoded.pixel_format, pix, "{} format mismatch", pix.name());
    assert_eq!(decoded.surfaces.len(), 1);
    assert_eq!(
        decoded.surfaces[0].plane.data,
        orig,
        "{} payload not byte-identical",
        pix.name()
    );
}

#[test]
fn roundtrip_float_formats() {
    roundtrip_one(DdsPixelFormat::R16Float, DxgiFormat::R16Float);
    roundtrip_one(DdsPixelFormat::R16G16Float, DxgiFormat::R16G16Float);
    roundtrip_one(
        DdsPixelFormat::R16G16B16A16Float,
        DxgiFormat::R16G16B16A16Float,
    );
    roundtrip_one(DdsPixelFormat::R32Float, DxgiFormat::R32Float);
    roundtrip_one(DdsPixelFormat::R32G32Float, DxgiFormat::R32G32Float);
    roundtrip_one(
        DdsPixelFormat::R32G32B32A32Float,
        DxgiFormat::R32G32B32A32Float,
    );
}

#[test]
fn roundtrip_high_bit_depth_norm() {
    roundtrip_one(
        DdsPixelFormat::R16G16B16A16Unorm,
        DxgiFormat::R16G16B16A16Unorm,
    );
    roundtrip_one(
        DdsPixelFormat::R16G16B16A16Snorm,
        DxgiFormat::R16G16B16A16Snorm,
    );
}

#[test]
fn roundtrip_packed_hdr() {
    roundtrip_one(
        DdsPixelFormat::R10G10B10A2Unorm,
        DxgiFormat::R10G10B10A2Unorm,
    );
    roundtrip_one(DdsPixelFormat::R10G10B10A2Uint, DxgiFormat::R10G10B10A2Uint);
    roundtrip_one(DdsPixelFormat::R8G8B8G8Unorm, DxgiFormat::R8G8B8G8Unorm);
    roundtrip_one(DdsPixelFormat::G8R8G8B8Unorm, DxgiFormat::G8R8G8B8Unorm);
}

#[test]
fn roundtrip_integer_formats() {
    roundtrip_one(DdsPixelFormat::R8Uint, DxgiFormat::R8Uint);
    roundtrip_one(DdsPixelFormat::R8Sint, DxgiFormat::R8Sint);
    roundtrip_one(DdsPixelFormat::R8G8Uint, DxgiFormat::R8G8Uint);
    roundtrip_one(DdsPixelFormat::R8G8Sint, DxgiFormat::R8G8Sint);
    roundtrip_one(DdsPixelFormat::R8G8B8A8Uint, DxgiFormat::R8G8B8A8Uint);
    roundtrip_one(DdsPixelFormat::R8G8B8A8Sint, DxgiFormat::R8G8B8A8Sint);
    roundtrip_one(DdsPixelFormat::R16Uint, DxgiFormat::R16Uint);
    roundtrip_one(DdsPixelFormat::R16Sint, DxgiFormat::R16Sint);
    roundtrip_one(DdsPixelFormat::R16G16Uint, DxgiFormat::R16G16Uint);
    roundtrip_one(DdsPixelFormat::R16G16Sint, DxgiFormat::R16G16Sint);
    roundtrip_one(
        DdsPixelFormat::R16G16B16A16Uint,
        DxgiFormat::R16G16B16A16Uint,
    );
    roundtrip_one(
        DdsPixelFormat::R16G16B16A16Sint,
        DxgiFormat::R16G16B16A16Sint,
    );
    roundtrip_one(DdsPixelFormat::R32Uint, DxgiFormat::R32Uint);
    roundtrip_one(DdsPixelFormat::R32Sint, DxgiFormat::R32Sint);
    roundtrip_one(DdsPixelFormat::R32G32Uint, DxgiFormat::R32G32Uint);
    roundtrip_one(DdsPixelFormat::R32G32Sint, DxgiFormat::R32G32Sint);
    roundtrip_one(DdsPixelFormat::R32G32B32Uint, DxgiFormat::R32G32B32Uint);
    roundtrip_one(DdsPixelFormat::R32G32B32Sint, DxgiFormat::R32G32B32Sint);
    roundtrip_one(
        DdsPixelFormat::R32G32B32A32Uint,
        DxgiFormat::R32G32B32A32Uint,
    );
    roundtrip_one(
        DdsPixelFormat::R32G32B32A32Sint,
        DxgiFormat::R32G32B32A32Sint,
    );
}

#[test]
fn roundtrip_normalised_small_channel() {
    roundtrip_one(DdsPixelFormat::R8Snorm, DxgiFormat::R8Snorm);
    roundtrip_one(DdsPixelFormat::R8G8Snorm, DxgiFormat::R8G8Snorm);
    roundtrip_one(DdsPixelFormat::R8G8B8A8Snorm, DxgiFormat::R8G8B8A8Snorm);
    roundtrip_one(DdsPixelFormat::R16Unorm, DxgiFormat::R16Unorm);
    roundtrip_one(DdsPixelFormat::R16Snorm, DxgiFormat::R16Snorm);
    roundtrip_one(DdsPixelFormat::R16G16Unorm, DxgiFormat::R16G16Unorm);
    roundtrip_one(DdsPixelFormat::R16G16Snorm, DxgiFormat::R16G16Snorm);
}

#[test]
fn roundtrip_depth_formats() {
    roundtrip_one(DdsPixelFormat::D16Unorm, DxgiFormat::D16Unorm);
    roundtrip_one(DdsPixelFormat::D32Float, DxgiFormat::D32Float);
    roundtrip_one(DdsPixelFormat::D24UnormS8Uint, DxgiFormat::D24UnormS8Uint);
    roundtrip_one(
        DdsPixelFormat::D32FloatS8X24Uint,
        DxgiFormat::D32FloatS8X24Uint,
    );
}

#[test]
fn roundtrip_with_supplied_mip_chain() {
    // 8x8 R16G16B16A16_FLOAT (8 bpp) with 4 mips: 8,4,2,1. Supply each
    // level explicitly so the byte-domain box filter is not invoked
    // (these are >8-bit channels).
    let pix = DdsPixelFormat::R16G16B16A16Float;
    let bpp = pix.bytes_per_pixel().unwrap();
    let dims = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
    let mut surfaces = Vec::new();
    let mut tag: u8 = 1;
    for (level, &(w, h)) in dims.iter().enumerate() {
        let data = vec![tag; (w * h * bpp) as usize];
        surfaces.push(DdsSurface {
            width: w,
            height: h,
            mip_level: level as u32,
            array_slice: 0,
            face: None,
            depth_slice: 0,
            plane: DdsPlane {
                stride: (w * bpp) as usize,
                data,
            },
        });
        tag = tag.wrapping_add(1);
    }
    let img = DdsImage {
        width: 8,
        height: 8,
        pixel_format: pix,
        planes: vec![surfaces[0].plane.clone()],
        surfaces: surfaces.clone(),
        pts: None,
        mip_map_count: 4,
        has_dxt10_header: true,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };

    let bytes = encode_dds_uncompressed_dx10(&img).expect("encode mipped dx10");
    let decoded = parse_dds(&bytes).expect("re-parse mipped dx10");

    assert_eq!(decoded.mip_map_count, 4);
    assert_eq!(decoded.surfaces.len(), 4);
    for (o, g) in surfaces.iter().zip(&decoded.surfaces) {
        assert_eq!((o.width, o.height), (g.width, g.height));
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn rejects_legacy_mask_format() {
    // A8R8G8B8 has a legacy mask layout; the DX10 encoder must defer.
    let img = make_image(4, 4, DdsPixelFormat::A8R8G8B8, 1);
    assert!(
        encode_dds_uncompressed_dx10(&img).is_err(),
        "legacy mask format must be rejected by the DX10 encoder"
    );
}

#[test]
fn rejects_block_compressed() {
    let img = DdsImage {
        width: 4,
        height: 4,
        pixel_format: DdsPixelFormat::Bc1,
        planes: vec![DdsPlane {
            stride: 8,
            data: vec![0u8; 8],
        }],
        surfaces: Vec::new(),
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: true,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    assert!(
        encode_dds_uncompressed_dx10(&img).is_err(),
        "BC format must be rejected by the DX10 uncompressed encoder"
    );
}

#[test]
fn dxgi_format_override_is_honoured() {
    // Setting image.dxgi_format overrides the canonical code (used to
    // preserve a specific typeless / srgb variant on round-trip).
    let mut img = make_image(4, 4, DdsPixelFormat::R8G8B8A8Uint, 1);
    img.dxgi_format = Some(DxgiFormat::R8G8B8A8Uint);
    let bytes = encode_dds_uncompressed_dx10(&img).expect("encode");
    let decoded = parse_dds(&bytes).expect("parse");
    assert_eq!(decoded.dxgi_format, Some(DxgiFormat::R8G8B8A8Uint));
}
