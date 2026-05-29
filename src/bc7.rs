//! BC7 block decompression to RGBA8.
//!
//! BC7 is the modern Direct3D 11 LDR block format: 16 bytes per 4×4
//! block, 8 modes with different subset / endpoint / index trade-offs.
//!
//! Reference: Microsoft's public "BC7" article on learn.microsoft.com
//! (under direct3d11 reference) and the public Khronos
//! `KHR_DF_MODEL_BC7` description in the Khronos Data Format
//! specification (which is the authoritative description of the BC7
//! block layout that Microsoft mandates Direct3D 11 hardware to
//! decode bit-for-bit). No DirectXTex, NVTT, bc7enc, ISPC, ARM
//! astc-encoder, basisu, or any other implementation source was
//! consulted; only the public spec text + tables.
//!
//! Block layout (LSB → MSB across the 128-bit block):
//!
//! ```text
//!   mode   1..8 bits (unary, value k = "0...01" with k zeros + 1)
//!   partition / rotation / index-selection bits per mode
//!   endpoint colour bits  (R, G, B, A, in subset-major order)
//!   per-endpoint p-bits   (for modes that carry them)
//!   index bits            (colour indices then optional alpha indices)
//! ```
//!
//! Modes summary:
//!
//! | Mode | Subsets | Part bits | Rot/Idx | Colour bits | Alpha bits | P bits | Idx bits | Idx2 bits |
//! |------|---------|-----------|---------|-------------|------------|--------|----------|-----------|
//! |   0  |    3    |     4     |    -    |    4 / ch   |     0      |  6     |    3     |     -     |
//! |   1  |    2    |     6     |    -    |    6 / ch   |     0      |  2     |    3     |     -     |
//! |   2  |    3    |     6     |    -    |    5 / ch   |     0      |  0     |    2     |     -     |
//! |   3  |    2    |     6     |    -    |    7 / ch   |     0      |  4     |    2     |     -     |
//! |   4  |    1    |     0     |  2 + 1  |    5 / ch   |     6      |  0     |    2     |     3     |
//! |   5  |    1    |     0     |    2    |    7 / ch   |     8      |  0     |    2     |     2     |
//! |   6  |    1    |     0     |    -    |    7 / ch   |     7      |  2     |    4     |     -     |
//! |   7  |    2    |     6     |    -    |    5 / ch   |     5      |  4     |    2     |     -     |
//!
//! Each block's first non-zero bit (LSB-first) selects the mode (mode 0
//! has a single `1` in bit 0; mode 1 is `01` = bits 0=0, 1=1; mode 7 is
//! `00000001`). A block with all 8 leading bits zero is "invalid" and
//! decodes to opaque black per Direct3D 11.

use crate::bcn::rgba8_surface_bytes;
use crate::error::{DdsError, Result};

// ---- BC7 partition tables (clean-room transcribed from the public Khronos
//      Data Format spec, Annex on BC7 — same numeric tables Microsoft
//      mandates Direct3D 11 hardware to use). ----------------------------

/// 64-entry table of 2-subset partition assignments. Each entry is a
/// 16-element array: `partition_table_2[p][i]` is the subset index
/// (0 or 1) for pixel `i` (row-major, x-major within the row) in the
/// 4×4 block when partition selector is `p`.
#[rustfmt::skip]
pub(crate) const PART_2: [[u8; 16]; 64] = [
    // Partitions 0..63 of the 2-subset table.
    [0,0,1,1, 0,0,1,1, 0,0,1,1, 0,0,1,1],
    [0,0,0,1, 0,0,0,1, 0,0,0,1, 0,0,0,1],
    [0,1,1,1, 0,1,1,1, 0,1,1,1, 0,1,1,1],
    [0,0,0,1, 0,0,1,1, 0,0,1,1, 0,1,1,1],
    [0,0,0,0, 0,0,0,1, 0,0,0,1, 0,0,1,1],
    [0,0,1,1, 0,1,1,1, 0,1,1,1, 1,1,1,1],
    [0,0,0,1, 0,0,1,1, 0,1,1,1, 1,1,1,1],
    [0,0,0,0, 0,0,0,1, 0,0,1,1, 0,1,1,1],
    [0,0,0,0, 0,0,0,0, 0,0,0,1, 0,0,1,1],
    [0,0,1,1, 0,1,1,1, 1,1,1,1, 1,1,1,1],
    [0,0,0,0, 0,0,0,1, 0,1,1,1, 1,1,1,1],
    [0,0,0,0, 0,0,0,0, 0,0,0,1, 0,1,1,1],
    [0,0,0,1, 0,1,1,1, 1,1,1,1, 1,1,1,1],
    [0,0,0,0, 0,0,0,0, 1,1,1,1, 1,1,1,1],
    [0,0,0,0, 1,1,1,1, 1,1,1,1, 1,1,1,1],
    [0,0,0,0, 0,0,0,0, 0,0,0,0, 1,1,1,1],
    [0,0,0,0, 1,0,0,0, 1,1,1,0, 1,1,1,1],
    [0,1,1,1, 0,0,1,1, 0,0,0,1, 0,0,0,0],
    [0,0,0,0, 1,0,0,0, 1,1,0,0, 1,1,1,0],
    [0,1,1,1, 0,0,1,1, 0,0,1,1, 0,0,0,1],
    [0,0,1,1, 0,0,0,1, 0,0,0,0, 0,0,0,0],
    [0,0,0,0, 1,0,0,0, 1,0,0,0, 1,1,0,0],
    [0,1,1,1, 0,0,1,1, 0,0,1,1, 0,0,0,1],
    [0,0,1,1, 0,0,0,1, 0,0,0,1, 0,0,0,0],
    [0,0,0,0, 1,0,0,0, 1,0,0,0, 1,1,0,0],
    [0,1,1,1, 0,0,1,1, 0,0,1,1, 0,0,0,1],
    [0,0,1,1, 0,1,1,1, 0,1,1,1, 0,0,1,1],
    [0,0,1,1, 1,0,0,1, 1,1,0,0, 1,1,0,0],
    [0,0,0,1, 0,1,1,1, 1,1,1,0, 1,0,0,0],
    [0,0,0,0, 1,1,1,1, 1,1,1,1, 0,0,0,0],
    [0,1,1,1, 0,0,0,1, 1,0,0,0, 1,1,1,0],
    [0,0,1,1, 1,0,0,1, 1,0,0,1, 1,1,0,0],
    [0,1,0,1, 0,1,0,1, 0,1,0,1, 0,1,0,1],
    [0,0,0,0, 1,1,1,1, 0,0,0,0, 1,1,1,1],
    [0,1,0,1, 1,0,1,0, 0,1,0,1, 1,0,1,0],
    [0,0,1,1, 0,0,1,1, 1,1,0,0, 1,1,0,0],
    [0,0,1,1, 1,1,0,0, 0,0,1,1, 1,1,0,0],
    [0,1,0,1, 0,1,0,1, 1,0,1,0, 1,0,1,0],
    [0,1,1,0, 1,0,0,1, 0,1,1,0, 1,0,0,1],
    [0,1,0,1, 1,0,1,0, 1,0,1,0, 0,1,0,1],
    [0,1,1,1, 0,0,1,1, 1,1,0,0, 1,1,1,0],
    [0,0,0,1, 0,0,1,1, 1,1,0,0, 1,0,0,0],
    [0,0,1,1, 0,0,1,0, 0,1,0,0, 1,1,0,0],
    [0,0,1,1, 1,0,1,1, 1,1,0,1, 1,1,0,0],
    [0,1,1,0, 1,0,0,1, 1,0,0,1, 0,1,1,0],
    [0,0,1,1, 1,1,0,0, 1,1,0,0, 0,0,1,1],
    [0,1,1,0, 0,1,1,0, 1,0,0,1, 1,0,0,1],
    [0,0,0,0, 0,1,1,0, 0,1,1,0, 0,0,0,0],
    [0,1,0,0, 1,1,1,0, 0,1,0,0, 0,0,0,0],
    [0,0,1,0, 0,1,1,1, 0,0,1,0, 0,0,0,0],
    [0,0,0,0, 0,0,1,0, 0,1,1,1, 0,0,1,0],
    [0,0,0,0, 0,1,0,0, 1,1,1,0, 0,1,0,0],
    [0,1,1,0, 1,1,0,0, 1,0,0,1, 0,0,1,1],
    [0,0,1,1, 0,1,1,0, 1,1,0,0, 1,0,0,1],
    [0,1,1,0, 0,0,1,1, 1,0,0,1, 1,1,0,0],
    [0,0,1,1, 1,0,0,1, 1,1,0,0, 0,1,1,0],
    [0,1,1,0, 1,1,0,0, 1,1,0,0, 1,0,0,1],
    [0,1,1,0, 0,0,1,1, 0,0,1,1, 1,0,0,1],
    [0,1,1,1, 1,1,1,0, 1,0,0,0, 0,0,0,1],
    [0,0,0,1, 1,0,0,0, 1,1,1,0, 0,1,1,1],
    [0,0,0,0, 1,1,1,1, 0,0,1,1, 0,0,1,1],
    [0,0,1,1, 0,0,1,1, 1,1,1,1, 0,0,0,0],
    [0,0,1,0, 0,0,1,0, 1,1,1,0, 1,1,1,0],
    [0,1,0,0, 0,1,0,0, 0,1,1,1, 0,1,1,1],
];

/// 64-entry table of 3-subset partition assignments. Each entry is a
/// 16-element array: `partition_table_3[p][i]` is the subset index
/// (0, 1 or 2) for pixel `i`.
#[rustfmt::skip]
pub(crate) const PART_3: [[u8; 16]; 64] = [
    [0,0,1,1, 0,0,1,1, 0,2,2,1, 2,2,2,2],
    [0,0,0,1, 0,0,1,1, 2,2,1,1, 2,2,2,1],
    [0,0,0,0, 2,0,0,1, 2,2,1,1, 2,2,1,1],
    [0,2,2,2, 0,0,2,2, 0,0,1,1, 0,1,1,1],
    [0,0,0,0, 0,0,0,0, 1,1,2,2, 1,1,2,2],
    [0,0,1,1, 0,0,1,1, 0,0,2,2, 0,0,2,2],
    [0,0,2,2, 0,0,2,2, 1,1,1,1, 1,1,1,1],
    [0,0,1,1, 0,0,1,1, 2,2,1,1, 2,2,1,1],
    [0,0,0,0, 0,0,0,0, 1,1,1,1, 2,2,2,2],
    [0,0,0,0, 1,1,1,1, 1,1,1,1, 2,2,2,2],
    [0,0,0,0, 1,1,1,1, 2,2,2,2, 2,2,2,2],
    [0,0,1,2, 0,0,1,2, 0,0,1,2, 0,0,1,2],
    [0,1,1,2, 0,1,1,2, 0,1,1,2, 0,1,1,2],
    [0,1,2,2, 0,1,2,2, 0,1,2,2, 0,1,2,2],
    [0,0,1,1, 0,1,1,2, 1,1,2,2, 1,2,2,2],
    [0,0,1,1, 2,0,0,1, 2,2,0,0, 2,2,2,0],
    [0,0,0,1, 0,0,1,1, 0,1,1,2, 1,1,2,2],
    [0,1,1,1, 0,0,1,1, 2,0,0,1, 2,2,0,0],
    [0,0,0,0, 1,1,2,2, 1,1,2,2, 1,1,2,2],
    [0,0,2,2, 0,0,2,2, 0,0,2,2, 1,1,1,1],
    [0,1,1,1, 0,1,1,1, 0,2,2,2, 0,2,2,2],
    [0,0,0,1, 0,0,0,1, 2,2,2,1, 2,2,2,1],
    [0,0,0,0, 0,0,1,1, 0,1,2,2, 0,1,2,2],
    [0,0,0,0, 1,1,0,0, 2,2,1,0, 2,2,1,0],
    [0,1,2,2, 0,1,2,2, 0,0,1,1, 0,0,0,0],
    [0,0,1,2, 0,0,1,2, 1,1,2,2, 2,2,2,2],
    [0,1,1,0, 1,2,2,1, 1,2,2,1, 0,1,1,0],
    [0,0,0,0, 0,1,1,0, 1,2,2,1, 1,2,2,1],
    [0,0,2,2, 1,1,0,2, 1,1,0,2, 0,0,2,2],
    [0,1,1,0, 0,1,1,0, 2,0,0,2, 2,2,2,2],
    [0,0,1,1, 0,1,2,2, 0,1,2,2, 0,0,1,1],
    [0,0,0,0, 2,0,0,0, 2,2,1,1, 2,2,2,1],
    [0,0,0,0, 0,0,0,2, 1,1,2,2, 1,2,2,2],
    [0,2,2,2, 0,0,2,2, 0,0,1,2, 0,0,1,1],
    [0,0,1,1, 0,0,1,2, 0,0,2,2, 0,2,2,2],
    [0,1,2,0, 0,1,2,0, 0,1,2,0, 0,1,2,0],
    [0,0,0,0, 1,1,1,1, 2,2,2,2, 0,0,0,0],
    [0,1,2,0, 1,2,0,1, 2,0,1,2, 0,1,2,0],
    [0,1,2,0, 2,0,1,2, 1,2,0,1, 0,1,2,0],
    [0,0,1,1, 2,2,0,0, 1,1,2,2, 0,0,1,1],
    [0,0,1,1, 1,1,2,2, 2,2,0,0, 0,0,1,1],
    [0,1,0,1, 0,1,0,1, 2,2,2,2, 2,2,2,2],
    [0,0,0,0, 0,0,0,0, 2,1,2,1, 2,1,2,1],
    [0,0,2,2, 1,1,2,2, 0,0,2,2, 1,1,2,2],
    [0,0,2,2, 0,0,1,1, 0,0,2,2, 0,0,1,1],
    [0,2,2,0, 1,2,2,1, 0,2,2,0, 1,2,2,1],
    [0,1,0,1, 2,2,2,2, 2,2,2,2, 0,1,0,1],
    [0,0,0,0, 2,1,2,1, 2,1,2,1, 2,1,2,1],
    [0,1,0,1, 0,1,0,1, 0,1,0,1, 2,2,2,2],
    [0,2,2,2, 0,1,1,1, 0,2,2,2, 0,1,1,1],
    [0,0,0,2, 1,1,1,2, 0,0,0,2, 1,1,1,2],
    [0,0,0,0, 2,1,1,2, 2,1,1,2, 2,1,1,2],
    [0,2,2,2, 0,1,1,1, 0,1,1,1, 0,2,2,2],
    [0,0,0,2, 1,1,1,2, 1,1,1,2, 0,0,0,2],
    [0,1,1,0, 0,1,1,0, 0,1,1,0, 2,2,2,2],
    [0,0,0,0, 0,0,0,0, 2,1,1,2, 2,1,1,2],
    [0,1,1,0, 0,1,1,0, 2,2,2,2, 2,2,2,2],
    [0,0,2,2, 0,0,1,1, 0,0,1,1, 0,0,2,2],
    [0,0,2,2, 1,1,2,2, 1,1,2,2, 0,0,2,2],
    [0,0,0,0, 0,0,0,0, 0,0,0,0, 2,1,1,2],
    [0,0,0,2, 0,0,0,1, 0,0,0,2, 0,0,0,1],
    [0,2,2,2, 1,2,2,2, 0,2,2,2, 1,2,2,2],
    [0,1,0,1, 2,2,2,2, 2,2,2,2, 2,2,2,2],
    [0,1,1,1, 2,0,1,1, 2,2,0,1, 2,2,2,0],
];

/// Per-partition fixed anchor index for the second subset of a 2-subset
/// partition (the index that's stored short-by-one bit because its
/// MSB is implicitly zero).
#[rustfmt::skip]
pub(crate) const ANCHOR_2_SUBSET_2: [u8; 64] = [
    15,15,15,15, 15,15,15,15, 15,15,15,15, 15,15,15,15,
    15, 2, 8, 2,  2, 8, 8,15,  2, 8, 2, 2,  8, 8, 2, 2,
    15,15, 6, 8,  2, 8,15,15,  2, 8, 2, 2,  2,15,15, 6,
     6, 2, 6, 8, 15,15, 2, 2, 15,15,15,15, 15, 2, 2,15,
];

/// Per-partition fixed anchor index for the second subset of a 3-subset
/// partition.
#[rustfmt::skip]
pub(crate) const ANCHOR_3_SUBSET_2: [u8; 64] = [
     3, 3,15,15,  8, 3,15,15,  8, 8, 6, 6,  6, 5, 3, 3,
     3, 3, 8,15,  3, 3, 6,10,  5, 8, 8, 6,  8, 5,15,15,
     8,15, 3, 5,  6,10, 8,15, 15, 3,15, 5, 15,15,15,15,
     3,15, 5, 5,  5, 8, 5,10,  5,10, 8,13, 15,12, 3, 3,
];

/// Per-partition fixed anchor index for the third subset of a 3-subset
/// partition.
#[rustfmt::skip]
pub(crate) const ANCHOR_3_SUBSET_3: [u8; 64] = [
    15, 8, 8, 3, 15,15, 3, 8, 15,15,15,15, 15,15,15, 8,
    15, 8,15, 3, 15, 8,15, 8,  3,15, 6,10, 15,15,10, 8,
    15, 3,15,10, 10, 8, 9,10,  6,15, 8,15,  3, 6, 6, 8,
    15, 3,15,15, 15,15,15,15, 15,15,15,15,  3,15,15, 8,
];

/// Interpolation factors for 2/3/4-bit indices (Microsoft / Khronos
/// `aWeight2 / aWeight3 / aWeight4` tables — 1, 2 or 3 weight tables
/// indexed by the index value).
const WEIGHT_2: [u32; 4] = [0, 21, 43, 64];
const WEIGHT_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHT_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

#[inline]
fn weight_table(idx_bits: u32) -> &'static [u32] {
    match idx_bits {
        2 => &WEIGHT_2,
        3 => &WEIGHT_3,
        4 => &WEIGHT_4,
        _ => unreachable!("BC7 index bit width must be 2, 3 or 4"),
    }
}

/// Interpolate two 8-bit endpoint values e0, e1 using the BC7 weight
/// table for `idx_bits` and the chosen index. Matches Microsoft's
/// public formula: `((64 - w) * e0 + w * e1 + 32) >> 6`.
#[inline]
fn interpolate(e0: u8, e1: u8, idx: u32, idx_bits: u32) -> u8 {
    let w = weight_table(idx_bits)[idx as usize];
    (((64 - w) * e0 as u32 + w * e1 as u32 + 32) >> 6) as u8
}

// ---- Bit-stream reader (LSB-first) -------------------------------------

struct BitReader<'a> {
    bytes: &'a [u8; 16],
    pos: u32,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8; 16]) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Read `n` bits (0..=32) starting at the current position, LSB-first
    /// across the byte sequence (bit 0 of byte 0 is the lowest bit).
    fn read(&mut self, n: u32) -> u32 {
        debug_assert!(n <= 32);
        let mut out: u64 = 0;
        for i in 0..n {
            let bit_pos = self.pos + i;
            let byte = bit_pos / 8;
            let shift = bit_pos & 7;
            if (byte as usize) >= 16 {
                break;
            }
            let b = (self.bytes[byte as usize] >> shift) & 1;
            out |= (b as u64) << i;
        }
        self.pos += n;
        out as u32
    }
}

// ---- Mode descriptor ---------------------------------------------------

struct ModeInfo {
    /// Number of subsets (1, 2 or 3).
    subsets: u32,
    /// Number of partition selector bits (0 for single-subset modes).
    partition_bits: u32,
    /// Number of rotation bits (modes 4 and 5 only; otherwise 0).
    rotation_bits: u32,
    /// Number of index-selection bits (mode 4 only; otherwise 0).
    idx_sel_bits: u32,
    /// Bits per colour channel per endpoint (R = G = B).
    colour_bits: u32,
    /// Bits per alpha endpoint (0 if alpha is implicit-255).
    alpha_bits: u32,
    /// Number of p-bits stored — `subsets * 2` for "shared" / "per-endpoint"
    /// p-bits as Microsoft specifies; 0 for modes with no p-bits.
    p_bits: u32,
    /// Whether p-bits are shared per subset (one p-bit per endpoint, both
    /// endpoints of a subset share the same value) — mode 1 only — or
    /// per-endpoint — modes 0, 3, 6, 7. Encoded as `true` for shared.
    p_bits_shared: bool,
    /// Index bit count (primary).
    idx_bits: u32,
    /// Secondary (alpha) index bit count, modes 4/5 only; 0 otherwise.
    idx2_bits: u32,
}

const MODES: [ModeInfo; 8] = [
    // Mode 0: 3 subsets, 4-bit partition, no rotation, 4-bit colour, 0 alpha,
    // 6 per-endpoint p-bits, 3-bit indices.
    ModeInfo {
        subsets: 3,
        partition_bits: 4,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 4,
        alpha_bits: 0,
        p_bits: 6,
        p_bits_shared: false,
        idx_bits: 3,
        idx2_bits: 0,
    },
    // Mode 1: 2 subsets, 6-bit partition, 6-bit colour, 0 alpha,
    // 2 shared p-bits (one per subset, shared by its 2 endpoints), 3-bit indices.
    ModeInfo {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 6,
        alpha_bits: 0,
        p_bits: 2,
        p_bits_shared: true,
        idx_bits: 3,
        idx2_bits: 0,
    },
    // Mode 2: 3 subsets, 6-bit partition, 5-bit colour, no alpha, no p-bits,
    // 2-bit indices.
    ModeInfo {
        subsets: 3,
        partition_bits: 6,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 5,
        alpha_bits: 0,
        p_bits: 0,
        p_bits_shared: false,
        idx_bits: 2,
        idx2_bits: 0,
    },
    // Mode 3: 2 subsets, 6-bit partition, 7-bit colour, no alpha, 4 per-endpoint
    // p-bits, 2-bit indices.
    ModeInfo {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 7,
        alpha_bits: 0,
        p_bits: 4,
        p_bits_shared: false,
        idx_bits: 2,
        idx2_bits: 0,
    },
    // Mode 4: 1 subset, 0 partition, 2-bit rotation, 1-bit idx_sel,
    // 5-bit colour, 6-bit alpha, no p-bits, 2-bit indices, 3-bit idx2.
    ModeInfo {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 2,
        idx_sel_bits: 1,
        colour_bits: 5,
        alpha_bits: 6,
        p_bits: 0,
        p_bits_shared: false,
        idx_bits: 2,
        idx2_bits: 3,
    },
    // Mode 5: 1 subset, 0 partition, 2-bit rotation, 7-bit colour, 8-bit alpha,
    // no p-bits, 2-bit indices, 2-bit idx2.
    ModeInfo {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 2,
        idx_sel_bits: 0,
        colour_bits: 7,
        alpha_bits: 8,
        p_bits: 0,
        p_bits_shared: false,
        idx_bits: 2,
        idx2_bits: 2,
    },
    // Mode 6: 1 subset, 0 partition, 7-bit colour, 7-bit alpha, 2 per-endpoint
    // p-bits, 4-bit indices.
    ModeInfo {
        subsets: 1,
        partition_bits: 0,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 7,
        alpha_bits: 7,
        p_bits: 2,
        p_bits_shared: false,
        idx_bits: 4,
        idx2_bits: 0,
    },
    // Mode 7: 2 subsets, 6-bit partition, 5-bit colour, 5-bit alpha,
    // 4 per-endpoint p-bits, 2-bit indices.
    ModeInfo {
        subsets: 2,
        partition_bits: 6,
        rotation_bits: 0,
        idx_sel_bits: 0,
        colour_bits: 5,
        alpha_bits: 5,
        p_bits: 4,
        p_bits_shared: false,
        idx_bits: 2,
        idx2_bits: 0,
    },
];

#[inline]
fn anchor_index(mode_info: &ModeInfo, partition: u32, subset: u32) -> u32 {
    if subset == 0 {
        return 0;
    }
    if mode_info.subsets == 2 {
        ANCHOR_2_SUBSET_2[partition as usize] as u32
    } else {
        // 3-subset
        if subset == 1 {
            ANCHOR_3_SUBSET_2[partition as usize] as u32
        } else {
            ANCHOR_3_SUBSET_3[partition as usize] as u32
        }
    }
}

#[inline]
fn partition_table(mode_info: &ModeInfo, partition: u32, pixel: u32) -> u32 {
    if mode_info.subsets == 1 {
        0
    } else if mode_info.subsets == 2 {
        PART_2[partition as usize][pixel as usize] as u32
    } else {
        PART_3[partition as usize][pixel as usize] as u32
    }
}

/// Expand an n-bit endpoint value to 8 bits by left-shifting then
/// OR-ing the high bits into the low (Microsoft "bit-replication"
/// rule). When `p_bit` is provided, it is appended below the value's
/// LSB before the bit-replication step (so a `colour_bits = 7` value
/// with a p-bit becomes 8 bits, then needs no replication).
#[inline]
fn expand_to_8(value: u32, bits: u32, p_bit: Option<u32>) -> u8 {
    let (v, total) = if let Some(pb) = p_bit {
        ((value << 1) | (pb & 1), bits + 1)
    } else {
        (value, bits)
    };
    if total >= 8 {
        return v as u8;
    }
    let shift = 8 - total;
    let high = (v << shift) as u8;
    // Replicate high bits into the low padding (Microsoft's
    // "high-bits-into-low" rule). For total >= 4 a single right-shift
    // produces enough low-side bits; for total < 4 the formula still
    // holds by virtue of the right-shift simply producing zeros for the
    // unfilled top bits.
    high | (high >> total)
}

/// Decoded BC7 endpoint pair for one subset (already promoted to 8-bit).
#[derive(Default, Clone, Copy)]
struct Endpoints {
    e0: [u8; 4],
    e1: [u8; 4],
}

/// Decode a single BC7 16-byte block into a 4×4 RGBA8 grid.
pub(crate) fn decode_bc7_block(block: &[u8; 16]) -> [[u8; 4]; 16] {
    let mut br = BitReader::new(block);

    // ---- Mode select: count leading zeros, mode = number of zeros.
    let mut mode: u32 = 8;
    for m in 0..8u32 {
        if br.read(1) == 1 {
            mode = m;
            break;
        }
    }
    if mode >= 8 {
        // Reserved mode → opaque black per Microsoft / DX11 rule.
        return [[0, 0, 0, 0]; 16];
    }
    let mi = &MODES[mode as usize];

    // ---- Partition / rotation / index-selection bits.
    let partition = if mi.partition_bits > 0 {
        br.read(mi.partition_bits)
    } else {
        0
    };
    let rotation = if mi.rotation_bits > 0 {
        br.read(mi.rotation_bits)
    } else {
        0
    };
    let idx_sel = if mi.idx_sel_bits > 0 {
        br.read(mi.idx_sel_bits)
    } else {
        0
    };

    // ---- Colour endpoints: subsets * 2 endpoints * 3 channels (R, G, B),
    //      stored channel-major (all R values first, then all G, then all B).
    //      Max 6 endpoint slots (3-subset modes), max 6 p-bits.
    let n_endpoints = mi.subsets as usize * 2;
    let mut raw_r = [0u32; 6];
    let mut raw_g = [0u32; 6];
    let mut raw_b = [0u32; 6];
    let mut raw_a = [0u32; 6];
    for slot in raw_r[..n_endpoints].iter_mut() {
        *slot = br.read(mi.colour_bits);
    }
    for slot in raw_g[..n_endpoints].iter_mut() {
        *slot = br.read(mi.colour_bits);
    }
    for slot in raw_b[..n_endpoints].iter_mut() {
        *slot = br.read(mi.colour_bits);
    }
    if mi.alpha_bits > 0 {
        for slot in raw_a[..n_endpoints].iter_mut() {
            *slot = br.read(mi.alpha_bits);
        }
    }

    // ---- P-bits.
    let mut p_bits = [0u32; 6];
    for slot in p_bits[..mi.p_bits as usize].iter_mut() {
        *slot = br.read(1);
    }

    // ---- Build 8-bit endpoints with p-bit append + bit-replication.
    let mut endpoints = [Endpoints::default(); 3];
    for s in 0..mi.subsets as usize {
        let i0 = s * 2;
        let i1 = s * 2 + 1;

        // P-bit assignment differs per mode: mode 1 shares one p-bit
        // across both endpoints of a subset (so 2 p-bits = one per
        // subset). Other p-bit modes (0, 3, 6, 7) carry one p-bit per
        // endpoint (so n_endpoints p-bits total).
        let (p0, p1) = if mi.p_bits == 0 {
            (None, None)
        } else if mi.p_bits_shared {
            let p = p_bits[s];
            (Some(p), Some(p))
        } else {
            (Some(p_bits[i0]), Some(p_bits[i1]))
        };

        endpoints[s].e0[0] = expand_to_8(raw_r[i0], mi.colour_bits, p0);
        endpoints[s].e0[1] = expand_to_8(raw_g[i0], mi.colour_bits, p0);
        endpoints[s].e0[2] = expand_to_8(raw_b[i0], mi.colour_bits, p0);
        endpoints[s].e1[0] = expand_to_8(raw_r[i1], mi.colour_bits, p1);
        endpoints[s].e1[1] = expand_to_8(raw_g[i1], mi.colour_bits, p1);
        endpoints[s].e1[2] = expand_to_8(raw_b[i1], mi.colour_bits, p1);

        if mi.alpha_bits > 0 {
            // Modes 4, 5: alpha endpoints have no p-bits even when colour
            // does. Modes 6, 7: alpha shares the p-bit attached to its
            // matching endpoint position (Microsoft "RGBAPRGBAP" layout).
            let (ap0, ap1) = if mi.p_bits == 0 || mode == 4 || mode == 5 {
                (None, None)
            } else {
                (Some(p_bits[i0]), Some(p_bits[i1]))
            };
            endpoints[s].e0[3] = expand_to_8(raw_a[i0], mi.alpha_bits, ap0);
            endpoints[s].e1[3] = expand_to_8(raw_a[i1], mi.alpha_bits, ap1);
        } else {
            endpoints[s].e0[3] = 255;
            endpoints[s].e1[3] = 255;
        }
    }

    // ---- Indices: 16 colour indices, with one anchor index short by 1
    //      bit per subset (because its MSB is implicitly 0).
    let mut idx_primary = [0u32; 16];
    for (px, slot) in idx_primary.iter_mut().enumerate() {
        let s = partition_table(mi, partition, px as u32);
        let anchor = anchor_index(mi, partition, s);
        let nbits = if px as u32 == anchor {
            mi.idx_bits - 1
        } else {
            mi.idx_bits
        };
        *slot = br.read(nbits);
    }

    // Mode 4/5: secondary (alpha) index plane.
    let mut idx_secondary = [0u32; 16];
    if mi.idx2_bits > 0 {
        for (px, slot) in idx_secondary.iter_mut().enumerate() {
            // Subset 0 always (single-subset mode); anchor is pixel 0.
            let nbits = if px == 0 {
                mi.idx2_bits - 1
            } else {
                mi.idx2_bits
            };
            *slot = br.read(nbits);
        }
    }

    // ---- Per-pixel interpolate + assemble RGBA.
    let mut out = [[0u8; 4]; 16];
    for px in 0..16usize {
        let s = partition_table(mi, partition, px as u32) as usize;
        let e0 = endpoints[s].e0;
        let e1 = endpoints[s].e1;

        // Pick which index plane drives RGB vs Alpha.
        // Mode 4: idx_sel == 0 → primary 2-bit drives colour, secondary
        //                       3-bit drives alpha.
        //         idx_sel == 1 → primary drives alpha, secondary drives colour.
        // Mode 5: primary drives colour (2-bit), secondary drives alpha (2-bit).
        // Other modes: only primary; alpha uses the same index plane.
        let (colour_idx, colour_bits, alpha_idx, alpha_bits) = if mode == 4 {
            if idx_sel == 0 {
                (idx_primary[px], 2u32, idx_secondary[px], 3u32)
            } else {
                (idx_secondary[px], 3u32, idx_primary[px], 2u32)
            }
        } else if mode == 5 {
            (idx_primary[px], 2u32, idx_secondary[px], 2u32)
        } else {
            (idx_primary[px], mi.idx_bits, idx_primary[px], mi.idx_bits)
        };

        let mut rgba = [
            interpolate(e0[0], e1[0], colour_idx, colour_bits),
            interpolate(e0[1], e1[1], colour_idx, colour_bits),
            interpolate(e0[2], e1[2], colour_idx, colour_bits),
            if mi.alpha_bits == 0 {
                255
            } else {
                interpolate(e0[3], e1[3], alpha_idx, alpha_bits)
            },
        ];

        // Channel-rotation (modes 4 and 5): swap A with the named
        // channel before output. Microsoft / Khronos rule:
        // 0 = no swap, 1 = swap A↔R, 2 = swap A↔G, 3 = swap A↔B.
        if mi.rotation_bits > 0 {
            match rotation {
                0 => {}
                1 => rgba.swap(0, 3),
                2 => rgba.swap(1, 3),
                3 => rgba.swap(2, 3),
                _ => unreachable!(),
            }
        }

        out[px] = rgba;
    }

    out
}

/// Decode a BC7 surface to RGBA8.
///
/// `input` must hold `ceil(w/4) × ceil(h/4) × 16` bytes; `output`
/// must hold `width × height × 4` bytes.
pub fn decode_bc7(input: &[u8], width: u32, height: u32, output: &mut [u8]) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    // Saturate on overflow so the length check below rejects rather
    // than triggering `panic_const_mul_overflow` (any real slice length
    // is `< usize::MAX`). Fuzz-driven `width = height = u32::MAX`
    // exercises this path.
    let want_in = bw.saturating_mul(bh).saturating_mul(16);
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC7 input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height
        )));
    }
    let want_out = rgba8_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC7 output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height
        )));
    }
    let stride = width as usize * 4;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let block: [u8; 16] = input[off..off + 16].try_into().unwrap();
            let pixels = decode_bc7_block(&block);
            for py in 0..4u32 {
                let yy = (by as u32) * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = (bx as u32) * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    let dst = yy as usize * stride + xx as usize * 4;
                    let p = pixels[(py * 4 + px) as usize];
                    output[dst..dst + 4].copy_from_slice(&p);
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper that packs a sequence of `(value, bit_count)` pairs LSB-first
    /// into a 16-byte BC7 block.
    fn pack_block(fields: &[(u32, u32)]) -> [u8; 16] {
        let mut block = [0u8; 16];
        let mut pos = 0u32;
        for &(value, n) in fields {
            for i in 0..n {
                let bit = (value >> i) & 1;
                let byte = (pos / 8) as usize;
                let shift = pos & 7;
                if byte >= 16 {
                    break;
                }
                block[byte] |= (bit as u8) << shift;
                pos += 1;
            }
        }
        block
    }

    /// Build a mode-6 BC7 block whose two endpoints are both opaque
    /// white and whose 4-bit indices are all 0 → every pixel is white.
    /// Mode-6 layout: 7-bit mode prefix `1000000` (mode 6 = bit 6 set,
    /// so first 6 bits are 0 and the 7th bit is 1).
    #[test]
    fn bc7_mode6_solid_white() {
        // Construct the block bit-by-bit at LSB-first (matching the reader).
        let mut bits = [0u8; 128]; // bit array (one byte = one bit)

        // Mode prefix: 6 zeros then a 1 = mode 6 (the leading-zero count).
        bits[6] = 1;
        // No partition, rotation, idx_sel, p-bits-only-from-end-position.
        // Endpoint colours: 7-bit R, then G, then B for each endpoint.
        // 7-bit value 127 + p-bit 1 → 8-bit 255 → bit-replicated to 0xFF.
        let mut pos = 7usize;
        // R, G, B each: 2 endpoints * 7 bits = 14 bits per channel.
        // Then alpha: 2 endpoints * 7 bits = 14 bits.
        // For each endpoint slot we want raw value 127 (seven 1's).
        for _ in 0..(2 * 4) {
            // 2 endpoints * 4 channels (RGBA)
            for _ in 0..7 {
                bits[pos] = 1;
                pos += 1;
            }
        }
        // 2 p-bits, both 1 → endpoint becomes (raw<<1|1)=255.
        bits[pos] = 1;
        pos += 1;
        bits[pos] = 1;
        // 16 indices, 4 bits each. Anchor for subset 0 is pixel 0 → 3 bits.
        // All zero → all map to e0 (white).
        // (We leave the remaining bits as zero — that's "all indices 0".)

        // Pack bit array into bytes.
        let mut block = [0u8; 16];
        for (i, &b) in bits.iter().enumerate() {
            if i >= 128 {
                break;
            }
            block[i / 8] |= (b & 1) << (i % 8);
        }
        let pixels = decode_bc7_block(&block);
        for p in pixels.iter() {
            assert_eq!(*p, [255, 255, 255, 255], "got {:?}", p);
        }
        // Sanity: the pos accounting matches Microsoft's mode-6 layout
        // (1 mode + 56 colour + 14 alpha + 2 p + 63 index = 136 ... wait
        // mode 6's total = 1 + 56 + 14 + 2 + 63 = 136 bits which is 8 over.
        // The indices are 4-bit × 16 = 64 minus 1 (anchor) = 63 bits. Plus
        // mode 1 + colour 56 + alpha 14 + pbit 2 + index 63 = 136 ... but
        // BC7 blocks are exactly 128 bits. Let's recount: alpha 7-bit × 2 = 14,
        // colour 7-bit × 2 × 3 = 42, total endpoint = 56, mode prefix = 7,
        // p-bits = 2, indices = 63 → 7 + 56 + 2 + 63 = 128. Good.
    }

    #[test]
    fn bc7_invalid_mode_returns_black() {
        // All zeros → no leading 1 in the first 8 bits → reserved mode.
        let block = [0u8; 16];
        let pixels = decode_bc7_block(&block);
        for p in pixels.iter() {
            assert_eq!(*p, [0, 0, 0, 0]);
        }
    }

    #[test]
    fn bc7_mode6_endpoint1_via_max_index() {
        // Same as mode6_solid_white but endpoints differ: e0 = black,
        // e1 = white, indices = max (15) for every pixel except the
        // anchor. Anchor pixel 0 has only 3 bits → max index 7 there.
        // Every interpolation therefore hits weight 64 → e1 = white.
        let mut fields: Vec<(u32, u32)> = vec![
            (1u32 << 6, 7), // mode prefix: bit 6 set
        ];
        // Endpoints: 7-bit raw RGBA, e0 then e1, channel-major (R0,R1,G0,G1,B0,B1,A0,A1).
        for _ in 0..4 {
            fields.push((0, 7)); // e0
            fields.push((127, 7)); // e1 = max
        }
        // 2 p-bits — set both to 1 so e1 expands fully to 0xff and e0
        // stays at 0 (raw 0 + p-bit 1 → (0<<1)|1 = 1, which after the
        // bit-replication = 0x00 plus high-bit replicate stays close to 0).
        // For idx=15 we get weight 64 → result = e1 only.
        fields.push((1, 1));
        fields.push((1, 1));
        // Indices: anchor pixel 0 = 3 bits, value 7 (max). Pixels 1..15: 4 bits each, value 15.
        fields.push((7, 3));
        for _ in 1..16 {
            fields.push((15, 4));
        }
        let block = pack_block(&fields);
        let pixels = decode_bc7_block(&block);
        // Pixel 0 is the anchor: stored with one fewer bit, so the
        // 3-bit value 7 corresponds to 4-bit index 7, weight 30 — the
        // result is roughly half-way between e0 (black) and e1 (white).
        // Allow some leeway around (30/64) * 255 = 119.5.
        let p0 = pixels[0];
        assert!(
            p0[0] >= 117 && p0[0] <= 122,
            "anchor pixel R={} not in [117, 122]",
            p0[0]
        );
        // All other pixels have full 4-bit index 15 = weight 64 → e1 = white.
        for p in pixels.iter().skip(1) {
            assert_eq!(*p, [255, 255, 255, 255]);
        }
    }

    #[test]
    fn bc7_mode5_no_rotation_solid_red() {
        // Mode 5: 1 subset, 2-bit rotation, 7-bit colour, 8-bit alpha,
        // no p-bits, 2-bit colour idx + 2-bit alpha idx.
        //
        // Endpoints e0 = e1 = red, alpha 255. Indices all 0 → every
        // pixel = e0 = (255, 0, 0, 255).
        let mut fields: Vec<(u32, u32)> = vec![
            (1u32 << 5, 6), // mode prefix: bit 5 set (mode 5)
            (0, 2),         // rotation = 0 (no swap)
        ];
        // Colour endpoints: R0, R1, G0, G1, B0, B1 — 7 bits each.
        // Want endpoint = 0xFF after expand. raw=127 (7 ones) → 7-bit value
        // 127 → expand_to_8(127, 7, None) = (127<<1) | (127>>6) = 0xFE | 0x01 = 0xFF.
        fields.push((127, 7)); // R0
        fields.push((127, 7)); // R1
        fields.push((0, 7)); // G0
        fields.push((0, 7)); // G1
        fields.push((0, 7)); // B0
        fields.push((0, 7)); // B1
                             // Alpha endpoints: A0, A1 — 8 bits each.
        fields.push((255, 8));
        fields.push((255, 8));
        // No p-bits.
        // Colour indices: anchor pixel 0 = 1 bit, others 2 bits.
        fields.push((0, 1));
        for _ in 1..16 {
            fields.push((0, 2));
        }
        // Alpha indices: anchor pixel 0 = 1 bit, others 2 bits.
        fields.push((0, 1));
        for _ in 1..16 {
            fields.push((0, 2));
        }
        let block = pack_block(&fields);
        let pixels = decode_bc7_block(&block);
        for p in pixels.iter() {
            assert_eq!(*p, [255, 0, 0, 255]);
        }
    }

    #[test]
    fn bc7_decode_surface_solid_white_4x4() {
        // Same single block as bc7_mode6_solid_white, run through the
        // surface entry point.
        let mut bits = [0u8; 128];
        bits[6] = 1;
        let mut pos = 7usize;
        for _ in 0..(2 * 4) {
            for _ in 0..7 {
                bits[pos] = 1;
                pos += 1;
            }
        }
        bits[pos] = 1;
        pos += 1;
        bits[pos] = 1;
        let mut block = [0u8; 16];
        for (i, &b) in bits.iter().enumerate() {
            if i >= 128 {
                break;
            }
            block[i / 8] |= (b & 1) << (i % 8);
        }
        let mut out = vec![0u8; 4 * 4 * 4];
        decode_bc7(&block, 4, 4, &mut out).unwrap();
        for chunk in out.chunks_exact(4) {
            assert_eq!(chunk, &[255, 255, 255, 255]);
        }
    }
}
