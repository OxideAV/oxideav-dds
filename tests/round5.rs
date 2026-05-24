//! Round-3 / round-5 features: BC6H + BC7 encoders + mipmap-chain emission.
//!
//! Covers the new public surface added in this round:
//! * [`oxideav_dds::encode_bc6h`] / [`oxideav_dds::encode_bc6h_from_f32`]
//!   — BC6H mode-10 encoder. Validated via roundtrip through
//!   [`oxideav_dds::decode_bc6h`].
//! * [`oxideav_dds::encode_bc7`] — BC7 mode-6 encoder. Validated via
//!   roundtrip through [`oxideav_dds::decode_bc7`].
//! * [`oxideav_dds::encode_dds_uncompressed`] mipmap-chain emission
//!   when `DdsImage::mip_map_count > 1`. Each subsequent level is
//!   either copied from `image.surfaces` (caller pre-supplied) or
//!   fabricated by box-filter downsampling mip 0.

use oxideav_dds::{
    decode_bc1, decode_bc6h, decode_bc7, encode_bc1, encode_bc6h, encode_bc6h_from_f32, encode_bc7,
    encode_dds_block_compressed, encode_dds_block_compressed_from_rgba8, encode_dds_uncompressed,
    parse_dds, CubemapFace, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface,
};

/// BC7 encoder + decoder roundtrip on a 4×4 solid-white opaque block.
#[test]
fn bc7_encode_solid_white_roundtrip() {
    let input = vec![0xffu8; 4 * 4 * 4];
    let mut bc = vec![0u8; 16];
    encode_bc7(&input, 4, 4, &mut bc).unwrap();
    let mut decoded = vec![0u8; 4 * 4 * 4];
    decode_bc7(&bc, 4, 4, &mut decoded).unwrap();
    for chunk in decoded.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// BC7 encoder on an 8×8 grayscale gradient — mode-6 PSNR ≥ 30 dB.
#[test]
fn bc7_encode_8x8_grayscale_gradient_psnr() {
    let mut input = vec![0u8; 8 * 8 * 4];
    for y in 0..8 {
        for x in 0..8 {
            let off = (y * 8 + x) * 4;
            let v = ((x + y) * 16) as u8;
            input[off] = v;
            input[off + 1] = v;
            input[off + 2] = v;
            input[off + 3] = 0xff;
        }
    }
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
    encode_bc7(&input, 8, 8, &mut bc).unwrap();
    let mut decoded = vec![0u8; 8 * 8 * 4];
    decode_bc7(&bc, 8, 8, &mut decoded).unwrap();
    let mut sse: u64 = 0;
    let mut count: u64 = 0;
    for (a, b) in input.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        for c in 0..3 {
            let d = a[c] as i32 - b[c] as i32;
            sse += (d * d) as u64;
            count += 1;
        }
    }
    let mse = sse as f64 / count as f64;
    let psnr = 10.0 * (255.0_f64 * 255.0 / mse).log10();
    assert!(
        psnr > 30.0,
        "BC7 8x8 grayscale gradient PSNR = {:.2} dB",
        psnr
    );
}

/// BC6H encoder + decoder roundtrip on a 4×4 solid HDR block.
#[test]
fn bc6h_encode_solid_block_roundtrip() {
    // Solid pixel = (0.5, 0.5, 0.5) in linear. Pack as half-float RGBA.
    let half_v = 0x3800u16;
    let mut input = vec![0u8; 4 * 4 * 8];
    for i in 0..16 {
        let off = i * 8;
        input[off..off + 2].copy_from_slice(&half_v.to_le_bytes());
        input[off + 2..off + 4].copy_from_slice(&half_v.to_le_bytes());
        input[off + 4..off + 6].copy_from_slice(&half_v.to_le_bytes());
        input[off + 6..off + 8].copy_from_slice(&0x3c00u16.to_le_bytes());
    }
    let mut bc = vec![0u8; 16];
    encode_bc6h(&input, 4, 4, &mut bc).unwrap();
    let mut decoded = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
    // Every pixel should decode close to half(0.5).
    for chunk in decoded.chunks_exact(8) {
        let r = u16::from_le_bytes([chunk[0], chunk[1]]);
        // Decoded R should be in the neighborhood of half(0.5)=0x3800.
        // Mode-10 quantisation gives ~10-bit precision; allow a band.
        let target = 0x3800u32;
        let v = r as u32;
        let delta = v.abs_diff(target);
        assert!(
            delta < 0x100,
            "decoded R = 0x{:04x} (target 0x{:04x}, delta 0x{:x})",
            r,
            target,
            delta
        );
    }
}

/// BC6H encoder on a grayscale HDR gradient — PSNR ≥ 30 dB (peak 1.0).
#[test]
fn bc6h_encode_8x8_grayscale_gradient_psnr() {
    let mut input_f32 = vec![0f32; 8 * 8 * 3];
    for y in 0..8 {
        for x in 0..8 {
            let off = (y * 8 + x) * 3;
            let v = (x + y) as f32 / 14.0;
            input_f32[off] = v;
            input_f32[off + 1] = v;
            input_f32[off + 2] = v;
        }
    }
    let mut bc = vec![0u8; (8 / 4) * (8 / 4) * 16];
    encode_bc6h_from_f32(&input_f32, 8, 8, &mut bc).unwrap();
    let mut decoded = vec![0u8; 8 * 8 * 8];
    decode_bc6h(&bc, 8, 8, false, &mut decoded).unwrap();

    fn half_to_f32(h: u16) -> f32 {
        oxideav_dds::bc6h::half_to_f32(h)
    }

    let mut sse = 0.0_f64;
    let mut count = 0u64;
    for i in 0..(8 * 8) {
        let off = i * 8;
        for c in 0..3 {
            let h = u16::from_le_bytes([decoded[off + c * 2], decoded[off + c * 2 + 1]]);
            let v = half_to_f32(h) as f64;
            let target = input_f32[i * 3 + c] as f64;
            let d = v - target;
            sse += d * d;
            count += 1;
        }
    }
    let mse = sse / count as f64;
    let psnr = if mse <= 0.0 {
        f64::INFINITY
    } else {
        10.0 * (1.0_f64 / mse).log10()
    };
    assert!(
        psnr > 30.0,
        "BC6H 8x8 grayscale gradient PSNR = {:.2} dB",
        psnr
    );
}

/// Mipmap-chain emission: an 8×8 surface with `mip_map_count = 4` (8 →
/// 4 → 2 → 1) emits 4 levels in the on-disk byte stream and parses
/// back with 4 surfaces.
#[test]
fn encode_mipmap_chain_8x8() {
    let w = 8u32;
    let h = 8u32;
    let mip = 4u32; // 8x8, 4x4, 2x2, 1x1
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i & 0xff) as u8;
    }
    let img = DdsImage {
        width: w,
        height: h,
        pixel_format: DdsPixelFormat::A8R8G8B8,
        planes: vec![DdsPlane {
            stride: w as usize * 4,
            data: data.clone(),
        }],
        surfaces: Vec::new(),
        pts: None,
        mip_map_count: mip,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).expect("encode mipmap chain");
    let parsed = parse_dds(&bytes).expect("parse mipmap chain");
    assert_eq!(parsed.mip_map_count, mip);
    assert_eq!(parsed.surfaces.len(), mip as usize);
    assert_eq!(parsed.surfaces[0].width, 8);
    assert_eq!(parsed.surfaces[0].height, 8);
    assert_eq!(parsed.surfaces[1].width, 4);
    assert_eq!(parsed.surfaces[1].height, 4);
    assert_eq!(parsed.surfaces[2].width, 2);
    assert_eq!(parsed.surfaces[2].height, 2);
    assert_eq!(parsed.surfaces[3].width, 1);
    assert_eq!(parsed.surfaces[3].height, 1);

    // Mip 0 must round-trip exactly.
    assert_eq!(parsed.surfaces[0].plane.data, data);
}

/// 5×5 surface with mip_count = 3 → expected dims 5x5, 2x2, 1x1.
/// Exercises the floor-divide mip dimension rule.
#[test]
fn encode_mipmap_chain_odd_dimensions() {
    let w = 5u32;
    let h = 5u32;
    let mip = 3u32;
    let mut data = vec![0u8; (w * h * 4) as usize];
    for (i, b) in data.iter_mut().enumerate() {
        *b = (i & 0xff) as u8;
    }
    let img = DdsImage {
        width: w,
        height: h,
        pixel_format: DdsPixelFormat::A8R8G8B8,
        planes: vec![DdsPlane {
            stride: w as usize * 4,
            data,
        }],
        surfaces: Vec::new(),
        pts: None,
        mip_map_count: mip,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).expect("encode 5x5 mipmaps");
    let parsed = parse_dds(&bytes).expect("parse 5x5 mipmaps");
    assert_eq!(parsed.mip_map_count, 3);
    assert_eq!(parsed.surfaces.len(), 3);
    assert_eq!(
        (parsed.surfaces[0].width, parsed.surfaces[0].height),
        (5, 5)
    );
    assert_eq!(
        (parsed.surfaces[1].width, parsed.surfaces[1].height),
        (2, 2)
    );
    assert_eq!(
        (parsed.surfaces[2].width, parsed.surfaces[2].height),
        (1, 1)
    );
}

/// Round-4 lift: BC1 mip chain emission. Caller pre-encodes per-mip
/// surfaces via [`encode_bc1`] and the writer concatenates them with a
/// legacy DXT1 FourCC header. Parsing the result recovers the same
/// per-mip block bytes.
#[test]
fn encode_bc1_mipmap_chain_via_block_compressed() {
    let w = 8u32;
    let h = 8u32;
    let mip = 4u32; // 8x8, 4x4, 2x2, 1x1

    // Build a solid-white RGBA8 image, downsample to each mip dimension,
    // and encode each level to BC1.
    let make_rgba_solid = |w: u32, h: u32| -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for chunk in v.chunks_exact_mut(4) {
            chunk[0] = 0xff;
            chunk[1] = 0xff;
            chunk[2] = 0xff;
            chunk[3] = 0xff;
        }
        v
    };

    let mut surfaces: Vec<DdsSurface> = Vec::with_capacity(mip as usize);
    let dims = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
    for (level, &(mw, mh)) in dims.iter().enumerate() {
        let rgba = make_rgba_solid(mw, mh);
        let bw = mw.max(1).div_ceil(4) as usize;
        let bh = mh.max(1).div_ceil(4) as usize;
        let mut bc = vec![0u8; bw * bh * 8];
        encode_bc1(&rgba, mw, mh, false, &mut bc).expect("encode_bc1");
        surfaces.push(DdsSurface {
            width: mw,
            height: mh,
            mip_level: level as u32,
            array_slice: 0,
            face: None,
            depth_slice: 0,
            plane: DdsPlane {
                stride: bw * 8,
                data: bc,
            },
        });
    }

    let img = DdsImage {
        width: w,
        height: h,
        pixel_format: DdsPixelFormat::Bc1,
        planes: vec![surfaces[0].plane.clone()],
        surfaces: surfaces.clone(),
        pts: None,
        mip_map_count: mip,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };

    let bytes = encode_dds_block_compressed(&img).expect("encode BC1 mip chain");
    let parsed = parse_dds(&bytes).expect("parse BC1 mip chain");
    assert_eq!(parsed.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(parsed.mip_map_count, mip);
    assert_eq!(parsed.surfaces.len(), mip as usize);
    for level in 0..mip as usize {
        assert_eq!(parsed.surfaces[level].width, dims[level].0);
        assert_eq!(parsed.surfaces[level].height, dims[level].1);
        assert_eq!(
            parsed.surfaces[level].plane.data,
            surfaces[level].plane.data
        );
    }

    // Decode mip 0 back to RGBA to confirm round-trip correctness.
    let mut decoded = vec![0u8; 8 * 8 * 4];
    decode_bc1(&parsed.surfaces[0].plane.data, 8, 8, &mut decoded).unwrap();
    for chunk in decoded.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// Round-4 lift: BC7 mip chain emission via the DX10 extension header.
/// BC7 has no legacy FourCC, so the encoder always emits the DX10
/// extension when the format is BC7.
#[test]
fn encode_bc7_mipmap_chain_via_block_compressed() {
    let w = 8u32;
    let h = 8u32;
    let mip = 4u32;

    let make_rgba_solid = |w: u32, h: u32| -> Vec<u8> {
        let mut v = vec![0u8; (w * h * 4) as usize];
        for chunk in v.chunks_exact_mut(4) {
            chunk[0] = 0xff;
            chunk[1] = 0xff;
            chunk[2] = 0xff;
            chunk[3] = 0xff;
        }
        v
    };

    let dims = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
    let mut surfaces: Vec<DdsSurface> = Vec::with_capacity(mip as usize);
    for (level, &(mw, mh)) in dims.iter().enumerate() {
        let rgba = make_rgba_solid(mw, mh);
        let bw = mw.max(1).div_ceil(4) as usize;
        let bh = mh.max(1).div_ceil(4) as usize;
        let mut bc = vec![0u8; bw * bh * 16];
        encode_bc7(&rgba, mw, mh, &mut bc).expect("encode_bc7");
        surfaces.push(DdsSurface {
            width: mw,
            height: mh,
            mip_level: level as u32,
            array_slice: 0,
            face: None,
            depth_slice: 0,
            plane: DdsPlane {
                stride: bw * 16,
                data: bc,
            },
        });
    }

    let img = DdsImage {
        width: w,
        height: h,
        pixel_format: DdsPixelFormat::Bc7Unorm,
        planes: vec![surfaces[0].plane.clone()],
        surfaces: surfaces.clone(),
        pts: None,
        mip_map_count: mip,
        has_dxt10_header: true,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };

    let bytes = encode_dds_block_compressed(&img).expect("encode BC7 mip chain");
    let parsed = parse_dds(&bytes).expect("parse BC7 mip chain");
    assert_eq!(parsed.pixel_format, DdsPixelFormat::Bc7Unorm);
    assert_eq!(parsed.mip_map_count, mip);
    assert_eq!(parsed.surfaces.len(), mip as usize);
    for level in 0..mip as usize {
        assert_eq!(parsed.surfaces[level].width, dims[level].0);
        assert_eq!(parsed.surfaces[level].height, dims[level].1);
        assert_eq!(
            parsed.surfaces[level].plane.data,
            surfaces[level].plane.data
        );
    }
    assert!(parsed.has_dxt10_header);

    // Decode mip 0 back to RGBA.
    let mut decoded = vec![0u8; 8 * 8 * 4];
    decode_bc7(&parsed.surfaces[0].plane.data, 8, 8, &mut decoded).unwrap();
    for chunk in decoded.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// Block-compressed encoder rejects mismatched mip dimensions.
#[test]
fn encode_block_compressed_rejects_mismatched_dims() {
    let bw = 1usize;
    let bh = 1usize;
    let bc = vec![0u8; bw * bh * 8];
    let img = DdsImage {
        width: 8,
        height: 8,
        pixel_format: DdsPixelFormat::Bc1,
        planes: vec![DdsPlane {
            stride: bw * 8,
            data: bc.clone(),
        }],
        surfaces: vec![DdsSurface {
            width: 4, // wrong — should be 8 for mip 0 of an 8x8 image
            height: 4,
            mip_level: 0,
            array_slice: 0,
            face: None,
            depth_slice: 0,
            plane: DdsPlane {
                stride: bw * 8,
                data: bc,
            },
        }],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    assert!(encode_dds_block_compressed(&img).is_err());
}

/// Block-compressed encoder rejects uncompressed pixel formats.
#[test]
fn encode_block_compressed_rejects_uncompressed() {
    let img = DdsImage {
        width: 4,
        height: 4,
        pixel_format: DdsPixelFormat::A8R8G8B8,
        planes: vec![DdsPlane {
            stride: 16,
            data: vec![0u8; 64],
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
    assert!(encode_dds_block_compressed(&img).is_err());
}

/// Round-5 lift: BC1 mip chain emission directly from RGBA8 — no
/// caller-side pre-encoding. The writer fabricates each mip by
/// box-downsampling the RGBA8 source then encoding to BC1 blocks.
#[test]
fn encode_bc1_mipmap_chain_from_rgba8() {
    let w = 8u32;
    let h = 8u32;
    let mip = 4u32;
    let rgba = vec![0xffu8; (w * h * 4) as usize]; // solid white
    let bytes = encode_dds_block_compressed_from_rgba8(
        &rgba,
        w,
        h,
        DdsPixelFormat::Bc1,
        mip,
        false,
        1,
        false,
    )
    .expect("encode BC1 from RGBA8");
    let parsed = parse_dds(&bytes).expect("parse BC1 mip chain");
    assert_eq!(parsed.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(parsed.mip_map_count, mip);
    assert_eq!(parsed.surfaces.len(), mip as usize);
    let dims = [(8u32, 8u32), (4, 4), (2, 2), (1, 1)];
    for (level, &(mw, mh)) in dims.iter().enumerate().take(mip as usize) {
        assert_eq!(parsed.surfaces[level].width, mw);
        assert_eq!(parsed.surfaces[level].height, mh);
    }
    // Decode mip 0 — should reproduce solid white.
    let mut decoded = vec![0u8; (w * h * 4) as usize];
    decode_bc1(&parsed.surfaces[0].plane.data, w, h, &mut decoded).unwrap();
    for chunk in decoded.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// Round-5 lift: BC7 mip chain emission directly from RGBA8.
#[test]
fn encode_bc7_mipmap_chain_from_rgba8() {
    let w = 8u32;
    let h = 8u32;
    let mip = 4u32;
    // Smooth gradient — mip 0 should round-trip with high PSNR.
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let off = ((y * w + x) * 4) as usize;
            rgba[off] = (x * 32) as u8;
            rgba[off + 1] = (y * 32) as u8;
            rgba[off + 2] = ((x + y) * 16) as u8;
            rgba[off + 3] = 0xff;
        }
    }
    let bytes = encode_dds_block_compressed_from_rgba8(
        &rgba,
        w,
        h,
        DdsPixelFormat::Bc7Unorm,
        mip,
        false,
        1,
        true,
    )
    .expect("encode BC7 from RGBA8");
    let parsed = parse_dds(&bytes).expect("parse BC7 mip chain");
    assert_eq!(parsed.pixel_format, DdsPixelFormat::Bc7Unorm);
    assert_eq!(parsed.mip_map_count, mip);
    assert_eq!(parsed.surfaces.len(), mip as usize);
    assert!(parsed.has_dxt10_header);

    // Decode mip 0 + measure PSNR against the source RGBA8.
    let mut decoded = vec![0u8; (w * h * 4) as usize];
    decode_bc7(&parsed.surfaces[0].plane.data, w, h, &mut decoded).unwrap();
    let mut sse: u64 = 0;
    let mut count: u64 = 0;
    for (a, b) in rgba.chunks_exact(4).zip(decoded.chunks_exact(4)) {
        for c in 0..3 {
            let d = a[c] as i32 - b[c] as i32;
            sse += (d * d) as u64;
            count += 1;
        }
    }
    let mse = sse as f64 / count as f64;
    let psnr = 10.0 * (255.0_f64 * 255.0 / mse).log10();
    assert!(
        psnr > 25.0,
        "BC7-from-RGBA8 mip 0 PSNR = {:.2} dB (want > 25 dB)",
        psnr
    );
}

/// Round-5 lift: cubemap BC1 emission from a 6-slice RGBA8 source.
#[test]
fn encode_bc1_cubemap_from_rgba8() {
    let w = 4u32;
    let h = 4u32;
    let mip = 1u32;
    // Per-face solid colour: face_idx-coded so we can verify
    // the surfaces come back in PX, NX, PY, NY, PZ, NZ order.
    let face_count = 6u32;
    let mut rgba = vec![0u8; (face_count * w * h * 4) as usize];
    for f in 0..face_count {
        let face_off = (f * w * h * 4) as usize;
        for chunk in rgba[face_off..face_off + (w * h * 4) as usize].chunks_exact_mut(4) {
            chunk[0] = (f as u8) * 40 + 10;
            chunk[1] = (f as u8) * 40 + 10;
            chunk[2] = (f as u8) * 40 + 10;
            chunk[3] = 0xff;
        }
    }
    let bytes = encode_dds_block_compressed_from_rgba8(
        &rgba,
        w,
        h,
        DdsPixelFormat::Bc1,
        mip,
        true,
        1,
        false,
    )
    .expect("encode cubemap BC1 from RGBA8");
    let parsed = parse_dds(&bytes).expect("parse cubemap BC1");
    assert!(parsed.is_cubemap);
    assert_eq!(parsed.surfaces.len(), 6);
    let order = [
        CubemapFace::PositiveX,
        CubemapFace::NegativeX,
        CubemapFace::PositiveY,
        CubemapFace::NegativeY,
        CubemapFace::PositiveZ,
        CubemapFace::NegativeZ,
    ];
    for (i, expected_face) in order.iter().enumerate() {
        assert_eq!(parsed.surfaces[i].face, Some(*expected_face));
    }
}

/// `encode_dds_block_compressed_from_rgba8` rejects BC6H (it's HDR —
/// callers must pre-encode via `encode_bc6h_from_f32`).
#[test]
fn encode_block_compressed_from_rgba8_rejects_bc6h() {
    let rgba = vec![0u8; 4 * 4 * 4];
    let r = encode_dds_block_compressed_from_rgba8(
        &rgba,
        4,
        4,
        DdsPixelFormat::Bc6hUf16,
        1,
        false,
        1,
        false,
    );
    assert!(r.is_err());
}

/// `encode_dds_block_compressed_from_rgba8` rejects uncompressed
/// pixel formats — the API contract is BC* only.
#[test]
fn encode_block_compressed_from_rgba8_rejects_uncompressed() {
    let rgba = vec![0u8; 4 * 4 * 4];
    let r = encode_dds_block_compressed_from_rgba8(
        &rgba,
        4,
        4,
        DdsPixelFormat::A8R8G8B8,
        1,
        false,
        1,
        false,
    );
    assert!(r.is_err());
}

/// Round-6 lift: smoke-test DX10 array BC7 emission via
/// `encode_dds_block_compressed_from_rgba8`. A 2-slice array of solid
/// 4×4 RGBA8 should encode + parse with `array_size = 2` and produce 2
/// surfaces with the correct DX10 header.
#[test]
fn encode_bc7_dx10_array_from_rgba8() {
    let w = 4u32;
    let h = 4u32;
    let mip = 1u32;
    let array_size = 2u32;
    let face_count = 1u32;
    // Two slices: slice 0 = pure red, slice 1 = pure blue.
    let mut rgba = vec![0u8; (array_size * face_count * w * h * 4) as usize];
    let stride = (w * h * 4) as usize;
    for chunk in rgba[..stride].chunks_exact_mut(4) {
        chunk[0] = 0xff;
        chunk[3] = 0xff;
    }
    for chunk in rgba[stride..2 * stride].chunks_exact_mut(4) {
        chunk[2] = 0xff;
        chunk[3] = 0xff;
    }
    let bytes = encode_dds_block_compressed_from_rgba8(
        &rgba,
        w,
        h,
        DdsPixelFormat::Bc7Unorm,
        mip,
        false,
        array_size,
        true,
    )
    .expect("encode array BC7 from RGBA8");
    let parsed = parse_dds(&bytes).expect("parse array BC7");
    assert_eq!(parsed.pixel_format, DdsPixelFormat::Bc7Unorm);
    assert_eq!(parsed.array_size, array_size);
    assert_eq!(parsed.surfaces.len(), array_size as usize);
    assert!(parsed.has_dxt10_header);
    assert!(!parsed.is_cubemap);
}

/// Round-6 lift: BC6H encoder picks 2-subset modes when content
/// benefits from partition. A 4×4 block with two clusters (left = dark,
/// right = light) should encode losslessly — cluster intra-spread = 0,
/// so any 2-subset mode fits with bit-exact reconstruction (modulo the
/// 31/64 BC6H_UF16 finalise scale). Validates the new partition + mode
/// dispatch path.
#[test]
fn bc6h_encode_two_cluster_block_high_psnr() {
    let mut input_f32 = vec![0f32; 4 * 4 * 3];
    for y in 0..4 {
        for x in 0..4 {
            let off = (y * 4 + x) * 3;
            let v = if x < 2 { 0.4 } else { 0.6 };
            input_f32[off] = v;
            input_f32[off + 1] = v;
            input_f32[off + 2] = v;
        }
    }
    let mut bc = vec![0u8; 16];
    encode_bc6h_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
    let mut decoded = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
    fn half_to_f32(h: u16) -> f32 {
        oxideav_dds::bc6h::half_to_f32(h)
    }
    let mut sse = 0.0_f64;
    let mut count = 0u64;
    for i in 0..16 {
        let off = i * 8;
        for c in 0..3 {
            let h = u16::from_le_bytes([decoded[off + c * 2], decoded[off + c * 2 + 1]]);
            let v = half_to_f32(h) as f64;
            let target = input_f32[i * 3 + c] as f64;
            sse += (v - target).powi(2);
            count += 1;
        }
    }
    let mse = sse / count as f64;
    let psnr = 10.0 * (1.0_f64 / mse).log10();
    assert!(
        psnr > 35.0,
        "BC6H 2-cluster PSNR = {:.2} dB (want > 35 dB)",
        psnr
    );
}

/// Round-6 lift: BC6H delta-encoded mode 11/12/13 picks a mode for a
/// tight-range block (all pixels in [0.4, 0.5]). Verify the encoder
/// produces a valid block that decodes back to the same range.
#[test]
fn bc6h_encode_tight_gradient_high_psnr() {
    let mut input_f32 = vec![0f32; 4 * 4 * 3];
    for y in 0..4 {
        for x in 0..4 {
            let off = (y * 4 + x) * 3;
            let v = 0.4 + ((x + y) as f32 / 6.0) * 0.1;
            input_f32[off] = v;
            input_f32[off + 1] = v;
            input_f32[off + 2] = v;
        }
    }
    let mut bc = vec![0u8; 16];
    encode_bc6h_from_f32(&input_f32, 4, 4, &mut bc).unwrap();
    let mut decoded = vec![0u8; 4 * 4 * 8];
    decode_bc6h(&bc, 4, 4, false, &mut decoded).unwrap();
    fn half_to_f32(h: u16) -> f32 {
        oxideav_dds::bc6h::half_to_f32(h)
    }
    let mut sse = 0.0_f64;
    let mut count = 0u64;
    for i in 0..16 {
        let off = i * 8;
        for c in 0..3 {
            let h = u16::from_le_bytes([decoded[off + c * 2], decoded[off + c * 2 + 1]]);
            let v = half_to_f32(h) as f64;
            let target = input_f32[i * 3 + c] as f64;
            sse += (v - target).powi(2);
            count += 1;
        }
    }
    let mse = sse / count as f64;
    let psnr = 10.0 * (1.0_f64 / mse).log10();
    assert!(
        psnr > 30.0,
        "BC6H tight gradient PSNR = {:.2} dB (want > 30 dB)",
        psnr
    );
}

/// Single-level surface (`mip_map_count = 1`) emits no mip flag and
/// parses back with one surface — regression check that the round-3
/// mipmap path doesn't break the pre-existing single-level case.
#[test]
fn encode_no_mipmaps_round_trip_unchanged() {
    let w = 4u32;
    let h = 3u32;
    let data = vec![0xa5u8; (w * h * 4) as usize];
    let img = DdsImage {
        width: w,
        height: h,
        pixel_format: DdsPixelFormat::A8R8G8B8,
        planes: vec![DdsPlane {
            stride: w as usize * 4,
            data: data.clone(),
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
    let bytes = encode_dds_uncompressed(&img).unwrap();
    let parsed = parse_dds(&bytes).unwrap();
    assert_eq!(parsed.mip_map_count, 1);
    assert_eq!(parsed.surfaces.len(), 1);
    assert_eq!(parsed.surfaces[0].plane.data, data);
}
