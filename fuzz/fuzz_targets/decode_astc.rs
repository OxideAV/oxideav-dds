#![no_main]

//! Drive arbitrary fuzz-supplied bytes through the ASTC LDR block and
//! surface decoders. Both entry points must be panic-free and
//! bounds-safe on every input:
//!
//!   * `decode_astc_ldr_block(&[u8; 16], block_w, block_h)` decodes one
//!     128-bit block. The block bytes are entirely attacker-controlled,
//!     so every block-mode classification, BISE trit/quint unpack,
//!     endpoint/weight ISE, partition hash, dual-plane selection, and
//!     void-extent path is reached from raw bytes. The function must
//!     never panic and must return exactly `block_w * block_h` texels
//!     for a valid footprint.
//!   * `decode_astc_ldr(data, width, height, block_w, block_h)` tiles
//!     blocks across a surface; truncated / missing blocks decode to
//!     the error colour. Width / height are fuzzer-controlled (bounded
//!     so the harness's own RGBA8 allocation stays small); the block
//!     footprint cycles through the 14 valid LDR footprints plus a few
//!     invalid ones.
//!
//! Fuzz strategy: the first byte selects a footprint (mod over the 14
//! LDR footprints, with a couple of invalid sizes mixed in); the next
//! two bytes steer a bounded width / height; the rest is the block /
//! surface byte stream — used both as a single 16-byte block (padded /
//! truncated) and as the multi-block surface payload.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{decode_astc_ldr, decode_astc_ldr_block, LDR_BLOCK_FOOTPRINTS};

// Cap the surface dimensions so `width * height * 4` stays a small
// allocation in the harness. The decoders themselves tolerate larger
// values; the cap is purely to keep the fuzzer fast.
const MAX_DIM: u32 = 64;

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }

    // Footprint selector: most of the time a valid LDR footprint; a
    // fraction of the time a deliberately invalid one (7×7, 3×3, 16×16,
    // 0×0) to exercise the reject-path.
    let sel = data[0] as usize;
    let (bw, bh): (u32, u32) = match sel % 18 {
        n if n < 14 => {
            let (w, h) = LDR_BLOCK_FOOTPRINTS[n];
            (w as u32, h as u32)
        }
        14 => (7, 7),
        15 => (3, 3),
        16 => (16, 16),
        _ => (0, 0),
    };

    let width = (u32::from(data[1]) % MAX_DIM) + 1;
    let height = (u32::from(data[2]) % MAX_DIM) + 1;
    let rest = &data[3..];

    // ---- Single-block path ----
    // Build a 16-byte block from `rest` (zero-padded / truncated). For a
    // valid footprint the result must have exactly bw*bh texels.
    let mut block = [0u8; 16];
    let n = rest.len().min(16);
    block[..n].copy_from_slice(&rest[..n]);
    if bw > 0 && bh > 0 {
        let texels = decode_astc_ldr_block(&block, bw, bh);
        // Only valid footprints are guaranteed bw*bh; invalid footprints
        // still must not panic. The function returns bw*bh for both, so
        // just assert it produced *something* without crashing.
        assert_eq!(texels.len(), (bw as usize) * (bh as usize));
    }

    // ---- Surface path ----
    // Feed the whole stream as the block array. Truncated trailing
    // blocks decode to the error colour; the output is always
    // width*height*4 bytes.
    let out = decode_astc_ldr(rest, width, height, bw, bh);
    assert_eq!(out.len(), (width as usize) * (height as usize) * 4);

    // ---- Adversarial dimensions ----
    // Extreme width with a tiny height keeps the allocation small while
    // exercising the per-block coordinate math at scale. (Capped so the
    // harness allocation stays bounded.)
    let _ = decode_astc_ldr(rest, MAX_DIM, 1, bw.max(4), bh.max(4));
    let _ = decode_astc_ldr(rest, 1, MAX_DIM, bw.max(4), bh.max(4));
});
