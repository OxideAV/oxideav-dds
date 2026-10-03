//! The image-crate contract surface (`IMAGE_CRATE_API.md`): root
//! vocabulary, native-layout round trips, multi-surface
//! `decode_all` / `encode_all`, limits, and byte parity between the
//! contract writer and the pre-contract depth writers.

use oxideav_dds::{
    decode, decode_all, decode_all_with, decode_from, decode_rgb8, decode_rgba8, decode_with,
    encode, encode_all, encode_dds_astc, encode_dds_block_compressed_from_rgba8,
    encode_dds_uncompressed, encode_dds_uncompressed_cubemap_array, encode_dds_uncompressed_dx10,
    encode_rgb8, encode_rgba8, encode_to, info, parse_dds, probe, ColorInfo, CubemapFace, DdsError,
    DdsFile, DdsImage, DecodeOptions, EncodeOptions, Frame, PixelFormat, Plane, SurfaceFormat,
};

const RED16: &[u8] = include_bytes!("fixtures/red16.dds");
const GRAD8: &[u8] = include_bytes!("fixtures/grad8.dds");

fn pattern(len: usize, seed: u32) -> Vec<u8> {
    let mut s = seed;
    (0..len)
        .map(|_| {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            (s >> 24) as u8
        })
        .collect()
}

fn image(w: u32, h: u32, f: PixelFormat, seed: u32) -> DdsImage {
    let mut data = pattern(w as usize * h as usize * f.bytes_per_pixel(), seed);
    if f.is_float() {
        // Finite, in-range floats so the float paths stay exact.
        for (i, c) in data.chunks_exact_mut(4).enumerate() {
            c.copy_from_slice(&((i % 97) as f32 / 97.0).to_le_bytes());
        }
    }
    DdsImage::new(
        w,
        h,
        f,
        vec![Plane::new(w as usize * f.bytes_per_pixel(), data)],
    )
    .unwrap()
}

#[test]
fn probe_is_total_and_exact() {
    assert!(!probe(&[]));
    assert!(!probe(b"DDS "));
    assert!(!probe(b"DDS \x7b\0\0\0"));
    assert!(probe(RED16));
    assert!(probe(GRAD8));
    assert!(!probe(&[0xff; 128]));
}

#[test]
fn info_describes_header_only() {
    let i = info(RED16).unwrap();
    assert_eq!((i.width, i.height), (16, 16));
    assert_eq!(i.format, PixelFormat::Rgba);
    assert_eq!(i.surface_format, SurfaceFormat::Bc1);
    assert_eq!(i.frames, 1);
    assert!(i.has_alpha);
    assert!(!i.has_icc && !i.has_exif && !i.has_xmp);
    assert_eq!(i.color, ColorInfo::dds_default());
    assert!(!i.dx10_header);
    // Truncated payload is still rejected by `info` (geometry walk).
    assert!(matches!(
        info(&RED16[..RED16.len() - 1]),
        Err(DdsError::InvalidData(_))
    ));
}

#[test]
fn fixtures_decode_to_rgba() {
    let img = decode(RED16).unwrap();
    assert_eq!(img.format, PixelFormat::Rgba);
    assert!(img
        .to_rgba8()
        .chunks_exact(4)
        .all(|p| p == [255, 0, 0, 255]));
    let rgb = decode_rgb8(GRAD8).unwrap();
    assert_eq!(rgb.data.len(), 8 * 8 * 3);
    assert_eq!(&rgb.data[..3], &[255, 255, 255]);
    let rgba = decode_rgba8(GRAD8).unwrap();
    assert_eq!(rgba.data.len(), 8 * 8 * 4);
    let from_reader = decode_from(std::io::Cursor::new(GRAD8)).unwrap();
    assert_eq!(from_reader, decode(GRAD8).unwrap());
    let frames = decode_all(GRAD8).unwrap();
    assert_eq!(frames.len(), 1);
    assert!(frames[0].delay.is_none());
}

#[test]
fn lossless_round_trip_every_native_layout() {
    for f in [
        PixelFormat::Gray8,
        PixelFormat::Ya8,
        PixelFormat::Gray16Le,
        PixelFormat::Bgr24,
        PixelFormat::Rgba,
        PixelFormat::Bgra,
        PixelFormat::Rgba64Le,
        PixelFormat::GrayF32Le,
        PixelFormat::RgbaF32Le,
    ] {
        let img = image(5, 3, f, 0x1234_5678 ^ f.bytes_per_pixel() as u32);
        let bytes = encode(&img, &EncodeOptions::default()).unwrap();
        assert!(probe(&bytes));
        let i = info(&bytes).unwrap();
        assert_eq!(i.format, f, "{f:?}");
        assert_eq!(i.frames, 1);
        let back = decode(&bytes).unwrap();
        assert_eq!(back, img, "{f:?}");
        let mut via_writer = Vec::new();
        encode_to(&img, &EncodeOptions::default(), &mut via_writer).unwrap();
        assert_eq!(via_writer, bytes);
    }
}

#[test]
fn rgb24_is_stored_as_bgr_and_float_rgb_gains_alpha() {
    let rgb = image(4, 2, PixelFormat::Rgb24, 7);
    let bytes = encode(&rgb, &EncodeOptions::default()).unwrap();
    let back = decode(&bytes).unwrap();
    assert_eq!(back.format, PixelFormat::Bgr24);
    assert_eq!(back.to_rgb8(), rgb.to_rgb8());
    let raw = encode_rgb8(4, 2, rgb.as_bytes().unwrap(), &EncodeOptions::default()).unwrap();
    assert_eq!(raw, bytes);

    let f = image(2, 2, PixelFormat::RgbF32Le, 9);
    let back = decode(&encode(&f, &EncodeOptions::default()).unwrap()).unwrap();
    assert_eq!(back.format, PixelFormat::RgbaF32Le);
    assert_eq!(back.to_rgba8(), f.to_rgba8());
}

#[test]
fn encode_rgba8_round_trips_through_decode_rgba8() {
    let rgba = pattern(6 * 4 * 4, 3);
    let bytes = encode_rgba8(6, 4, &rgba, &EncodeOptions::default()).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(i.surface_format, SurfaceFormat::A8B8G8R8);
    assert!(!i.dx10_header);
    assert_eq!(decode_rgba8(&bytes).unwrap().data, rgba);
    assert!(matches!(
        encode_rgba8(6, 4, &rgba[..10], &EncodeOptions::default()),
        Err(DdsError::InvalidData(_))
    ));
}

#[test]
fn explicit_surface_formats_convert_and_decode_back() {
    let img = image(8, 8, PixelFormat::Rgba, 11);
    for sf in [
        SurfaceFormat::A8R8G8B8,
        SurfaceFormat::X8R8G8B8,
        SurfaceFormat::R5G6B5,
        SurfaceFormat::A1R5G5B5,
        SurfaceFormat::A4R4G4B4,
        SurfaceFormat::A8R3G3B2,
        SurfaceFormat::A4B4G4R4Unorm,
        SurfaceFormat::R10G10B10A2Unorm,
        SurfaceFormat::R16G16B16A16Unorm,
        SurfaceFormat::R16G16B16A16Float,
        SurfaceFormat::R32G32B32A32Float,
        SurfaceFormat::R8G8B8A8Snorm,
        SurfaceFormat::Bc1,
        SurfaceFormat::Bc2,
        SurfaceFormat::Bc3,
        SurfaceFormat::Bc7Unorm,
        SurfaceFormat::Astc {
            block_w: 4,
            block_h: 4,
            srgb: false,
        },
    ] {
        let opts = EncodeOptions::default().with_surface_format(sf);
        let bytes = encode(&img, &opts).unwrap();
        let i = info(&bytes).unwrap();
        assert_eq!(i.surface_format, sf, "{}", sf.name());
        let back = decode(&bytes).unwrap();
        assert_eq!(back.format, sf.contract_format().unwrap(), "{}", sf.name());
        assert_eq!((back.width, back.height), (8, 8));
        // Re-encoding the decoded image in the same stored layout is
        // byte-stable for every integer layout (the block encoders are
        // lossy, so compare their decode instead).
        let again = encode(&back, &opts).unwrap();
        if sf.is_block_compressed() || sf.astc_footprint().is_some() {
            let twice = decode(&again).unwrap();
            assert_eq!(twice.width, 8);
        } else {
            assert_eq!(again, bytes, "{}", sf.name());
        }
    }
}

#[test]
fn srgb_dxgi_code_sets_colour_and_round_trips() {
    let img = image(4, 4, PixelFormat::Rgba, 5);
    let opts = EncodeOptions::default().with_surface_format(SurfaceFormat::Bc7UnormSrgb);
    let bytes = encode(&img, &opts).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(i.color, ColorInfo::srgb());
    assert!(i.dx10_header);
    let back = decode(&bytes).unwrap();
    assert_eq!(back.color, ColorInfo::srgb());
    let plain = decode(&encode(&img, &EncodeOptions::default()).unwrap()).unwrap();
    assert_eq!(plain.color, ColorInfo::dds_default());
}

#[test]
fn unsupported_conversions_are_reported_not_silent() {
    let rgb = image(2, 2, PixelFormat::Rgb24, 1);
    for sf in [
        SurfaceFormat::L8,
        SurfaceFormat::A8,
        SurfaceFormat::R32Uint,
        SurfaceFormat::D16Unorm,
        SurfaceFormat::R11G11B10Float,
    ] {
        assert!(
            matches!(
                encode(&rgb, &EncodeOptions::default().with_surface_format(sf)),
                Err(DdsError::Unsupported(_))
            ),
            "{}",
            sf.name()
        );
    }
    // A UINT file parses with the depth API but has no contract layout.
    let f = DdsFile::single(1, 1, SurfaceFormat::R32Uint, Plane::new(4, vec![0; 4])).unwrap();
    let bytes = oxideav_dds::write_dds_file(&f, false).unwrap();
    assert!(probe(&bytes));
    assert!(matches!(info(&bytes), Err(DdsError::Unsupported(_))));
    assert!(matches!(decode(&bytes), Err(DdsError::Unsupported(_))));
    assert_eq!(
        parse_dds(&bytes).unwrap().pixel_format,
        SurfaceFormat::R32Uint
    );
}

#[test]
fn mip_chain_generation_and_decode_all() {
    let img = image(8, 4, PixelFormat::Bgra, 21);
    let bytes = encode(&img, &EncodeOptions::default().with_mip_levels(0)).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(i.mip_levels, 4);
    assert_eq!(i.frames, 4);
    let frames = decode_all(&bytes).unwrap();
    assert_eq!(frames.len(), 4);
    for (m, f) in frames.iter().enumerate() {
        assert_eq!(f.mip_level, m as u32);
        assert_eq!(f.image.width, (8 >> m).max(1));
        assert_eq!(f.image.height, (4 >> m).max(1));
        assert!(f.face.is_none() && f.array_slice == 0 && f.depth_slice == 0);
    }
    assert_eq!(frames[0].image, img);
    // `decode` is the top level.
    assert_eq!(decode(&bytes).unwrap(), img);
    // A requested chain longer than the geometry allows is rejected.
    assert!(matches!(
        encode(&img, &EncodeOptions::default().with_mip_levels(5)),
        Err(DdsError::InvalidData(_))
    ));
    // Feeding the frames back through `encode_all` reproduces the file.
    assert_eq!(
        encode_all(&frames, &EncodeOptions::default()).unwrap(),
        bytes
    );
}

#[test]
fn encode_all_builds_cubemaps_arrays_and_volumes() {
    // Cubemap with generated mips.
    let faces: Vec<Frame> = CubemapFace::ALL
        .iter()
        .enumerate()
        .map(|(i, f)| Frame::new(image(4, 4, PixelFormat::Rgba, 100 + i as u32)).with_face(*f))
        .collect();
    let bytes = encode_all(&faces, &EncodeOptions::default().with_mip_levels(0)).unwrap();
    let i = info(&bytes).unwrap();
    assert!(i.cubemap);
    assert_eq!(i.mip_levels, 3);
    assert_eq!(i.frames, 18);
    let frames = decode_all(&bytes).unwrap();
    assert_eq!(frames.len(), 18);
    assert_eq!(frames[0].face, Some(CubemapFace::PositiveX));
    assert_eq!(frames[3].face, Some(CubemapFace::NegativeX));
    assert_eq!(frames[3].mip_level, 0);
    assert_eq!(frames[3].image, faces[1].image);
    assert_eq!(
        encode_all(&frames, &EncodeOptions::default()).unwrap(),
        bytes
    );
    // Five faces are not a cubemap.
    assert!(matches!(
        encode_all(&faces[..5], &EncodeOptions::default()),
        Err(DdsError::InvalidData(_))
    ));

    // Texture array (DX10 forced by the shape).
    let slices: Vec<Frame> = (0..3)
        .map(|s| Frame::new(image(2, 2, PixelFormat::Gray8, 200 + s)).with_array_slice(s))
        .collect();
    let bytes = encode_all(&slices, &EncodeOptions::default()).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(i.array_size, 3);
    assert!(i.dx10_header);
    let frames = decode_all(&bytes).unwrap();
    assert_eq!(frames.len(), 3);
    assert_eq!(frames[2].array_slice, 2);
    assert_eq!(frames[2].image, slices[2].image);

    // Volume texture with generated mips (depth halves with the mips).
    let zs: Vec<Frame> = (0..4)
        .map(|z| Frame::new(image(2, 2, PixelFormat::Bgra, 300 + z)).with_depth_slice(z))
        .collect();
    let bytes = encode_all(&zs, &EncodeOptions::default().with_mip_levels(0)).unwrap();
    let i = info(&bytes).unwrap();
    assert_eq!(i.depth, 4);
    assert_eq!(i.mip_levels, 3);
    assert_eq!(i.frames, 4 + 2 + 1);
    let frames = decode_all(&bytes).unwrap();
    assert_eq!(frames.len(), 7);
    assert_eq!((frames[4].mip_level, frames[4].depth_slice), (1, 0));
    assert_eq!((frames[5].mip_level, frames[5].depth_slice), (1, 1));
    assert_eq!((frames[6].mip_level, frames[6].depth_slice), (2, 0));
    assert_eq!(frames[3].image, zs[3].image);
    assert_eq!(
        encode_all(&frames, &EncodeOptions::default()).unwrap(),
        bytes
    );
}

#[test]
fn decode_options_limits_and_strict() {
    let img = image(16, 8, PixelFormat::Rgba, 2);
    let bytes = encode(&img, &EncodeOptions::default()).unwrap();
    let small = DecodeOptions::default().with_max_width(8);
    assert!(matches!(
        decode_with(&bytes, &small),
        Err(DdsError::LimitExceeded(_))
    ));
    assert!(matches!(
        decode_with(&bytes, &DecodeOptions::default().with_max_pixels(100)),
        Err(DdsError::LimitExceeded(_))
    ));
    assert!(matches!(
        decode_with(&bytes, &DecodeOptions::default().with_max_bytes(100)),
        Err(DdsError::LimitExceeded(_))
    ));
    assert!(decode_with(&bytes, &DecodeOptions::default().unlimited()).is_ok());
    assert!(decode_all_with(&bytes, &DecodeOptions::default()).is_ok());

    // Trailing garbage: tolerated by default, rejected under strict.
    let mut padded = bytes.clone();
    padded.extend_from_slice(&[0xAB; 7]);
    assert_eq!(decode(&padded).unwrap(), img);
    assert!(matches!(
        decode_with(&padded, &DecodeOptions::default().with_strict(true)),
        Err(DdsError::InvalidData(_))
    ));
    // A wrong pitch: tolerated by default, rejected under strict.
    let mut bad_pitch = bytes.clone();
    bad_pitch[20..24].copy_from_slice(&7u32.to_le_bytes());
    assert_eq!(decode(&bad_pitch).unwrap(), img);
    assert!(matches!(
        decode_with(&bad_pitch, &DecodeOptions::default().with_strict(true)),
        Err(DdsError::InvalidData(_))
    ));
    assert!(decode_with(&bytes, &DecodeOptions::default().with_strict(true)).is_ok());
}

#[test]
fn hostile_inputs_return_errors() {
    for cut in [0usize, 3, 4, 8, 64, 127, 128, 140, GRAD8.len() - 1] {
        let _ = probe(&GRAD8[..cut]);
        assert!(info(&GRAD8[..cut]).is_err(), "info at {cut}");
        assert!(decode(&GRAD8[..cut]).is_err(), "decode at {cut}");
    }
    let mut huge = GRAD8.to_vec();
    huge[12..16].copy_from_slice(&u32::MAX.to_le_bytes());
    huge[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(decode(&huge).is_err());
    assert!(info(&huge).is_err());
}

// ---- Parity with the pre-contract depth writers ---------------------------

#[test]
fn contract_writer_matches_legacy_uncompressed_writer() {
    let img = image(8, 8, PixelFormat::Bgra, 77);
    let file = DdsFile::single(
        8,
        8,
        SurfaceFormat::A8R8G8B8,
        Plane::new(32, img.as_bytes().unwrap().to_vec()),
    )
    .unwrap()
    .with_mip_map_count(4);
    let old = encode_dds_uncompressed(&file).unwrap();
    let new = encode(&img, &EncodeOptions::default().with_mip_levels(4)).unwrap();
    assert_eq!(new, old, "legacy A8R8G8B8 mip chain");

    let g16 = image(4, 4, PixelFormat::Rgba64Le, 78);
    let file = DdsFile::single(
        4,
        4,
        SurfaceFormat::R16G16B16A16Unorm,
        Plane::new(32, g16.as_bytes().unwrap().to_vec()),
    )
    .unwrap();
    let old = encode_dds_uncompressed_dx10(&file).unwrap();
    let new = encode(&g16, &EncodeOptions::default()).unwrap();
    assert_eq!(new, old, "DX10 R16G16B16A16_UNORM");
}

#[test]
fn contract_writer_matches_legacy_block_and_astc_writers() {
    let img = image(8, 8, PixelFormat::Rgba, 79);
    let rgba = img.as_bytes().unwrap();
    for sf in [
        SurfaceFormat::Bc1,
        SurfaceFormat::Bc3,
        SurfaceFormat::Bc7Unorm,
    ] {
        let old =
            encode_dds_block_compressed_from_rgba8(rgba, 8, 8, sf, 4, false, 1, false).unwrap();
        let new = encode(
            &img,
            &EncodeOptions::default()
                .with_surface_format(sf)
                .with_mip_levels(4),
        )
        .unwrap();
        assert_eq!(new, old, "{}", sf.name());
    }
    let astc = SurfaceFormat::Astc {
        block_w: 4,
        block_h: 4,
        srgb: true,
    };
    let old = encode_dds_astc(rgba, 8, 8, astc, 2).unwrap();
    let new = encode(
        &img,
        &EncodeOptions::default()
            .with_surface_format(astc)
            .with_mip_levels(2),
    )
    .unwrap();
    assert_eq!(new, old, "ASTC 4x4 sRGB");
}

#[test]
fn contract_writer_matches_legacy_cubemap_array_writer() {
    let faces: Vec<Frame> = CubemapFace::ALL
        .iter()
        .enumerate()
        .map(|(i, f)| Frame::new(image(4, 4, PixelFormat::Bgra, 400 + i as u32)).with_face(*f))
        .collect();
    let surfaces = faces
        .iter()
        .map(|f| {
            oxideav_dds::DdsSurface::new(4, 4, Plane::new(16, f.image.as_bytes().unwrap().to_vec()))
                .with_face(f.face)
        })
        .collect();
    let file = DdsFile::new(4, 4, SurfaceFormat::A8R8G8B8, surfaces)
        .unwrap()
        .with_cubemap(true);
    let old = encode_dds_uncompressed_cubemap_array(&file).unwrap();
    let new = encode_all(&faces, &EncodeOptions::default()).unwrap();
    assert_eq!(new, old);
}
