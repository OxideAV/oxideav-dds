//! Round 375: uncompressed cubemap / texture-array encode round-trips.
//!
//! `encode_dds_uncompressed_cubemap_array` writes a cubemap or DX10
//! texture array from a pre-populated `DdsImage::surfaces` list (in the
//! mandated slice → face → mip order). Legacy-mask single cubemaps use
//! the legacy header with all six face-presence bits; texture arrays and
//! DX10-only formats use the `DDS_HEADER_DXT10` extension. Each path
//! round-trips byte-for-byte through `parse_dds`.
//!
//! Reference: Microsoft's public "DDS file layout for cubic environment
//! maps", "DDS header (DXT10 extension)", and `DXGI_FORMAT` pages on
//! learn.microsoft.com. No external library source consulted.

use oxideav_dds::{
    encode_dds_uncompressed_cubemap_array, parse_dds, CubemapFace, DdsImage, DdsPixelFormat,
    DdsPlane, DdsSurface, DxgiFormat,
};

/// Build per-(slice, face, mip) surfaces, each filled with a distinct
/// running tag byte so ordering can be verified on read.
fn build_surfaces(
    width: u32,
    height: u32,
    pix: DdsPixelFormat,
    is_cubemap: bool,
    array_size: u32,
    mip_count: u32,
) -> Vec<DdsSurface> {
    let bpp = pix.bytes_per_pixel().unwrap();
    let face_count = if is_cubemap { 6 } else { 1 };
    let mut surfaces = Vec::new();
    let mut tag: u8 = 0;
    for slice in 0..array_size.max(1) {
        for f in 0..face_count {
            let face = if is_cubemap {
                Some(CubemapFace::ALL[f as usize])
            } else {
                None
            };
            for m in 0..mip_count {
                let mw = (width >> m).max(1);
                let mh = (height >> m).max(1);
                let data = vec![tag; (mw * mh * bpp) as usize];
                surfaces.push(DdsSurface {
                    width: mw,
                    height: mh,
                    mip_level: m,
                    array_slice: slice,
                    face,
                    depth_slice: 0,
                    plane: DdsPlane {
                        stride: (mw * bpp) as usize,
                        data,
                    },
                });
                tag = tag.wrapping_add(1);
            }
        }
    }
    surfaces
}

fn make_image(
    width: u32,
    height: u32,
    pix: DdsPixelFormat,
    is_cubemap: bool,
    array_size: u32,
    mip_count: u32,
) -> DdsImage {
    let surfaces = build_surfaces(width, height, pix, is_cubemap, array_size, mip_count);
    DdsImage {
        width,
        height,
        pixel_format: pix,
        planes: vec![surfaces[0].plane.clone()],
        surfaces,
        pts: None,
        mip_map_count: mip_count,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap,
        array_size,
        depth: 1,
    }
}

#[test]
fn roundtrip_legacy_cubemap_single_mip() {
    let pix = DdsPixelFormat::A8R8G8B8;
    let img = make_image(4, 4, pix, true, 1, 1);
    let orig = img.surfaces.clone();

    let bytes = encode_dds_uncompressed_cubemap_array(&img).expect("encode cubemap");
    let decoded = parse_dds(&bytes).expect("re-parse cubemap");

    assert!(decoded.is_cubemap);
    assert!(!decoded.has_dxt10_header, "legacy cubemap must not be DX10");
    assert_eq!(decoded.surfaces.len(), 6);
    for (o, g) in orig.iter().zip(&decoded.surfaces) {
        assert_eq!(o.face, g.face);
        assert_eq!((o.width, o.height), (g.width, g.height));
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn roundtrip_legacy_cubemap_with_mips() {
    // 8x8 cubemap, 4 mips → 6 faces × 4 mips = 24 surfaces.
    let pix = DdsPixelFormat::A8R8G8B8;
    let img = make_image(8, 8, pix, true, 1, 4);
    let orig = img.surfaces.clone();
    assert_eq!(orig.len(), 24);

    let bytes = encode_dds_uncompressed_cubemap_array(&img).expect("encode mipped cubemap");
    let decoded = parse_dds(&bytes).expect("re-parse mipped cubemap");

    assert_eq!(decoded.mip_map_count, 4);
    assert_eq!(decoded.surfaces.len(), 24);
    for (o, g) in orig.iter().zip(&decoded.surfaces) {
        assert_eq!(
            (o.mip_level, o.face, o.width, o.height),
            (g.mip_level, g.face, g.width, g.height)
        );
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn roundtrip_dx10_texture_array() {
    // 4x4 R8G8B8A8 array of 3 slices, single mip. array_size > 1 forces
    // the DX10 extension header.
    let pix = DdsPixelFormat::A8B8G8R8;
    let img = make_image(4, 4, pix, false, 3, 1);
    let orig = img.surfaces.clone();

    let bytes = encode_dds_uncompressed_cubemap_array(&img).expect("encode array");
    let decoded = parse_dds(&bytes).expect("re-parse array");

    assert!(decoded.has_dxt10_header, "array must use DX10 header");
    assert_eq!(decoded.array_size, 3);
    assert!(!decoded.is_cubemap);
    assert_eq!(decoded.surfaces.len(), 3);
    for (o, g) in orig.iter().zip(&decoded.surfaces) {
        assert_eq!(o.array_slice, g.array_slice);
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn roundtrip_dx10_only_format_cubemap() {
    // A DX10-only format (R16G16B16A16_FLOAT) cubemap → DX10 header with
    // TEXTURECUBE misc flag.
    let pix = DdsPixelFormat::R16G16B16A16Float;
    let img = make_image(4, 4, pix, true, 1, 1);
    let orig = img.surfaces.clone();

    let bytes = encode_dds_uncompressed_cubemap_array(&img).expect("encode dx10 cubemap");
    let decoded = parse_dds(&bytes).expect("re-parse dx10 cubemap");

    assert!(decoded.has_dxt10_header);
    assert!(decoded.is_cubemap);
    assert_eq!(decoded.dxgi_format, Some(DxgiFormat::R16G16B16A16Float));
    assert_eq!(decoded.pixel_format, pix);
    assert_eq!(decoded.surfaces.len(), 6);
    for (o, g) in orig.iter().zip(&decoded.surfaces) {
        assert_eq!(o.face, g.face);
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn roundtrip_cube_array() {
    // Cube array: 2 cubes × 6 faces × 1 mip = 12 surfaces.
    let pix = DdsPixelFormat::A8B8G8R8;
    let img = make_image(4, 4, pix, true, 2, 1);
    let orig = img.surfaces.clone();
    assert_eq!(orig.len(), 12);

    let bytes = encode_dds_uncompressed_cubemap_array(&img).expect("encode cube array");
    let decoded = parse_dds(&bytes).expect("re-parse cube array");

    assert!(decoded.has_dxt10_header);
    assert!(decoded.is_cubemap);
    assert_eq!(decoded.array_size, 2);
    assert_eq!(decoded.surfaces.len(), 12);
    for (o, g) in orig.iter().zip(&decoded.surfaces) {
        assert_eq!((o.array_slice, o.face), (g.array_slice, g.face));
        assert_eq!(o.plane.data, g.plane.data);
    }
}

#[test]
fn rejects_plain_2d() {
    let pix = DdsPixelFormat::A8R8G8B8;
    let img = make_image(4, 4, pix, false, 1, 1);
    assert!(
        encode_dds_uncompressed_cubemap_array(&img).is_err(),
        "plain 2D should be rejected (use encode_dds_uncompressed)"
    );
}

#[test]
fn rejects_block_compressed() {
    let mut img = make_image(4, 4, DdsPixelFormat::A8R8G8B8, true, 1, 1);
    img.pixel_format = DdsPixelFormat::Bc1;
    assert!(
        encode_dds_uncompressed_cubemap_array(&img).is_err(),
        "BC format must be rejected"
    );
}
