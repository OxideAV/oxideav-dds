#![no_main]

//! Decode arbitrary fuzz-supplied bytes through the DDS container
//! parser. `parse_dds` must always return a `Result` and never panic,
//! abort, integer-overflow (in a debug build), index out of bounds, or
//! preallocate a buffer based on the unverified `width * height *
//! depth * mip * array_size * face_count * bpp` product the header
//! claims.
//!
//! The DDS container is uniquely hostile for an unchecked parser:
//!
//!   * The 124-byte `DDS_HEADER` is fixed-layout, so the parser will
//!     happily read every field — but the fields it reads
//!     (`width`, `height`, `depth`, `mip_map_count`, `pitch_or_linear_size`,
//!     and the 11 `caps` / `caps2` bits) are *unverified* at the on-disk
//!     level. A hostile fixture can claim `mip_map_count = u32::MAX`
//!     with a 12-byte payload.
//!   * The optional 20-byte `DDS_HEADER_DXT10` adds `array_size`,
//!     `resource_dimension` (1D/2D/3D), `misc_flag` (cubemap bit), and
//!     the full DXGI format table (1..=132 valid, plus reserved
//!     integers a malicious file can put there).
//!   * `array_size` for a cubemap multiplies by 6, so the loop bound
//!     for surface allocation is `array_size * 6 * mip_count` — three
//!     independent attacker fields the parser must combine without
//!     overflow.
//!
//! The contract under test is purely that `parse_dds` *returns*: a
//! malformed stream yields `Err(DdsError::…)`, a well-formed one yields
//! `Ok(DdsImage)`, and neither path may panic / abort / OOM. The
//! `Result` is intentionally discarded.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::parse_dds;

fuzz_target!(|data: &[u8]| {
    let _ = parse_dds(data);
});
