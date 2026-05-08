//! BC6H (DXGI `BC6H_UF16` / `BC6H_SF16`) HDR-float block decompression.
//!
//! BC6H stores RGB half-float (no alpha) data at 8 bpp / 16 bytes per
//! 4x4 block. The format defines 14 modes; each block carries:
//!
//! 1. A mode prefix (2 or 5 bits, LSB-first at the start of the block).
//! 2. Endpoint colour values (R / G / B for two endpoints per subset),
//!    laid out as a per-mode bit-interleave specified by Microsoft's
//!    public DXGI / Direct3D 11 reference. Most modes "delta-encode"
//!    the second endpoint as a small signed offset relative to the
//!    first endpoint; modes 9 and 10 store every endpoint as an
//!    absolute value (no delta).
//! 3. (TWO-subset modes only) A 5-bit partition / shape index `d`
//!    selecting one of 32 partition assignments at bit 77.
//! 4. Per-pixel index bits — 3 bits/pixel for TWO-subset modes (with
//!    the anchor index of each subset short by one bit), 4 bits/pixel
//!    for ONE-subset modes (with the pixel-0 anchor short by one bit).
//!
//! Decoded output is a 4x4 RGB half-float (binary16) grid; the caller
//! receives 16 RGBA samples per block where alpha is the binary16
//! one-constant (`0x3c00`).
//!
//! ## Per-mode bit layout
//!
//! The 14 modes are encoded in a compact way: for each mode Microsoft
//! mandates a specific bit-interleave that maps each block-bit
//! position to a symbolic field (`gy[4]`, `bz[3]`, ...). The
//! [`MODES`] table here is a compact transcription: each entry lists
//! `(field, dest_bit_position)` pairs in the order Microsoft writes
//! them, plus the per-channel endpoint widths after sign / zero
//! extension and the index width.
//!
//! ## Subset / partition handling
//!
//! Two-subset BC6H modes use the same 32 partition assignments as
//! BC7's 2-subset table (entries 0..31), kept here as a private copy
//! so the bc6h module is self-contained. Anchor indices match BC7's
//! 2-subset anchor table (entries 0..31).
//!
//! ## Reference
//!
//! Microsoft "BC6H Format" article on learn.microsoft.com and the
//! Intel Open Source Programmer's Reference Manual (Vol. 5: Memory
//! Data Formats), which is published under the 0BSD license and
//! documents the same per-mode bit layout textually. No DirectXTex,
//! NVTT, ISPC `ispc_texcomp`, basisu, or `bc6h_enc` source was
//! consulted; only public spec text + tables.

use crate::error::{DdsError, Result};

/// Total bytes for an RGBA half-float surface (`width x height x 8`).
#[inline]
pub(crate) fn rgba_half_surface_bytes(width: u32, height: u32) -> usize {
    width as usize * height as usize * 8
}

// ---- Partition + anchor tables (first 32 entries of the BC7 table) ------

#[rustfmt::skip]
pub(crate) const PART_2: [[u8; 16]; 32] = [
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
];

#[rustfmt::skip]
pub(crate) const ANCHOR_2_SUBSET_2: [u8; 32] = [
    15,15,15,15, 15,15,15,15, 15,15,15,15, 15,15,15,15,
    15, 2, 8, 2,  2, 8, 8,15,  2, 8, 2, 2,  8, 8, 2, 2,
];

// ---- Interpolation weights ---------------------------------------------

const WEIGHT_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHT_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];

// ---- Bit reader (LSB-first across the 16 bytes) -------------------------

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
        let mut out: u32 = 0;
        for i in 0..n {
            let bit_pos = self.pos + i;
            let byte = (bit_pos / 8) as usize;
            let shift = bit_pos & 7;
            if byte >= 16 {
                break;
            }
            let b = (self.bytes[byte] >> shift) & 1;
            out |= (b as u32) << i;
        }
        self.pos += n;
        out
    }

    fn bit_at(bytes: &[u8; 16], bit: u32) -> u32 {
        ((bytes[(bit / 8) as usize] >> (bit & 7)) & 1) as u32
    }
}

// ---- Per-mode field descriptor table -----------------------------------
//
// The descriptor for each of the 14 modes is:
//   - prefix length (2 or 5 bits)
//   - subsets (1 or 2)
//   - per-channel endpoint precision (the width of the "w" / base
//     endpoint, before delta inversion)
//   - per-channel delta width for {x, y, z} endpoints (0 when the mode
//     stores absolute values rather than deltas)
//   - the bit layout: an ordered list of (channel, endpoint, dest_bit)
//     triples consumed in block-bit order STARTING AFTER the mode
//     prefix. Each triple says "the next block bit is bit `dest_bit`
//     of channel `channel`'s endpoint `endpoint`".
//   - index bit count (3 for TWO-subset, 4 for ONE-subset)
//
// Endpoint enumeration follows Microsoft's `w x y z` notation:
//   endpoint 0 = w (subset 0, endpoint A) — always the absolute base
//   endpoint 1 = x (subset 0, endpoint B)
//   endpoint 2 = y (subset 1, endpoint A) — TWO modes only
//   endpoint 3 = z (subset 1, endpoint B) — TWO modes only

#[derive(Clone, Copy)]
pub(crate) struct FieldBit {
    /// 0 = R, 1 = G, 2 = B.
    pub(crate) channel: u8,
    /// 0..=3, endpoint index.
    pub(crate) endpoint: u8,
    /// Destination bit position within the channel value (0 = LSB).
    pub(crate) dest_bit: u8,
}

/// Per-mode descriptor. Bit-allocation tables transcribed from the
/// Intel Open Source PRM Vol. 5 (BC6H section), which republishes
/// Microsoft's authoritative per-mode bit-interleave table in textual
/// form under the 0BSD license.
pub(crate) struct ModeInfo {
    /// Number of subsets (1 or 2).
    pub(crate) subsets: u8,
    /// "w" endpoint precision per channel (R, G, B). For ONE-subset
    /// modes 11..=13 with asymmetric precision, this is the width of
    /// the SAME channel — w is the base endpoint, x is the delta.
    pub(crate) prec_r: u8,
    pub(crate) prec_g: u8,
    pub(crate) prec_b: u8,
    /// Delta width per channel. 0 = no delta (the field stores the
    /// absolute endpoint value, like modes 9 and 10).
    pub(crate) delta_r: u8,
    pub(crate) delta_g: u8,
    pub(crate) delta_b: u8,
    /// Bit ordering of fields after the mode prefix (and before the
    /// 5-bit partition / index bits, which the caller reads
    /// separately).
    pub(crate) fields: &'static [FieldBit],
    /// Index bit count per pixel (3 for TWO-subset, 4 for ONE-subset).
    pub(crate) idx_bits: u8,
}

macro_rules! fb {
    ($ch:literal, $ep:literal, $bit:literal) => {
        FieldBit {
            channel: $ch,
            endpoint: $ep,
            dest_bit: $bit,
        }
    };
}

/// Helper to expand a contiguous bit range `lo..=hi` of channel `ch`,
/// endpoint `ep` into a list of `FieldBit` entries (LSB first).
const fn _range_check(_lo: u8, _hi: u8) {}

// To keep the per-mode tables short and audit-friendly, we predefine
// the channel + endpoint layout as one ordered list of (channel,
// endpoint, dest_bit) triples in source-bit order. The macros below
// generate "rw[hi:lo]"-style ranges.

/// Generate FieldBit entries for `channel` / `endpoint` covering bits
/// `lo..=hi` in source-bit order (LSB-first dest bits).
const fn rng(channel: u8, endpoint: u8, lo: u8, hi: u8) -> [FieldBit; 16] {
    let mut out = [FieldBit {
        channel: 0,
        endpoint: 0,
        dest_bit: 0,
    }; 16];
    let mut i = 0u8;
    let mut b = lo;
    while b <= hi {
        out[i as usize] = FieldBit {
            channel,
            endpoint,
            dest_bit: b,
        };
        i += 1;
        b += 1;
    }
    out
}

// ---- Mode 0 (TWO; 10.5.5.5; prefix 00) ---------------------------------
// rw[9:0] gw[9:0] bw[9:0] rx[4:0] gx[4:0] bx[4:0] ry[4:0] gy[4:0]
// by[4:0] rz[4:0] gz[4:0] bz[4:0] but with INTERLEAVED layout:
//
//   bit  2 | gy[4]
//   bit  3 | by[4]
//   bit  4 | bz[4]
//   bit  5..14 | rw[9:0]
//   bit 15..24 | gw[9:0]
//   bit 25..34 | bw[9:0]
//   bit 35..39 | rx[4:0]
//   bit 40 | gz[4]
//   bit 41..44 | gy[3:0]
//   bit 45..49 | gx[4:0]
//   bit 50 | bz[0]
//   bit 51..54 | gz[3:0]
//   bit 55..59 | bx[4:0]
//   bit 60 | bz[1]
//   bit 61..64 | by[3:0]
//   bit 65..69 | ry[4:0]
//   bit 70 | bz[2]
//   bit 71..75 | rz[4:0]
//   bit 76 | bz[3]
//
// Followed by partition (bits 77..81) and indices (bits 82..127).

#[rustfmt::skip]
const M0_FIELDS: &[FieldBit] = &[
    // After 2-bit prefix at bits 0..1, we start at bit 2.
    fb!(1, 2, 4),                                                // bit 2:  gy[4]
    fb!(2, 2, 4),                                                // bit 3:  by[4]
    fb!(2, 3, 4),                                                // bit 4:  bz[4]
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // bits 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // bits 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // bits 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),      // bits 35..39: rx[4:0]
    fb!(1,3,4),                                                  // bit 40: gz[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // bits 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),      // bits 45..49: gx[4:0]
    fb!(2,3,0),                                                  // bit 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // bits 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),      // bits 55..59: bx[4:0]
    fb!(2,3,1),                                                  // bit 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // bits 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),      // bits 65..69: ry[4:0]
    fb!(2,3,2),                                                  // bit 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),      // bits 71..75: rz[4:0]
    fb!(2,3,3),                                                  // bit 76: bz[3]
];

// ---- Mode 1 (TWO; 7.6.6.6; prefix 01) ----------------------------------
// bit  2 | gy[5]
// bit  3 | gz[4]
// bit  4 | gz[5]
// bit  5..11 | rw[6:0]
// bit 12 | bz[0]
// bit 13 | bz[1]
// bit 14 | by[4]
// bit 15..21 | gw[6:0]
// bit 22 | by[5]
// bit 23 | bz[2]
// bit 24 | gy[4]
// bit 25..31 | bw[6:0]
// bit 32 | bz[3]
// bit 33 | bz[5]
// bit 34 | bz[4]
// bit 35..40 | rx[5:0]
// bit 41..44 | gy[3:0]
// bit 45..50 | gx[5:0]
// bit 51..54 | gz[3:0]
// bit 55..60 | bx[5:0]
// bit 61..64 | by[3:0]
// bit 65..70 | ry[5:0]
// bit 71..76 | rz[5:0]

#[rustfmt::skip]
const M1_FIELDS: &[FieldBit] = &[
    fb!(1,2,5),                                                  // bit 2: gy[5]
    fb!(1,3,4),                                                  // bit 3: gz[4]
    fb!(1,3,5),                                                  // bit 4: gz[5]
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),                                       // bits 5..11: rw[6:0]
    fb!(2,3,0),                                                  // bit 12: bz[0]
    fb!(2,3,1),                                                  // bit 13: bz[1]
    fb!(2,2,4),                                                  // bit 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),                                       // bits 15..21: gw[6:0]
    fb!(2,2,5),                                                  // bit 22: by[5]
    fb!(2,3,2),                                                  // bit 23: bz[2]
    fb!(1,2,4),                                                  // bit 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),                                       // bits 25..31: bw[6:0]
    fb!(2,3,3),                                                  // bit 32: bz[3]
    fb!(2,3,5),                                                  // bit 33: bz[5]
    fb!(2,3,4),                                                  // bit 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),
    fb!(0,1,5),                                                  // bits 35..40: rx[5:0]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // bits 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),
    fb!(1,1,5),                                                  // bits 45..50: gx[5:0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // bits 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),
    fb!(2,1,5),                                                  // bits 55..60: bx[5:0]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // bits 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),
    fb!(0,2,5),                                                  // bits 65..70: ry[5:0]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),
    fb!(0,3,5),                                                  // bits 71..76: rz[5:0]
];

// ---- Mode 2 (TWO; R: 11.5, G:11.4, B:11.4; prefix 00010) ----------------
// bit  5..14 | rw[9:0]
// bit 15..24 | gw[9:0]
// bit 25..34 | bw[9:0]
// bit 35..39 | rx[4:0]
// bit 40 | rw[10]
// bit 41..44 | gy[3:0]
// bit 45..48 | gx[3:0]
// bit 49 | gw[10]
// bit 50 | bz[0]
// bit 51..54 | gz[3:0]
// bit 55..58 | bx[3:0]
// bit 59 | bw[10]
// bit 60 | bz[1]
// bit 61..64 | by[3:0]
// bit 65..69 | ry[4:0]
// bit 70 | bz[2]
// bit 71..75 | rz[4:0]
// bit 76 | bz[3]

#[rustfmt::skip]
const M2_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // bits 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // bits 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // bits 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),      // bits 35..39: rx[4:0]
    fb!(0,0,10),                                                 // bit 40: rw[10]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // bits 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),                 // bits 45..48: gx[3:0]
    fb!(1,0,10),                                                 // bit 49: gw[10]
    fb!(2,3,0),                                                  // bit 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // bits 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),                 // bits 55..58: bx[3:0]
    fb!(2,0,10),                                                 // bit 59: bw[10]
    fb!(2,3,1),                                                  // bit 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // bits 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),      // bits 65..69: ry[4:0]
    fb!(2,3,2),                                                  // bit 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),      // bits 71..75: rz[4:0]
    fb!(2,3,3),                                                  // bit 76: bz[3]
];

// ---- Mode 3 (TWO; R:11.4, G:11.5, B:11.4; prefix 00110) ----------------
#[rustfmt::skip]
const M3_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),                 // 35..38: rx[3:0]
    fb!(0,0,10),                                                 // 39: rw[10]
    fb!(1,3,4),                                                  // 40: gz[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),      // 45..49: gx[4:0]
    fb!(1,0,10),                                                 // 50: gw[10]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),                 // 55..58: bx[3:0]
    fb!(2,0,10),                                                 // 59: bw[10]
    fb!(2,3,1),                                                  // 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),                 // 65..68: ry[3:0]
    fb!(2,3,0),                                                  // 69: bz[0]
    fb!(2,3,2),                                                  // 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),                 // 71..74: rz[3:0]
    fb!(1,2,4),                                                  // 75: gy[4]
    fb!(2,3,3),                                                  // 76: bz[3]
];

// ---- Mode 4 (TWO; R:11.4, G:11.4, B:11.5; prefix 01010) ----------------
#[rustfmt::skip]
const M4_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),                 // 35..38: rx[3:0]
    fb!(0,0,10),                                                 // 39: rw[10]
    fb!(2,2,4),                                                  // 40: by[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),                 // 45..48: gx[3:0]
    fb!(1,0,10),                                                 // 49: gw[10]
    fb!(2,3,0),                                                  // 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),      // 55..59: bx[4:0]
    fb!(2,0,10),                                                 // 60: bw[10]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),                 // 65..68: ry[3:0]
    fb!(2,3,1),                                                  // 69: bz[1]
    fb!(2,3,2),                                                  // 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),                 // 71..74: rz[3:0]
    fb!(2,3,4),                                                  // 75: bz[4]
    fb!(2,3,3),                                                  // 76: bz[3]
];

// ---- Mode 5 (TWO; 9.5.5.5; prefix 01110) -------------------------------
#[rustfmt::skip]
const M5_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),                 // 5..13: rw[8:0]
    fb!(2,2,4),                                                  // 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),                 // 15..23: gw[8:0]
    fb!(1,2,4),                                                  // 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),                 // 25..33: bw[8:0]
    fb!(2,3,4),                                                  // 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),      // 35..39: rx[4:0]
    fb!(1,3,4),                                                  // 40: gz[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),      // 45..49: gx[4:0]
    fb!(2,3,0),                                                  // 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),      // 55..59: bx[4:0]
    fb!(2,3,1),                                                  // 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),      // 65..69: ry[4:0]
    fb!(2,3,2),                                                  // 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),      // 71..75: rz[4:0]
    fb!(2,3,3),                                                  // 76: bz[3]
];

// ---- Mode 6 (TWO; R:8.6, G:8.5, B:8.5; prefix 10010) -------------------
#[rustfmt::skip]
const M6_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),                            // 5..12: rw[7:0]
    fb!(1,3,4),                                                  // 13: gz[4]
    fb!(2,2,4),                                                  // 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),                            // 15..22: gw[7:0]
    fb!(2,3,2),                                                  // 23: bz[2]
    fb!(1,2,4),                                                  // 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),                            // 25..32: bw[7:0]
    fb!(2,3,3),                                                  // 33: bz[3]
    fb!(2,3,4),                                                  // 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),
    fb!(0,1,5),                                                  // 35..40: rx[5:0]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),      // 45..49: gx[4:0]
    fb!(2,3,0),                                                  // 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),      // 55..59: bx[4:0]
    fb!(2,3,1),                                                  // 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),
    fb!(0,2,5),                                                  // 65..70: ry[5:0]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),
    fb!(0,3,5),                                                  // 71..76: rz[5:0]
];

// ---- Mode 7 (TWO; R:8.5, G:8.6, B:8.5; prefix 10110) -------------------
#[rustfmt::skip]
const M7_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),                            // 5..12: rw[7:0]
    fb!(2,3,0),                                                  // 13: bz[0]
    fb!(2,2,4),                                                  // 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),                            // 15..22: gw[7:0]
    fb!(1,2,5),                                                  // 23: gy[5]
    fb!(1,2,4),                                                  // 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),                            // 25..32: bw[7:0]
    fb!(1,3,5),                                                  // 33: gz[5]
    fb!(2,3,4),                                                  // 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),      // 35..39: rx[4:0]
    fb!(1,3,4),                                                  // 40: gz[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),
    fb!(1,1,5),                                                  // 45..50: gx[5:0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),      // 55..59: bx[4:0]
    fb!(2,3,1),                                                  // 60: bz[1]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),      // 65..69: ry[4:0]
    fb!(2,3,2),                                                  // 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),      // 71..75: rz[4:0]
    fb!(2,3,3),                                                  // 76: bz[3]
];

// ---- Mode 8 (TWO; R:8.5, G:8.5, B:8.6; prefix 11010) -------------------
#[rustfmt::skip]
const M8_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),                            // 5..12: rw[7:0]
    fb!(2,3,1),                                                  // 13: bz[1]
    fb!(2,2,4),                                                  // 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),                            // 15..22: gw[7:0]
    fb!(2,2,5),                                                  // 23: by[5]
    fb!(1,2,4),                                                  // 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),                            // 25..32: bw[7:0]
    fb!(2,3,5),                                                  // 33: bz[5]
    fb!(2,3,4),                                                  // 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),      // 35..39: rx[4:0]
    fb!(1,3,4),                                                  // 40: gz[4]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                 // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),      // 45..49: gx[4:0]
    fb!(2,3,0),                                                  // 50: bz[0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                 // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),
    fb!(2,1,5),                                                  // 55..60: bx[5:0]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                 // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),      // 65..69: ry[4:0]
    fb!(2,3,2),                                                  // 70: bz[2]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),      // 71..75: rz[4:0]
    fb!(2,3,3),                                                  // 76: bz[3]
];

// ---- Mode 9 (TWO; 6.6.6.6; no deltas; prefix 11110) --------------------
// All four endpoints stored as absolute 6-bit values.
#[rustfmt::skip]
const M9_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),fb!(0,0,5), // 5..10: rw[5:0]
    fb!(1,3,4),                                                          // 11: gz[4]
    fb!(2,3,0),                                                          // 12: bz[0]
    fb!(2,3,1),                                                          // 13: bz[1]
    fb!(2,2,4),                                                          // 14: by[4]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),fb!(1,0,5), // 15..20: gw[5:0]
    fb!(1,2,5),                                                          // 21: gy[5]
    fb!(2,2,5),                                                          // 22: by[5]
    fb!(2,3,2),                                                          // 23: bz[2]
    fb!(1,2,4),                                                          // 24: gy[4]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),fb!(2,0,5), // 25..30: bw[5:0]
    fb!(1,3,5),                                                          // 31: gz[5]
    fb!(2,3,3),                                                          // 32: bz[3]
    fb!(2,3,5),                                                          // 33: bz[5]
    fb!(2,3,4),                                                          // 34: bz[4]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),fb!(0,1,5), // 35..40: rx[5:0]
    fb!(1,2,0),fb!(1,2,1),fb!(1,2,2),fb!(1,2,3),                       // 41..44: gy[3:0]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),fb!(1,1,5), // 45..50: gx[5:0]
    fb!(1,3,0),fb!(1,3,1),fb!(1,3,2),fb!(1,3,3),                       // 51..54: gz[3:0]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),fb!(2,1,5), // 55..60: bx[5:0]
    fb!(2,2,0),fb!(2,2,1),fb!(2,2,2),fb!(2,2,3),                       // 61..64: by[3:0]
    fb!(0,2,0),fb!(0,2,1),fb!(0,2,2),fb!(0,2,3),fb!(0,2,4),fb!(0,2,5), // 65..70: ry[5:0]
    fb!(0,3,0),fb!(0,3,1),fb!(0,3,2),fb!(0,3,3),fb!(0,3,4),fb!(0,3,5), // 71..76: rz[5:0]
];

// ---- Mode 10 (ONE; 10.10; no deltas; prefix 00011) ---------------------
// Dense layout: rw[9:0] gw[9:0] bw[9:0] rx[9:0] gx[9:0] bx[9:0]
#[rustfmt::skip]
const M10_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),
    fb!(0,1,5),fb!(0,1,6),fb!(0,1,7),fb!(0,1,8),fb!(0,1,9),
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),
    fb!(1,1,5),fb!(1,1,6),fb!(1,1,7),fb!(1,1,8),fb!(1,1,9),
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),
    fb!(2,1,5),fb!(2,1,6),fb!(2,1,7),fb!(2,1,8),fb!(2,1,9),
];

// ---- Mode 11 (ONE; 11.9; prefix 00111) ---------------------------------
// rw[9:0] gw[9:0] bw[9:0] rx[8:0] rw[10] gx[8:0] gw[10] bx[8:0] bw[10]
#[rustfmt::skip]
const M11_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),
    fb!(0,1,5),fb!(0,1,6),fb!(0,1,7),fb!(0,1,8),                 // 35..43: rx[8:0]
    fb!(0,0,10),                                                  // 44: rw[10]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),
    fb!(1,1,5),fb!(1,1,6),fb!(1,1,7),fb!(1,1,8),                 // 45..53: gx[8:0]
    fb!(1,0,10),                                                  // 54: gw[10]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),
    fb!(2,1,5),fb!(2,1,6),fb!(2,1,7),fb!(2,1,8),                 // 55..63: bx[8:0]
    fb!(2,0,10),                                                  // 64: bw[10]
];

// ---- Mode 12 (ONE; 12.8; prefix 01011) ---------------------------------
// rw[9:0] gw[9:0] bw[9:0] rx[7:0] rw[11] rw[10] gx[7:0] gw[11] gw[10]
// bx[7:0] bw[11] bw[10]
//
// Note: rw[11] / gw[11] / bw[11] are the high bits of the 12-bit base
// endpoint. The Intel PRM describes these as the bit-reversed pair
// (rw[11], rw[10]) at bits (43, 44) etc.
#[rustfmt::skip]
const M12_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),fb!(0,1,4),
    fb!(0,1,5),fb!(0,1,6),fb!(0,1,7),                            // 35..42: rx[7:0]
    fb!(0,0,11),                                                  // 43: rw[11]
    fb!(0,0,10),                                                  // 44: rw[10]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),fb!(1,1,4),
    fb!(1,1,5),fb!(1,1,6),fb!(1,1,7),                            // 45..52: gx[7:0]
    fb!(1,0,11),                                                  // 53: gw[11]
    fb!(1,0,10),                                                  // 54: gw[10]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),fb!(2,1,4),
    fb!(2,1,5),fb!(2,1,6),fb!(2,1,7),                            // 55..62: bx[7:0]
    fb!(2,0,11),                                                  // 63: bw[11]
    fb!(2,0,10),                                                  // 64: bw[10]
];

// ---- Mode 13 (ONE; 16.4; prefix 01111) ---------------------------------
// rw[9:0] gw[9:0] bw[9:0] rx[3:0] rw[15..10] gx[3:0] gw[15..10]
// bx[3:0] bw[15..10]
//
// Bit 39: rw[15], 40: rw[14], 41: rw[13], 42: rw[12], 43: rw[11], 44: rw[10].
#[rustfmt::skip]
const M13_FIELDS: &[FieldBit] = &[
    fb!(0,0,0),fb!(0,0,1),fb!(0,0,2),fb!(0,0,3),fb!(0,0,4),
    fb!(0,0,5),fb!(0,0,6),fb!(0,0,7),fb!(0,0,8),fb!(0,0,9),      // 5..14: rw[9:0]
    fb!(1,0,0),fb!(1,0,1),fb!(1,0,2),fb!(1,0,3),fb!(1,0,4),
    fb!(1,0,5),fb!(1,0,6),fb!(1,0,7),fb!(1,0,8),fb!(1,0,9),      // 15..24: gw[9:0]
    fb!(2,0,0),fb!(2,0,1),fb!(2,0,2),fb!(2,0,3),fb!(2,0,4),
    fb!(2,0,5),fb!(2,0,6),fb!(2,0,7),fb!(2,0,8),fb!(2,0,9),      // 25..34: bw[9:0]
    fb!(0,1,0),fb!(0,1,1),fb!(0,1,2),fb!(0,1,3),                 // 35..38: rx[3:0]
    fb!(0,0,15),                                                  // 39: rw[15]
    fb!(0,0,14),                                                  // 40: rw[14]
    fb!(0,0,13),                                                  // 41: rw[13]
    fb!(0,0,12),                                                  // 42: rw[12]
    fb!(0,0,11),                                                  // 43: rw[11]
    fb!(0,0,10),                                                  // 44: rw[10]
    fb!(1,1,0),fb!(1,1,1),fb!(1,1,2),fb!(1,1,3),                 // 45..48: gx[3:0]
    fb!(1,0,15),                                                  // 49: gw[15]
    fb!(1,0,14),                                                  // 50: gw[14]
    fb!(1,0,13),                                                  // 51: gw[13]
    fb!(1,0,12),                                                  // 52: gw[12]
    fb!(1,0,11),                                                  // 53: gw[11]
    fb!(1,0,10),                                                  // 54: gw[10]
    fb!(2,1,0),fb!(2,1,1),fb!(2,1,2),fb!(2,1,3),                 // 55..58: bx[3:0]
    fb!(2,0,15),                                                  // 59: bw[15]
    fb!(2,0,14),                                                  // 60: bw[14]
    fb!(2,0,13),                                                  // 61: bw[13]
    fb!(2,0,12),                                                  // 62: bw[12]
    fb!(2,0,11),                                                  // 63: bw[11]
    fb!(2,0,10),                                                  // 64: bw[10]
];

/// Returns `(prefix_len, mode_index)` for a BC6H block, or
/// `(prefix_len, INVALID)` for the four reserved 5-bit prefixes
/// (`10011`, `10111`, `11011`, `11111`).
const INVALID_MODE: u32 = u32::MAX;

fn decode_mode(block: &[u8; 16]) -> (u32, u32) {
    // Inspect the first 5 bits LSB-first.
    let lo2 = (block[0] & 0x03) as u32;
    let hi3 = ((block[0] >> 2) & 0x07) as u32;
    let prefix5 = lo2 | (hi3 << 2);
    match prefix5 {
        // 5-bit prefixes (Intel PRM table values):
        0b00010 => (5, 2),
        0b00110 => (5, 3),
        0b01010 => (5, 4),
        0b01110 => (5, 5),
        0b10010 => (5, 6),
        0b10110 => (5, 7),
        0b11010 => (5, 8),
        0b11110 => (5, 9),
        0b00011 => (5, 10),
        0b00111 => (5, 11),
        0b01011 => (5, 12),
        0b01111 => (5, 13),
        // 5-bit prefix is reserved iff prefix5 in {10011, 10111, 11011, 11111}.
        0b10011 | 0b10111 | 0b11011 | 0b11111 => (5, INVALID_MODE),
        _ => {
            // Fall through to 2-bit prefixes (mode 0 / 1 use lo2 == 00 / 01).
            // Note: the 5-bit prefix branch above already disambiguated all
            // values where the high 3 bits matter; here lo2 must be 00 or 01.
            if lo2 == 0b00 {
                (2, 0)
            } else if lo2 == 0b01 {
                (2, 1)
            } else {
                // lo2 in {10, 11} with hi3 not matching any 5-bit pattern.
                // This should not happen given the 5-bit table above is
                // exhaustive for hi3, but treat as reserved.
                (5, INVALID_MODE)
            }
        }
    }
}

pub(crate) fn mode_info(mode: u32) -> Option<ModeInfo> {
    Some(match mode {
        0 => ModeInfo {
            subsets: 2,
            prec_r: 10,
            prec_g: 10,
            prec_b: 10,
            delta_r: 5,
            delta_g: 5,
            delta_b: 5,
            fields: M0_FIELDS,
            idx_bits: 3,
        },
        1 => ModeInfo {
            subsets: 2,
            prec_r: 7,
            prec_g: 7,
            prec_b: 7,
            delta_r: 6,
            delta_g: 6,
            delta_b: 6,
            fields: M1_FIELDS,
            idx_bits: 3,
        },
        2 => ModeInfo {
            subsets: 2,
            prec_r: 11,
            prec_g: 11,
            prec_b: 11,
            delta_r: 5,
            delta_g: 4,
            delta_b: 4,
            fields: M2_FIELDS,
            idx_bits: 3,
        },
        3 => ModeInfo {
            subsets: 2,
            prec_r: 11,
            prec_g: 11,
            prec_b: 11,
            delta_r: 4,
            delta_g: 5,
            delta_b: 4,
            fields: M3_FIELDS,
            idx_bits: 3,
        },
        4 => ModeInfo {
            subsets: 2,
            prec_r: 11,
            prec_g: 11,
            prec_b: 11,
            delta_r: 4,
            delta_g: 4,
            delta_b: 5,
            fields: M4_FIELDS,
            idx_bits: 3,
        },
        5 => ModeInfo {
            subsets: 2,
            prec_r: 9,
            prec_g: 9,
            prec_b: 9,
            delta_r: 5,
            delta_g: 5,
            delta_b: 5,
            fields: M5_FIELDS,
            idx_bits: 3,
        },
        6 => ModeInfo {
            subsets: 2,
            prec_r: 8,
            prec_g: 8,
            prec_b: 8,
            delta_r: 6,
            delta_g: 5,
            delta_b: 5,
            fields: M6_FIELDS,
            idx_bits: 3,
        },
        7 => ModeInfo {
            subsets: 2,
            prec_r: 8,
            prec_g: 8,
            prec_b: 8,
            delta_r: 5,
            delta_g: 6,
            delta_b: 5,
            fields: M7_FIELDS,
            idx_bits: 3,
        },
        8 => ModeInfo {
            subsets: 2,
            prec_r: 8,
            prec_g: 8,
            prec_b: 8,
            delta_r: 5,
            delta_g: 5,
            delta_b: 6,
            fields: M8_FIELDS,
            idx_bits: 3,
        },
        9 => ModeInfo {
            subsets: 2,
            prec_r: 6,
            prec_g: 6,
            prec_b: 6,
            delta_r: 0,
            delta_g: 0,
            delta_b: 0,
            fields: M9_FIELDS,
            idx_bits: 3,
        },
        10 => ModeInfo {
            subsets: 1,
            prec_r: 10,
            prec_g: 10,
            prec_b: 10,
            delta_r: 0,
            delta_g: 0,
            delta_b: 0,
            fields: M10_FIELDS,
            idx_bits: 4,
        },
        11 => ModeInfo {
            subsets: 1,
            prec_r: 11,
            prec_g: 11,
            prec_b: 11,
            delta_r: 9,
            delta_g: 9,
            delta_b: 9,
            fields: M11_FIELDS,
            idx_bits: 4,
        },
        12 => ModeInfo {
            subsets: 1,
            prec_r: 12,
            prec_g: 12,
            prec_b: 12,
            delta_r: 8,
            delta_g: 8,
            delta_b: 8,
            fields: M12_FIELDS,
            idx_bits: 4,
        },
        13 => ModeInfo {
            subsets: 1,
            prec_r: 16,
            prec_g: 16,
            prec_b: 16,
            delta_r: 4,
            delta_g: 4,
            delta_b: 4,
            fields: M13_FIELDS,
            idx_bits: 4,
        },
        _ => return None,
    })
}

// ---- Half-float helpers --------------------------------------------------

/// Reinterpret a u16 as IEEE-754 binary16 -> f32 (full precision).
pub fn half_to_f32(h: u16) -> f32 {
    let sign = (h >> 15) & 1;
    let exp = ((h >> 10) & 0x1f) as i32;
    let mant = (h & 0x3ff) as u32;
    let bits = if exp == 0 {
        if mant == 0 {
            (sign as u32) << 31
        } else {
            // Subnormal — normalise.
            let mut m = mant;
            let mut e = -14;
            while (m & 0x400) == 0 {
                m <<= 1;
                e -= 1;
            }
            m &= 0x3ff;
            let exp_f32 = (e + 127) as u32;
            ((sign as u32) << 31) | (exp_f32 << 23) | (m << 13)
        }
    } else if exp == 0x1f {
        ((sign as u32) << 31) | (0xff << 23) | (mant << 13)
    } else {
        let exp_f32 = (exp - 15 + 127) as u32;
        ((sign as u32) << 31) | (exp_f32 << 23) | (mant << 13)
    };
    f32::from_bits(bits)
}

// ---- Endpoint pipeline ---------------------------------------------------
//
// Microsoft's pipeline (mirrored verbatim from the Intel PRM, which uses
// the same algorithm):
//
//   1. Read raw integer endpoints from the bit stream.
//   2. Sign-extend each delta to the per-channel delta width (signed
//      formats also sign-extend the absolute "w" base from prec_x).
//   3. Apply transform inversion: for delta endpoints, x = (w + x) &
//      ((1 << prec) - 1); same for y and z.
//   4. Sign-extend again to prec width if the format is signed (so the
//      wrapped value carries its sign bit at prec-1).
//   5. Unquantize the prec-width value to a 17-bit signed integer.
//   6. Interpolate weight palette: palette[i] = (w * (64 - W[i]) +
//      x * W[i] + 32) >> 6.
//   7. Finalise: BC6H_UF16 multiplies by 31/64 (>> 6); BC6H_SF16
//      multiplies by 31/32 (>> 5) and re-applies the sign.
//
// We implement this with `i32` values throughout.

#[inline]
fn sign_extend(value: i32, bits: u32) -> i32 {
    if bits == 0 {
        return value;
    }
    let sign_bit = 1i32 << (bits - 1);
    let mask = (1i32 << bits) - 1;
    let v = value & mask;
    if (v & sign_bit) != 0 {
        v | !mask
    } else {
        v
    }
}

fn unquantize(comp: i32, bits: u32, signed: bool) -> i32 {
    if !signed {
        if bits >= 15 {
            return comp;
        }
        if comp == 0 {
            return 0;
        }
        if comp == ((1i32 << bits) - 1) {
            return 0xffff;
        }
        ((comp << 16) + 0x8000) >> bits
    } else {
        if bits >= 16 {
            return comp;
        }
        let s = comp < 0;
        let mut c = if s { -comp } else { comp };
        let unq = if c == 0 {
            0
        } else if c >= ((1i32 << (bits - 1)) - 1) {
            0x7fff
        } else {
            ((c << 15) + 0x4000) >> (bits - 1)
        };
        c = unq;
        if s {
            -c
        } else {
            c
        }
    }
}

fn finish_unquantize(comp: i32, signed: bool) -> u16 {
    if !signed {
        let v = (comp.max(0) as u32 * 31) >> 6;
        v as u16
    } else {
        // BC6H_SF16: scale magnitude by 31/32, re-attach sign bit at top.
        let (s, c) = if comp < 0 {
            (1u16, ((-comp) as u32 * 31) >> 5)
        } else {
            (0u16, (comp as u32 * 31) >> 5)
        };
        (s << 15) | (c as u16)
    }
}

/// Decode one BC6H block (16 bytes) into a 4x4 RGBA half-float grid
/// (alpha = 1.0 = `0x3c00`). `signed` selects between `BC6H_SF16` and
/// `BC6H_UF16`. Returns `None` when the block uses one of the four
/// reserved modes (in which case the caller fills with zero per spec).
pub(crate) fn decode_bc6h_block(block: &[u8; 16], signed: bool) -> Option<[[u16; 4]; 16]> {
    let (prefix_bits, mode) = decode_mode(block);
    if mode == INVALID_MODE {
        return None;
    }
    let mi = mode_info(mode)?;

    let mut br = BitReader::new(block);
    br.pos = prefix_bits;

    // Read field bits in declared order, accumulating into per-
    // channel/per-endpoint integers.
    let mut raw = [[0i32; 4]; 3];
    for f in mi.fields.iter() {
        let bit = br.read(1);
        raw[f.channel as usize][f.endpoint as usize] |= (bit as i32) << f.dest_bit;
    }

    // Read partition (TWO modes only).
    let partition = if mi.subsets == 2 { br.read(5) } else { 0 } as usize;

    // For delta modes, sign-extend the deltas to their delta width.
    // For absolute (no-delta) modes, no delta extension; the value is
    // already an absolute. For BC6H_SF16, also sign-extend the "w"
    // base from prec width.
    let prec = [mi.prec_r as u32, mi.prec_g as u32, mi.prec_b as u32];
    let delta = [mi.delta_r as u32, mi.delta_g as u32, mi.delta_b as u32];
    let endpoint_count = (mi.subsets as usize) * 2;

    for (ch, raw_ch) in raw.iter_mut().enumerate().take(3) {
        // Endpoint 0 (w): always absolute. Sign-extend if signed format.
        if signed {
            raw_ch[0] = sign_extend(raw_ch[0], prec[ch]);
        }
        // Endpoints 1..endpoint_count: deltas if delta[ch] != 0, else
        // absolute. Sign-extend deltas always (a delta of 0 width
        // means no delta, which means treat as absolute).
        for slot in raw_ch.iter_mut().take(endpoint_count).skip(1) {
            if delta[ch] != 0 {
                *slot = sign_extend(*slot, delta[ch]);
            } else if signed {
                *slot = sign_extend(*slot, prec[ch]);
            }
        }
    }

    // Transform inversion: for delta-endpoint modes, add w to each of
    // x/y/z and wrap to prec width. Then sign-extend (signed formats).
    for (ch, raw_ch) in raw.iter_mut().enumerate().take(3) {
        if delta[ch] == 0 {
            continue; // no-delta mode: x/y/z are absolute already.
        }
        let mask: i32 = if prec[ch] >= 32 {
            -1
        } else {
            (1i32 << prec[ch]) - 1
        };
        let w = raw_ch[0];
        for slot in raw_ch.iter_mut().take(endpoint_count).skip(1) {
            let v = w.wrapping_add(*slot) & mask;
            *slot = if signed { sign_extend(v, prec[ch]) } else { v };
        }
    }

    // Unquantize each endpoint to 17-bit signed.
    let mut endpoints = [[0i32; 4]; 3];
    for ch in 0..3usize {
        for ep in 0..endpoint_count {
            endpoints[ch][ep] = unquantize(raw[ch][ep], prec[ch], signed);
        }
    }

    // Read indices.
    let mut idx = [0u32; 16];
    for (px, slot) in idx.iter_mut().enumerate() {
        let s = if mi.subsets == 2 {
            PART_2[partition][px] as u32
        } else {
            0
        };
        // Anchor pixels: subset-0 anchor is always pixel 0; subset-1
        // anchor is partition-dependent (BC7's 2-subset table).
        let is_anchor = if mi.subsets == 2 {
            if s == 0 {
                px == 0
            } else {
                px as u32 == ANCHOR_2_SUBSET_2[partition] as u32
            }
        } else {
            px == 0
        };
        let nbits = if is_anchor {
            (mi.idx_bits - 1) as u32
        } else {
            mi.idx_bits as u32
        };
        *slot = br.read(nbits);
    }

    // Per-pixel interpolate + finalise.
    let weights: &[u32] = if mi.idx_bits == 3 {
        &WEIGHT_3
    } else {
        &WEIGHT_4
    };
    let mut out = [[0u16; 4]; 16];
    for px in 0..16usize {
        let s = if mi.subsets == 2 {
            PART_2[partition][px] as usize
        } else {
            0
        };
        let i0 = s * 2;
        let i1 = s * 2 + 1;
        let w = weights[idx[px] as usize] as i64;
        for ch in 0..3usize {
            let a = endpoints[ch][i0] as i64;
            let b = endpoints[ch][i1] as i64;
            let v = ((a * (64 - w) + b * w + 32) >> 6) as i32;
            out[px][ch] = finish_unquantize(v, signed);
        }
        out[px][3] = 0x3c00;
    }

    Some(out)
}

/// Decode a BC6H surface to interleaved RGBA half-float (binary16) bytes.
///
/// `output` must hold `width * height * 8` bytes (4 channels * 2 bytes).
/// The R / G / B half-float values are interpolated per the BC6H spec;
/// alpha is hard-coded to half-float 1.0 (`0x3c00`).
///
/// `signed` selects between `BC6H_SF16` and `BC6H_UF16`.
///
/// All 14 BC6H modes are implemented. The four reserved modes
/// (10011, 10111, 11011, 11111) are decoded as zero RGB per spec
/// without producing an error.
pub fn decode_bc6h(
    input: &[u8],
    width: u32,
    height: u32,
    signed: bool,
    output: &mut [u8],
) -> Result<()> {
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    let want_in = bw * bh * 16;
    if input.len() < want_in {
        return Err(DdsError::invalid(format!(
            "BC6H input {} bytes < expected {} bytes for {}x{}",
            input.len(),
            want_in,
            width,
            height,
        )));
    }
    let want_out = rgba_half_surface_bytes(width, height);
    if output.len() < want_out {
        return Err(DdsError::invalid(format!(
            "BC6H output {} bytes < expected {} bytes for {}x{}",
            output.len(),
            want_out,
            width,
            height,
        )));
    }
    let stride = width as usize * 8;
    for by in 0..bh {
        for bx in 0..bw {
            let off = (by * bw + bx) * 16;
            let block: [u8; 16] = input[off..off + 16].try_into().unwrap();
            // Reserved modes: spec mandates zero RGB output, alpha = 1.
            let pixels = decode_bc6h_block(&block, signed).unwrap_or([[0u16, 0, 0, 0x3c00]; 16]);
            for py in 0..4u32 {
                let yy = by as u32 * 4 + py;
                if yy >= height {
                    continue;
                }
                for px in 0..4u32 {
                    let xx = bx as u32 * 4 + px;
                    if xx >= width {
                        continue;
                    }
                    let p = pixels[(py * 4 + px) as usize];
                    let dst = yy as usize * stride + xx as usize * 8;
                    output[dst..dst + 2].copy_from_slice(&p[0].to_le_bytes());
                    output[dst + 2..dst + 4].copy_from_slice(&p[1].to_le_bytes());
                    output[dst + 4..dst + 6].copy_from_slice(&p[2].to_le_bytes());
                    output[dst + 6..dst + 8].copy_from_slice(&p[3].to_le_bytes());
                }
            }
        }
    }
    Ok(())
}

// Suppress unused warnings for helpers kept for clarity / future use.
#[allow(dead_code)]
fn _unused_helpers() {
    let _ = BitReader::bit_at(&[0u8; 16], 0);
    let _ = rng(0, 0, 0, 0);
    _range_check(0, 0);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pack a sequence of `(value, bit_count)` pairs LSB-first into a
    /// 16-byte BC6H block. Each value is consumed with its low `n` bits.
    fn pack_block(fields: &[(u64, u32)]) -> [u8; 16] {
        let mut block = [0u8; 16];
        let mut pos = 0u32;
        for &(value, n) in fields {
            for i in 0..n {
                let bit = ((value >> i) & 1) as u8;
                let byte = (pos / 8) as usize;
                let shift = pos & 7;
                if byte >= 16 {
                    break;
                }
                block[byte] |= bit << shift;
                pos += 1;
            }
        }
        block
    }

    /// Pack endpoints into the field order specified by `fields` of
    /// the relevant ModeInfo. Returns the 16-byte block.
    ///
    /// `endpoints[ch][ep]` provides the integer to encode for channel
    /// `ch` (0=R, 1=G, 2=B), endpoint `ep` (0..3).
    /// `prefix` is the mode prefix value with `prefix_len` bits.
    fn pack_endpoints(
        prefix: u32,
        prefix_len: u32,
        fields: &[FieldBit],
        endpoints: [[u32; 4]; 3],
        partition: u32,
        partition_len: u32,
        indices: &[(u32, u32)],
    ) -> [u8; 16] {
        let mut block = [0u8; 16];
        let mut pos = 0u32;
        let put = |bit: u32, b: &mut [u8; 16], pos: &mut u32| {
            let byte = (*pos / 8) as usize;
            let shift = *pos & 7;
            b[byte] |= ((bit & 1) as u8) << shift;
            *pos += 1;
        };
        // Mode prefix.
        for i in 0..prefix_len {
            put((prefix >> i) & 1, &mut block, &mut pos);
        }
        // Field bits.
        for f in fields {
            let v = endpoints[f.channel as usize][f.endpoint as usize];
            put((v >> f.dest_bit) & 1, &mut block, &mut pos);
        }
        // Partition.
        for i in 0..partition_len {
            put((partition >> i) & 1, &mut block, &mut pos);
        }
        // Indices.
        for &(value, n) in indices {
            for i in 0..n {
                put((value >> i) & 1, &mut block, &mut pos);
            }
        }
        block
    }

    #[test]
    fn half_to_f32_one() {
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0x0000), 0.0);
        assert_eq!(half_to_f32(0x3800), 0.5);
    }

    /// Mode 10 (1-subset, 10/10 absolute endpoints) — prefix 00011.
    /// Both endpoints set to 0x3ff (10-bit max → unquantizes to 0xffff).
    /// All indices 0 → every pixel is endpoint 0.
    #[test]
    fn bc6h_mode10_max_unsigned() {
        // Mode 10 prefix = 00011 (5 bits, value 0b00011 = 3).
        // Then endpoints: rw[9:0] gw[9:0] bw[9:0] rx[9:0] gx[9:0] bx[9:0]
        let mut fields = vec![(0b00011u64, 5)];
        for _ in 0..6 {
            fields.push((0x3ff, 10));
        }
        // 63 index bits all zero.
        fields.push((0, 63));
        let block = pack_block(&fields);
        let pixels = decode_bc6h_block(&block, false).expect("mode 10 decoded");
        let expected = ((0xffffu32) * 31 / 64) as u16;
        for p in pixels.iter() {
            assert_eq!(p[0], expected, "R");
            assert_eq!(p[1], expected, "G");
            assert_eq!(p[2], expected, "B");
            assert_eq!(p[3], 0x3c00, "alpha");
        }
    }

    /// Mode 10 with split endpoints — pixel 0 anchor (3 bits) vs full
    /// indices to validate the 4-bit anchor short-by-1 rule.
    #[test]
    fn bc6h_mode10_split_endpoints() {
        let mut fields = vec![(0b00011u64, 5)];
        fields.push((0, 10)); // rw = 0
        fields.push((0, 10)); // gw = 0
        fields.push((0, 10)); // bw = 0
        fields.push((0x3ff, 10)); // rx = 0x3ff
        fields.push((0x3ff, 10)); // gx = 0x3ff
        fields.push((0x3ff, 10)); // bx = 0x3ff
                                  // Pixel 0: 3-bit anchor index = 7 (max 3-bit). Pixels 1..15: 4-bit = 15.
        fields.push((7, 3));
        for _ in 1..16 {
            fields.push((15, 4));
        }
        let block = pack_block(&fields);
        let pixels = decode_bc6h_block(&block, false).expect("mode 10 decoded");
        // Pixel 0 anchor: weight for index 7 = 30 (out of WEIGHT_4).
        let w0 = WEIGHT_4[7] as i64;
        let unq_a = 0i32;
        let unq_b = 0xffffi32;
        let v = ((unq_a as i64 * (64 - w0) + unq_b as i64 * w0 + 32) >> 6) as i32;
        let exp_anchor = (v as u32 * 31 / 64) as u16;
        assert_eq!(pixels[0][0], exp_anchor);
        // Pixels 1..15: weight 64 → endpoint B exactly → 0xffff * 31 / 64.
        let exp_b = (0xffffu32 * 31 / 64) as u16;
        for p in pixels.iter().skip(1) {
            assert_eq!(p[0], exp_b);
            assert_eq!(p[1], exp_b);
            assert_eq!(p[2], exp_b);
            assert_eq!(p[3], 0x3c00);
        }
    }

    /// Mode 9 (TWO; 6.6.6.6 absolute) — prefix 11110. Confirms field
    /// table works for a 2-subset no-delta mode.
    #[test]
    fn bc6h_mode9_solid() {
        let mi = mode_info(9).unwrap();
        let endpoints = [[0u32, 0, 0, 0]; 3]; // all endpoints zero
        let mut indices = vec![(0u32, 2)]; // pixel 0 anchor (subset 0): 2 bits
        for _ in 1..16 {
            indices.push((0, 3));
        }
        // Note: subset-1 anchor pixel needs a 2-bit index too, but with all
        // partitions producing pixel-0 anchored at 0, we don't know which
        // pixel is anchor without pinning the partition. For partition 0,
        // anchors are pixel 0 (subset 0) and pixel 15 (subset 1). Use
        // partition=0 and then make pixel 15 an anchor with 2 bits.
        let mut indices = vec![(0u32, 2)]; // pixel 0
        for _ in 1..15 {
            indices.push((0, 3));
        }
        indices.push((0, 2)); // pixel 15 (subset-1 anchor for partition 0)
        let block = pack_endpoints(0b11110, 5, mi.fields, endpoints, 0, 5, &indices);
        let pixels = decode_bc6h_block(&block, false).expect("mode 9 decoded");
        for p in pixels.iter() {
            assert_eq!(p[0], 0);
            assert_eq!(p[1], 0);
            assert_eq!(p[2], 0);
            assert_eq!(p[3], 0x3c00);
        }
    }

    /// Mode 0 — TWO 10.5.5.5. With endpoints 0,0,0,0 the output is zero.
    #[test]
    fn bc6h_mode0_zero() {
        let mi = mode_info(0).unwrap();
        let endpoints = [[0u32, 0, 0, 0]; 3];
        let mut indices = vec![(0u32, 2)];
        for _ in 1..15 {
            indices.push((0, 3));
        }
        indices.push((0, 2));
        let block = pack_endpoints(0b00, 2, mi.fields, endpoints, 0, 5, &indices);
        let pixels = decode_bc6h_block(&block, false).unwrap();
        for p in pixels.iter() {
            assert_eq!(p[0], 0);
            assert_eq!(p[3], 0x3c00);
        }
    }

    /// Mode 1 — TWO 7.6.6.6.
    #[test]
    fn bc6h_mode1_zero() {
        let mi = mode_info(1).unwrap();
        let endpoints = [[0u32, 0, 0, 0]; 3];
        let mut indices = vec![(0u32, 2)];
        for _ in 1..15 {
            indices.push((0, 3));
        }
        indices.push((0, 2));
        let block = pack_endpoints(0b01, 2, mi.fields, endpoints, 0, 5, &indices);
        let pixels = decode_bc6h_block(&block, false).unwrap();
        for p in pixels.iter() {
            assert_eq!(p[0], 0);
            assert_eq!(p[3], 0x3c00);
        }
    }

    /// All 12 missing modes (0, 2, 3, 4, 5, 6, 7, 8, 9, 12, 13) decode
    /// without panic for a zero-endpoint block; outputs all zero RGB.
    #[test]
    fn bc6h_all_modes_zero_block_decodes() {
        let prefixes = [
            (0u32, 2, 0u32), // mode 0
            (1, 2, 1),       // mode 1
            (0b00010, 5, 2),
            (0b00110, 5, 3),
            (0b01010, 5, 4),
            (0b01110, 5, 5),
            (0b10010, 5, 6),
            (0b10110, 5, 7),
            (0b11010, 5, 8),
            (0b11110, 5, 9),
            (0b00011, 5, 10),
            (0b00111, 5, 11),
            (0b01011, 5, 12),
            (0b01111, 5, 13),
        ];
        for (prefix, plen, mode) in prefixes {
            let mi = mode_info(mode).unwrap();
            let endpoints = [[0u32, 0, 0, 0]; 3];
            let mut indices: Vec<(u32, u32)> = vec![];
            if mi.subsets == 2 {
                indices.push((0, 2));
                for _ in 1..15 {
                    indices.push((0, 3));
                }
                indices.push((0, 2));
            } else {
                indices.push((0, 3));
                for _ in 1..16 {
                    indices.push((0, 4));
                }
            }
            let part = 0u32;
            let plen2 = if mi.subsets == 2 { 5 } else { 0 };
            let block = pack_endpoints(prefix, plen, mi.fields, endpoints, part, plen2, &indices);
            let pixels = decode_bc6h_block(&block, false)
                .unwrap_or_else(|| panic!("mode {} should decode", mode));
            for p in pixels.iter() {
                assert_eq!(p[0], 0, "mode {} R != 0", mode);
                assert_eq!(p[1], 0, "mode {} G != 0", mode);
                assert_eq!(p[2], 0, "mode {} B != 0", mode);
                assert_eq!(p[3], 0x3c00, "mode {} alpha != 1.0", mode);
            }
        }
    }

    /// Reserved 5-bit prefixes (10011, 10111, 11011, 11111) decode to
    /// zero RGB without erroring at the surface level.
    #[test]
    fn bc6h_reserved_prefix_zero_rgb() {
        let mut block = [0u8; 16];
        block[0] = 0b10011;
        let mut out = vec![0u8; 4 * 4 * 8];
        decode_bc6h(&block, 4, 4, false, &mut out).expect("reserved decodes to zero");
        for chunk in out.chunks_exact(8) {
            let r = u16::from_le_bytes([chunk[0], chunk[1]]);
            let g = u16::from_le_bytes([chunk[2], chunk[3]]);
            let b = u16::from_le_bytes([chunk[4], chunk[5]]);
            let a = u16::from_le_bytes([chunk[6], chunk[7]]);
            assert_eq!(r, 0);
            assert_eq!(g, 0);
            assert_eq!(b, 0);
            assert_eq!(a, 0x3c00);
        }
    }

    /// Mode 13 — ONE 16.4 with rw / gw / bw set to a moderate value.
    #[test]
    fn bc6h_mode13_zero_endpoints() {
        let mi = mode_info(13).unwrap();
        let endpoints = [[0u32, 0, 0, 0]; 3];
        let mut indices = vec![(0u32, 3)];
        for _ in 1..16 {
            indices.push((0, 4));
        }
        let block = pack_endpoints(0b01111, 5, mi.fields, endpoints, 0, 0, &indices);
        let pixels = decode_bc6h_block(&block, false).unwrap();
        for p in pixels.iter() {
            assert_eq!(p[0], 0);
            assert_eq!(p[1], 0);
            assert_eq!(p[2], 0);
            assert_eq!(p[3], 0x3c00);
        }
    }

    /// Surface-level decode should succeed for all 14 modes.
    #[test]
    fn bc6h_decode_surface_all_modes() {
        let prefixes = [
            (0u32, 2, 0u32),
            (1, 2, 1),
            (0b00010, 5, 2),
            (0b00110, 5, 3),
            (0b01010, 5, 4),
            (0b01110, 5, 5),
            (0b10010, 5, 6),
            (0b10110, 5, 7),
            (0b11010, 5, 8),
            (0b11110, 5, 9),
            (0b00011, 5, 10),
            (0b00111, 5, 11),
            (0b01011, 5, 12),
            (0b01111, 5, 13),
        ];
        for (prefix, plen, mode) in prefixes {
            let mi = mode_info(mode).unwrap();
            let endpoints = [[0u32, 0, 0, 0]; 3];
            let mut indices: Vec<(u32, u32)> = vec![];
            if mi.subsets == 2 {
                indices.push((0, 2));
                for _ in 1..15 {
                    indices.push((0, 3));
                }
                indices.push((0, 2));
            } else {
                indices.push((0, 3));
                for _ in 1..16 {
                    indices.push((0, 4));
                }
            }
            let plen2 = if mi.subsets == 2 { 5 } else { 0 };
            let block = pack_endpoints(prefix, plen, mi.fields, endpoints, 0, plen2, &indices);
            let mut out = vec![0u8; 4 * 4 * 8];
            decode_bc6h(&block, 4, 4, false, &mut out)
                .unwrap_or_else(|e| panic!("mode {} surface decode: {:?}", mode, e));
        }
    }

    /// Mode 10 with non-trivial endpoints: a graded ramp should produce
    /// a smooth interpolated result without overflow.
    #[test]
    fn bc6h_mode10_ramp() {
        // r0=0, r1=0x3ff. Indices: pixel i = i (4-bit). Pixel 0 = 3-bit
        // anchor. So pixel 0 has index 0 (3 bits stored), pixels 1..15
        // have index i (4 bits).
        let mut fields = vec![(0b00011u64, 5)];
        fields.push((0, 10)); // rw = 0
        fields.push((0, 10)); // gw = 0
        fields.push((0, 10)); // bw = 0
        fields.push((0x3ff, 10)); // rx = 0x3ff
        fields.push((0x3ff, 10)); // gx = 0x3ff
        fields.push((0x3ff, 10)); // bx = 0x3ff
        fields.push((0, 3)); // pixel 0 (anchor, 3 bits) = 0
        for i in 1..16u32 {
            fields.push((i as u64, 4));
        }
        let block = pack_block(&fields);
        let pixels = decode_bc6h_block(&block, false).unwrap();
        // Pixel 0: index 0 → endpoint A → 0.
        assert_eq!(pixels[0][0], 0);
        assert_eq!(pixels[0][1], 0);
        assert_eq!(pixels[0][2], 0);
        // Pixel 15: index 15 → endpoint B → 0xffff*31/64.
        let exp_b = (0xffffu32 * 31 / 64) as u16;
        assert_eq!(pixels[15][0], exp_b);
        assert_eq!(pixels[15][1], exp_b);
        assert_eq!(pixels[15][2], exp_b);
        // Interpolated values strictly increasing.
        for i in 1..16 {
            assert!(
                pixels[i][0] >= pixels[i - 1][0],
                "pixel {} R={:#x} < pixel {} R={:#x}",
                i,
                pixels[i][0],
                i - 1,
                pixels[i - 1][0]
            );
        }
    }
}
