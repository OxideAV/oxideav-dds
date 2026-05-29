//! Round 162: injection-robustness property tests for `parse_dds` and the
//! `decode_bc*` surface decoders.
//!
//! Every test here builds a known-good DDS byte stream, mutates a single
//! field, and asserts that `parse_dds` returns `Err(DdsError::…)`
//! rather than panicking, aborting, or allocating gigabytes from a
//! forged size field. The standalone `decode_bc1/2/3/4_unorm/5_unorm/7`
//! and `decode_bc6h` entry points get the same adversarial-slice
//! treatment: short inputs, short outputs, zero-sized surfaces.
//!
//! These tests intentionally avoid `#[should_panic]` — every error path
//! must surface through the public `Result` so a defensive caller can
//! report (or recover from) the malformed stream. Panic = ungraceful
//! handling = bug.
//!
//! Reference: Microsoft's public "DDS file layout for textures" guide.
//! No DirectXTex / D3DX / NVTT / squish source consulted.

use oxideav_dds::types::{
    DDPF_ALPHAPIXELS, DDPF_FOURCC, DDPF_RGB, DDSCAPS2_VOLUME, DDSCAPS_COMPLEX, DDSCAPS_TEXTURE,
    DDSD_CAPS, DDSD_DEPTH, DDSD_HEIGHT, DDSD_MIPMAPCOUNT, DDSD_PIXELFORMAT, DDSD_WIDTH,
    DDS_DIMENSION_TEXTURE3D, DDS_HEADER_SIZE, DDS_MAGIC, DDS_RESOURCE_MISC_TEXTURECUBE,
    FOURCC_DX10, FOURCC_DXT1,
};
use oxideav_dds::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_unorm, decode_bc5_snorm,
    decode_bc5_unorm, decode_bc6h, decode_bc7, parse_dds, DxgiFormat,
};

// ---------------------------------------------------------------------------
// Fixture builders.
// ---------------------------------------------------------------------------

/// Pack a u32 little-endian onto the end of a Vec.
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Build a known-good legacy A8R8G8B8 (BGRA-on-disk) DDS of `w × h`, no
/// mipmaps, no DX10 extension. 32 bpp, payload filled with `tag`.
fn build_legacy_argb(w: u32, h: u32, tag: u8) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    push_u32(
        &mut out,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT,
    );
    push_u32(&mut out, h);
    push_u32(&mut out, w);
    push_u32(&mut out, w * 4); // pitch
    push_u32(&mut out, 0); // depth
    push_u32(&mut out, 0); // mip count
    for _ in 0..11 {
        push_u32(&mut out, 0); // reserved1
    }
    // DDS_PIXELFORMAT — A8R8G8B8 (BGRA on disk).
    push_u32(&mut out, 32); // pixel format size
    push_u32(&mut out, DDPF_RGB | DDPF_ALPHAPIXELS);
    push_u32(&mut out, 0); // four_cc
    push_u32(&mut out, 32); // rgb bit count
    push_u32(&mut out, 0x00ff_0000); // R
    push_u32(&mut out, 0x0000_ff00); // G
    push_u32(&mut out, 0x0000_00ff); // B
    push_u32(&mut out, 0xff00_0000); // A
    push_u32(&mut out, DDSCAPS_TEXTURE);
    push_u32(&mut out, 0); // caps2
    push_u32(&mut out, 0); // caps3
    push_u32(&mut out, 0); // caps4
    push_u32(&mut out, 0); // reserved2

    out.extend(vec![tag; (w * h * 4) as usize]);
    out
}

/// Build a known-good DX10 cubemap with a single 4×4 BC1 face per the
/// six cubemap faces, all 6 faces present, single mip. The DXGI format
/// is BC1_UNORM. Payload runs `tag` for the first face's first block,
/// incrementing across all surfaces.
fn build_dx10_cubemap_bc1(w: u32, mip_count: u32, array_size: u32) -> Vec<u8> {
    let mut out = Vec::new();
    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    let with_mips = mip_count > 1;
    let flags = DDSD_CAPS
        | DDSD_HEIGHT
        | DDSD_WIDTH
        | DDSD_PIXELFORMAT
        | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    push_u32(&mut out, flags);
    push_u32(&mut out, w); // height
    push_u32(&mut out, w); // width
    push_u32(&mut out, 0); // linear size
    push_u32(&mut out, 0); // depth
    push_u32(&mut out, mip_count); // mip count
    for _ in 0..11 {
        push_u32(&mut out, 0);
    }
    push_u32(&mut out, 32); // pixel format size
    push_u32(&mut out, DDPF_FOURCC);
    push_u32(&mut out, FOURCC_DX10);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, DDSCAPS_TEXTURE | DDSCAPS_COMPLEX);
    push_u32(&mut out, 0); // caps2 — DX10 carries cubemap bit in misc_flag
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    push_u32(&mut out, 0);
    // DDS_HEADER_DXT10.
    push_u32(&mut out, DxgiFormat::Bc1Unorm.to_u32());
    push_u32(&mut out, 3); // DDS_DIMENSION_TEXTURE2D
    push_u32(&mut out, DDS_RESOURCE_MISC_TEXTURECUBE);
    push_u32(&mut out, array_size);
    push_u32(&mut out, 0);

    // Each face: per-mip 4×4-block byte payload, then halve.
    let mut tag: u8 = 0;
    for _ai in 0..array_size {
        for _face in 0..6u32 {
            for m in 0..mip_count {
                let mw = (w >> m).max(1);
                let mh = (w >> m).max(1);
                let bw = mw.div_ceil(4) as usize;
                let bh = mh.div_ceil(4) as usize;
                out.extend(vec![tag; bw * bh * 8]);
                tag = tag.wrapping_add(1);
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Container header injection — `parse_dds` must Err, never panic.
// ---------------------------------------------------------------------------

#[test]
fn empty_buffer_errors_not_panics() {
    let err = parse_dds(&[]).expect_err("empty must error");
    assert!(format!("{err}").contains("buffer too small"));
}

#[test]
fn single_byte_buffer_errors() {
    let err = parse_dds(&[0xff]).expect_err("1 byte must error");
    assert!(format!("{err}").contains("buffer too small"));
}

#[test]
fn header_only_127_bytes_errors() {
    // 4 + 124 = 128 bytes is the minimum for magic + header; one short.
    let buf = vec![0u8; 127];
    let err = parse_dds(&buf).expect_err("127 bytes must error");
    assert!(format!("{err}").contains("buffer too small"));
}

#[test]
fn bad_magic_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // Clobber the first dword.
    bytes[0..4].copy_from_slice(&0xdead_beefu32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("bad magic must error");
    assert!(format!("{err}").contains("bad DDS magic"));
}

#[test]
fn bad_header_size_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // Header.size at offset 4.
    bytes[4..8].copy_from_slice(&127u32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("bad header.size must error");
    assert!(format!("{err}").contains("DDS_HEADER.size"));
}

#[test]
fn bad_pixel_format_size_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // DDS_PIXELFORMAT.size sits at offset 4 (magic) + 72 (header up to
    // and including reserved1) = 76.
    bytes[76..80].copy_from_slice(&33u32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("bad pixel format size must error");
    assert!(format!("{err}").contains("DDS_PIXELFORMAT.size"));
}

#[test]
fn zero_width_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // Width at offset 4 + 12 = 16.
    bytes[16..20].copy_from_slice(&0u32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("zero width must error");
    assert!(format!("{err}").contains("zero-sized"));
}

#[test]
fn zero_height_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // Height at offset 4 + 8 = 12.
    bytes[12..16].copy_from_slice(&0u32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("zero height must error");
    assert!(format!("{err}").contains("zero-sized"));
}

#[test]
fn missing_required_flags_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    // Flags at offset 4 + 4 = 8.
    bytes[8..12].copy_from_slice(&0u32.to_le_bytes());
    let err = parse_dds(&bytes).expect_err("missing required flags must error");
    assert!(format!("{err}").contains("missing required bits"));
}

#[test]
fn dx10_fourcc_without_extension_errors() {
    // Build a legacy header but flip the fourCC to DX10 — the parser must
    // notice the 20 trailing bytes are missing instead of reading past.
    // DDS_PIXELFORMAT starts at offset 4 + 72 = 76. size at +0, flags at
    // +4, fourCC at +8.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    bytes[76..80].copy_from_slice(&32u32.to_le_bytes()); // pixfmt size
    bytes[80..84].copy_from_slice(&DDPF_FOURCC.to_le_bytes()); // pixfmt flags
    bytes[84..88].copy_from_slice(&FOURCC_DX10.to_le_bytes()); // pixfmt fourCC
                                                               // Trim everything past the legacy header so the DX10 extension is missing.
    bytes.truncate(4 + DDS_HEADER_SIZE);
    let err = parse_dds(&bytes).expect_err("missing DX10 extension must error");
    assert!(format!("{err}").contains("DDS_HEADER_DXT10"));
}

#[test]
fn unsupported_legacy_pixel_format_errors() {
    // Flags = DDPF_FOURCC + unknown fourCC. The parser should yield
    // `Unsupported`, not panic. Pixel format struct sits at file offset
    // 76; size at +0, flags at +4, fourCC at +8.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    bytes[76..80].copy_from_slice(&32u32.to_le_bytes()); // pixfmt size
    bytes[80..84].copy_from_slice(&DDPF_FOURCC.to_le_bytes()); // pixfmt flags
    bytes[84..88].copy_from_slice(&u32::from_le_bytes(*b"ZZZZ").to_le_bytes()); // pixfmt fourCC
    let err = parse_dds(&bytes).expect_err("unsupported fourCC must error");
    assert!(format!("{err}").contains("unsupported"));
}

#[test]
fn unsupported_dxgi_format_errors() {
    // DX10 cubemap with a DXGI format the resolver intentionally refuses
    // to lay out (R32G32B32A32_Float — HDR float, not on the supported
    // table). The parser yields Unsupported.
    let mut bytes = build_dx10_cubemap_bc1(4, 1, 1);
    let dxt10_off = 4 + DDS_HEADER_SIZE;
    bytes[dxt10_off..dxt10_off + 4]
        .copy_from_slice(&DxgiFormat::R32G32B32A32Float.to_u32().to_le_bytes());
    let err = parse_dds(&bytes).expect_err("unsupported DXGI format must error");
    assert!(format!("{err}").contains("unsupported"));
}

#[test]
fn truncated_payload_one_byte_short_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    bytes.pop();
    let err = parse_dds(&bytes).expect_err("payload short by 1 must error");
    assert!(format!("{err}").contains("truncated"));
}

#[test]
fn truncated_payload_to_header_only_errors() {
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    bytes.truncate(4 + DDS_HEADER_SIZE);
    let err = parse_dds(&bytes).expect_err("no payload must error");
    assert!(format!("{err}").contains("truncated"));
}

#[test]
fn forged_huge_mip_count_legacy_errors_gracefully() {
    // DDSD_MIPMAPCOUNT set + mip_map_count = 2^20 (1M). Width 4 / height 4
    // → mip 0 ≥ payload. The parser must surface "truncated" not panic /
    // OOM-alloc.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    let flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_MIPMAPCOUNT;
    bytes[8..12].copy_from_slice(&flags.to_le_bytes());
    bytes[28..32].copy_from_slice(&(1u32 << 20).to_le_bytes()); // mip_map_count
    let res = parse_dds(&bytes);
    assert!(res.is_err(), "huge mip count must error, got {res:?}");
}

#[test]
fn forged_max_mip_count_legacy_errors_gracefully() {
    // u32::MAX mip count. The parser must not panic when computing
    // `0..mip_count` mip_dims allocation or summing volumetric
    // surface-per-slice counts.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    let flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_MIPMAPCOUNT;
    bytes[8..12].copy_from_slice(&flags.to_le_bytes());
    bytes[28..32].copy_from_slice(&u32::MAX.to_le_bytes());
    let res = parse_dds(&bytes);
    assert!(res.is_err(), "u32::MAX mip count must error, got {res:?}");
}

#[test]
fn forged_huge_array_size_dx10_errors_gracefully() {
    // DX10 array_size = u32::MAX. Combined with 1 mip × 1 face the surface
    // count is u32::MAX. The parser must not preallocate the resulting
    // multi-billion-entry `Vec<DdsSurface>`.
    let mut bytes = build_dx10_cubemap_bc1(4, 1, 1);
    // Clear cube bit so face_count = 1.
    let dxt10_off = 4 + DDS_HEADER_SIZE;
    bytes[dxt10_off + 8..dxt10_off + 12].copy_from_slice(&0u32.to_le_bytes()); // misc_flag = 0
    bytes[dxt10_off + 12..dxt10_off + 16].copy_from_slice(&u32::MAX.to_le_bytes()); // array_size
    let res = parse_dds(&bytes);
    assert!(res.is_err(), "huge array_size must error, got {res:?}");
}

#[test]
fn forged_huge_cubemap_array_dx10_errors_gracefully() {
    // DX10 cubemap with array_size = u32::MAX → surface_count = 6 ×
    // mip_count × u32::MAX. Multiplication must be checked.
    let mut bytes = build_dx10_cubemap_bc1(4, 1, 1);
    let dxt10_off = 4 + DDS_HEADER_SIZE;
    bytes[dxt10_off + 12..dxt10_off + 16].copy_from_slice(&u32::MAX.to_le_bytes());
    let res = parse_dds(&bytes);
    assert!(
        res.is_err(),
        "huge cubemap array_size must error, got {res:?}"
    );
}

#[test]
fn forged_volume_with_huge_depth_errors_gracefully() {
    // Legacy volume header with depth = u32::MAX. The parser computes
    // mip_depth slice counts; an u32::MAX slice loop must not run. caps2
    // sits at file offset 4 (magic) + 108 (header up to caps) = 112.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    let flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_DEPTH;
    bytes[8..12].copy_from_slice(&flags.to_le_bytes());
    bytes[24..28].copy_from_slice(&u32::MAX.to_le_bytes()); // depth
    bytes[112..116].copy_from_slice(&DDSCAPS2_VOLUME.to_le_bytes()); // caps2
    let res = parse_dds(&bytes);
    assert!(res.is_err(), "huge depth must error, got {res:?}");
}

#[test]
fn volume_and_cubemap_combined_errors() {
    // DX10 header asking for TEXTURE3D + cubemap misc flag.
    let mut bytes = build_dx10_cubemap_bc1(4, 1, 1);
    let dxt10_off = 4 + DDS_HEADER_SIZE;
    bytes[dxt10_off + 4..dxt10_off + 8].copy_from_slice(&DDS_DIMENSION_TEXTURE3D.to_le_bytes());
    // Need DDSD_DEPTH flag too so the parser walks the volume path.
    let flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT | DDSD_DEPTH;
    bytes[8..12].copy_from_slice(&flags.to_le_bytes());
    bytes[24..28].copy_from_slice(&4u32.to_le_bytes()); // depth
    bytes[112..116].copy_from_slice(&DDSCAPS2_VOLUME.to_le_bytes()); // caps2 mirrors volume
    let err = parse_dds(&bytes).expect_err("volume+cubemap must error");
    let msg = format!("{err}");
    assert!(
        msg.contains("volume") || msg.contains("cubemap"),
        "unexpected error: {msg}"
    );
}

#[test]
fn forged_huge_width_height_errors_gracefully() {
    // Width and height both = u32::MAX. width * height * 4 overflows
    // even u64; the parser must surface a layout error rather than
    // panic on an overflowed multiplication.
    let mut bytes = build_legacy_argb(4, 4, 0xaa);
    bytes[12..16].copy_from_slice(&u32::MAX.to_le_bytes()); // height
    bytes[16..20].copy_from_slice(&u32::MAX.to_le_bytes()); // width
    let res = parse_dds(&bytes);
    assert!(res.is_err(), "huge width × height must error, got {res:?}");
}

#[test]
fn random_garbage_after_valid_magic_does_not_panic() {
    // Magic is good, header field bits are random — every code path the
    // parser walks must either Ok or Err; never panic.
    for seed in 0..32u8 {
        let mut bytes = vec![0u8; 4 + DDS_HEADER_SIZE + 256];
        bytes[0..4].copy_from_slice(&DDS_MAGIC.to_le_bytes());
        for (i, b) in bytes[4..].iter_mut().enumerate() {
            *b = (i as u8).wrapping_mul(seed).wrapping_add(seed);
        }
        // We don't assert on the Result — only that we return. A panic
        // here would fail the test.
        let _ = parse_dds(&bytes);
    }
}

#[test]
fn fully_random_bytes_do_not_panic() {
    // Same idea without the magic guarantee — these mostly bail out on
    // the magic check, but the test confirms a bad magic doesn't reach
    // further code paths that might panic.
    for seed in 0..16u8 {
        let bytes: Vec<u8> = (0..512u16)
            .map(|i| (i as u8).wrapping_mul(seed).wrapping_add(0x55))
            .collect();
        let _ = parse_dds(&bytes);
    }
}

// ---------------------------------------------------------------------------
// BC4 / BC5 input underflow — `decode_bc4_unorm` / `decode_bc4_snorm` /
// `decode_bc5_unorm` / `decode_bc5_snorm` must Err on short input.
// ---------------------------------------------------------------------------

#[test]
fn decode_bc1_short_input_errors() {
    // 4×4 = 1 block = 8 bytes; give it 7.
    let input = vec![0u8; 7];
    let mut out = vec![0u8; 4 * 4 * 4];
    let err = decode_bc1(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC1 input"));
}

#[test]
fn decode_bc1_short_output_errors() {
    let input = vec![0u8; 8];
    // 4×4×4 = 64 bytes; give it 63.
    let mut out = vec![0u8; 63];
    let err = decode_bc1(&input, 4, 4, &mut out).expect_err("short output must error");
    assert!(format!("{err}").contains("BC1 output"));
}

#[test]
fn decode_bc2_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 4];
    let err = decode_bc2(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC2 input"));
}

#[test]
fn decode_bc3_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 4];
    let err = decode_bc3(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC3 input"));
}

#[test]
fn decode_bc4_unorm_short_input_errors() {
    let input = vec![0u8; 7];
    let mut out = vec![0u8; 4 * 4];
    let err = decode_bc4_unorm(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC4 input"));
}

#[test]
fn decode_bc4_snorm_short_input_errors() {
    let input = vec![0u8; 7];
    let mut out = vec![0u8; 4 * 4];
    let err = decode_bc4_snorm(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC4 input"));
}

#[test]
fn decode_bc4_unorm_short_output_errors() {
    let input = vec![0u8; 8];
    let mut out = vec![0u8; 15];
    let err = decode_bc4_unorm(&input, 4, 4, &mut out).expect_err("short output must error");
    assert!(format!("{err}").contains("BC4 output"));
}

#[test]
fn decode_bc5_unorm_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 2];
    let err = decode_bc5_unorm(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC5 input"));
}

#[test]
fn decode_bc5_snorm_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 2];
    let err = decode_bc5_snorm(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC5 input"));
}

#[test]
fn decode_bc7_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 4];
    let err = decode_bc7(&input, 4, 4, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC7 input"));
}

#[test]
fn decode_bc7_short_output_errors() {
    let input = vec![0u8; 16];
    let mut out = vec![0u8; 63];
    let err = decode_bc7(&input, 4, 4, &mut out).expect_err("short output must error");
    assert!(format!("{err}").contains("BC7 output"));
}

#[test]
fn decode_bc6h_short_input_errors() {
    let input = vec![0u8; 15];
    let mut out = vec![0u8; 4 * 4 * 8];
    let err = decode_bc6h(&input, 4, 4, false, &mut out).expect_err("short input must error");
    assert!(format!("{err}").contains("BC6H input"));
}

#[test]
fn decode_bc6h_short_output_errors() {
    let input = vec![0u8; 16];
    let mut out = vec![0u8; 127];
    let err = decode_bc6h(&input, 4, 4, false, &mut out).expect_err("short output must error");
    assert!(format!("{err}").contains("BC6H output"));
}

#[test]
fn decode_bc1_non_multiple_of_4_dimensions_padded_ok() {
    // 5×3 surface → 2×1 = 2 blocks of 8 bytes each. Output buffer must
    // be width*height*4 = 60 bytes. Just confirms we don't fall off the
    // edge handling odd dims.
    let input = vec![0xff; 16];
    let mut out = vec![0u8; 5 * 3 * 4];
    decode_bc1(&input, 5, 3, &mut out).expect("padded decode ok");
}

#[test]
fn round_trip_full_dx10_cubemap_parses() {
    // Sanity check that build_dx10_cubemap_bc1 produces a parsable file
    // when not perturbed. Anchors the "everything else is a perturbation"
    // story for the test file.
    let bytes = build_dx10_cubemap_bc1(4, 1, 1);
    let img = parse_dds(&bytes).expect("good cubemap parses");
    assert!(img.is_cubemap);
    assert_eq!(img.array_size, 1);
    assert_eq!(img.surfaces.len(), 6);
    assert!(img.has_dxt10_header);
}

#[test]
fn round_trip_good_legacy_argb_parses() {
    let bytes = build_legacy_argb(4, 3, 0x42);
    let img = parse_dds(&bytes).expect("good argb parses");
    assert_eq!(img.width, 4);
    assert_eq!(img.height, 3);
    assert!(!img.is_cubemap);
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(img.surfaces[0].plane.data.len(), 4 * 3 * 4);
}

// ---------------------------------------------------------------------------
// Pixel format fourCC = DXT1 with too-short payload — typical
// container-level injection.
// ---------------------------------------------------------------------------

#[test]
fn legacy_dxt1_truncated_block_payload_errors() {
    // Header claims 8×8 BC1 (4 × 8 = 32 byte payload) but we give 16
    // bytes. parse_dds must Err.
    let mut bytes = Vec::new();
    push_u32(&mut bytes, DDS_MAGIC);
    push_u32(&mut bytes, DDS_HEADER_SIZE as u32);
    push_u32(
        &mut bytes,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT,
    );
    push_u32(&mut bytes, 8); // height
    push_u32(&mut bytes, 8); // width
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    for _ in 0..11 {
        push_u32(&mut bytes, 0);
    }
    push_u32(&mut bytes, 32);
    push_u32(&mut bytes, DDPF_FOURCC);
    push_u32(&mut bytes, FOURCC_DXT1);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, DDSCAPS_TEXTURE);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    push_u32(&mut bytes, 0);
    // Only 16 of the required 32 payload bytes.
    bytes.extend(vec![0xaa; 16]);
    let err = parse_dds(&bytes).expect_err("DXT1 truncated payload must error");
    assert!(format!("{err}").contains("truncated"));
}

// ---------------------------------------------------------------------------
// Round 176: BC block-grid overflow regressions.
//
// `decode_bc6h` / `decode_bc7` / `decode_bc{1..=5}` were panicking with
// `panic_const_mul_overflow` when the caller supplied
// `width = height = u32::MAX`: the ceil-div block-grid produced a
// `usize × usize × 16` product that wrapped the `usize::MAX` boundary
// before the `input.len() < want_in` length check ran. The fuzz
// harness's "Adversarial: extreme dimensions" probe surfaced the crash
// on three targets simultaneously
// (`decode_bcn` / `decode_bc6h` / `decode_bc7`). Each test below
// reproduces the fuzz input shape — short input slice + tiny output
// slice + maximal dimensions — and asserts the decoder returns
// `Err`, never panics.
// ---------------------------------------------------------------------------

#[test]
fn decode_bc1_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc1(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc2_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc2(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc3_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc3(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc4_unorm_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc4_unorm(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc4_snorm_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc4_snorm(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc5_unorm_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc5_unorm(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc5_snorm_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc5_snorm(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc6h_unsigned_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc6h(&input, u32::MAX, u32::MAX, false, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc6h_signed_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc6h(&input, u32::MAX, u32::MAX, true, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc7_max_dimensions_does_not_panic() {
    let input = [0u8; 0];
    let mut tiny = [0u8; 4];
    let res = decode_bc7(&input, u32::MAX, u32::MAX, &mut tiny);
    assert!(res.is_err());
}

#[test]
fn decode_bc6h_fuzz_crash_ebc0c3370c_does_not_panic() {
    // Verbatim reproduction of the fuzz crash artifact
    // `decode_bc6h/crash-ebc0c3370c96b4245e1a2c01efdaaa7a9165213a`
    // surfaced by the daily fuzz workflow on 2026-05-28.
    // First 4 bytes drive width / height: w = 1+(0x0004) % 256 + 1 = 6,
    // h = 1+(0x0004) % 256 + 1 = 6. The harness then probes
    // `(width = height = u32::MAX, tiny = [0;4])` which is the path
    // that overflowed before this round.
    let data: &[u8] = &[
        0x04, 0x00, 0x04, 0x00, 0x0f, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00,
    ];
    let rest = &data[4..];
    let mut tiny = [0u8; 4];
    let _ = decode_bc6h(rest, u32::MAX, u32::MAX, false, &mut tiny);
    let _ = decode_bc6h(rest, u32::MAX, u32::MAX, true, &mut tiny);
}

#[test]
fn decode_bc7_fuzz_crash_c382ab7c10_does_not_panic() {
    // Verbatim reproduction of
    // `decode_bc7/crash-c382ab7c100b0ccc80b8e0bbd9c56e725acee627`.
    let data: &[u8] = &[
        0x04, 0x00, 0x04, 0x00, 0x40, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00,
    ];
    let rest = &data[4..];
    let mut tiny = [0u8; 4];
    let _ = decode_bc7(rest, u32::MAX, u32::MAX, &mut tiny);
}

#[test]
fn decode_bcn_fuzz_crash_3d19281e55_does_not_panic() {
    // Verbatim reproduction of
    // `decode_bcn/crash-3d19281e55d94671c76bcc859247014a5303e9aa`.
    let data: &[u8] = &[
        0x04, 0x00, 0x04, 0x00, 0x00, 0xf8, 0x00, 0xf8, 0x00, 0x00, 0x00, 0x00,
    ];
    let rest = &data[4..];
    let mut tiny = [0u8; 4];
    let _ = decode_bc1(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc2(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc3(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc4_unorm(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc5_unorm(rest, u32::MAX, u32::MAX, &mut tiny);
}
