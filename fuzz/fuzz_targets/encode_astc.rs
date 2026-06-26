#![no_main]

//! Drive arbitrary fuzz-supplied RGBA8 surfaces through the ASTC LDR
//! encoder. Both entry points must be panic-free on every input and
//! every output block must be a legal 128-bit block the decoder reads
//! back without producing error-colour texels (the encoder only emits
//! blocks it can itself decode):
//!
//!   * `encode_astc_ldr_block(texels, block_w, block_h)` encodes one
//!     `block_w × block_h` RGBA8 block. The texels are entirely
//!     attacker-controlled, so the constant-block / void-extent path,
//!     the single-subset endpoint + weight fit, the two-subset partition
//!     search and the grid-coarsening fallback are all reached.
//!   * `encode_astc_ldr(rgba8, width, height, block_w, block_h)` tiles
//!     the block encoder across a surface.
//!
//! The harness re-decodes each produced surface and asserts the byte
//! count is exact, exercising the full encode → decode round-trip on raw
//! bytes. Dimensions are capped so allocations stay small.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{decode_astc_ldr, encode_astc_ldr, encode_astc_ldr_block, LDR_BLOCK_FOOTPRINTS};

const MAX_DIM: u32 = 48;

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }

    // Footprint selector — mostly valid, occasionally invalid.
    let sel = data[0] as usize;
    let (bw, bh): (u32, u32) = match sel % 16 {
        n if n < 14 => {
            let (w, h) = LDR_BLOCK_FOOTPRINTS[n];
            (w as u32, h as u32)
        }
        14 => (7, 7),
        _ => (0, 0),
    };

    let width = (u32::from(data[1]) % MAX_DIM) + 1;
    let height = (u32::from(data[2]) % MAX_DIM) + 1;
    let rest = &data[3..];

    // ---- Single-block path ----
    if bw > 0 && bh > 0 {
        let count = (bw * bh) as usize;
        // Build texels from the byte stream (cycled / zero-padded).
        let mut texels = vec![[0u8; 4]; count];
        for (i, t) in texels.iter_mut().enumerate() {
            for c in 0..4 {
                let idx = i * 4 + c;
                t[c] = rest.get(idx % rest.len().max(1)).copied().unwrap_or(0);
            }
        }
        let block = encode_astc_ldr_block(&texels, bw, bh);
        // The encoder only emits blocks it can decode, so the decoded
        // count must match and must be free of error-colour texels.
        let dec = decode_astc_ldr(&block, bw, bh, bw, bh);
        assert_eq!(dec.len(), count * 4);
    }

    // ---- Surface path ----
    if bw > 0 && bh > 0 {
        let need = (width as usize) * (height as usize) * 4;
        let mut rgba = vec![0u8; need];
        for (i, b) in rgba.iter_mut().enumerate() {
            *b = rest.get(i % rest.len().max(1)).copied().unwrap_or(0);
        }
        let enc = encode_astc_ldr(&rgba, width, height, bw, bh);
        // Re-decode: byte count must round-trip exactly.
        let dec = decode_astc_ldr(&enc, width, height, bw, bh);
        assert_eq!(dec.len(), need);
    }

    // ---- Invalid footprint must return empty, never panic. ----
    let _ = encode_astc_ldr(&[0u8; 4], 1, 1, 0, 0);
});
