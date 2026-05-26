#![no_main]

//! BC6H is the richest BC-family decoder by panic surface: 14 mode
//! prefixes (with four reserved prefixes that decode to zero RGB
//! rather than erroring), per-mode bit-allocation tables (10.5.5.5 /
//! 7.6.6.6 / 11.5.5.5 / 11.4.4.4 / 11.4.5.4 / 11.4.4.5 / 9.5.5.5 /
//! 8.6.6.6 / 8.5.6.5 / 8.5.5.6 / 6.6.6.6 absolute-two-subset variants
//! and 10.10 / 11.9 / 12.8 / 16.4 one-subset variants), signed and
//! unsigned finalisation paths, and 16-bit half-float output.
//!
//! The harness feeds arbitrary fuzz bytes as a BC6H block stream
//! against both `signed = false` (`BC6H_UF16`) and `signed = true`
//! (`BC6H_SF16`). The decoder must always return a `Result` and never:
//!
//!   * panic on a reserved mode prefix (10011 / 10111 / 11011 / 11111
//!     must decode to zero RGB without erroring).
//!   * shift by ≥ 16 / 32 when extracting fields from a malformed
//!     bit-reader cursor.
//!   * NaN / inf in the half-float finalize step trip the encoder's
//!     `assert!` or `unwrap` on a half-to-half conversion that's
//!     undefined for those bit patterns.
//!   * write past the destination plane when the caller passes
//!     `output.len() < ceil(w/4) * ceil(h/4) * 16 * 2` (RGBA × 2 bytes
//!     per half-float channel × 4 channels = 16 bytes per pixel? no —
//!     8 bytes per pixel: RGBA × 2 = 8).

use libfuzzer_sys::fuzz_target;
use oxideav_dds::decode_bc6h;

const MAX_DIM: u32 = 256;

fuzz_target!(|data: &[u8]| {
    if data.len() < 4 {
        return;
    }
    let width = (1u32 + (u32::from(data[0]) | (u32::from(data[1]) << 8))) % MAX_DIM + 1;
    let height = (1u32 + (u32::from(data[2]) | (u32::from(data[3]) << 8))) % MAX_DIM + 1;
    let rest = &data[4..];

    // BC6H lays out 8 bytes (4 RGBA half-floats) per output pixel.
    let want = (width as usize) * (height as usize) * 8;
    let mut out = vec![0u8; want];

    // Both unsigned + signed finalisation paths.
    let _ = decode_bc6h(rest, width, height, false, &mut out);
    let _ = decode_bc6h(rest, width, height, true, &mut out);

    // Adversarial: too-small destination — every input length must
    // surface as `Err`, never write past the slice.
    let mut tiny = vec![0u8; 4];
    let _ = decode_bc6h(rest, width, height, false, &mut tiny);
    let _ = decode_bc6h(rest, width, height, true, &mut tiny);

    // Adversarial: extreme dimensions with zero-length input. The
    // ceil-div + multiply on `(u32::MAX, u32::MAX)` must clamp, not
    // overflow the block-byte size compare.
    let _ = decode_bc6h(rest, u32::MAX, u32::MAX, false, &mut tiny);
    let _ = decode_bc6h(rest, u32::MAX, u32::MAX, true, &mut tiny);
});
