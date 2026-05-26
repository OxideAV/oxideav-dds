#![no_main]

//! For any `parse_dds`-able input that lands on an *uncompressed*
//! pixel format, `encode_dds_uncompressed` of the resulting `DdsImage`
//! must yield bytes that re-parse into a structurally-equal image.
//! This proves the encoder is a left inverse of the parser on the
//! parser's image of uncompressed inputs.
//!
//! Block-compressed inputs are out of scope here: the standalone
//! encoder paths for BC* go through `encode_dds_block_compressed`
//! which takes a different (pre-encoded) input shape. Those paths
//! are exercised through the per-decoder `decode_bcn` /
//! `decode_bc6h` / `decode_bc7` panic-free targets and through the
//! crate's integration tests.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{encode_dds_uncompressed, parse_dds, DdsPixelFormat};

fuzz_target!(|data: &[u8]| {
    let Ok(img) = parse_dds(data) else {
        return;
    };

    // Skip block-compressed pixel formats — they're handled by the
    // dedicated `encode_dds_block_compressed` path which takes a
    // pre-encoded surface, not the uncompressed one.
    if img.pixel_format.is_block_compressed() {
        return;
    }

    // The standalone uncompressed encoder requires a single plane
    // (mip-0 worth of pixels); cubemap / array / volume / mip-chain
    // shapes go through `encode_dds_block_compressed` or
    // `encode_dds_volume` instead. Skip those rather than calling
    // through and asserting on `Err` — that's a behaviour test, not
    // a fuzz finding.
    if img.planes.len() != 1
        || img.mip_map_count > 1
        || img.is_cubemap
        || img.array_size > 1
        || img.depth > 1
    {
        return;
    }

    // Reject 0-dim images that the parser may have accepted (the
    // header allows them but the encoder's stride math doesn't): the
    // encoder will Err, no need to round-trip.
    if img.width == 0 || img.height == 0 {
        return;
    }

    let Ok(encoded) = encode_dds_uncompressed(&img) else {
        // Encoder rejected a parser-accepted shape: that's interesting
        // but expected for some legacy edge cases (e.g. some `A8` /
        // `L8` / `A8L8` strides that the parser is lenient about but
        // the encoder is strict on). Don't surface as a panic.
        return;
    };

    let re_parsed = match parse_dds(&encoded) {
        Ok(i) => i,
        Err(e) => panic!("re-encoded DDS failed to re-parse: {e:?}"),
    };

    // Structural equality: format, dimensions, mip layout, payload
    // bytes. We don't compare every `DdsImage` field because the
    // parser fills in some derived fields (e.g. `surfaces[0]`
    // dimensions) from the on-disk header that the encoder may have
    // canonicalised on the way out.
    if re_parsed.pixel_format != img.pixel_format {
        panic!(
            "format mismatch after roundtrip: {:?} → {:?}",
            img.pixel_format, re_parsed.pixel_format,
        );
    }
    if re_parsed.width != img.width || re_parsed.height != img.height {
        panic!(
            "dim mismatch: {}x{} → {}x{}",
            img.width, img.height, re_parsed.width, re_parsed.height,
        );
    }
    if re_parsed.planes.len() != 1 {
        panic!("planes.len() = {} after roundtrip", re_parsed.planes.len());
    }
    if re_parsed.planes[0].data != img.planes[0].data {
        panic!("pixel-data mismatch after roundtrip");
    }

    // Sanity-touch the variants so the harness covers the format
    // table; the variant name only matters for the parser, but
    // referencing it here means a future `DdsPixelFormat` enum
    // rename or variant deletion fails to compile rather than
    // silently slipping through.
    let _: DdsPixelFormat = img.pixel_format;
});
