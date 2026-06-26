#![no_main]

//! Panic-free fuzz target for the round-375 encoders:
//!
//!   * `encode_dds_uncompressed_dx10` — DX10-header uncompressed writer.
//!   * `encode_dds_volume_block_compressed` — BC* volume (3D) writer.
//!   * `encode_dds_uncompressed_cubemap_array` — uncompressed cubemap /
//!     texture-array writer.
//!
//! Strategy: parse the fuzz bytes; for any parser-accepted `DdsImage`,
//! feed it to whichever new encoder its shape matches. Every encoder
//! returns `Result`, so the contract under test is simply "never panic"
//! — a `usize` overflow, slice out-of-bounds, or unchecked arithmetic in
//! the new code would surface here. When an encoder succeeds, the output
//! is re-parsed to catch a writer that emits a header the reader cannot
//! round-trip (a structural mismatch, not a hard crash, is still a
//! finding worth fixing — so a re-parse failure panics).
//!
//! Built with the default `registry` feature OFF (standalone path).

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{
    encode_dds_uncompressed_cubemap_array, encode_dds_uncompressed_dx10,
    encode_dds_volume_block_compressed, parse_dds,
};

fuzz_target!(|data: &[u8]| {
    let Ok(img) = parse_dds(data) else {
        return;
    };

    // BC* volume (3D) re-encode.
    if img.depth > 1 && img.pixel_format.is_block_compressed() && !img.is_cubemap {
        if let Ok(bytes) = encode_dds_volume_block_compressed(&img) {
            // A successful encode must re-parse.
            if parse_dds(&bytes).is_err() {
                panic!("BC volume re-encode failed to re-parse");
            }
        }
        return;
    }

    // Uncompressed cubemap / texture array re-encode.
    if (img.is_cubemap || img.array_size > 1)
        && img.depth <= 1
        && !img.pixel_format.is_block_compressed()
        && img.pixel_format.astc_footprint().is_none()
    {
        if let Ok(bytes) = encode_dds_uncompressed_cubemap_array(&img) {
            if parse_dds(&bytes).is_err() {
                panic!("cubemap/array re-encode failed to re-parse");
            }
        }
        return;
    }

    // Plain 2D DX10-only uncompressed re-encode.
    if !img.is_cubemap
        && img.array_size <= 1
        && img.depth <= 1
        && img.planes.len() == 1
        && !img.pixel_format.is_block_compressed()
        && img.pixel_format.astc_footprint().is_none()
    {
        // The encoder rejects legacy-mask formats and zero dims; only a
        // success is asserted to round-trip. Either outcome must not
        // panic.
        if let Ok(bytes) = encode_dds_uncompressed_dx10(&img) {
            if parse_dds(&bytes).is_err() {
                panic!("DX10 uncompressed re-encode failed to re-parse");
            }
        }
    }
});
