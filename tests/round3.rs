//! Round-3 features: BC1 encoder + BC7 decoder + container demuxer/muxer.
//!
//! Covers the new public surface added in round 3:
//! * [`oxideav_dds::encode_bc1`] — BC1 encoder, validated via
//!   roundtrip through [`oxideav_dds::decode_bc1`].
//! * [`oxideav_dds::decode_bc7`] — BC7 decoder, exercised against a
//!   hand-constructed mode-6 block (the most common single-subset BC7
//!   layout, used by virtually every modern texture-compression
//!   pipeline for opaque content).
//! * `.dds` container probe / extension / demuxer (round-3 lift over
//!   the round-2 extension-only registration).

use oxideav_dds::types::{
    DDPF_FOURCC, DDSCAPS_TEXTURE, DDSD_REQUIRED, DDS_DIMENSION_TEXTURE2D, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE, FOURCC_DX10,
};
use oxideav_dds::{decode_bc1, decode_bc7, encode_bc1, parse_dds, DdsPixelFormat};

fn build_fourcc_dds(four_cc: u32, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
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
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&four_cc.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // rgb_bit_count
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(payload);
    out
}

#[test]
fn bc1_encode_solid_block_then_decode_matches() {
    // Solid red 4x4 RGBA8.
    let mut input = vec![0u8; 4 * 4 * 4];
    for chunk in input.chunks_exact_mut(4) {
        chunk[0] = 0xff;
        chunk[3] = 0xff;
    }
    let mut bc = vec![0u8; 8];
    encode_bc1(&input, 4, 4, false, &mut bc).expect("encode_bc1");

    // Decode back through the BC1 decoder. RGB565 red 0xf800 expands to
    // (255, 0, 0) per Microsoft's bit-replication rule.
    let mut out = vec![0u8; 4 * 4 * 4];
    decode_bc1(&bc, 4, 4, &mut out).expect("decode_bc1");
    for chunk in out.chunks_exact(4) {
        assert_eq!(chunk, &[255, 0, 0, 255]);
    }
}

#[test]
fn bc1_encode_then_dds_wrap_then_parse() {
    // 4x4 white block → encode to BC1 → wrap in a DDS file (FourCC
    // DXT1) → parse_dds → recover the same bytes.
    let input = vec![0xffu8; 4 * 4 * 4];
    let mut bc = vec![0u8; 8];
    encode_bc1(&input, 4, 4, false, &mut bc).unwrap();

    // FOURCC_DXT1 — re-export not needed; build it inline:
    let four_cc_dxt1 = u32::from_le_bytes(*b"DXT1");
    let dds = build_fourcc_dds(four_cc_dxt1, 4, 4, &bc);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(img.surfaces.len(), 1);
    assert_eq!(&img.surfaces[0].plane.data, &bc);

    // Decode and assert white.
    let mut out = vec![0u8; 4 * 4 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 4, 4, &mut out).unwrap();
    for chunk in out.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// Wrap a single block payload in a DX10-extension DDS file with the
/// supplied DXGI format (e.g. `98` = `DXGI_FORMAT_BC7_UNORM`).
fn build_dx10_dds(dxgi: u32, w: u32, h: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + DDS_HEADER_DXT10_SIZE + payload.len());
    out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
    out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDSD_REQUIRED.to_le_bytes());
    out.extend_from_slice(&h.to_le_bytes());
    out.extend_from_slice(&w.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    for _ in 0..11 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
    out.extend_from_slice(&DDPF_FOURCC.to_le_bytes());
    out.extend_from_slice(&FOURCC_DX10.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
    for _ in 0..4 {
        out.extend_from_slice(&0u32.to_le_bytes());
    }
    // DXT10 extension.
    out.extend_from_slice(&dxgi.to_le_bytes());
    out.extend_from_slice(&DDS_DIMENSION_TEXTURE2D.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flag
    out.extend_from_slice(&1u32.to_le_bytes()); // array_size = 1
    out.extend_from_slice(&0u32.to_le_bytes()); // misc_flags2
    out.extend_from_slice(payload);
    out
}

#[test]
fn bc7_dds_wrap_with_mode6_solid_white_block() {
    // Hand-build a mode-6 BC7 block whose endpoints are both opaque
    // white and indices are all zero. Then wrap it in a DDS file via
    // the DXT10 extension (DXGI_FORMAT_BC7_UNORM = 98) and parse
    // through the full pipeline.
    let mut block = [0u8; 16];

    // Mode 6: bit 6 = 1, bits 0..5 = 0.
    block[0] = 1 << 6;

    // After the 7-bit mode prefix we have: 7-bit R0, R1, G0, G1, B0,
    // B1, A0, A1 (6 colour + 2 alpha = 8 endpoint slots × 7 bits =
    // 56 bits) followed by 2 p-bits and then 63 index bits.
    //
    // We write all-1s for the 56 endpoint bits and both p-bits, so
    // every endpoint expands to 0xFF. Indices stay at 0 → every pixel
    // = e0 = white.
    let mut pos = 7usize; // skip mode prefix
    for _ in 0..56 {
        let byte = pos / 8;
        let shift = pos & 7;
        block[byte] |= 1u8 << shift;
        pos += 1;
    }
    // 2 p-bits.
    for _ in 0..2 {
        let byte = pos / 8;
        let shift = pos & 7;
        block[byte] |= 1u8 << shift;
        pos += 1;
    }

    let dds = build_dx10_dds(98 /* BC7_UNORM */, 4, 4, &block);
    let img = parse_dds(&dds).unwrap();
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc7Unorm);

    let mut out = vec![0u8; 4 * 4 * 4];
    decode_bc7(&img.surfaces[0].plane.data, 4, 4, &mut out).unwrap();
    for chunk in out.chunks_exact(4) {
        assert_eq!(chunk, &[255, 255, 255, 255]);
    }
}

/// The container-side probe + demuxer + muxer round-trip — exercises
/// the new framework-side surface registered by
/// [`oxideav_dds::registry::register_containers`].
#[cfg(feature = "registry")]
mod container_tests {
    use std::io::Cursor;

    use oxideav_core::{
        ContainerRegistry, NullCodecResolver, ProbeData, ReadSeek, MAX_PROBE_SCORE,
    };

    use oxideav_dds::registry::register_containers;

    fn build_dds_a8r8g8b8(w: u32, h: u32) -> Vec<u8> {
        use oxideav_dds::types::{
            DDPF_ALPHAPIXELS, DDPF_RGB, DDSCAPS_TEXTURE, DDSD_PITCH, DDSD_REQUIRED,
            DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
        };
        let pitch = w * 4;
        let data_len = (pitch as usize) * (h as usize);
        let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + data_len);
        out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
        out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&(DDSD_REQUIRED | DDSD_PITCH).to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&pitch.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        for _ in 0..11 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&(DDPF_RGB | DDPF_ALPHAPIXELS).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&0x00ff_0000u32.to_le_bytes());
        out.extend_from_slice(&0x0000_ff00u32.to_le_bytes());
        out.extend_from_slice(&0x0000_00ffu32.to_le_bytes());
        out.extend_from_slice(&0xff00_0000u32.to_le_bytes());
        out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
        for _ in 0..4 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend(vec![0u8; data_len]);
        out
    }

    #[test]
    fn container_registry_resolves_dds_extension_and_probe() {
        let mut reg = ContainerRegistry::new();
        register_containers(&mut reg);

        // Extension lookup still works (round-2 surface).
        assert_eq!(reg.container_for_extension("dds"), Some("dds"));

        // Probe-input recognises the magic at the head of a real file.
        let bytes = build_dds_a8r8g8b8(4, 4);
        let mut cursor: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes));
        let name = reg
            .probe_input(&mut *cursor, Some("dds"))
            .expect("probe_input recognises DDS magic");
        assert_eq!(name, "dds");
    }

    #[test]
    fn container_demuxer_emits_one_packet() {
        let mut reg = ContainerRegistry::new();
        register_containers(&mut reg);

        let bytes = build_dds_a8r8g8b8(8, 4);
        let len = bytes.len();
        let cursor: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes));
        let mut dx = reg
            .open_demuxer("dds", cursor, &NullCodecResolver)
            .expect("open_demuxer");
        let pkt = dx.next_packet().expect("first packet");
        assert_eq!(pkt.data.len(), len);
        assert!(pkt.flags.keyframe);
        assert!(matches!(dx.next_packet(), Err(oxideav_core::Error::Eof)));
    }

    #[test]
    fn probe_score_is_max_for_dds_magic() {
        let bytes = build_dds_a8r8g8b8(4, 4);
        let s = oxideav_dds::container::probe(&ProbeData {
            buf: &bytes,
            ext: None,
        });
        assert_eq!(s, MAX_PROBE_SCORE);
    }
}
