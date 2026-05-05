//! Cross-validation against ImageMagick's `convert/magick` DDS
//! decoder.
//!
//! Each fixture in `tests/fixtures/` was produced by ImageMagick
//! 7.1.2 via `magick -size NxN ... -define dds:compression=dxtN
//! -define dds:mipmaps=0 file.dds`. The fixture is parsed through
//! this crate's [`oxideav_dds::parse_dds`] +
//! [`oxideav_dds::decode_bc*`], then the decoded pixels are
//! compared against the value ImageMagick produces when decoding
//! the same file (read out via `magick file.dds txt:-`).
//!
//! ImageMagick is the black-box reference; the fixture bytes plus
//! the expected pixel values are baked in so the tests stay green
//! offline. Workspace policy bars in-tree DXTC reference library
//! code, so we call out to a binary fixture instead.

use oxideav_dds::{decode_bc1, parse_dds, DdsPixelFormat};

const RED16: &[u8] = include_bytes!("fixtures/red16.dds");
const GRAD8: &[u8] = include_bytes!("fixtures/grad8.dds");

#[test]
fn imagemagick_bc1_solid_red_16x16() {
    let img = parse_dds(RED16).expect("parse ImageMagick BC1 red fixture");
    assert_eq!(img.width, 16);
    assert_eq!(img.height, 16);
    assert_eq!(img.pixel_format, DdsPixelFormat::Bc1);
    assert_eq!(img.surfaces.len(), 1);

    let mut rgba = vec![0u8; 16 * 16 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 16, 16, &mut rgba)
        .expect("decode_bc1 over ImageMagick fixture");

    // ImageMagick reports every pixel as (255, 0, 0). BC1's RGB565
    // red endpoint 0xf800 expands to (255, 0, 0) under 5-bit
    // bit-replication ((31<<3)|(31>>2) = 248 + 7 = 255).
    for (i, chunk) in rgba.chunks_exact(4).enumerate() {
        assert_eq!(chunk, &[255, 0, 0, 255], "pixel {i}");
    }
}

#[test]
fn imagemagick_bc1_gradient_8x8_top_row_white() {
    let img = parse_dds(GRAD8).expect("parse ImageMagick gradient fixture");
    assert_eq!(img.width, 8);
    assert_eq!(img.height, 8);
    let mut rgba = vec![0u8; 8 * 8 * 4];
    decode_bc1(&img.surfaces[0].plane.data, 8, 8, &mut rgba)
        .expect("decode_bc1 over ImageMagick fixture");
    // ImageMagick prints (0, 0) ... (7, 0) all as #FFFFFF (255, 255,
    // 255). Block 0 has c0 = 0xffff (white) and c1 = 0x9492 (mid
    // grey), so the index-0 lookup is white.
    for x in 0..8 {
        let off = x * 4;
        assert_eq!(
            &rgba[off..off + 4],
            &[255, 255, 255, 255],
            "pixel ({x}, 0) should be white"
        );
    }
}
