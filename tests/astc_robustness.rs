//! Panic-free / bounds-safe property tests for the ASTC LDR decoder.
//!
//! These tests sweep arbitrary block bytes across every LDR footprint
//! and every surface-tiling edge case to confirm the decoder never
//! panics and always returns exactly the expected number of texels /
//! bytes. They complement the in-tree unit tests in `src/astc.rs` and
//! the `decode_astc` cargo-fuzz target by running inside CI.
//!
//! Source of truth for the footprint list and the error-colour rule is
//! `docs/image/astc/astc-ldr-notes.md` (Khronos DFS 1.4 ch.23). No
//! external ASTC tooling is consulted.

use oxideav_dds::{
    decode_astc_ldr, decode_astc_ldr_block, decode_astc_ldr_surface, DdsPixelFormat,
    ASTC_ERROR_COLOR, LDR_BLOCK_FOOTPRINTS,
};

/// A small xorshift PRNG so the sweep is deterministic and dependency-free.
struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn fill(&mut self, b: &mut [u8; 16]) {
        for chunk in b.chunks_mut(8) {
            let v = self.next().to_le_bytes();
            chunk.copy_from_slice(&v[..chunk.len()]);
        }
    }
}

#[test]
fn every_footprint_block_decode_is_panic_free_and_sized() {
    let mut rng = Rng(0xDEAD_BEEF_CAFE_F00D);
    for &(bw, bh) in &LDR_BLOCK_FOOTPRINTS {
        let (bw, bh) = (bw as u32, bh as u32);
        for _ in 0..5000 {
            let mut block = [0u8; 16];
            rng.fill(&mut block);
            let texels = decode_astc_ldr_block(&block, bw, bh);
            assert_eq!(texels.len(), (bw * bh) as usize);
            // Every texel is a 4-byte RGBA value (trivially true by type,
            // but assert the alpha is a valid byte for good measure).
            for t in &texels {
                let _ = t[3];
            }
        }
    }
}

#[test]
fn invalid_footprints_yield_error_colour() {
    // Footprints not in the LDR table must produce the error colour for
    // every texel (and never panic).
    for &(bw, bh) in &[(7u32, 7u32), (3, 3), (16, 16), (1, 1), (13, 13)] {
        let block = [0xABu8; 16];
        let texels = decode_astc_ldr_block(&block, bw, bh);
        assert_eq!(texels.len(), (bw * bh) as usize);
        for t in texels {
            assert_eq!(t, ASTC_ERROR_COLOR);
        }
    }
}

#[test]
fn surface_tiling_clips_partial_edge_blocks() {
    // Sweep non-multiple-of-footprint surface sizes for several
    // footprints; the output is always width*height*4 and the decode
    // never panics regardless of the (arbitrary) payload.
    let mut rng = Rng(0x0123_4567_89AB_CDEF);
    for &(bw, bh) in &[(4u8, 4u8), (5, 4), (6, 5), (8, 8), (10, 6), (12, 12)] {
        let (bw, bh) = (bw as u32, bh as u32);
        for w in [1u32, bw - 1, bw, bw + 1, 2 * bw + 3] {
            for h in [1u32, bh - 1, bh, bh + 1, 2 * bh + 1] {
                let blocks = w.div_ceil(bw) * h.div_ceil(bh);
                let mut payload = vec![0u8; (blocks * 16) as usize];
                for byte in payload.iter_mut() {
                    *byte = rng.next() as u8;
                }
                let out = decode_astc_ldr(&payload, w, h, bw, bh);
                assert_eq!(out.len(), (w * h * 4) as usize);
            }
        }
    }
}

#[test]
fn missing_blocks_decode_to_error_colour() {
    // Surface claims more blocks than the payload supplies; the missing
    // trailing blocks must decode to the error colour, never OOB.
    let out = decode_astc_ldr(&[], 8, 8, 4, 4);
    assert_eq!(out.len(), 8 * 8 * 4);
    for px in out.chunks_exact(4) {
        assert_eq!(px, ASTC_ERROR_COLOR);
    }
}

#[test]
fn format_wrapper_rejects_non_astc() {
    let data = [0u8; 16];
    assert!(decode_astc_ldr_surface(DdsPixelFormat::Bc7Unorm, &data, 4, 4).is_none());
    assert!(decode_astc_ldr_surface(DdsPixelFormat::A8R8G8B8, &data, 4, 4).is_none());
    let pix = DdsPixelFormat::Astc {
        block_w: 6,
        block_h: 6,
        srgb: false,
    };
    let out = decode_astc_ldr_surface(pix, &data, 6, 6).expect("astc");
    assert_eq!(out.len(), 6 * 6 * 4);
}

#[test]
fn zero_dimension_surface_is_empty() {
    // A zero-width or zero-height surface yields an empty buffer without
    // panicking (the container layer rejects these earlier, but the raw
    // decoder must still be safe).
    assert!(decode_astc_ldr(&[0u8; 16], 0, 4, 4, 4).is_empty());
    assert!(decode_astc_ldr(&[0u8; 16], 4, 0, 4, 4).is_empty());
}

#[test]
fn block_mode_field_sweep_is_panic_free() {
    // Exhaustively sweep all 2^11 block-mode fields (in the low 11 bits)
    // with otherwise-zero bytes across a few footprints. This guarantees
    // every block-mode classification branch (normal rows, void-extent,
    // reserved) is hit without a crash.
    for &(bw, bh) in &[(4u8, 4u8), (8, 8), (12, 12), (10, 5)] {
        let (bw, bh) = (bw as u32, bh as u32);
        for mode in 0u32..2048 {
            let mut block = [0u8; 16];
            block[0] = mode as u8;
            block[1] = (mode >> 8) as u8;
            let texels = decode_astc_ldr_block(&block, bw, bh);
            assert_eq!(texels.len(), (bw * bh) as usize);
        }
    }
}
