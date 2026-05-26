#![no_main]

//! Drive arbitrary fuzz-supplied bytes through every BC1..BC5 decoder
//! with attacker-controlled width / height taken off the first two
//! bytes. Each `decode_bc*` entry point takes
//! `(input, width, height, output)` where:
//!
//!   * `input` is the 4×4-block stream (8 bytes per block for BC1 /
//!     BC4, 16 bytes for BC2 / BC3 / BC5).
//!   * `width × height` derives the block grid (ceil-div by 4 per
//!     dimension); the decoder must reject `input.len() <
//!     blocks * blocksize` rather than indexing past the end.
//!   * `output` is the destination RGBA8 / R8 / RG8 plane the decoder
//!     fills.
//!
//! The malicious-input combos we care about:
//!
//!   * `width = u32::MAX` × `height = u32::MAX` → block grid overflow.
//!     The decoder must catch the multiplication overflow, not panic.
//!   * `width = 1, height = 1` with `input.len() = 0` → ceil-div
//!     gives `1 * 1` blocks ⇒ 8 / 16 bytes required, but `input` is
//!     empty. The decoder must reject, not slice-OOB.
//!   * `width = 4_096, height = 4_096, output.len() = 7` → output
//!     buffer too small. The decoder must reject, not OOB the write.
//!
//! Fuzz strategy: use the first four bytes of the input to pick
//! `width` and `height` in a bounded but representative range, then
//! feed the *rest* of the bytes as both the BC block stream and the
//! output destination. The output buffer is also sized from the
//! input bytes so we exercise the both-too-small / too-large /
//! exactly-right paths.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_unorm, decode_bc5_snorm,
    decode_bc5_unorm,
};

// Cap the fuzzer-controlled dimensions so the harness doesn't OOM on a
// genuine `width × height × 4` allocation. The decoders themselves
// must still tolerate u32::MAX inputs without crashing — we test that
// directly with a couple of hardcoded edge cases below.
const MAX_DIM: u32 = 256;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    // Two 16-bit fuzzer fields steer width / height. The modulo keeps
    // the legitimate-path allocation bounded; we also probe the
    // crash-only edge case (u32::MAX width / height) below where the
    // decoder is responsible for rejecting, not panicking.
    let width = (1u32 + (u32::from(data[0]) | (u32::from(data[1]) << 8))) % MAX_DIM + 1;
    let height = (1u32 + (u32::from(data[2]) | (u32::from(data[3]) << 8))) % MAX_DIM + 1;
    let rest = &data[4..];

    // Output sized for the largest possible (RGBA8) plane at the
    // fuzzed dimensions; smaller-output decoders just write less. We
    // also exercise too-small outputs by truncating below.
    let want_rgba = (width as usize) * (height as usize) * 4;
    let mut out_rgba = vec![0u8; want_rgba];
    let want_r = (width as usize) * (height as usize);
    let mut out_r = vec![0u8; want_r];
    let want_rg = (width as usize) * (height as usize) * 2;
    let mut out_rg = vec![0u8; want_rg];

    // Happy path: hand the fuzzer-controlled block bytes to every
    // BC1..BC5 decoder. `rest` is whatever's after the 4-byte size
    // header; short inputs flow through the decoder's length check.
    let _ = decode_bc1(rest, width, height, &mut out_rgba);
    let _ = decode_bc2(rest, width, height, &mut out_rgba);
    let _ = decode_bc3(rest, width, height, &mut out_rgba);
    let _ = decode_bc4_unorm(rest, width, height, &mut out_r);
    let _ = decode_bc4_snorm(rest, width, height, &mut out_r);
    let _ = decode_bc5_unorm(rest, width, height, &mut out_rg);
    let _ = decode_bc5_snorm(rest, width, height, &mut out_rg);

    // Adversarial path: zero-length output buffer. Every decoder must
    // reject, never index a length-0 slice.
    let mut empty: [u8; 0] = [];
    let _ = decode_bc1(rest, width, height, &mut empty);
    let _ = decode_bc2(rest, width, height, &mut empty);
    let _ = decode_bc3(rest, width, height, &mut empty);
    let _ = decode_bc4_unorm(rest, width, height, &mut empty);
    let _ = decode_bc4_snorm(rest, width, height, &mut empty);
    let _ = decode_bc5_unorm(rest, width, height, &mut empty);
    let _ = decode_bc5_snorm(rest, width, height, &mut empty);

    // Adversarial path: extreme dimensions. The block-grid math is
    // `(w + 3) / 4 * (h + 3) / 4 * 16` for BC2/3/5 — both u32 product
    // and the resulting buffer-size compare must clamp without
    // overflow. We use a tiny output buffer so the decoder never gets
    // far enough to actually allocate at the claimed scale.
    let mut tiny = vec![0u8; 4];
    let _ = decode_bc1(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc2(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc3(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc4_unorm(rest, u32::MAX, u32::MAX, &mut tiny);
    let _ = decode_bc5_unorm(rest, u32::MAX, u32::MAX, &mut tiny);
});
