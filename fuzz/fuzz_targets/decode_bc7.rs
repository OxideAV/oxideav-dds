#![no_main]

//! BC7 has 8 modes (0..7) covering 1- / 2- / 3-subset partitions,
//! optional channel rotation (modes 4 / 5), per-endpoint p-bits, and a
//! secondary alpha index plane (modes 4 / 5). The mode is signalled by
//! the lowest 1..8 bits of the block as a unary prefix — a hostile
//! fuzz block can therefore make the decoder consume up to 8 bits of
//! `0` before the mode index resolves, which is a useful stress case
//! for the bit-reader bounds check.
//!
//! Reserved mode (mode 8 — eight leading zero bits) decodes to a
//! cleared 4×4 block per Direct3D 11 spec, *not* an error. The harness
//! verifies that doesn't panic for any input.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::decode_bc7;

const MAX_DIM: u32 = 256;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let width = (1u32 + (u32::from(data[0]) | (u32::from(data[1]) << 8))) % MAX_DIM + 1;
    let height = (1u32 + (u32::from(data[2]) | (u32::from(data[3]) << 8))) % MAX_DIM + 1;
    let rest = &data[4..];

    // BC7 expands to RGBA8: 4 bytes per output pixel.
    let want = (width as usize) * (height as usize) * 4;
    let mut out = vec![0u8; want];
    let _ = decode_bc7(rest, width, height, &mut out);

    // Adversarial: too-small destination.
    let mut empty: [u8; 0] = [];
    let _ = decode_bc7(rest, width, height, &mut empty);

    // Adversarial: extreme dimensions. The block-grid product must
    // not overflow the byte-size compare.
    let mut tiny = vec![0u8; 4];
    let _ = decode_bc7(rest, u32::MAX, u32::MAX, &mut tiny);
});
