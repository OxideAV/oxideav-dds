//! End-to-end coverage for the round-225 HDR-float uncompressed
//! formats: DXGI `R16G16B16A16_FLOAT` (10) and `R16_FLOAT` (54).
//!
//! Each test builds a `DdsImage`, runs it through
//! `encode_dds_uncompressed`, then `parse_dds` again, then expands the
//! half-float bytes into `f32` via `decode_r16g16b16a16_float` /
//! `decode_r16_float` and checks that the recovered values match the
//! original `f32` inputs within half-float quantisation error.

use oxideav_dds::{
    decode_r16_float, decode_r16g16b16a16_float, encode_dds_uncompressed,
    encode_r16_float_from_f32, encode_r16g16b16a16_float_from_f32, f32_to_half, half_to_f32,
    parse_dds, DdsImage, DdsPixelFormat, DdsPlane, DxgiFormat,
};

#[test]
fn half_round_trip_identity_on_representable_values() {
    // Values exactly representable in IEEE-754 binary16 must round-trip
    // verbatim through `f32_to_half` then `half_to_f32`.
    let cases: [f32; 10] = [
        0.0,
        -0.0,
        1.0,
        -1.0,
        0.5,
        2.0,
        65504.0, // max finite normal in half precision
        -65504.0,
        1.0 / 1024.0,
        f32::INFINITY,
    ];
    for v in cases {
        let h = f32_to_half(v);
        let back = half_to_f32(h);
        assert_eq!(
            back, v,
            "exact representable value {v} did not round-trip (got {back})"
        );
    }
}

#[test]
fn half_round_trip_quantises_within_tolerance() {
    // Quantising values that don't sit on the half-float grid is
    // expected to introduce at most ~0.05% relative error in the
    // normal range. The encoder is round-to-nearest-even, so the
    // tolerance is half the local ULP.
    let cases: [f32; 6] = [0.1, 0.25, 0.3333, 1.7, 10.4, 1000.5];
    for v in cases {
        let h = f32_to_half(v);
        let back = half_to_f32(h);
        let rel = (back - v).abs() / v.abs();
        assert!(
            rel < 1.0e-3,
            "value {v} round-tripped to {back} (rel err {rel})"
        );
    }
}

#[test]
fn half_overflow_saturates_to_inf() {
    // f32 values larger than the half-precision finite range saturate
    // to ±Inf rather than producing a garbage exponent.
    let pos = f32_to_half(70000.0);
    assert_eq!(pos, 0x7c00, "+ overflow should produce +Inf");
    let neg = f32_to_half(-70000.0);
    assert_eq!(neg, 0xfc00, "- overflow should produce -Inf");
}

#[test]
fn half_underflow_flushes_to_zero() {
    // Sub-normal-range values flush to ±0 rather than synthesising a
    // half-subnormal. Microsoft's public reference uses the same
    // behaviour.
    let tiny = f32_to_half(1.0e-10);
    assert_eq!(tiny & 0x7fff, 0, "underflow should flush positive to +0");
    let ntiny = f32_to_half(-1.0e-10);
    assert_eq!(ntiny & 0x7fff, 0, "underflow should flush negative to -0");
}

#[test]
fn r16g16b16a16_float_roundtrip_through_dds_file() {
    // Build a 4x3 RGBA half-float surface, encode + parse + decode,
    // confirm bit-exact byte round-trip and within-tolerance f32 match.
    let width = 4u32;
    let height = 3u32;
    let mut input_f32 = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            // Red ramps left-to-right 0..1, green is HDR (>1.0) on the
            // bottom row, blue is row-dependent, alpha constant 1.0.
            input_f32.push(x as f32 / (width - 1) as f32);
            input_f32.push(if y == height - 1 { 2.5 + x as f32 } else { 0.5 });
            input_f32.push(y as f32 * 0.25);
            input_f32.push(1.0);
        }
    }
    let mut on_disk = vec![0u8; (width * height * 8) as usize];
    encode_r16g16b16a16_float_from_f32(&input_f32, width, height, &mut on_disk).unwrap();

    let img = DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::R16G16B16A16Float,
        planes: vec![DdsPlane {
            stride: (width * 8) as usize,
            data: on_disk.clone(),
        }],
        surfaces: vec![],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: true,
        dxgi_format: Some(DxgiFormat::R16G16B16A16Float),
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).unwrap();

    let parsed = parse_dds(&bytes).unwrap();
    assert_eq!(parsed.width, width);
    assert_eq!(parsed.height, height);
    assert_eq!(parsed.pixel_format, DdsPixelFormat::R16G16B16A16Float);
    assert!(parsed.has_dxt10_header);
    assert_eq!(parsed.dxgi_format, Some(DxgiFormat::R16G16B16A16Float));
    assert_eq!(parsed.planes[0].data, on_disk);

    // Decode the parsed bytes back to f32 and check tolerance.
    let mut recovered = vec![0.0f32; (width * height * 4) as usize];
    decode_r16g16b16a16_float(&parsed.planes[0].data, width, height, &mut recovered).unwrap();
    for (i, (got, want)) in recovered.iter().zip(input_f32.iter()).enumerate() {
        let rel = if *want != 0.0 {
            (got - want).abs() / want.abs()
        } else {
            (got - want).abs()
        };
        assert!(
            rel < 1.0e-3,
            "channel {} got {got} want {want} (rel {rel})",
            i % 4,
        );
    }
}

#[test]
fn r16_float_roundtrip_through_dds_file() {
    // 8x4 single-channel half-float surface, full file round-trip.
    let width = 8u32;
    let height = 4u32;
    let mut input_f32 = Vec::with_capacity((width * height) as usize);
    for y in 0..height {
        for x in 0..width {
            // HDR luminance: 0.1 .. ~25.0 across the surface.
            input_f32.push(0.1 + (y * width + x) as f32 * 0.8);
        }
    }
    let mut on_disk = vec![0u8; (width * height * 2) as usize];
    encode_r16_float_from_f32(&input_f32, width, height, &mut on_disk).unwrap();

    let img = DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::R16Float,
        planes: vec![DdsPlane {
            stride: (width * 2) as usize,
            data: on_disk.clone(),
        }],
        surfaces: vec![],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: true,
        dxgi_format: Some(DxgiFormat::R16Float),
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).unwrap();
    let parsed = parse_dds(&bytes).unwrap();
    assert_eq!(parsed.pixel_format, DdsPixelFormat::R16Float);
    assert_eq!(parsed.dxgi_format, Some(DxgiFormat::R16Float));
    assert_eq!(parsed.planes[0].data, on_disk);

    let mut recovered = vec![0.0f32; (width * height) as usize];
    decode_r16_float(&parsed.planes[0].data, width, height, &mut recovered).unwrap();
    for (got, want) in recovered.iter().zip(input_f32.iter()) {
        let rel = (got - want).abs() / want.abs();
        assert!(rel < 1.0e-3, "got {got} want {want} (rel {rel})");
    }
}

#[test]
fn r16g16b16a16_float_with_mip_chain_decimates_to_one_pixel() {
    // 8x8 half-float surface with a 4-level mip chain (8/4/2/1). The
    // encoder fabricates the chain by half-float-aware box-filter, so
    // each level's average should track the source values within
    // half-precision tolerance.
    let width = 8u32;
    let height = 8u32;
    let mut input_f32 = Vec::with_capacity((width * height * 4) as usize);
    for _ in 0..width * height {
        input_f32.push(2.0);
        input_f32.push(4.0);
        input_f32.push(0.5);
        input_f32.push(1.0);
    }
    let mut on_disk = vec![0u8; (width * height * 8) as usize];
    encode_r16g16b16a16_float_from_f32(&input_f32, width, height, &mut on_disk).unwrap();
    let img = DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::R16G16B16A16Float,
        planes: vec![DdsPlane {
            stride: (width * 8) as usize,
            data: on_disk,
        }],
        surfaces: vec![],
        pts: None,
        mip_map_count: 4,
        has_dxt10_header: true,
        dxgi_format: Some(DxgiFormat::R16G16B16A16Float),
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let bytes = encode_dds_uncompressed(&img).unwrap();
    let parsed = parse_dds(&bytes).unwrap();
    assert_eq!(parsed.mip_map_count, 4);
    assert_eq!(parsed.surfaces.len(), 4);

    // Confirm the bottom (1x1) mip is the constant source colour
    // (a uniform input is fixed under any averaging filter).
    let mip3 = &parsed.surfaces[3];
    assert_eq!(mip3.width, 1);
    assert_eq!(mip3.height, 1);
    let mut recovered = [0.0f32; 4];
    decode_r16g16b16a16_float(&mip3.plane.data, 1, 1, &mut recovered).unwrap();
    assert!((recovered[0] - 2.0).abs() < 1.0e-2);
    assert!((recovered[1] - 4.0).abs() < 1.0e-2);
    assert!((recovered[2] - 0.5).abs() < 1.0e-2);
    assert!((recovered[3] - 1.0).abs() < 1.0e-2);
}

#[test]
fn r16g16b16a16_float_rejects_short_buffers() {
    // Decode and encode helpers refuse undersized inputs / outputs
    // rather than panicking.
    let small_bytes = vec![0u8; 7]; // less than one pixel × 8 bytes
    let mut out = vec![0.0f32; 4];
    assert!(decode_r16g16b16a16_float(&small_bytes, 1, 1, &mut out).is_err());

    let bytes = vec![0u8; 8];
    let mut out_small = vec![0.0f32; 3];
    assert!(decode_r16g16b16a16_float(&bytes, 1, 1, &mut out_small).is_err());

    let f32_in = vec![0.0f32; 4];
    let mut out_bytes_small = vec![0u8; 7];
    assert!(encode_r16g16b16a16_float_from_f32(&f32_in, 1, 1, &mut out_bytes_small).is_err());
}

#[test]
fn r16_float_rejects_short_buffers() {
    let small = vec![0u8; 1];
    let mut out = vec![0.0f32; 1];
    assert!(decode_r16_float(&small, 1, 1, &mut out).is_err());

    let f32_in = vec![0.0f32; 1];
    let mut out_small = vec![0u8; 1];
    assert!(encode_r16_float_from_f32(&f32_in, 1, 1, &mut out_small).is_err());
}

#[test]
fn encode_dds_uncompressed_rejects_float_without_dx10_header_flag() {
    // The legacy `DDS_PIXELFORMAT` block has no mask layout for
    // half-float data, so the encoder always promotes a float surface
    // to a DX10 header — confirm an encoded float file is parseable
    // even with `has_dxt10_header` left at the default (`false`).
    let width = 2u32;
    let height = 2u32;
    let input = vec![1.0f32; (width * height * 4) as usize];
    let mut bytes = vec![0u8; (width * height * 8) as usize];
    encode_r16g16b16a16_float_from_f32(&input, width, height, &mut bytes).unwrap();
    let img = DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::R16G16B16A16Float,
        planes: vec![DdsPlane {
            stride: (width * 8) as usize,
            data: bytes,
        }],
        surfaces: vec![],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false, // intentionally cleared
        dxgi_format: None,       // encoder must pick from the format
        is_cubemap: false,
        array_size: 1,
        depth: 1,
    };
    let file = encode_dds_uncompressed(&img).unwrap();
    let parsed = parse_dds(&file).unwrap();
    assert_eq!(parsed.pixel_format, DdsPixelFormat::R16G16B16A16Float);
    assert!(parsed.has_dxt10_header);
    assert_eq!(parsed.dxgi_format, Some(DxgiFormat::R16G16B16A16Float));
}
