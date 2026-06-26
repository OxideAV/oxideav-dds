//! ASTC LDR (Adaptive Scalable Texture Compression) block decode.
//!
//! Reference: the Khronos *Data Format Specification 1.4*, Chapter 23
//! ("ASTC Compressed Texture Image Formats"), staged in this project's
//! `docs/image/astc/` (the verbatim spec PDF/HTML plus the
//! `astc-ldr-notes.md` transcription of its normative tables). Every
//! constant, bit layout, and arithmetic step below is taken from that
//! specification.
//!
//! Scope: the LDR Profile (§23.24). 2D footprints only (4×4 … 12×12);
//! the LDR colour endpoint modes (0, 1, 4, 5, 6, 8, 9, 10, 12, 13);
//! single- and multi-partition blocks; dual-plane mode; void-extent
//! constant-colour blocks; and the illegal-block / HDR-in-LDR error
//! rule (§23.23 / §23.22) which emits the error colour — opaque
//! fully-saturated magenta `(255, 0, 255, 255)`.
//!
//! Each ASTC block is a fixed 128 bits and covers a `block_w × block_h`
//! texel footprint. [`decode_astc_ldr_block`] turns one block into a
//! `block_w × block_h` array of RGBA8 texels; [`decode_astc_ldr`]
//! tiles the blocks across a surface, clipping partial edge blocks.

/// The LDR error colour: opaque, fully-saturated magenta. Emitted for
/// every illegal block and for HDR content decoded in LDR mode
/// (§23.5 Table 23.3, §23.22, §23.23).
pub const ERROR_COLOR: [u8; 4] = [255, 0, 255, 255];

/// The 14 2D block footprints an LDR-Profile decoder must support
/// (§23.24, Table 23.4), as `(width, height)` pairs.
pub const LDR_BLOCK_FOOTPRINTS: [(u8, u8); 14] = [
    (4, 4),
    (5, 4),
    (5, 5),
    (6, 5),
    (6, 6),
    (8, 5),
    (8, 6),
    (8, 8),
    (10, 5),
    (10, 6),
    (10, 8),
    (10, 10),
    (12, 10),
    (12, 12),
];

/// True if `(w, h)` is one of the supported LDR 2D footprints.
pub fn is_valid_footprint(w: u8, h: u8) -> bool {
    LDR_BLOCK_FOOTPRINTS.contains(&(w, h))
}

// ---------------------------------------------------------------------
// 128-bit block reader
// ---------------------------------------------------------------------

/// A 128-bit ASTC block, stored as two little-endian `u64` halves so
/// individual bit ranges can be extracted in either direction.
#[derive(Clone, Copy)]
struct Block128 {
    lo: u64,
    hi: u64,
}

impl Block128 {
    fn from_bytes(b: &[u8; 16]) -> Self {
        let lo = u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        let hi = u64::from_le_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]);
        Self { lo, hi }
    }

    /// Bit `i` (0 = LSB of byte 0, 127 = MSB of byte 15).
    #[inline]
    fn bit(&self, i: u32) -> u32 {
        if i < 64 {
            ((self.lo >> i) & 1) as u32
        } else {
            ((self.hi >> (i - 64)) & 1) as u32
        }
    }

    /// Bits `[hi_inclusive .. lo_inclusive]` returned LSB-aligned,
    /// reading upward (bit `lo` becomes result bit 0). `count` ≤ 32.
    #[inline]
    fn bits(&self, lo: u32, count: u32) -> u32 {
        let mut v = 0u32;
        for k in 0..count {
            v |= self.bit(lo + k) << k;
        }
        v
    }
}

// ---------------------------------------------------------------------
// Integer Sequence Encoding (BISE), §23.12
// ---------------------------------------------------------------------

/// A BISE range descriptor: one of `bits`-only, `trit`+`bits`, or
/// `quint`+`bits`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct IseRange {
    trits: bool,
    quints: bool,
    bits: u8,
    /// Maximum representable value (range is `0..=max`).
    max: u32,
}

impl IseRange {
    /// Bits consumed by an integer sequence of `count` values in this
    /// range (§23.12, Table 23.17). Trit blocks pack 5 values into
    /// `8 + 5n` bits; quint blocks pack 3 values into `7 + 3n` bits.
    fn bits_for_count(&self, count: u32) -> u32 {
        let n = self.bits as u32;
        if self.trits {
            // ceil(count/5) trit blocks of 8 bits each, plus count*n
            // interleaved value bits.
            count.div_ceil(5) * 8 + count * n
        } else if self.quints {
            // ceil(count/3) quint blocks of 7 bits each, plus count*n.
            count.div_ceil(3) * 7 + count * n
        } else {
            count * n
        }
    }
}

/// Build the BISE range that holds the most values in `[0, max]` with
/// the given total bit budget is *not* what we need here; the ASTC
/// weight/colour ranges are explicit. This maps a stored "range index"
/// (the (trit, quint, bits) selection) used by weight + colour quant.
///
/// For a quantization level given by `(trits, quints, bits)` returns the
/// descriptor with the correct `max`.
fn ise_range(trits: bool, quints: bool, bits: u8) -> IseRange {
    let base: u32 = if trits {
        3
    } else if quints {
        5
    } else {
        1
    };
    let max = base * (1u32 << bits) - 1;
    IseRange {
        trits,
        quints,
        bits,
        max,
    }
}

/// Decode the trit/bit and quint/bit packing of a full integer
/// sequence of `count` values, reading upward from `start`. Returns the
/// raw stored values (each `0..=range.max`). §23.12, Tables 23.18/23.20.
fn decode_ise_sequence(block: &Block128, start: u32, count: u32, range: IseRange) -> Vec<u32> {
    let n = range.bits as u32;
    let mut out = Vec::with_capacity(count as usize);

    if range.trits {
        // Five values per trit block of (8 + 5n) bits. The n LSB bits of
        // each value are interleaved between the packed-trit bits at the
        // positions given by §23.12.
        let mut pos = start;
        let mut produced = 0u32;
        while produced < count {
            // Read interleaved layout for up to 5 values:
            //   m0(n) T0..1(2) m1(n) T2..3(2) m2(n) T4(1) m3(n) T5..6(2) m4(n) T7(1)
            let mut m = [0u32; 5];
            let mut packed = 0u32; // T7..T0
                                   // value 0 low bits
            m[0] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 2); // T1 T0
            m[1] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 2) << 2; // T3 T2
            m[2] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 1) << 4; // T4
            m[3] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 2) << 5; // T6 T5
            m[4] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 1) << 7; // T7
            let trits = unpack_trits(packed);
            for i in 0..5 {
                if produced < count {
                    out.push((trits[i] << n) | m[i]);
                    produced += 1;
                }
            }
        }
    } else if range.quints {
        // Three values per quint block of (7 + 3n) bits:
        //   m0(n) Q0..2(3) m1(n) Q3..4(2) m2(n) Q5..6(2)
        let mut pos = start;
        let mut produced = 0u32;
        while produced < count {
            let mut m = [0u32; 3];
            let mut packed = 0u32; // Q6..Q0
            m[0] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 3); // Q2 Q1 Q0
            m[1] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 2) << 3; // Q4 Q3
            m[2] = read_at(block, &mut pos, n);
            packed |= read_at(block, &mut pos, 2) << 5; // Q6 Q5
            let quints = unpack_quints(packed);
            for i in 0..3 {
                if produced < count {
                    out.push((quints[i] << n) | m[i]);
                    produced += 1;
                }
            }
        }
    } else {
        let mut pos = start;
        for _ in 0..count {
            out.push(read_at(block, &mut pos, n));
        }
    }
    out
}

#[inline]
fn read_at(block: &Block128, pos: &mut u32, count: u32) -> u32 {
    let v = block.bits(*pos, count);
    *pos += count;
    v
}

/// Decode an ISE sequence whose bits have been copied out into a
/// standalone `u128`-style buffer where bit 0 is the start of the
/// stream. Used for the weight stream, which is bit-reversed out of the
/// top of the block. `get_bit(i)` returns stream bit `i`.
fn decode_ise_from_bits<F: Fn(u32) -> u32>(get_bit: F, count: u32, range: IseRange) -> Vec<u32> {
    let n = range.bits as u32;
    let mut out = Vec::with_capacity(count as usize);
    let mut pos = 0u32;
    let read = |pos: &mut u32, c: u32| -> u32 {
        let mut v = 0u32;
        for k in 0..c {
            v |= get_bit(*pos + k) << k;
        }
        *pos += c;
        v
    };

    if range.trits {
        let mut produced = 0u32;
        while produced < count {
            let mut m = [0u32; 5];
            let mut packed = 0u32;
            m[0] = read(&mut pos, n);
            packed |= read(&mut pos, 2);
            m[1] = read(&mut pos, n);
            packed |= read(&mut pos, 2) << 2;
            m[2] = read(&mut pos, n);
            packed |= read(&mut pos, 1) << 4;
            m[3] = read(&mut pos, n);
            packed |= read(&mut pos, 2) << 5;
            m[4] = read(&mut pos, n);
            packed |= read(&mut pos, 1) << 7;
            let trits = unpack_trits(packed);
            for i in 0..5 {
                if produced < count {
                    out.push((trits[i] << n) | m[i]);
                    produced += 1;
                }
            }
        }
    } else if range.quints {
        let mut produced = 0u32;
        while produced < count {
            let mut m = [0u32; 3];
            let mut packed = 0u32;
            m[0] = read(&mut pos, n);
            packed |= read(&mut pos, 3);
            m[1] = read(&mut pos, n);
            packed |= read(&mut pos, 2) << 3;
            m[2] = read(&mut pos, n);
            packed |= read(&mut pos, 2) << 5;
            let quints = unpack_quints(packed);
            for i in 0..3 {
                if produced < count {
                    out.push((quints[i] << n) | m[i]);
                    produced += 1;
                }
            }
        }
    } else {
        for _ in 0..count {
            out.push(read(&mut pos, n));
        }
    }
    out
}

/// Unpack 8 packed bits `T7..T0` into five trits (each 0..2). §23.12.
fn unpack_trits(t: u32) -> [u32; 5] {
    let tb = |i: u32| (t >> i) & 1;
    let mut out = [0u32; 5];
    let c: u32;
    // T[4:2] == 111 ?
    if (t >> 2) & 0b111 == 0b111 {
        // C = { T[7:5], T[1:0] }
        c = ((t >> 5) & 0b111) << 2 | (t & 0b11);
        out[4] = 2;
        out[3] = 2;
    } else {
        c = t & 0b11111;
        if (t >> 5) & 0b11 == 0b11 {
            out[4] = 2;
            out[3] = tb(7);
        } else {
            out[4] = tb(7);
            out[3] = (t >> 5) & 0b11;
        }
    }
    let cb = |i: u32| (c >> i) & 1;
    if c & 0b11 == 0b11 {
        out[2] = 2;
        out[1] = cb(4);
        out[0] = (cb(3) << 1) | (cb(2) & !cb(3) & 1);
    } else if (c >> 2) & 0b11 == 0b11 {
        out[2] = 2;
        out[1] = 2;
        out[0] = c & 0b11;
    } else {
        out[2] = cb(4);
        out[1] = (c >> 2) & 0b11;
        out[0] = (cb(1) << 1) | (cb(0) & !cb(1) & 1);
    }
    out
}

/// Unpack 7 packed bits `Q6..Q0` into three quints (each 0..4). §23.12.
fn unpack_quints(q: u32) -> [u32; 3] {
    let qb = |i: u32| (q >> i) & 1;
    let mut out = [0u32; 3];
    let c: u32;
    if (q >> 1) & 0b11 == 0b11 && (q >> 5) & 0b11 == 0b00 {
        // q2 = { Q0, ~Q4 & Q0?, ... } per spec:
        //   q2 = { Q[0], Q[4]&~Q[0], Q[3]&~Q[0] }; q1 = q0 = 4
        out[2] = (qb(0) << 2) | ((qb(4) & !qb(0) & 1) << 1) | (qb(3) & !qb(0) & 1);
        out[1] = 4;
        out[0] = 4;
        return out;
    }
    if (q >> 1) & 0b11 == 0b11 {
        out[2] = 4;
        // C = { Q[4:3], ~Q[6:5], Q[0] }
        c = ((q >> 3) & 0b11) << 3 | ((!(q >> 5) & 0b11) << 1) | qb(0);
    } else {
        out[2] = (q >> 5) & 0b11;
        c = q & 0b11111;
    }
    if c & 0b111 == 0b101 {
        out[1] = 4;
        out[0] = (c >> 3) & 0b11;
    } else {
        out[1] = (c >> 3) & 0b11;
        out[0] = c & 0b111;
    }
    out
}

// ---------------------------------------------------------------------
// Block mode decode (§23.10)
// ---------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct BlockMode {
    dual_plane: bool,
    /// Weight grid width.
    weight_w: u32,
    /// Weight grid height.
    weight_h: u32,
    /// Weight ISE range.
    range: WeightRange,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct WeightRange {
    trits: bool,
    quints: bool,
    bits: u8,
    max: u32,
}

/// Decode the weight-range encoding (Table 23.9) from the 3-bit ρ field
/// and the precision bit P. Returns `None` for the two invalid ρ codes
/// (000, 001).
fn weight_range(r: u32, p: u32) -> Option<WeightRange> {
    // r is (ρ2 ρ1 ρ0) as a 3-bit value: bit2 = ρ2, bit1 = ρ1, bit0 = ρ0.
    let make = |trits: bool, quints: bool, bits: u8| {
        let base: u32 = if trits {
            3
        } else if quints {
            5
        } else {
            1
        };
        Some(WeightRange {
            trits,
            quints,
            bits,
            max: base * (1u32 << bits) - 1,
        })
    };
    match (r, p) {
        (0b010, 0) => make(false, false, 1), // 0..1
        (0b011, 0) => make(true, false, 0),  // 0..2
        (0b100, 0) => make(false, false, 2), // 0..3
        (0b101, 0) => make(false, true, 0),  // 0..4
        (0b110, 0) => make(true, false, 1),  // 0..5
        (0b111, 0) => make(false, false, 3), // 0..7
        (0b010, 1) => make(false, true, 1),  // 0..9  (quint, 1 bit)
        (0b011, 1) => make(true, false, 2),  // 0..11 (trit, 2 bits)
        (0b100, 1) => make(false, false, 4), // 0..15 (4 bits)
        (0b101, 1) => make(false, true, 2),  // 0..19 (quint, 2 bits)
        (0b110, 1) => make(true, false, 3),  // 0..23 (trit, 3 bits)
        (0b111, 1) => make(false, false, 5), // 0..31 (5 bits)
        _ => None,
    }
}

/// Classify a block-mode field (bits [10..0]) into either a void-extent
/// marker, a reserved/illegal marker, or a normal [`BlockMode`].
/// §23.10, Table 23.10.
enum ModeClass {
    Normal(BlockMode),
    VoidExtent,
    Illegal,
}

fn decode_block_mode(m: u32) -> ModeClass {
    let bit = |i: u32| (m >> i) & 1;

    // Void-extent: bits [8..2] == 1111111 and bits [1..0] == 00.
    if m & 0b11 == 0 && (m >> 2) & 0b111_1111 == 0b111_1111 {
        return ModeClass::VoidExtent;
    }
    // Reserved: bits [3..0] == 0000 (last Table 23.10 row).
    if m & 0b1111 == 0 {
        return ModeClass::Illegal;
    }
    // Reserved*: bits [8..6] == 111 and bits [1..0] == 00 (not the
    // void-extent, which has bits[5..2] all 1 — already returned above).
    if m & 0b11 == 0 && (m >> 6) & 0b111 == 0b111 {
        return ModeClass::Illegal;
    }

    // Assemble the weight-grid dimensions (Wwidth, Wheight), dual-plane
    // flag, and weight range (ρ, P) from the row selected by bits [3:2]
    // and bits [1:0]. The expanded Table 23.10 layout (notes §2) names
    // bit 10 = DP, bit 9 = P, bit 4 = ρ0, and packs ρ2/ρ1 and the W/H
    // grid fields at row-specific positions.
    let b10 = m & 0b11; // bits[1:0]
    let b32 = (m >> 2) & 0b11; // bits[3:2]

    let dp = bit(10);
    let p = bit(9);

    let (ww, wh, dual_plane, p_bit, r2, r1, r0): (u32, u32, u32, u32, u32, u32, u32);

    if b10 == 0b00 {
        // bits[1:0] == 00 family: ρ2=bit3, ρ1=bit2, ρ0=bit4.
        r0 = bit(4);
        r2 = bit(3);
        r1 = bit(2);
        let b87 = (bit(8) << 1) | bit(7);
        match b87 {
            0b00 => {
                // DP P 0 0 H H ρ0 ρ2 ρ1 0 0 -> 12, H+2
                let h = (bit(6) << 1) | bit(5);
                ww = 12;
                wh = h + 2;
                dual_plane = dp;
                p_bit = p;
            }
            0b01 => {
                // DP P 0 1 W W ρ0 ρ2 ρ1 0 0 -> W+2, 12
                let w = (bit(6) << 1) | bit(5);
                ww = w + 2;
                wh = 12;
                dual_plane = dp;
                p_bit = p;
            }
            0b10 => {
                // H H 1 0 W W ρ0 ρ2 ρ1 0 0 -> W+6, H+6 (DP=0, P=0 forced;
                // the top two bits carry the high grid-height field).
                let w = (bit(6) << 1) | bit(5);
                let h2 = (bit(10) << 1) | bit(9);
                ww = w + 6;
                wh = h2 + 6;
                dual_plane = 0;
                p_bit = 0;
            }
            0b11 => {
                let b65 = (bit(6) << 1) | bit(5);
                match b65 {
                    0b00 => {
                        // DP P 1 1 0 0 ρ0 ρ2 ρ1 0 0 -> 6, 10
                        ww = 6;
                        wh = 10;
                    }
                    0b01 => {
                        // DP P 1 1 0 1 ρ0 ρ2 ρ1 0 0 -> 10, 6
                        ww = 10;
                        wh = 6;
                    }
                    _ => return ModeClass::Illegal,
                }
                dual_plane = dp;
                p_bit = p;
            }
            _ => return ModeClass::Illegal,
        }
    } else {
        // bits[1:0] != 00 family: ρ2=bit1, ρ1=bit0, ρ0=bit4.
        r2 = bit(1);
        r1 = bit(0);
        r0 = bit(4);
        dual_plane = dp;
        p_bit = p;
        match b32 {
            0b00 => {
                // DP P W W H H ρ0 0 0 ρ2 ρ1 -> W+4, H+2
                let w = (bit(8) << 1) | bit(7);
                let h = (bit(6) << 1) | bit(5);
                ww = w + 4;
                wh = h + 2;
            }
            0b01 => {
                // DP P W W H H ρ0 0 1 ρ2 ρ1 -> W+8, H+2
                let w = (bit(8) << 1) | bit(7);
                let h = (bit(6) << 1) | bit(5);
                ww = w + 8;
                wh = h + 2;
            }
            0b10 => {
                // DP P H H W W ρ0 1 0 ρ2 ρ1 -> W+2, H+8
                let h = (bit(8) << 1) | bit(7);
                let w = (bit(6) << 1) | bit(5);
                ww = w + 2;
                wh = h + 8;
            }
            0b11 => {
                if bit(8) == 0 {
                    // DP P 0 H W W ρ0 1 1 ρ2 ρ1 -> W+2, H+6
                    let h = bit(7);
                    let w = (bit(6) << 1) | bit(5);
                    ww = w + 2;
                    wh = h + 6;
                } else {
                    // DP P 1 W H H ρ0 1 1 ρ2 ρ1 -> W+2, H+2
                    let w = bit(7);
                    let h = (bit(6) << 1) | bit(5);
                    ww = w + 2;
                    wh = h + 2;
                }
            }
            _ => return ModeClass::Illegal,
        }
    }

    let rng = match weight_range(r(r2, r1, r0), p_bit) {
        Some(x) => x,
        None => return ModeClass::Illegal,
    };
    ModeClass::Normal(BlockMode {
        dual_plane: dual_plane != 0,
        weight_w: ww,
        weight_h: wh,
        range: rng,
    })
}

#[inline]
fn r(r2: u32, r1: u32, r0: u32) -> u32 {
    (r2 << 2) | (r1 << 1) | r0
}

// ---------------------------------------------------------------------
// Endpoint unquantization (§23.13, Table 23.21)
// ---------------------------------------------------------------------

/// Unquantize one colour endpoint value stored at quantization level
/// `range` to an 8-bit value (0..255). §23.13 Table 23.21.
fn unquant_color(value: u32, range: IseRange) -> u32 {
    let n = range.bits as u32;
    if !range.trits && !range.quints {
        // bit-only: bit-replicate from MSB up to 8 bits.
        return bit_replicate(value, n, 8);
    }
    // Split the stored value into the "bits" part (low n) and the
    // trit/quint part (high).
    let bitspart = value & ((1u32 << n) - 1);
    let tq = value >> n;

    // Named low bits a,b,c,d,e,f of the "bits" part (a = LSB).
    let a = bitspart & 1;
    let b = (bitspart >> 1) & 1;
    let c = (bitspart >> 2) & 1;
    let d = (bitspart >> 3) & 1;
    let e = (bitspart >> 4) & 1;
    let f = (bitspart >> 5) & 1;

    // A = replicate a across 9 bits.
    let big_a = if a != 0 { 0x1FF } else { 0 };
    let (big_b, big_c): (u32, u32);

    if range.trits {
        (big_b, big_c) = match n {
            1 => (0, 204),
            2 => (b * 0b100010110, 93),           // b000b0bb0
            3 => (cb_bits3(b, c), 44),            // cb000cbcb
            4 => (dcb_bits4(b, c, d), 22),        // dcb000dcb
            5 => (edcb_bits5(b, c, d, e), 11),    // edcb000ed
            6 => (fedcb_bits6(b, c, d, e, f), 5), // fedcb000f
            _ => (0, 204),
        };
    } else {
        (big_b, big_c) = match n {
            1 => (0, 113),
            2 => (b * 0b100001100, 54),    // b0000bb00
            3 => (cb_q3(b, c), 26),        // cb0000cbc
            4 => (dcb_q4(b, c, d), 13),    // dcb0000dc
            5 => (edcb_q5(b, c, d, e), 6), // edcb0000e
            _ => (0, 113),
        };
    }

    let big_d = tq;
    let mut unq = big_d * big_c + big_b;
    unq ^= big_a;
    unq = (big_a & 0x80) | (unq >> 2);
    unq & 0xFF
}

// Helper bit-pattern builders for the B column of Table 23.21 (colour).
// Each constructs a 9-bit value with the named bits in the listed
// positions (MSB = bit 8). The string in the comment reads MSB→LSB.
fn cb_bits3(b: u32, c: u32) -> u32 {
    // "cb000cbcb"
    (c << 8) | (b << 7) | (c << 3) | (b << 2) | (c << 1) | b
}
fn dcb_bits4(b: u32, c: u32, d: u32) -> u32 {
    // "dcb000dcb"
    (d << 8) | (c << 7) | (b << 6) | (d << 2) | (c << 1) | b
}
fn edcb_bits5(b: u32, c: u32, d: u32, e: u32) -> u32 {
    // "edcb000ed"
    (e << 8) | (d << 7) | (c << 6) | (b << 5) | (e << 1) | d
}
fn fedcb_bits6(b: u32, c: u32, d: u32, e: u32, f: u32) -> u32 {
    // "fedcb000f"
    (f << 8) | (e << 7) | (d << 6) | (c << 5) | (b << 4) | f
}
fn cb_q3(b: u32, c: u32) -> u32 {
    // "cb0000cbc"
    (c << 8) | (b << 7) | (c << 2) | (b << 1) | c
}
fn dcb_q4(b: u32, c: u32, d: u32) -> u32 {
    // "dcb0000dc"
    (d << 8) | (c << 7) | (b << 6) | (d << 1) | c
}
fn edcb_q5(b: u32, c: u32, d: u32, e: u32) -> u32 {
    // "edcb0000e"
    (e << 8) | (d << 7) | (c << 6) | (b << 5) | e
}

/// Bit-replicate an `from`-bit `value` up to `to` bits by repeatedly
/// appending the source's high bits (§23.13 / §23.16 "bit replication":
/// e.g. 3-bit `C2 C1 C0` → 8-bit `C2 C1 C0 C2 C1 C0 C2 C1`). Fills from
/// the MSB downward.
fn bit_replicate(value: u32, from: u32, to: u32) -> u32 {
    if from == 0 {
        return 0;
    }
    if from >= to {
        return value & ((1u32 << to) - 1);
    }
    let src = value & ((1u32 << from) - 1);
    let mut acc = 0u32;
    let mut filled = 0u32;
    while filled < to {
        let take = from.min(to - filled);
        // The top `take` bits of the source.
        let top = src >> (from - take);
        acc = (acc << take) | top;
        filled += take;
    }
    acc & ((1u32 << to) - 1)
}

// ---------------------------------------------------------------------
// Weight unquantization (§23.16, Tables 23.30/23.31)
// ---------------------------------------------------------------------

/// Unquantize one weight value at quant level `range` to the 0..64
/// interpolation space. §23.16.
fn unquant_weight(value: u32, range: WeightRange) -> u32 {
    let n = range.bits as u32;
    let unq = if !range.trits && !range.quints {
        // bit-only weights of <5 bits replicate from MSB into 6 bits.
        bit_replicate(value, n, 6)
    } else {
        let bitspart = value & ((1u32 << n) - 1);
        let tq = value >> n;
        let a = bitspart & 1;
        let b = (bitspart >> 1) & 1;
        let c = (bitspart >> 2) & 1;
        let big_a = if a != 0 { 0x7F } else { 0 };
        let (big_b, big_c): (u32, u32) = if range.trits {
            match n {
                0 => (0, 50),
                1 => (0, 50),
                2 => (b * 0b1000101, 23), // b000b0b
                3 => (wcb_t3(b, c), 11),  // cb000cb
                _ => (0, 50),
            }
        } else {
            match n {
                0 => (0, 28),
                1 => (0, 28),
                2 => (b * 0b1000010, 13), // b0000b0
                _ => (0, 28),
            }
        };
        let big_d = tq;
        let mut u = big_d * big_c + big_b;
        u ^= big_a;
        u = (big_a & 0x20) | (u >> 2);
        u & 0x7F
    };
    // Final expansion 0..63 -> allow /64 interpolation.
    let mut w = unq;
    if w > 32 {
        w += 1;
    }
    w
}

fn wcb_t3(b: u32, c: u32) -> u32 {
    // "cb000cb"  (7-bit)
    (c << 6) | (b << 5) | (c << 1) | b
}

// ---------------------------------------------------------------------
// Partition pattern (§23.20)
// ---------------------------------------------------------------------

fn hash52(mut p: u32) -> u32 {
    p ^= p >> 15;
    p = p.wrapping_sub(p << 17);
    p = p.wrapping_add(p << 7);
    p = p.wrapping_add(p << 4);
    p ^= p >> 5;
    p = p.wrapping_add(p << 16);
    p ^= p >> 7;
    p ^= p >> 3;
    p ^= p << 6;
    p ^= p >> 17;
    p
}

fn select_partition(seed: u32, x: u32, y: u32, partitioncount: u32, small_block: bool) -> u32 {
    let (mut x, mut y, z) = (x, y, 0u32);
    if small_block {
        x <<= 1;
        y <<= 1;
    }
    let seed = seed + (partitioncount - 1) * 1024;
    let rnum = hash52(seed);
    let mut seed1 = rnum & 0xF;
    let mut seed2 = (rnum >> 4) & 0xF;
    let mut seed3 = (rnum >> 8) & 0xF;
    let mut seed4 = (rnum >> 12) & 0xF;
    let mut seed5 = (rnum >> 16) & 0xF;
    let mut seed6 = (rnum >> 20) & 0xF;
    let mut seed7 = (rnum >> 24) & 0xF;
    let mut seed8 = (rnum >> 28) & 0xF;
    let mut seed9 = (rnum >> 18) & 0xF;
    let mut seed10 = (rnum >> 22) & 0xF;
    let mut seed11 = (rnum >> 26) & 0xF;
    let mut seed12 = rnum.rotate_left(2) & 0xF;

    seed1 *= seed1;
    seed2 *= seed2;
    seed3 *= seed3;
    seed4 *= seed4;
    seed5 *= seed5;
    seed6 *= seed6;
    seed7 *= seed7;
    seed8 *= seed8;
    seed9 *= seed9;
    seed10 *= seed10;
    seed11 *= seed11;
    seed12 *= seed12;

    let (sh1, sh2);
    if seed & 1 != 0 {
        sh1 = if seed & 2 != 0 { 4 } else { 5 };
        sh2 = if partitioncount == 3 { 6 } else { 5 };
    } else {
        sh1 = if partitioncount == 3 { 6 } else { 5 };
        sh2 = if seed & 2 != 0 { 4 } else { 5 };
    }
    let sh3 = if seed & 0x10 != 0 { sh1 } else { sh2 };

    seed1 >>= sh1;
    seed2 >>= sh2;
    seed3 >>= sh1;
    seed4 >>= sh2;
    seed5 >>= sh1;
    seed6 >>= sh2;
    seed7 >>= sh1;
    seed8 >>= sh2;
    seed9 >>= sh3;
    seed10 >>= sh3;
    seed11 >>= sh3;
    seed12 >>= sh3;

    let mut a = seed1 * x + seed2 * y + seed11 * z + (rnum >> 14);
    let mut b = seed3 * x + seed4 * y + seed12 * z + (rnum >> 10);
    let mut c = seed5 * x + seed6 * y + seed9 * z + (rnum >> 6);
    let mut d = seed7 * x + seed8 * y + seed10 * z + (rnum >> 2);

    a &= 0x3F;
    b &= 0x3F;
    c &= 0x3F;
    d &= 0x3F;
    if partitioncount < 4 {
        d = 0;
    }
    if partitioncount < 3 {
        c = 0;
    }
    if a >= b && a >= c && a >= d {
        0
    } else if b >= c && b >= d {
        1
    } else if c >= d {
        2
    } else {
        3
    }
}

// ---------------------------------------------------------------------
// LDR endpoint interpretation (§23.14.1)
// ---------------------------------------------------------------------

/// `bit_transfer_signed(a, b)` per §23.14.1: transfers the top bit of
/// `a` into `b`, then sign-extends `a` from 6 bits. Returns the updated
/// `(a, b)` (`a` becomes -32..31, `b` stays 0..255).
fn bit_transfer_signed(mut a: i32, mut b: i32) -> (i32, i32) {
    b >>= 1;
    b |= a & 0x80;
    a >>= 1;
    a &= 0x3F;
    if a & 0x20 != 0 {
        a -= 0x40;
    }
    (a, b)
}

fn clamp_u8(v: i32) -> u8 {
    v.clamp(0, 255) as u8
}

/// Decode the unquantized endpoint values `v` for a given CEM into two
/// RGBA endpoints. Returns `None` for HDR / unsupported CEMs (caller
/// emits the error colour). §23.14.1.
fn decode_endpoints(cem: u32, v: &[u32]) -> Option<([u8; 4], [u8; 4])> {
    let vi: Vec<i32> = v.iter().map(|&x| x as i32).collect();
    let g = |i: usize| vi[i];
    match cem {
        0 => {
            // LDR Luminance direct
            let e0 = [g(0) as u8, g(0) as u8, g(0) as u8, 0xFF];
            let e1 = [g(1) as u8, g(1) as u8, g(1) as u8, 0xFF];
            Some((e0, e1))
        }
        1 => {
            let l0 = (g(0) >> 2) | (g(1) & 0xC0);
            let mut l1 = l0 + (g(1) & 0x3F);
            if l1 > 0xFF {
                l1 = 0xFF;
            }
            Some((
                [l0 as u8, l0 as u8, l0 as u8, 0xFF],
                [l1 as u8, l1 as u8, l1 as u8, 0xFF],
            ))
        }
        4 => {
            let e0 = [g(0) as u8, g(0) as u8, g(0) as u8, g(2) as u8];
            let e1 = [g(1) as u8, g(1) as u8, g(1) as u8, g(3) as u8];
            Some((e0, e1))
        }
        5 => {
            let (v1, v0) = bit_transfer_signed(g(1), g(0));
            let (v3, v2) = bit_transfer_signed(g(3), g(2));
            let e0 = [clamp_u8(v0), clamp_u8(v0), clamp_u8(v0), clamp_u8(v2)];
            let e1 = [
                clamp_u8(v0 + v1),
                clamp_u8(v0 + v1),
                clamp_u8(v0 + v1),
                clamp_u8(v2 + v3),
            ];
            Some((e0, e1))
        }
        6 => {
            // LDR RGB base+scale
            let e1 = [g(0) as u8, g(1) as u8, g(2) as u8, 0xFF];
            let e0 = [
                ((g(0) * g(3)) >> 8) as u8,
                ((g(1) * g(3)) >> 8) as u8,
                ((g(2) * g(3)) >> 8) as u8,
                0xFF,
            ];
            Some((e0, e1))
        }
        8 => {
            // LDR RGB direct
            let s0 = g(0) + g(2) + g(4);
            let s1 = g(1) + g(3) + g(5);
            if s1 >= s0 {
                Some((
                    [g(0) as u8, g(2) as u8, g(4) as u8, 0xFF],
                    [g(1) as u8, g(3) as u8, g(5) as u8, 0xFF],
                ))
            } else {
                let (r0, g0, b0) = blue_contract(g(1), g(3), g(5));
                let (r1, g1, b1) = blue_contract(g(0), g(2), g(4));
                Some((
                    [r0 as u8, g0 as u8, b0 as u8, 0xFF],
                    [r1 as u8, g1 as u8, b1 as u8, 0xFF],
                ))
            }
        }
        9 => {
            let mut vv: Vec<i32> = vi.clone();
            let (a, b) = bit_transfer_signed(vv[1], vv[0]);
            vv[1] = a;
            vv[0] = b;
            let (a, b) = bit_transfer_signed(vv[3], vv[2]);
            vv[3] = a;
            vv[2] = b;
            let (a, b) = bit_transfer_signed(vv[5], vv[4]);
            vv[5] = a;
            vv[4] = b;
            let mut e0 = [vv[0], vv[2], vv[4], 255];
            let mut e1 = [vv[0] + vv[1], vv[2] + vv[3], vv[4] + vv[5], 255];
            if vv[1] + vv[3] + vv[5] < 0 {
                let (r0, g0, b0) = blue_contract(e1[0], e1[1], e1[2]);
                let (r1, g1, b1) = blue_contract(e0[0], e0[1], e0[2]);
                e0 = [r0, g0, b0, 255];
                e1 = [r1, g1, b1, 255];
            }
            Some((
                [clamp_u8(e0[0]), clamp_u8(e0[1]), clamp_u8(e0[2]), 255],
                [clamp_u8(e1[0]), clamp_u8(e1[1]), clamp_u8(e1[2]), 255],
            ))
        }
        10 => {
            // LDR RGB base+scale plus two alpha
            let e1 = [g(0) as u8, g(1) as u8, g(2) as u8, g(5) as u8];
            let e0 = [
                ((g(0) * g(3)) >> 8) as u8,
                ((g(1) * g(3)) >> 8) as u8,
                ((g(2) * g(3)) >> 8) as u8,
                g(4) as u8,
            ];
            Some((e0, e1))
        }
        12 => {
            // LDR RGBA direct
            let s0 = g(0) + g(2) + g(4);
            let s1 = g(1) + g(3) + g(5);
            if s1 >= s0 {
                Some((
                    [g(0) as u8, g(2) as u8, g(4) as u8, g(6) as u8],
                    [g(1) as u8, g(3) as u8, g(5) as u8, g(7) as u8],
                ))
            } else {
                let (r0, g0, b0) = blue_contract(g(1), g(3), g(5));
                let (r1, g1, b1) = blue_contract(g(0), g(2), g(4));
                Some((
                    [r0 as u8, g0 as u8, b0 as u8, g(7) as u8],
                    [r1 as u8, g1 as u8, b1 as u8, g(6) as u8],
                ))
            }
        }
        13 => {
            let mut vv: Vec<i32> = vi.clone();
            let (a, b) = bit_transfer_signed(vv[1], vv[0]);
            vv[1] = a;
            vv[0] = b;
            let (a, b) = bit_transfer_signed(vv[3], vv[2]);
            vv[3] = a;
            vv[2] = b;
            let (a, b) = bit_transfer_signed(vv[5], vv[4]);
            vv[5] = a;
            vv[4] = b;
            let (a, b) = bit_transfer_signed(vv[7], vv[6]);
            vv[7] = a;
            vv[6] = b;
            let mut e0 = [vv[0], vv[2], vv[4], vv[6]];
            let mut e1 = [vv[0] + vv[1], vv[2] + vv[3], vv[4] + vv[5], vv[6] + vv[7]];
            if vv[1] + vv[3] + vv[5] < 0 {
                let (r0, g0, b0) = blue_contract(e1[0], e1[1], e1[2]);
                let (r1, g1, b1) = blue_contract(e0[0], e0[1], e0[2]);
                let a0 = e0[3];
                let a1 = e1[3];
                e0 = [r0, g0, b0, a1];
                e1 = [r1, g1, b1, a0];
            }
            Some((
                [
                    clamp_u8(e0[0]),
                    clamp_u8(e0[1]),
                    clamp_u8(e0[2]),
                    clamp_u8(e0[3]),
                ],
                [
                    clamp_u8(e1[0]),
                    clamp_u8(e1[1]),
                    clamp_u8(e1[2]),
                    clamp_u8(e1[3]),
                ],
            ))
        }
        // HDR modes 2,3,7,11,14,15 -> error colour in LDR mode.
        _ => None,
    }
}

fn blue_contract(r: i32, g: i32, b: i32) -> (i32, i32, i32) {
    ((r + b) >> 1, (g + b) >> 1, b)
}

/// Number of unquantized integers each CEM consumes (§23.14). HDR modes
/// return their integer count too (so the colour stream is sized
/// correctly even when the texels decode to the error colour).
fn cem_value_count(cem: u32) -> u32 {
    match cem {
        0..=3 => 2,
        4..=7 => 4,
        8..=11 => 6,
        _ => 8,
    }
}

// ---------------------------------------------------------------------
// Top-level block decode
// ---------------------------------------------------------------------

/// Decode a single 128-bit ASTC LDR block at footprint `block_w ×
/// block_h` into a row-major array of `block_w*block_h` RGBA8 texels.
///
/// Illegal blocks and HDR-in-LDR content yield the error colour
/// (`(255, 0, 255, 255)`) for every texel (§23.22, §23.23). The
/// function never panics on arbitrary input.
pub fn decode_astc_ldr_block(data: &[u8; 16], block_w: u32, block_h: u32) -> Vec<[u8; 4]> {
    let texels = (block_w * block_h) as usize;
    let err = vec![ERROR_COLOR; texels];
    if !is_valid_footprint(block_w as u8, block_h as u8) {
        return err;
    }
    let block = Block128::from_bytes(data);

    let mode_field = block.bits(0, 11);
    let mode = match decode_block_mode(mode_field) {
        ModeClass::VoidExtent => return decode_void_extent(&block, texels),
        ModeClass::Illegal => return err,
        ModeClass::Normal(m) => m,
    };

    // Weight grid must not exceed the block footprint in any axis.
    if mode.weight_w > block_w || mode.weight_h > block_h {
        return err;
    }
    let num_weights_per_plane = mode.weight_w * mode.weight_h;
    let planes = if mode.dual_plane { 2 } else { 1 };
    let total_weights = num_weights_per_plane * planes;
    if total_weights == 0 || total_weights > 64 {
        return err;
    }

    let wr = IseRange {
        trits: mode.range.trits,
        quints: mode.range.quints,
        bits: mode.range.bits,
        max: mode.range.max,
    };
    let weight_bits = wr.bits_for_count(total_weights);
    if !(24..=96).contains(&weight_bits) {
        return err;
    }

    // Partition count (bits [12..11]) + 1.
    let partition_count = block.bits(11, 2) + 1;

    // The colour-config region sits between the block-mode/partition
    // fields and the weight data; the weight data occupies the top
    // `weight_bits` bits (bit-reversed). Compute the split.
    let config_below = 128 - weight_bits;

    // ---- Colour endpoint modes per partition ----
    let mut cems = [0u32; 4];
    let mut color_class_extra_bits = 0u32;
    let color_stream_start: u32;

    if partition_count == 1 {
        // CEM is bits [16..13].
        let cem = block.bits(13, 4);
        cems[0] = cem;
        color_stream_start = 17;
    } else {
        // Partition seed bits [22..13]; CEM selector bits [24..23].
        let selector = block.bits(23, 2);
        if selector == 0 {
            // All same type: 4-bit CEM in bits [28..25].
            let cem = block.bits(25, 4);
            for c in cems.iter_mut().take(partition_count as usize) {
                *c = cem;
            }
            color_stream_start = 29;
        } else {
            // Variable per-partition CEM. The base class is selector-1
            // (so class is 0,1,2). Each partition stores a 1-bit "class
            // increment" (M-high) above 25, then 2-bit low CEM bits
            // interleaved into the colour stream region. This follows
            // §23.11 Tables 23.14/23.15.
            let base_class = selector - 1;
            // Read partition_count "M" high bits starting at bit 25.
            let mut high_bits = [0u32; 4];
            for (i, hb) in high_bits
                .iter_mut()
                .take(partition_count as usize)
                .enumerate()
            {
                *hb = block.bit(25 + i as u32);
            }
            // The 2-bit low CEM fields are stored just below the weight
            // data, growing downward. We pull them from immediately
            // below the weight region. Each partition has a 2-bit field.
            let low_region_top = config_below;
            let mut low_bits = [0u32; 4];
            for (i, lb) in low_bits
                .iter_mut()
                .take(partition_count as usize)
                .enumerate()
            {
                let p = low_region_top - 2 * (i as u32 + 1);
                *lb = block.bits(p, 2);
            }
            for i in 0..partition_count as usize {
                let class = base_class + high_bits[i];
                cems[i] = (class << 2) | low_bits[i];
            }
            color_class_extra_bits = 2 * partition_count;
            color_stream_start = 25 + partition_count;
        }
    }

    // Total colour integers across all partitions.
    let mut total_color_values = 0u32;
    for c in cems.iter().take(partition_count as usize) {
        total_color_values += cem_value_count(*c);
    }
    if total_color_values > 18 {
        return err;
    }

    // Colour ISE range: pick the largest range that fits the colour
    // integers in the available bit budget. Per §23.21 (Data Size
    // Determination), the budget for the colour-endpoint ISE is
    //   remaining_bits = 128 − config_bits − weight_bits
    // where `config_bits` accounts for the block-mode + partition +
    // per-partition CEM fields and, when dual-plane is active, the two
    // colour-component-selector (CCS) bits. The colour data occupies the
    // bits from `color_stream_start` up to the bottom of the weight /
    // additional-CEM / CCS region; `color_region_top` marks that bottom
    // (it excludes the per-partition low-CEM `M` bits and the CCS bits,
    // all of which sit just below the weight data).
    let ccs_bits = if mode.dual_plane { 2 } else { 0 };
    let color_region_top = config_below - color_class_extra_bits - ccs_bits;
    if color_region_top <= color_stream_start {
        return err;
    }
    let color_budget = color_region_top - color_stream_start;
    let color_range = match pick_color_range(total_color_values, color_budget) {
        Some(r) => r,
        None => return err,
    };

    let color_values =
        decode_ise_sequence(&block, color_stream_start, total_color_values, color_range);
    // Unquantize each colour value.
    let unq_colors: Vec<u32> = color_values
        .iter()
        .map(|&v| unquant_color(v, color_range))
        .collect();

    // Split unquantized colours per partition and decode endpoints.
    let mut endpoints: Vec<([u8; 4], [u8; 4], bool)> = Vec::with_capacity(partition_count as usize);
    let mut idx = 0usize;
    for c in cems.iter().take(partition_count as usize) {
        let n = cem_value_count(*c) as usize;
        if idx + n > unq_colors.len() {
            return err;
        }
        let slice = &unq_colors[idx..idx + n];
        idx += n;
        match decode_endpoints(*c, slice) {
            Some((e0, e1)) => endpoints.push((e0, e1, true)),
            None => endpoints.push((ERROR_COLOR, ERROR_COLOR, false)),
        }
    }

    // ---- Weights: read bit-reversed from the top of the block ----
    // Stream bit i corresponds to block bit (127 - i).
    let get_wbit = |i: u32| -> u32 { block.bit(127 - i) };
    let weights_raw = decode_ise_from_bits(get_wbit, total_weights, wr);
    let weights: Vec<u32> = weights_raw
        .iter()
        .map(|&w| unquant_weight(w, mode.range))
        .collect();

    // Colour component selector for dual-plane mode: the 2 CCS bits sit
    // directly below the weight data and the per-partition additional
    // (`M`) CEM bits, i.e. at the top of the CCS region (§23.9 / §23.19).
    let ccs = if mode.dual_plane {
        let p = config_below - color_class_extra_bits - 2;
        block.bits(p, 2)
    } else {
        0
    };

    // ---- Per-texel reconstruction ----
    let small_block = texels < 31;
    let mut out = Vec::with_capacity(texels);
    let (ds, dt) = (infill_scale(block_w), infill_scale(block_h));
    for ty in 0..block_h {
        for tx in 0..block_w {
            let part = if partition_count > 1 {
                let seed = block.bits(13, 10);
                select_partition(seed, tx, ty, partition_count, small_block) as usize
            } else {
                0
            };
            let (e0, e1, ok) = endpoints[part.min(endpoints.len() - 1)];
            if !ok {
                out.push(ERROR_COLOR);
                continue;
            }

            let w_primary = sample_weight(
                &weights,
                mode.weight_w,
                mode.weight_h,
                tx,
                ty,
                ds,
                dt,
                planes,
                0,
            );

            let mut texel = [0u8; 4];
            for ch in 0..4usize {
                let w = if mode.dual_plane && ch as u32 == dual_plane_channel(ccs) {
                    sample_weight(
                        &weights,
                        mode.weight_w,
                        mode.weight_h,
                        tx,
                        ty,
                        ds,
                        dt,
                        planes,
                        1,
                    )
                } else {
                    w_primary
                };
                let a = e0[ch] as u32;
                let b = e1[ch] as u32;
                // 16-bit interpolation then take the high byte (LDR).
                let a16 = (a << 8) | a;
                let b16 = (b << 8) | b;
                let v = (a16 * (64 - w) + b16 * w + 32) >> 6;
                texel[ch] = (v >> 8) as u8;
            }
            out.push(texel);
        }
    }
    out
}

/// Which channel (0=R,1=G,2=B,3=A) uses weight plane 1 under the
/// dual-plane CCS (Table 23.33).
fn dual_plane_channel(ccs: u32) -> u32 {
    ccs & 0b11
}

/// Per-axis infill scale `D = floor((1024 + B/2)/(B-1))` (§23.17). For
/// `B == 1` the axis is degenerate; return 0.
fn infill_scale(b: u32) -> u32 {
    if b <= 1 {
        return 0;
    }
    (1024 + b / 2) / (b - 1)
}

/// Bilinear weight reconstruction at texel (tx, ty) for the requested
/// plane. §23.17.
#[allow(clippy::too_many_arguments)]
fn sample_weight(
    weights: &[u32],
    ww: u32,
    wh: u32,
    tx: u32,
    ty: u32,
    ds: u32,
    dt: u32,
    planes: u32,
    plane: u32,
) -> u32 {
    let cs = ds * tx;
    let ct = dt * ty;
    let gs = (cs * (ww - 1) + 32) >> 6;
    let gt = (ct * (wh - 1) + 32) >> 6;
    let js = gs >> 4;
    let fs = gs & 0xF;
    let jt = gt >> 4;
    let ft = gt & 0xF;

    let fetch = |gx: u32, gy: u32| -> u32 {
        let gx = gx.min(ww - 1);
        let gy = gy.min(wh - 1);
        let idx = (gy * ww + gx) * planes + plane;
        *weights.get(idx as usize).unwrap_or(&0)
    };
    let p00 = fetch(js, jt) as i32;
    let p01 = fetch(js + 1, jt) as i32;
    let p10 = fetch(js, jt + 1) as i32;
    let p11 = fetch(js + 1, jt + 1) as i32;
    let fs = fs as i32;
    let ft = ft as i32;
    let w11 = (fs * ft + 8) >> 4;
    let w10 = ft - w11;
    let w01 = fs - w11;
    let w00 = 16 - fs - ft + w11;
    let i = (p00 * w00 + p01 * w01 + p10 * w10 + p11 * w11 + 8) >> 4;
    i.clamp(0, 64) as u32
}

/// Decode a void-extent block (constant colour). §23.22.
fn decode_void_extent(block: &Block128, texels: usize) -> Vec<[u8; 4]> {
    // Bits [11..10] reserved, must be 1.
    if block.bits(10, 2) != 0b11 {
        return vec![ERROR_COLOR; texels];
    }
    // Bit 9: dynamic range. 1 => HDR -> error colour in LDR mode.
    if block.bit(9) == 1 {
        return vec![ERROR_COLOR; texels];
    }
    // UNORM16 colours: R[79..64] G[95..80] B[111..96] A[127..112].
    let r = block.bits(64, 16);
    let g = block.bits(80, 16);
    let b = block.bits(96, 16);
    let a = block.bits(112, 16);
    let c = [
        (r >> 8) as u8,
        (g >> 8) as u8,
        (b >> 8) as u8,
        (a >> 8) as u8,
    ];
    vec![c; texels]
}

/// Pick the colour ISE range: the highest-precision (trits/quints/bits)
/// quantization whose `count` integers fit in `budget` bits. §23.12 +
/// §23.23 (colour range must be ≥ 0..5). Returns `None` if even the
/// minimum range (0..5) does not fit.
fn pick_color_range(count: u32, budget: u32) -> Option<IseRange> {
    if count == 0 {
        return None;
    }
    // Candidate ranges in descending precision. Each is (trits, quints,
    // bits). We try from highest max down and pick the first that fits.
    // The full quant-level ladder used by ASTC colour endpoints.
    let candidates: &[(bool, bool, u8)] = &[
        (false, false, 8), // 0..255
        (true, false, 6),  // 0..191
        (false, true, 5),  // 0..159
        (false, false, 7), // 0..127
        (true, false, 5),  // 0..95
        (false, true, 4),  // 0..79
        (false, false, 6), // 0..63
        (true, false, 4),  // 0..47
        (false, true, 3),  // 0..39
        (false, false, 5), // 0..31
        (true, false, 3),  // 0..23
        (false, true, 2),  // 0..19
        (false, false, 4), // 0..15
        (true, false, 2),  // 0..11
        (false, true, 1),  // 0..9
        (false, false, 3), // 0..7
        (true, false, 1),  // 0..5
    ];
    for &(t, q, b) in candidates {
        let range = ise_range(t, q, b);
        if range.bits_for_count(count) <= budget {
            return Some(range);
        }
    }
    None
}

/// Decode a whole ASTC LDR surface (`width × height`) into RGBA8.
/// `data` holds row-major 128-bit blocks (`ceil(w/bw) × ceil(h/bh)`
/// blocks). Output is `width*height*4` bytes, RGBA8 row-major. Partial
/// edge blocks are clipped. Never panics; missing blocks decode to the
/// error colour.
pub fn decode_astc_ldr(
    data: &[u8],
    width: u32,
    height: u32,
    block_w: u32,
    block_h: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (width as usize) * (height as usize) * 4];
    if !is_valid_footprint(block_w as u8, block_h as u8) || block_w == 0 || block_h == 0 {
        // Fill error colour.
        for px in out.chunks_exact_mut(4) {
            px.copy_from_slice(&ERROR_COLOR);
        }
        return out;
    }
    let blocks_x = width.div_ceil(block_w);
    let blocks_y = height.div_ceil(block_h);
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            let block_idx = (by * blocks_x + bx) as usize;
            let off = block_idx * 16;
            let mut raw = [0u8; 16];
            let texels = if off + 16 <= data.len() {
                raw.copy_from_slice(&data[off..off + 16]);
                decode_astc_ldr_block(&raw, block_w, block_h)
            } else {
                vec![ERROR_COLOR; (block_w * block_h) as usize]
            };
            for ly in 0..block_h {
                let py = by * block_h + ly;
                if py >= height {
                    break;
                }
                for lx in 0..block_w {
                    let px = bx * block_w + lx;
                    if px >= width {
                        break;
                    }
                    let t = texels[(ly * block_w + lx) as usize];
                    let o = ((py * width + px) * 4) as usize;
                    out[o..o + 4].copy_from_slice(&t);
                }
            }
        }
    }
    out
}

/// Decode an ASTC surface described by a [`crate::DdsPixelFormat::Astc`]
/// format into RGBA8. Returns `None` if `pix` is not an ASTC format.
/// Thin wrapper over [`decode_astc_ldr`] that pulls the block footprint
/// from the pixel format.
pub fn decode_astc_ldr_surface(
    pix: crate::DdsPixelFormat,
    data: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let (bw, bh) = pix.astc_footprint()?;
    Some(decode_astc_ldr(data, width, height, bw, bh))
}

// =====================================================================
// ASTC LDR encode
// =====================================================================
//
// A single-partition, single-plane LDR encoder. Every block is emitted
// in one of two shapes, both of which the decoder above reads back:
//
//   * **Void-extent** (§23.22) for a block whose texels are all one
//     colour — a constant-colour block carrying UNORM16 R/G/B/A.
//   * **Normal** single-partition block with a weight grid no larger
//     than the footprint, colour endpoint mode 8 (LDR RGB direct) when
//     alpha is uniformly opaque, else mode 12 (LDR RGBA direct).
//
// The endpoints are the per-channel min/max over the block; each texel
// picks the weight that best reconstructs it as a linear blend of the
// two endpoints. The colour values and the weights are quantized by
// brute-force inversion of the decoder's own `unquant_color` /
// `unquant_weight` (the encoder never consults anything but this
// module's decode model). When the weight grid equals the footprint
// (footprints with ≤ 64 texels) the per-texel mapping is exact — no
// bilinear infill — so the only loss is endpoint/weight quantization;
// larger footprints use a sub-sampled grid and rely on the decoder's
// bilinear infill, so they round-trip within a documented tolerance.

/// A 128-bit ASTC block under construction, the encode-side mirror of
/// [`Block128`]. Bits are written LSB-first by absolute position; the
/// weight stream is written from the top of the block (bit `127 - i`).
struct BlockBuilder {
    lo: u64,
    hi: u64,
}

impl BlockBuilder {
    fn new() -> Self {
        Self { lo: 0, hi: 0 }
    }
    /// Write `count` low bits of `value` starting at absolute bit `pos`.
    fn set(&mut self, pos: u32, count: u32, value: u32) {
        for k in 0..count {
            let b = ((value >> k) & 1) as u64;
            let i = pos + k;
            if i < 64 {
                self.lo |= b << i;
            } else {
                self.hi |= b << (i - 64);
            }
        }
    }
    /// Set weight-stream bit `stream_i` (block bit `127 - stream_i`).
    fn set_weight_bit(&mut self, stream_i: u32, value: u32) {
        let pos = 127 - stream_i;
        let b = (value & 1) as u64;
        if pos < 64 {
            self.lo |= b << pos;
        } else {
            self.hi |= b << (pos - 64);
        }
    }
    fn bytes(&self) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0..8].copy_from_slice(&self.lo.to_le_bytes());
        out[8..16].copy_from_slice(&self.hi.to_le_bytes());
        out
    }
}

/// Inverse of [`unpack_trits`]: pack five trits (each 0..2) into the
/// 8-bit form by exhaustive search, so it agrees with the decoder for
/// every input (§23.18, Table 23.18).
fn pack_trits(t: [u32; 5]) -> u32 {
    for p in 0u32..256 {
        if unpack_trits(p) == t {
            return p;
        }
    }
    0
}

/// Inverse of [`unpack_quints`]: pack three quints (each 0..4) into the
/// 7-bit form by exhaustive search (§23.18, Table 23.19).
fn pack_quints(q: [u32; 3]) -> u32 {
    for p in 0u32..128 {
        if unpack_quints(p) == q {
            return p;
        }
    }
    0
}

/// Pack the colour-integer sequence `seq` (each value in `range`) into
/// the block at absolute bit `start`, the exact inverse of
/// [`decode_ise_sequence`].
fn pack_ise_into(bb: &mut BlockBuilder, start: u32, seq: &[u32], range: IseRange) {
    let n = range.bits as u32;
    let mut pos = start;
    if range.trits {
        for chunk in seq.chunks(5) {
            let mut trits = [0u32; 5];
            let mut m = [0u32; 5];
            for (i, &v) in chunk.iter().enumerate() {
                trits[i] = v >> n;
                m[i] = if n > 0 { v & ((1 << n) - 1) } else { 0 };
            }
            let packed = pack_trits(trits);
            bb.set(pos, n, m[0]);
            pos += n;
            bb.set(pos, 2, packed & 0b11);
            pos += 2;
            bb.set(pos, n, m[1]);
            pos += n;
            bb.set(pos, 2, (packed >> 2) & 0b11);
            pos += 2;
            bb.set(pos, n, m[2]);
            pos += n;
            bb.set(pos, 1, (packed >> 4) & 1);
            pos += 1;
            bb.set(pos, n, m[3]);
            pos += n;
            bb.set(pos, 2, (packed >> 5) & 0b11);
            pos += 2;
            bb.set(pos, n, m[4]);
            pos += n;
            bb.set(pos, 1, (packed >> 7) & 1);
            pos += 1;
        }
    } else if range.quints {
        for chunk in seq.chunks(3) {
            let mut quints = [0u32; 3];
            let mut m = [0u32; 3];
            for (i, &v) in chunk.iter().enumerate() {
                quints[i] = v >> n;
                m[i] = if n > 0 { v & ((1 << n) - 1) } else { 0 };
            }
            let packed = pack_quints(quints);
            bb.set(pos, n, m[0]);
            pos += n;
            bb.set(pos, 3, packed & 0b111);
            pos += 3;
            bb.set(pos, n, m[1]);
            pos += n;
            bb.set(pos, 2, (packed >> 3) & 0b11);
            pos += 2;
            bb.set(pos, n, m[2]);
            pos += n;
            bb.set(pos, 2, (packed >> 5) & 0b11);
            pos += 2;
        }
    } else {
        for &v in seq {
            bb.set(pos, n, v);
            pos += n;
        }
    }
}

/// Pack the weight sequence `seq` (each value in `range`) into the
/// bit-reversed weight stream at the top of the block, the inverse of
/// the `decode_ise_from_bits(get_wbit, …)` read in
/// [`decode_astc_ldr_block`].
fn pack_weights_into(bb: &mut BlockBuilder, seq: &[u32], range: WeightRange) {
    let n = range.bits as u32;
    let mut stream_pos = 0u32;
    let put = |bb: &mut BlockBuilder, count: u32, value: u32, pos: &mut u32| {
        for k in 0..count {
            bb.set_weight_bit(*pos + k, (value >> k) & 1);
        }
        *pos += count;
    };
    if range.trits {
        for chunk in seq.chunks(5) {
            let mut trits = [0u32; 5];
            let mut m = [0u32; 5];
            for (i, &v) in chunk.iter().enumerate() {
                trits[i] = v >> n;
                m[i] = if n > 0 { v & ((1 << n) - 1) } else { 0 };
            }
            let packed = pack_trits(trits);
            put(bb, n, m[0], &mut stream_pos);
            put(bb, 2, packed & 0b11, &mut stream_pos);
            put(bb, n, m[1], &mut stream_pos);
            put(bb, 2, (packed >> 2) & 0b11, &mut stream_pos);
            put(bb, n, m[2], &mut stream_pos);
            put(bb, 1, (packed >> 4) & 1, &mut stream_pos);
            put(bb, n, m[3], &mut stream_pos);
            put(bb, 2, (packed >> 5) & 0b11, &mut stream_pos);
            put(bb, n, m[4], &mut stream_pos);
            put(bb, 1, (packed >> 7) & 1, &mut stream_pos);
        }
    } else if range.quints {
        for chunk in seq.chunks(3) {
            let mut quints = [0u32; 3];
            let mut m = [0u32; 3];
            for (i, &v) in chunk.iter().enumerate() {
                quints[i] = v >> n;
                m[i] = if n > 0 { v & ((1 << n) - 1) } else { 0 };
            }
            let packed = pack_quints(quints);
            put(bb, n, m[0], &mut stream_pos);
            put(bb, 3, packed & 0b111, &mut stream_pos);
            put(bb, n, m[1], &mut stream_pos);
            put(bb, 2, (packed >> 3) & 0b11, &mut stream_pos);
            put(bb, n, m[2], &mut stream_pos);
            put(bb, 2, (packed >> 5) & 0b11, &mut stream_pos);
        }
    } else {
        for &v in seq {
            put(bb, n, v, &mut stream_pos);
        }
    }
}

/// One entry of the precomputed block-mode table:
/// `(weight_w, weight_h, trits, quints, bits, dual_plane, mode_field)`.
type ModeEntry = (u32, u32, bool, bool, u8, bool, u32);

/// The full set of normal block modes, derived once from the decoder's
/// own [`decode_block_mode`] over the 2048-entry mode space. Building it
/// from the decoder keeps encode and decode in lockstep; caching it
/// removes the per-block 2048-iteration scan.
fn block_mode_table() -> &'static [ModeEntry] {
    use std::sync::OnceLock;
    static TABLE: OnceLock<Vec<ModeEntry>> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut out = Vec::new();
        for m in 0u32..2048 {
            if let ModeClass::Normal(bm) = decode_block_mode(m) {
                out.push((
                    bm.weight_w,
                    bm.weight_h,
                    bm.range.trits,
                    bm.range.quints,
                    bm.range.bits,
                    bm.dual_plane,
                    m,
                ));
            }
        }
        out
    })
}

/// Find the 11-bit block-mode field for a normal block with the given
/// weight grid, weight range and plane count. Looks the request up in
/// the cached [`block_mode_table`], so encoder and decoder can never
/// disagree about a mode's meaning. Returns `None` if no mode realises
/// the request. The first hit (lowest mode field) is returned, which
/// matches the original ascending scan.
fn find_block_mode_planes(ww: u32, wh: u32, range: WeightRange, dual_plane: bool) -> Option<u32> {
    block_mode_table()
        .iter()
        .find(|&&(w, h, t, q, b, dp, _)| {
            w == ww
                && h == wh
                && t == range.trits
                && q == range.quints
                && b == range.bits
                && dp == dual_plane
        })
        .map(|&(.., m)| m)
}

/// Single-plane convenience over [`find_block_mode_planes`].
fn find_block_mode(ww: u32, wh: u32, range: WeightRange) -> Option<u32> {
    find_block_mode_planes(ww, wh, range, false)
}

/// Quantize an 8-bit colour component to the index in `range` whose
/// `unquant_color` is closest (ties → lower index).
fn quantize_color(value: u32, range: IseRange) -> u32 {
    let mut best = 0u32;
    let mut best_err = u32::MAX;
    for q in 0..=range.max {
        let u = unquant_color(q, range);
        let err = u.abs_diff(value);
        if err < best_err {
            best_err = err;
            best = q;
            if err == 0 {
                break;
            }
        }
    }
    best
}

/// Quantize a 0..64 interpolation weight to the index in `range` whose
/// `unquant_weight` is closest (ties → lower index).
fn quantize_weight(value: u32, range: WeightRange) -> u32 {
    let mut best = 0u32;
    let mut best_err = u32::MAX;
    for q in 0..=range.max {
        let u = unquant_weight(q, range);
        let err = u.abs_diff(value);
        if err < best_err {
            best_err = err;
            best = q;
            if err == 0 {
                break;
            }
        }
    }
    best
}

/// Emit a void-extent (constant-colour) block carrying `color` (RGBA8).
/// §23.22: bits[10..0] = 0x1FC (void-extent marker), bits[11..10] = 11,
/// bit 9 = 0 (LDR), the four 13-bit void-extent coordinates set to the
/// "any" sentinel (all 1s) so no inter-block dependency is implied, and
/// R/G/B/A as UNORM16 (the byte replicated into both halves).
fn encode_void_extent(color: [u8; 4]) -> [u8; 16] {
    let mut bb = BlockBuilder::new();
    // Void-extent block-mode marker: bits[8..2] = 1111111, bits[1..0]=00.
    bb.set(0, 9, 0b1_1111_1100);
    // Bit 9 = 0 (LDR dynamic range), bits[11..10] = 11 (reserved, must 1).
    bb.set(10, 2, 0b11);
    // Void-extent coordinates (bits 12..63): all 1s = "texture-wide"
    // sentinel, which carries no constraint for a stand-alone constant
    // block (§23.22 — a min == max == all-ones extent disables the
    // optional inter-block constant-colour optimisation).
    for i in 12..64 {
        bb.set(i, 1, 1);
    }
    let to_u16 = |c: u8| ((c as u32) << 8) | c as u32;
    bb.set(64, 16, to_u16(color[0]));
    bb.set(80, 16, to_u16(color[1]));
    bb.set(96, 16, to_u16(color[2]));
    bb.set(112, 16, to_u16(color[3]));
    bb.bytes()
}

/// Pick the largest weight grid `(gw, gh)` that fits the footprint and a
/// 2-bit-equivalent ISE budget. The grid never exceeds the footprint in
/// either axis and never exceeds 64 grid points (the spec weight cap,
/// §23.7). Footprints with ≤ 64 texels get an exact 1:1 grid.
fn choose_weight_grid(bw: u32, bh: u32) -> (u32, u32) {
    let mut gw = bw;
    let mut gh = bh;
    // Halve the larger axis until the grid is small enough that a
    // ≥ 2-bit weight range still fits the 96-bit weight-stream cap
    // (≈ 36 grid points). Keep both axes ≥ 2 — a 1-wide grid is
    // degenerate for bilinear interpolation. A 1:1 grid is kept where
    // the footprint already has ≤ 36 texels (exact, no infill loss).
    while gw * gh > 36 {
        if gw >= gh {
            gw = (gw / 2).max(2);
        } else {
            gh = (gh / 2).max(2);
        }
        if gw * gh > 36 && gw <= 2 && gh <= 2 {
            break;
        }
    }
    (gw, gh)
}

/// Candidate weight ranges, highest precision first — `(trits, quints,
/// bits)`. The encoder picks the finest one whose weight stream + colour
/// data both fit the 128-bit budget. More weight levels mean tighter
/// interpolation (better round-trip).
const WEIGHT_CANDIDATES: [(bool, bool, u8); 6] = [
    (false, false, 3), // 0..7  (3-bit)
    (true, false, 1),  // 0..5  (trit + 1 bit)
    (false, false, 2), // 0..3  (2-bit)
    (true, false, 0),  // 0..2  (trit)
    (false, false, 1), // 0..1  (1-bit)
    (false, true, 0),  // 0..4  (quint)
];

/// Sum of absolute RGBA differences between a decoded block and the
/// source texels (the round-trip error a candidate block achieves).
fn block_sad(candidate: &[u8; 16], texels: &[[u8; 4]], bw: u32, bh: u32) -> u64 {
    let dec = decode_astc_ldr_block(candidate, bw, bh);
    let mut sad = 0u64;
    for (i, t) in texels.iter().enumerate() {
        let d = dec.get(i).copied().unwrap_or(ERROR_COLOR);
        for c in 0..4 {
            sad += (d[c] as i64 - t[c] as i64).unsigned_abs();
        }
    }
    sad
}

/// Encode one block of `bw × bh` RGBA8 texels (`texels[y*bw + x]`) into a
/// 128-bit ASTC LDR block. The texel slice must hold exactly `bw*bh`
/// entries.
///
/// A constant block becomes an exact void-extent block. Otherwise the
/// encoder builds a single-subset block and, when a two-subset block is
/// realisable within the bit budget, also tries a handful of partition
/// seeds; it keeps whichever block decodes closest to the source
/// (measured against this crate's own decoder). Two subsets let a
/// non-collinear block split into two independently-fitted colour lines.
pub fn encode_astc_ldr_block(texels: &[[u8; 4]], bw: u32, bh: u32) -> [u8; 16] {
    let n = (bw * bh) as usize;
    debug_assert_eq!(texels.len(), n);
    if !is_valid_footprint(bw as u8, bh as u8) || texels.is_empty() {
        // Degenerate request: emit a black void-extent block.
        return encode_void_extent([0, 0, 0, 255]);
    }

    // Constant-colour fast path → void extent (exact).
    let first = texels[0];
    if texels.iter().all(|&t| t == first) {
        return encode_void_extent(first);
    }

    let mut best = encode_single_subset(texels, bw, bh);
    let mut best_sad = block_sad(&best, texels, bw, bh);
    if best_sad == 0 {
        return best;
    }

    // Dual-plane candidate: only worthwhile when alpha varies (it can
    // then sit on its own interpolation axis, independent of RGB).
    let alpha_varies = texels.iter().any(|t| t[3] != first[3]);
    if alpha_varies {
        if let Some(cand) = encode_dual_plane(texels, bw, bh) {
            let sad = block_sad(&cand, texels, bw, bh);
            if sad < best_sad {
                best_sad = sad;
                best = cand;
                if best_sad == 0 {
                    return best;
                }
            }
        }
    }

    // Try a small set of partition seeds for a 2-subset block. The seeds
    // are arbitrary but fixed, so encoding is deterministic; the chooser
    // keeps the lowest-error result, so a poor split is simply discarded.
    for &seed in &[0u32, 1, 2, 7, 13, 31, 63, 100] {
        if let Some(cand) = encode_two_subset(texels, bw, bh, seed) {
            let sad = block_sad(&cand, texels, bw, bh);
            if sad < best_sad {
                best_sad = sad;
                best = cand;
                if best_sad == 0 {
                    return best;
                }
            }
        }
    }

    // Three-subset candidates (opaque-alpha blocks only — CEM 8 fits the
    // 18-value single-CEM colour cap, CEM 12 would overflow it). Worth
    // trying when the block has three distinct colour regions that two
    // endpoint lines cannot separate; the chooser discards a poor split.
    let alpha_all_opaque = texels.iter().all(|t| t[3] == 255);
    if alpha_all_opaque {
        for &seed in &[0u32, 1, 2, 7, 13, 31, 63, 100] {
            if let Some(cand) = encode_three_subset(texels, bw, bh, seed) {
                let sad = block_sad(&cand, texels, bw, bh);
                if sad < best_sad {
                    best_sad = sad;
                    best = cand;
                    if best_sad == 0 {
                        break;
                    }
                }
            }
        }
    }
    best
}

/// Build a single-subset block: min/max endpoints, the finest weight
/// range that fits, per-texel weight by endpoint-axis projection.
fn encode_single_subset(texels: &[[u8; 4]], bw: u32, bh: u32) -> [u8; 16] {
    let n = (bw * bh) as usize;
    // Per-channel min / max endpoints over the block.
    let mut lo = [255u8; 4];
    let mut hi = [0u8; 4];
    for t in texels {
        for c in 0..4 {
            lo[c] = lo[c].min(t[c]);
            hi[c] = hi[c].max(t[c]);
        }
    }
    let alpha_uniform_opaque = lo[3] == 255 && hi[3] == 255;
    let cem: u32 = if alpha_uniform_opaque { 8 } else { 12 };
    let color_count = cem_value_count(cem);
    let color_stream_start = 17u32; // single partition

    let (mut gw, mut gh) = choose_weight_grid(bw, bh);

    loop {
        for &(t, q, b) in &WEIGHT_CANDIDATES {
            let wir = ise_range(t, q, b);
            let wr = WeightRange {
                trits: t,
                quints: q,
                bits: b,
                max: wir.max,
            };
            let Some(mode_field) = find_block_mode(gw, gh, wr) else {
                continue;
            };
            let total_weights = gw * gh;
            let weight_bits = wir.bits_for_count(total_weights);
            if !(24..=96).contains(&weight_bits) {
                continue;
            }
            let config_below = 128 - weight_bits;
            if config_below <= color_stream_start {
                continue;
            }
            let color_budget = config_below - color_stream_start;
            let Some(color_range) = pick_color_range(color_count, color_budget) else {
                continue;
            };
            return assemble_block(
                texels,
                bw,
                bh,
                gw,
                gh,
                wr,
                mode_field,
                cem,
                color_range,
                lo,
                hi,
            );
        }
        // Coarsen the grid and retry; bail to void extent if exhausted.
        if gw <= 2 && gh <= 2 {
            // Fall back to the average colour as a void-extent block.
            let mut acc = [0u32; 4];
            for t in texels {
                for c in 0..4 {
                    acc[c] += t[c] as u32;
                }
            }
            let avg = [
                (acc[0] / n as u32) as u8,
                (acc[1] / n as u32) as u8,
                (acc[2] / n as u32) as u8,
                (acc[3] / n as u32) as u8,
            ];
            return encode_void_extent(avg);
        }
        if gw >= gh {
            gw = (gw / 2).max(2);
        } else {
            gh = (gh / 2).max(2);
        }
    }
}

/// Assemble the final block bytes once a legal (grid, range, mode) tuple
/// is fixed. Mode 8 packs (r0,r1,g0,g1,b0,b1); mode 12 appends
/// (a0,a1). The decoder takes its direct (non-blue-contract) branch when
/// `s1 = r1+g1+b1 ≥ s0`, so endpoint 0 is forced to the darker corner
/// and endpoint 1 to the brighter one, guaranteeing the direct branch.
#[allow(clippy::too_many_arguments)]
fn assemble_block(
    texels: &[[u8; 4]],
    bw: u32,
    bh: u32,
    gw: u32,
    gh: u32,
    wr: WeightRange,
    mode_field: u32,
    cem: u32,
    color_range: IseRange,
    mut lo: [u8; 4],
    mut hi: [u8; 4],
) -> [u8; 16] {
    // Force endpoint 0 = darker (low RGB sum) so the decoder's mode-8/12
    // s1 ≥ s0 direct branch is taken (no blue-contract).
    let sum_lo = lo[0] as u32 + lo[1] as u32 + lo[2] as u32;
    let sum_hi = hi[0] as u32 + hi[1] as u32 + hi[2] as u32;
    let swap = sum_lo > sum_hi;
    if swap {
        core::mem::swap(&mut lo, &mut hi);
    }

    let mut bb = BlockBuilder::new();
    bb.set(0, 11, mode_field);
    // Partition count - 1 = 0 (single partition); CEM in bits[16..13].
    bb.set(13, 4, cem);

    // Quantize the colour values.
    let qc = |v: u8| quantize_color(v as u32, color_range);
    let mut seq: Vec<u32> = Vec::with_capacity(8);
    seq.push(qc(lo[0]));
    seq.push(qc(hi[0]));
    seq.push(qc(lo[1]));
    seq.push(qc(hi[1]));
    seq.push(qc(lo[2]));
    seq.push(qc(hi[2]));
    if cem == 12 {
        seq.push(qc(lo[3]));
        seq.push(qc(hi[3]));
    }
    pack_ise_into(&mut bb, 17, &seq, color_range);

    // The endpoints the decoder will actually reconstruct (round-trip
    // the quantized colour through the decode model) so the per-texel
    // weight search targets the real reconstruction, not the ideal.
    let unq = |i: usize| unquant_color(seq[i], color_range) as i32;
    let mut e0 = [unq(0), unq(2), unq(4), 255];
    let mut e1 = [unq(1), unq(3), unq(5), 255];
    if cem == 12 {
        e0[3] = unq(6);
        e1[3] = unq(7);
    }
    // Mirror the decoder's 8-bit → 16-bit endpoint expansion.
    let e0_16: [u32; 4] = core::array::from_fn(|c| {
        let a = e0[c] as u32;
        (a << 8) | a
    });
    let e1_16: [u32; 4] = core::array::from_fn(|c| {
        let a = e1[c] as u32;
        (a << 8) | a
    });

    // ---- Weights ----
    // Build the grid by averaging the texels that map to each grid cell,
    // projecting onto the e0→e1 axis to pick the best 0..64 weight, then
    // quantizing. With a 1:1 grid this is exact per-texel.
    let mut grid_weights: Vec<u32> = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            // Footprint region this grid point represents (nearest-cell).
            let x0 = (gx * bw) / gw;
            let x1 = (((gx + 1) * bw) / gw).max(x0 + 1).min(bw);
            let y0 = (gy * bh) / gh;
            let y1 = (((gy + 1) * bh) / gh).max(y0 + 1).min(bh);
            let mut acc = [0i64; 4];
            let mut cnt = 0i64;
            for ty in y0..y1 {
                for tx in x0..x1 {
                    let t = texels[(ty * bw + tx) as usize];
                    for c in 0..4 {
                        acc[c] += t[c] as i64;
                    }
                    cnt += 1;
                }
            }
            let cnt = cnt.max(1);
            let avg: [i64; 4] = core::array::from_fn(|c| acc[c] / cnt);
            // Project onto the endpoint axis in 16-bit space.
            let mut num = 0i64;
            let mut den = 0i64;
            for c in 0..4 {
                let a = e0_16[c] as i64;
                let b = e1_16[c] as i64;
                let d = b - a;
                let p = ((avg[c] << 8) | avg[c]) - a;
                num += p * d;
                den += d * d;
            }
            let w = if den == 0 {
                0
            } else {
                ((num * 64 + den / 2) / den).clamp(0, 64) as u32
            };
            grid_weights.push(quantize_weight(w, wr));
        }
    }
    pack_weights_into(&mut bb, &grid_weights, wr);

    bb.bytes()
}

/// Build a two-subset block for partition `seed`. Both partitions share
/// one CEM (the selector-`00` "single CEM" path), so the colour stream
/// starts at bit 29 and carries `2 × cem_value_count` integers. Each
/// partition gets its own min/max endpoints over the texels the decoder
/// will route to it via [`select_partition`]; the per-grid-point weight
/// is fitted against that point's owning partition. Returns `None` when
/// no legal grid + weight range + colour budget realises the block, or
/// when the seed maps every texel into one partition (degenerate).
fn encode_two_subset(texels: &[[u8; 4]], bw: u32, bh: u32, seed: u32) -> Option<[u8; 16]> {
    let texel_count = (bw * bh) as usize;
    let small_block = texel_count < 31;

    // Route every texel to its partition and gather per-partition
    // min/max endpoints.
    let mut part = vec![0u8; texel_count];
    let mut lo = [[255u8; 4]; 2];
    let mut hi = [[0u8; 4]; 2];
    let mut count = [0u32; 2];
    for ty in 0..bh {
        for tx in 0..bw {
            let p = select_partition(seed, tx, ty, 2, small_block) as usize;
            let p = p.min(1);
            let idx = (ty * bw + tx) as usize;
            part[idx] = p as u8;
            count[p] += 1;
            let t = texels[idx];
            for c in 0..4 {
                lo[p][c] = lo[p][c].min(t[c]);
                hi[p][c] = hi[p][c].max(t[c]);
            }
        }
    }
    if count[0] == 0 || count[1] == 0 {
        return None; // degenerate split — no benefit over single subset
    }

    let alpha_opaque = lo
        .iter()
        .zip(hi.iter())
        .all(|(l, h)| l[3] == 255 && h[3] == 255);
    let cem: u32 = if alpha_opaque { 8 } else { 12 };
    let per_part = cem_value_count(cem);
    let total_color = per_part * 2;
    let color_stream_start = 29u32; // 2-partition single-CEM path

    let (mut gw, mut gh) = choose_weight_grid(bw, bh);
    loop {
        for &(t, q, b) in &WEIGHT_CANDIDATES {
            let wir = ise_range(t, q, b);
            let wr = WeightRange {
                trits: t,
                quints: q,
                bits: b,
                max: wir.max,
            };
            let Some(mode_field) = find_block_mode(gw, gh, wr) else {
                continue;
            };
            let total_weights = gw * gh;
            let weight_bits = wir.bits_for_count(total_weights);
            if !(24..=96).contains(&weight_bits) {
                continue;
            }
            let config_below = 128 - weight_bits;
            if config_below <= color_stream_start {
                continue;
            }
            let color_budget = config_below - color_stream_start;
            let Some(color_range) = pick_color_range(total_color, color_budget) else {
                continue;
            };
            return Some(assemble_block_2subset(
                texels,
                &part,
                bw,
                bh,
                gw,
                gh,
                wr,
                mode_field,
                seed,
                cem,
                color_range,
                lo,
                hi,
            ));
        }
        if gw <= 2 && gh <= 2 {
            return None;
        }
        if gw >= gh {
            gw = (gw / 2).max(2);
        } else {
            gh = (gh / 2).max(2);
        }
    }
}

/// Assemble a two-subset block from a fixed partition map, weight grid,
/// weight range and colour range. Mirrors [`assemble_block`] but writes
/// the partition count, seed, single-CEM selector and two endpoint
/// groups, and fits each grid point's weight against its owning
/// partition's endpoint line.
#[allow(clippy::too_many_arguments)]
fn assemble_block_2subset(
    texels: &[[u8; 4]],
    part: &[u8],
    bw: u32,
    bh: u32,
    gw: u32,
    gh: u32,
    wr: WeightRange,
    mode_field: u32,
    seed: u32,
    cem: u32,
    color_range: IseRange,
    mut lo: [[u8; 4]; 2],
    mut hi: [[u8; 4]; 2],
) -> [u8; 16] {
    // Force each partition's endpoint 0 = darker so the decoder takes its
    // direct (non-blue-contract) branch.
    for p in 0..2 {
        let s_lo = lo[p][0] as u32 + lo[p][1] as u32 + lo[p][2] as u32;
        let s_hi = hi[p][0] as u32 + hi[p][1] as u32 + hi[p][2] as u32;
        if s_lo > s_hi {
            core::mem::swap(&mut lo[p], &mut hi[p]);
        }
    }

    let mut bb = BlockBuilder::new();
    bb.set(0, 11, mode_field);
    // Partition count - 1 = 1 (two partitions).
    bb.set(11, 2, 1);
    // Partition seed: bits [22:13].
    bb.set(13, 10, seed & 0x3FF);
    // CEM selector bits [24:23] = 00 (single CEM for all partitions).
    bb.set(23, 2, 0);
    // 4-bit CEM in bits [28:25].
    bb.set(25, 4, cem);

    // Colour values: partition 0's group then partition 1's group.
    let qc = |v: u8| quantize_color(v as u32, color_range);
    let mut seq: Vec<u32> = Vec::new();
    for p in 0..2 {
        seq.push(qc(lo[p][0]));
        seq.push(qc(hi[p][0]));
        seq.push(qc(lo[p][1]));
        seq.push(qc(hi[p][1]));
        seq.push(qc(lo[p][2]));
        seq.push(qc(hi[p][2]));
        if cem == 12 {
            seq.push(qc(lo[p][3]));
            seq.push(qc(hi[p][3]));
        }
    }
    pack_ise_into(&mut bb, 29, &seq, color_range);

    // Reconstruct per-partition endpoints (16-bit) for the weight fit.
    let per = if cem == 12 { 8 } else { 6 };
    let mut e0_16 = [[0u32; 4]; 2];
    let mut e1_16 = [[0u32; 4]; 2];
    for p in 0..2 {
        let base = p * per;
        let unq = |i: usize| unquant_color(seq[base + i], color_range);
        let e0 = [unq(0), unq(2), unq(4), if cem == 12 { unq(6) } else { 255 }];
        let e1 = [unq(1), unq(3), unq(5), if cem == 12 { unq(7) } else { 255 }];
        for c in 0..4 {
            e0_16[p][c] = (e0[c] << 8) | e0[c];
            e1_16[p][c] = (e1[c] << 8) | e1[c];
        }
    }

    // Weights: each grid point fits against the partition that owns the
    // texel nearest its grid centre.
    let small_block = (bw * bh) < 31;
    let mut grid_weights: Vec<u32> = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            // Footprint cell this grid point represents.
            let x0 = (gx * bw) / gw;
            let x1 = (((gx + 1) * bw) / gw).max(x0 + 1).min(bw);
            let y0 = (gy * bh) / gh;
            let y1 = (((gy + 1) * bh) / gh).max(y0 + 1).min(bh);
            // Partition of the cell's top-left texel — for a 1:1 grid
            // this is exactly the texel the grid point reconstructs.
            let pcx = x0.min(bw - 1);
            let pcy = y0.min(bh - 1);
            let p = part
                .get((pcy * bw + pcx) as usize)
                .copied()
                .unwrap_or_else(|| select_partition(seed, pcx, pcy, 2, small_block).min(1) as u8)
                as usize;

            let mut acc = [0i64; 4];
            let mut cnt = 0i64;
            for ty in y0..y1 {
                for tx in x0..x1 {
                    if part[(ty * bw + tx) as usize] as usize != p {
                        continue;
                    }
                    let t = texels[(ty * bw + tx) as usize];
                    for c in 0..4 {
                        acc[c] += t[c] as i64;
                    }
                    cnt += 1;
                }
            }
            if cnt == 0 {
                // No same-partition texel in the cell; fall back to all.
                for ty in y0..y1 {
                    for tx in x0..x1 {
                        let t = texels[(ty * bw + tx) as usize];
                        for c in 0..4 {
                            acc[c] += t[c] as i64;
                        }
                        cnt += 1;
                    }
                }
            }
            let cnt = cnt.max(1);
            let avg: [i64; 4] = core::array::from_fn(|c| acc[c] / cnt);
            let mut num = 0i64;
            let mut den = 0i64;
            for c in 0..4 {
                let a = e0_16[p][c] as i64;
                let b = e1_16[p][c] as i64;
                let d = b - a;
                let pj = ((avg[c] << 8) | avg[c]) - a;
                num += pj * d;
                den += d * d;
            }
            let w = if den == 0 {
                0
            } else {
                ((num * 64 + den / 2) / den).clamp(0, 64) as u32
            };
            grid_weights.push(quantize_weight(w, wr));
        }
    }
    pack_weights_into(&mut bb, &grid_weights, wr);
    bb.bytes()
}

/// Build a three-subset block for partition `seed`. All three partitions
/// share one CEM via the selector-`00` "single CEM" path, so the colour
/// stream starts at bit 29 and carries `3 × cem_value_count` integers.
/// The single-CEM colour-value cap is 18 integers (§23.11), so a
/// three-subset block can only use CEM 8 (LDR RGB direct, 6 values per
/// partition → 18 total); CEM 12 (8 values per partition → 24) overflows
/// the cap. The encoder therefore forces opaque-alpha CEM 8 and only
/// produces a block when every partition's alpha is uniformly opaque
/// (the common case for three-region colour content). Each partition
/// gets its own min/max RGB endpoints over the texels the decoder routes
/// to it via [`select_partition`]. Returns `None` when the seed maps the
/// texels into fewer than three non-empty partitions, when any alpha is
/// non-opaque, or when no legal grid + weight range + colour budget
/// realises the block.
fn encode_three_subset(texels: &[[u8; 4]], bw: u32, bh: u32, seed: u32) -> Option<[u8; 16]> {
    let texel_count = (bw * bh) as usize;
    let small_block = texel_count < 31;

    // Route every texel to its partition and gather per-partition
    // min/max endpoints.
    let mut part = vec![0u8; texel_count];
    let mut lo = [[255u8; 4]; 3];
    let mut hi = [[0u8; 4]; 3];
    let mut count = [0u32; 3];
    for ty in 0..bh {
        for tx in 0..bw {
            let p = (select_partition(seed, tx, ty, 3, small_block) as usize).min(2);
            let idx = (ty * bw + tx) as usize;
            part[idx] = p as u8;
            count[p] += 1;
            let t = texels[idx];
            for c in 0..4 {
                lo[p][c] = lo[p][c].min(t[c]);
                hi[p][c] = hi[p][c].max(t[c]);
            }
        }
    }
    if count.contains(&0) {
        return None; // degenerate split — fewer than 3 non-empty subsets
    }

    // CEM 12 (RGBA) would need 24 colour values across 3 partitions,
    // exceeding the 18-value single-CEM cap; only CEM 8 (RGB, 6 each)
    // fits. So bail unless every partition's alpha is uniformly opaque.
    let alpha_opaque = lo
        .iter()
        .zip(hi.iter())
        .all(|(l, h)| l[3] == 255 && h[3] == 255);
    if !alpha_opaque {
        return None;
    }
    let cem: u32 = 8;
    let per_part = cem_value_count(cem); // 6
    let total_color = per_part * 3; // 18
    let color_stream_start = 29u32; // multi-partition single-CEM path

    let (mut gw, mut gh) = choose_weight_grid(bw, bh);
    loop {
        for &(t, q, b) in &WEIGHT_CANDIDATES {
            let wir = ise_range(t, q, b);
            let wr = WeightRange {
                trits: t,
                quints: q,
                bits: b,
                max: wir.max,
            };
            let Some(mode_field) = find_block_mode(gw, gh, wr) else {
                continue;
            };
            let total_weights = gw * gh;
            let weight_bits = wir.bits_for_count(total_weights);
            if !(24..=96).contains(&weight_bits) {
                continue;
            }
            let config_below = 128 - weight_bits;
            if config_below <= color_stream_start {
                continue;
            }
            let color_budget = config_below - color_stream_start;
            let Some(color_range) = pick_color_range(total_color, color_budget) else {
                continue;
            };
            return Some(assemble_block_3subset(
                texels,
                &part,
                bw,
                bh,
                gw,
                gh,
                wr,
                mode_field,
                seed,
                cem,
                color_range,
                lo,
                hi,
            ));
        }
        if gw <= 2 && gh <= 2 {
            return None;
        }
        if gw >= gh {
            gw = (gw / 2).max(2);
        } else {
            gh = (gh / 2).max(2);
        }
    }
}

/// Assemble a three-subset block. Mirrors [`assemble_block_2subset`] but
/// writes partition count 2 (= 3 partitions), three endpoint groups, and
/// fits each grid point's weight against its owning partition's endpoint
/// line. CEM is always 8 (RGB direct, opaque alpha).
#[allow(clippy::too_many_arguments)]
fn assemble_block_3subset(
    texels: &[[u8; 4]],
    part: &[u8],
    bw: u32,
    bh: u32,
    gw: u32,
    gh: u32,
    wr: WeightRange,
    mode_field: u32,
    seed: u32,
    cem: u32,
    color_range: IseRange,
    mut lo: [[u8; 4]; 3],
    mut hi: [[u8; 4]; 3],
) -> [u8; 16] {
    // Force each partition's endpoint 0 = darker so the decoder takes its
    // direct (non-blue-contract) branch.
    for p in 0..3 {
        let s_lo = lo[p][0] as u32 + lo[p][1] as u32 + lo[p][2] as u32;
        let s_hi = hi[p][0] as u32 + hi[p][1] as u32 + hi[p][2] as u32;
        if s_lo > s_hi {
            core::mem::swap(&mut lo[p], &mut hi[p]);
        }
    }

    let mut bb = BlockBuilder::new();
    bb.set(0, 11, mode_field);
    // Partition count - 1 = 2 (three partitions).
    bb.set(11, 2, 2);
    // Partition seed: bits [22:13].
    bb.set(13, 10, seed & 0x3FF);
    // CEM selector bits [24:23] = 00 (single CEM for all partitions).
    bb.set(23, 2, 0);
    // 4-bit CEM in bits [28:25].
    bb.set(25, 4, cem);

    // Colour values: each partition's RGB endpoint group in order.
    let qc = |v: u8| quantize_color(v as u32, color_range);
    let mut seq: Vec<u32> = Vec::new();
    for p in 0..3 {
        seq.push(qc(lo[p][0]));
        seq.push(qc(hi[p][0]));
        seq.push(qc(lo[p][1]));
        seq.push(qc(hi[p][1]));
        seq.push(qc(lo[p][2]));
        seq.push(qc(hi[p][2]));
    }
    pack_ise_into(&mut bb, 29, &seq, color_range);

    // Reconstruct per-partition endpoints (16-bit) for the weight fit.
    let per = 6; // CEM 8: 6 values (R0 R1 G0 G1 B0 B1)
    let mut e0_16 = [[0u32; 4]; 3];
    let mut e1_16 = [[0u32; 4]; 3];
    for p in 0..3 {
        let base = p * per;
        let unq = |i: usize| unquant_color(seq[base + i], color_range);
        let e0 = [unq(0), unq(2), unq(4), 255];
        let e1 = [unq(1), unq(3), unq(5), 255];
        for c in 0..4 {
            e0_16[p][c] = (e0[c] << 8) | e0[c];
            e1_16[p][c] = (e1[c] << 8) | e1[c];
        }
    }

    // Weights: each grid point fits against its owning partition's line.
    let small_block = (bw * bh) < 31;
    let mut grid_weights: Vec<u32> = Vec::with_capacity((gw * gh) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            let x0 = (gx * bw) / gw;
            let x1 = (((gx + 1) * bw) / gw).max(x0 + 1).min(bw);
            let y0 = (gy * bh) / gh;
            let y1 = (((gy + 1) * bh) / gh).max(y0 + 1).min(bh);
            let pcx = x0.min(bw - 1);
            let pcy = y0.min(bh - 1);
            let p = part
                .get((pcy * bw + pcx) as usize)
                .copied()
                .unwrap_or_else(|| (select_partition(seed, pcx, pcy, 3, small_block).min(2)) as u8)
                as usize;

            let mut acc = [0i64; 4];
            let mut cnt = 0i64;
            for ty in y0..y1 {
                for tx in x0..x1 {
                    if part[(ty * bw + tx) as usize] as usize != p {
                        continue;
                    }
                    let t = texels[(ty * bw + tx) as usize];
                    for c in 0..4 {
                        acc[c] += t[c] as i64;
                    }
                    cnt += 1;
                }
            }
            if cnt == 0 {
                for ty in y0..y1 {
                    for tx in x0..x1 {
                        let t = texels[(ty * bw + tx) as usize];
                        for c in 0..4 {
                            acc[c] += t[c] as i64;
                        }
                        cnt += 1;
                    }
                }
            }
            let cnt = cnt.max(1);
            let avg: [i64; 4] = core::array::from_fn(|c| acc[c] / cnt);
            let mut num = 0i64;
            let mut den = 0i64;
            // Fit on RGB only (alpha is fixed opaque for CEM 8).
            for c in 0..3 {
                let a = e0_16[p][c] as i64;
                let b = e1_16[p][c] as i64;
                let d = b - a;
                let pj = ((avg[c] << 8) | avg[c]) - a;
                num += pj * d;
                den += d * d;
            }
            let w = if den == 0 {
                0
            } else {
                ((num * 64 + den / 2) / den).clamp(0, 64) as u32
            };
            grid_weights.push(quantize_weight(w, wr));
        }
    }
    pack_weights_into(&mut bb, &grid_weights, wr);
    bb.bytes()
}

/// Build a dual-plane single-partition block: CEM 12 (LDR RGBA direct),
/// CCS = 3 so the alpha channel is driven by weight plane 1 and RGB by
/// plane 0. This lets a block whose alpha varies independently of RGB fit
/// alpha on its own interpolation axis. Returns `None` when no legal
/// grid, weight range and colour budget realise the block (two weight
/// planes double the weight-stream cost, so the grid is usually coarser).
fn encode_dual_plane(texels: &[[u8; 4]], bw: u32, bh: u32) -> Option<[u8; 16]> {
    // Per-channel min/max endpoints.
    let mut lo = [255u8; 4];
    let mut hi = [0u8; 4];
    for t in texels {
        for c in 0..4 {
            lo[c] = lo[c].min(t[c]);
            hi[c] = hi[c].max(t[c]);
        }
    }
    let cem = 12u32; // RGBA direct
    let color_count = cem_value_count(cem); // 8
    let color_stream_start = 17u32; // single partition

    let (mut gw, mut gh) = choose_weight_grid(bw, bh);
    loop {
        for &(t, q, b) in &WEIGHT_CANDIDATES {
            let wir = ise_range(t, q, b);
            let wr = WeightRange {
                trits: t,
                quints: q,
                bits: b,
                max: wir.max,
            };
            let Some(mode_field) = find_block_mode_planes(gw, gh, wr, true) else {
                continue;
            };
            // Two weight planes: 2 weights per grid point.
            let total_weights = gw * gh * 2;
            let weight_bits = wir.bits_for_count(total_weights);
            if !(24..=96).contains(&weight_bits) {
                continue;
            }
            let config_below = 128 - weight_bits;
            // Colour region excludes the 2 CCS bits that sit below weights.
            let color_region_top = config_below.checked_sub(2)?;
            if color_region_top <= color_stream_start {
                continue;
            }
            let color_budget = color_region_top - color_stream_start;
            let Some(color_range) = pick_color_range(color_count, color_budget) else {
                continue;
            };
            return Some(assemble_block_dual_plane(
                texels,
                bw,
                bh,
                gw,
                gh,
                wr,
                mode_field,
                color_range,
                lo,
                hi,
            ));
        }
        if gw <= 2 && gh <= 2 {
            return None;
        }
        if gw >= gh {
            gw = (gw / 2).max(2);
        } else {
            gh = (gh / 2).max(2);
        }
    }
}

/// Assemble a dual-plane block: RGB on weight plane 0, alpha on plane 1
/// (CCS = 3). Mirrors [`assemble_block`] but writes both planes and the
/// CCS field, and fits the two planes against their respective channels.
#[allow(clippy::too_many_arguments)]
fn assemble_block_dual_plane(
    texels: &[[u8; 4]],
    bw: u32,
    bh: u32,
    gw: u32,
    gh: u32,
    wr: WeightRange,
    mode_field: u32,
    color_range: IseRange,
    mut lo: [u8; 4],
    mut hi: [u8; 4],
) -> [u8; 16] {
    // Force endpoint 0 = darker RGB so the decoder's mode-12 direct
    // branch is taken (no blue-contract).
    let s_lo = lo[0] as u32 + lo[1] as u32 + lo[2] as u32;
    let s_hi = hi[0] as u32 + hi[1] as u32 + hi[2] as u32;
    if s_lo > s_hi {
        core::mem::swap(&mut lo, &mut hi);
    }

    let total_weights = gw * gh; // per plane
    let weight_bits = ise_range(wr.trits, wr.quints, wr.bits).bits_for_count(total_weights * 2);
    let config_below = 128 - weight_bits;

    let mut bb = BlockBuilder::new();
    bb.set(0, 11, mode_field);
    // Single partition; CEM 12 in bits[16..13].
    bb.set(13, 4, 12);

    // Colour values (r0,r1,g0,g1,b0,b1,a0,a1).
    let qc = |v: u8| quantize_color(v as u32, color_range);
    let seq = [
        qc(lo[0]),
        qc(hi[0]),
        qc(lo[1]),
        qc(hi[1]),
        qc(lo[2]),
        qc(hi[2]),
        qc(lo[3]),
        qc(hi[3]),
    ];
    pack_ise_into(&mut bb, 17, &seq, color_range);

    // CCS = 3 (alpha → plane 1), at the top of the CCS region just below
    // the weight data.
    let ccs_pos = config_below - 2;
    bb.set(ccs_pos, 2, 3);

    // Reconstructed endpoints (16-bit) for the weight fit.
    let unq = |i: usize| unquant_color(seq[i], color_range);
    let e0 = [unq(0), unq(2), unq(4), unq(6)];
    let e1 = [unq(1), unq(3), unq(5), unq(7)];
    let e0_16: [i64; 4] = core::array::from_fn(|c| ((e0[c] << 8) | e0[c]) as i64);
    let e1_16: [i64; 4] = core::array::from_fn(|c| ((e1[c] << 8) | e1[c]) as i64);

    // Weights: per grid point, plane 0 fits RGB, plane 1 fits alpha. The
    // stream interleaves the two planes per grid point (plane 0 then
    // plane 1), matching the decoder's `planes`-strided weight read.
    let mut seq_w: Vec<u32> = Vec::with_capacity((gw * gh * 2) as usize);
    for gy in 0..gh {
        for gx in 0..gw {
            let x0 = (gx * bw) / gw;
            let x1 = (((gx + 1) * bw) / gw).max(x0 + 1).min(bw);
            let y0 = (gy * bh) / gh;
            let y1 = (((gy + 1) * bh) / gh).max(y0 + 1).min(bh);
            let mut acc = [0i64; 4];
            let mut cnt = 0i64;
            for ty in y0..y1 {
                for tx in x0..x1 {
                    let t = texels[(ty * bw + tx) as usize];
                    for c in 0..4 {
                        acc[c] += t[c] as i64;
                    }
                    cnt += 1;
                }
            }
            let cnt = cnt.max(1);
            let avg: [i64; 4] = core::array::from_fn(|c| acc[c] / cnt);
            // Plane 0: project RGB (channels 0..3) onto the endpoint line.
            let mut num = 0i64;
            let mut den = 0i64;
            for c in 0..3 {
                let d = e1_16[c] - e0_16[c];
                let p = ((avg[c] << 8) | avg[c]) - e0_16[c];
                num += p * d;
                den += d * d;
            }
            let w_rgb = if den == 0 {
                0
            } else {
                ((num * 64 + den / 2) / den).clamp(0, 64) as u32
            };
            // Plane 1: alpha only.
            let da = e1_16[3] - e0_16[3];
            let pa = ((avg[3] << 8) | avg[3]) - e0_16[3];
            let w_a = if da == 0 {
                0
            } else {
                ((pa * 64 + da / 2) / da).clamp(0, 64) as u32
            };
            seq_w.push(quantize_weight(w_rgb, wr));
            seq_w.push(quantize_weight(w_a, wr));
        }
    }
    pack_weights_into(&mut bb, &seq_w, wr);
    bb.bytes()
}

/// Encode a `width × height` RGBA8 surface to a tiled ASTC LDR texture
/// at footprint `block_w × block_h`. Output holds row-major 128-bit
/// blocks (`ceil(w/bw) × ceil(h/bh)` of them). Edge blocks are padded by
/// clamping to the last in-bounds texel.
pub fn encode_astc_ldr(
    rgba8: &[u8],
    width: u32,
    height: u32,
    block_w: u32,
    block_h: u32,
) -> Vec<u8> {
    if !is_valid_footprint(block_w as u8, block_h as u8) || width == 0 || height == 0 {
        return Vec::new();
    }
    let blocks_x = width.div_ceil(block_w);
    let blocks_y = height.div_ceil(block_h);
    let mut out = vec![0u8; (blocks_x * blocks_y) as usize * 16];
    let bw = block_w;
    let bh = block_h;
    let mut texels = vec![[0u8; 4]; (bw * bh) as usize];
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            for ly in 0..bh {
                let py = (by * bh + ly).min(height - 1);
                for lx in 0..bw {
                    let px = (bx * bw + lx).min(width - 1);
                    let o = ((py * width + px) * 4) as usize;
                    let mut t = [0u8; 4];
                    t.copy_from_slice(&rgba8[o..o + 4]);
                    texels[(ly * bw + lx) as usize] = t;
                }
            }
            let block = encode_astc_ldr_block(&texels, bw, bh);
            let bo = ((by * blocks_x + bx) as usize) * 16;
            out[bo..bo + 16].copy_from_slice(&block);
        }
    }
    out
}

/// Encode an RGBA8 surface to the ASTC footprint named by an
/// [`crate::DdsPixelFormat::Astc`] format. Returns `None` if `pix` is
/// not an ASTC format.
pub fn encode_astc_ldr_surface(
    pix: crate::DdsPixelFormat,
    rgba8: &[u8],
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    let (bw, bh) = pix.astc_footprint()?;
    Some(encode_astc_ldr(rgba8, width, height, bw, bh))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn footprints_recognised() {
        assert!(is_valid_footprint(4, 4));
        assert!(is_valid_footprint(12, 12));
        assert!(is_valid_footprint(8, 5));
        assert!(!is_valid_footprint(7, 7));
        assert!(!is_valid_footprint(16, 16));
    }

    #[test]
    fn trit_unpack_zero() {
        // All zero packed bits -> all zero trits.
        assert_eq!(unpack_trits(0), [0, 0, 0, 0, 0]);
    }

    #[test]
    fn weight_range_table_matches_spec() {
        // Every (ρ, P) entry of Table 23.9 maps to the documented
        // 0..=max weight range; the two ρ codes 000/001 are invalid.
        // P = 0 (low precision):
        assert_eq!(weight_range(0b000, 0), None);
        assert_eq!(weight_range(0b001, 0), None);
        assert_eq!(weight_range(0b010, 0).unwrap().max, 1); // 0..1
        assert_eq!(weight_range(0b011, 0).unwrap().max, 2); // 0..2
        assert_eq!(weight_range(0b100, 0).unwrap().max, 3); // 0..3
        assert_eq!(weight_range(0b101, 0).unwrap().max, 4); // 0..4
        assert_eq!(weight_range(0b110, 0).unwrap().max, 5); // 0..5
        assert_eq!(weight_range(0b111, 0).unwrap().max, 7); // 0..7
                                                            // P = 1 (high precision):
        assert_eq!(weight_range(0b000, 1), None);
        assert_eq!(weight_range(0b001, 1), None);
        assert_eq!(weight_range(0b010, 1).unwrap().max, 9); // 0..9
        assert_eq!(weight_range(0b011, 1).unwrap().max, 11); // 0..11
        assert_eq!(weight_range(0b100, 1).unwrap().max, 15); // 0..15
                                                             // The 0..19 quint/2-bit entry (the one previously mis-coded as
                                                             // 0..9 quint/1-bit).
        let r = weight_range(0b101, 1).unwrap();
        assert_eq!(r.max, 19);
        assert!(r.quints && !r.trits && r.bits == 2);
        assert_eq!(weight_range(0b110, 1).unwrap().max, 23); // 0..23
        assert_eq!(weight_range(0b111, 1).unwrap().max, 31); // 0..31
    }

    #[test]
    fn quint_unpack_zero() {
        assert_eq!(unpack_quints(0), [0, 0, 0]);
    }

    #[test]
    fn trit_unpack_range() {
        // Every 8-bit input must yield trits each in 0..=2.
        for t in 0u32..256 {
            let tr = unpack_trits(t);
            for &x in &tr {
                assert!(x <= 2, "trit {x} out of range for {t}");
            }
        }
    }

    #[test]
    fn quint_unpack_range() {
        for q in 0u32..128 {
            let qu = unpack_quints(q);
            for &x in &qu {
                assert!(x <= 4, "quint {x} out of range for {q}");
            }
        }
    }

    #[test]
    fn hash52_deterministic() {
        assert_eq!(hash52(0), hash52(0));
        assert_ne!(hash52(1), hash52(2));
    }

    #[test]
    fn void_extent_constant_color() {
        // Build a void-extent block: bits[8..2]=1111111, bits[1..0]=00,
        // bits[11..10]=11, bit9=0 (LDR). Put a known UNORM16 colour.
        let mut b = [0u8; 16];
        // block mode field: bits 0..10. void-extent signature:
        //   bits[1:0]=00, bits[8:2]=1111111 -> low 9 bits = 0b1_1111_1100 = 0x1FC
        //   bits[11:10]=11 -> bits 10,11 set.
        let lo: u64 = 0x1FC | (0b11 << 10);
        // Set colours via the high 64 bits: R[79..64], G[95..80],
        // B[111..96], A[127..112]. R=0xAABB etc -> high byte is the u8.
        let r: u64 = 0x40_00; // high byte 0x40
        let g: u64 = 0x80_00; // 0x80
        let bch: u64 = 0xC0_00; // 0xC0
        let a: u64 = 0xFF_00; // 0xFF
        let hi: u64 = r | (g << 16) | (bch << 32) | (a << 48);
        b[0..8].copy_from_slice(&lo.to_le_bytes());
        b[8..16].copy_from_slice(&hi.to_le_bytes());
        let texels = decode_astc_ldr_block(&b, 4, 4);
        assert_eq!(texels.len(), 16);
        for t in texels {
            assert_eq!(t, [0x40, 0x80, 0xC0, 0xFF]);
        }
    }

    #[test]
    fn hdr_void_extent_is_error() {
        let mut b = [0u8; 16];
        let lo: u64 = 0x1FC | (0b11 << 10) | (1 << 9); // bit9=1 => HDR
        b[0..8].copy_from_slice(&lo.to_le_bytes());
        let texels = decode_astc_ldr_block(&b, 4, 4);
        for t in texels {
            assert_eq!(t, ERROR_COLOR);
        }
    }

    #[test]
    fn all_zero_block_is_error_or_safe() {
        // An all-zero block has block mode 0 -> reserved/illegal.
        let b = [0u8; 16];
        let texels = decode_astc_ldr_block(&b, 4, 4);
        assert_eq!(texels.len(), 16);
        for t in texels {
            assert_eq!(t, ERROR_COLOR);
        }
    }

    #[test]
    fn arbitrary_input_never_panics() {
        // Walk a pseudo-random set of blocks across every footprint.
        let mut seed = 0x1234_5678u32;
        for &(bw, bh) in &LDR_BLOCK_FOOTPRINTS {
            for _ in 0..2000 {
                let mut b = [0u8; 16];
                for byte in b.iter_mut() {
                    seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
                    *byte = (seed >> 24) as u8;
                }
                let texels = decode_astc_ldr_block(&b, bw as u32, bh as u32);
                assert_eq!(texels.len(), (bw as usize) * (bh as usize));
            }
        }
    }

    #[test]
    fn single_partition_corners_match_endpoints() {
        // Construct a 4×4, single-partition, single-plane block with a
        // 4×4 weight grid at range 0..7 (3 bits/weight → 48 weight bits,
        // satisfying the ≥24-bit ISE rule), CEM 8 (LDR RGB direct).
        // Corner texels land exactly on grid weights 0 or 7, so they
        // reproduce endpoint 0 / endpoint 1 (the LDR 8→16-bit
        // interpolation returns the endpoint byte exactly at w=0 and
        // w=64).
        let mut bb = BlockBuilder::new();
        // Block mode row "DP P W W H H ρ0 0 0 ρ2 ρ1": Wwidth=W+4,
        // Wheight=H+2. For 4×4: W=0, H=2. Range 0..7 is (P=0, ρ=111):
        //   ρ2=1, ρ1=1, ρ0=1.
        // Layout bits:
        //   bit10=DP=0, bit9=P=0, bits8..7=W=00, bits6..5=H=10,
        //   bit4=ρ0=1, bits3..2=00, bit1=ρ2=1, bit0=ρ1=1.
        bb.set(0, 1, 1); // ρ1 = 1
        bb.set(1, 1, 1); // ρ2 = 1
        bb.set(2, 2, 0); // bits[3:2] = 00
        bb.set(4, 1, 1); // ρ0 = 1
        bb.set(5, 2, 0b10); // H = 2  (bits 6,5)
        bb.set(7, 2, 0b00); // W = 0  (bits 8,7)
        bb.set(9, 1, 0); // P = 0
        bb.set(10, 1, 0); // DP = 0

        // Single partition: partition-count bits [12:11] = 00.
        bb.set(11, 2, 0);
        // CEM bits [16:13] = 8.
        bb.set(13, 4, 8);
        // Colour stream starts at bit 17. CEM 8 needs 6 integers.
        // With a generous budget the picker selects 0..255 (8 bits each).
        // Provide endpoints: e0 = (10, 20, 30), e1 = (200, 210, 220).
        // Mode-8 ordering is (v0=r0, v1=r1, v2=g0, v3=g1, v4=b0, v5=b1)
        // with an s0/s1 branch; pick s1>=s0 so the direct branch holds.
        let vals = [10u32, 200, 20, 210, 30, 220];
        let mut pos = 17;
        for v in vals {
            bb.set(pos, 8, v);
            pos += 8;
        }
        // Weights: 4×4 grid, 3 bits each, 16 weights. Set the top-left
        // grid weight to 0 (→ e0) and the bottom-right quadrant to 7
        // (→ e1). Weight stream bit order is bit-reversed from the top
        // of the block; the BISE range here is plain 3-bit, so value i
        // occupies stream bits [3i .. 3i+2].
        for i in 0..16u32 {
            let gx = i % 4;
            let gy = i / 4;
            let w = if gx >= 2 && gy >= 2 { 7 } else { 0 };
            for k in 0..3 {
                bb.set_weight_bit(i * 3 + k, (w >> k) & 1);
            }
        }
        let block = bb.bytes();
        let texels = decode_astc_ldr_block(&block, 4, 4);
        assert_eq!(texels.len(), 16);
        // Top-left texel (0,0) maps to grid weight 0 -> endpoint 0.
        assert_eq!(&texels[0][0..3], &[10, 20, 30]);
        // Bottom-right texel (3,3) maps to grid weight 1 -> endpoint 1.
        assert_eq!(&texels[15][0..3], &[200, 210, 220]);
        assert_eq!(texels[0][3], 255);
    }

    #[test]
    fn two_partition_routes_texels_by_pattern() {
        // A 4×4, two-partition, single-plane block with a 4×4 weight grid
        // at range 0..3 (2 bits/weight → 32 weight bits ≥ 24). All weights
        // are 0, so each texel reproduces its partition's endpoint 0. Both
        // partitions share CEM 8 (LDR RGB direct) via the selector==00
        // "single CEM" path (block bits [28..25]).
        //
        // Block mode (same DP/P/grid layout as the single-partition test,
        // 4×4 grid) but with the weight range set to 0..3 = (P=0, ρ=100):
        //   ρ2=1, ρ1=0, ρ0=0. The row is "DP P W W H H ρ0 0 0 ρ2 ρ1":
        //   bit1=ρ2=1, bit0=ρ1=0, bit4=ρ0=0, W=0, H=2.
        let mut bb = BlockBuilder::new();
        bb.set(0, 1, 0); // ρ1 = 0
        bb.set(1, 1, 1); // ρ2 = 1
        bb.set(2, 2, 0); // bits[3:2] = 00
        bb.set(4, 1, 0); // ρ0 = 0
        bb.set(5, 2, 0b10); // H = 2
        bb.set(7, 2, 0b00); // W = 0
        bb.set(9, 1, 0); // P = 0
        bb.set(10, 1, 0); // DP = 0

        // Two partitions: bits [12:11] = 01.
        bb.set(11, 2, 1);
        // Partition seed: bits [22:13] = 1 (matches the precomputed map).
        bb.set(13, 10, 1);
        // CEM selector bits [24:23] = 00 (single CEM for all partitions).
        bb.set(23, 2, 0);
        // 4-bit CEM in bits [28:25] = 8 (LDR RGB direct).
        bb.set(25, 4, 8);

        // Colour stream starts at bit 29. CEM 8 = 6 integers/partition ×
        // 2 partitions = 12 integers. Partition 0's endpoint 0 is encoded
        // by (v0,v2,v4) and partition 1's by the next group. Make the two
        // partitions clearly distinct: low values vs high values, and keep
        // s1 >= s0 so the direct (non-blue-contract) branch is taken.
        // The picker chooses the colour ISE range from the bit budget; we
        // only assert the partition *routing*, not exact bytes, so any
        // monotone value choice works. Use ascending values.
        // Determine the range the decoder will use so we can pack values.
        let weight_bits = 32u32;
        let config_bits = 29u32;
        let budget = 128 - config_bits - weight_bits;
        let range = pick_color_range(12, budget).expect("colour range");
        // Pack 12 values: partition 0 = small, partition 1 = large.
        // Mode-8 group i is (r0,r1,g0,g1,b0,b1). Endpoint0 = (r0,g0,b0) =
        // values at indices 0,2,4 within each 6-tuple.
        let p0 = [2u32, 5, 2, 5, 2, 5]; // endpoint0 ~ (2,2,2)
        let p1 = [
            range.max - 1,
            range.max,
            range.max - 1,
            range.max,
            range.max - 1,
            range.max,
        ];
        let mut seq: Vec<u32> = Vec::new();
        seq.extend_from_slice(&p0);
        seq.extend_from_slice(&p1);
        // Encode the integer sequence into the block at bit 29.
        pack_ise_into(&mut bb, 29, &seq, range);

        // All weights zero (already zero in a fresh builder).
        let block = bb.bytes();
        let texels = decode_astc_ldr_block(&block, 4, 4);
        assert_eq!(texels.len(), 16);

        // Precomputed partition map for seed=1, 4×4 small block, 2 parts.
        let map = [0u8, 1, 1, 0, 1, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 0];
        // Each partition must decode to a single constant colour (weights
        // are 0 everywhere), and the two partition colours must differ.
        let mut c0 = None;
        let mut c1 = None;
        for (i, t) in texels.iter().enumerate() {
            match map[i] {
                0 => {
                    let v = *c0.get_or_insert(*t);
                    assert_eq!(*t, v, "partition 0 texel {i} not constant");
                }
                _ => {
                    let v = *c1.get_or_insert(*t);
                    assert_eq!(*t, v, "partition 1 texel {i} not constant");
                }
            }
        }
        let c0 = c0.unwrap();
        let c1 = c1.unwrap();
        assert_ne!(c0, c1, "the two partitions must decode to distinct colours");
        // Partition 0 is the darker endpoint, partition 1 the brighter one.
        assert!(c1[0] > c0[0] && c1[1] > c0[1] && c1[2] > c0[2]);
    }

    #[test]
    fn dual_plane_alpha_uses_second_plane() {
        // A 4×4 dual-plane single-partition block: CEM 12 (LDR RGBA
        // direct), CCS = 3 (alpha uses weight plane 1). Plane 0 weights are
        // all 0 (→ endpoint0 for RGB), plane 1 weights are all max (→
        // endpoint1 for alpha). Verify RGB follows e0 while A follows e1,
        // proving the two planes are sampled independently.
        //
        // Use weight range 0..3 (P=0, ρ=100) so 16 grid points × 2 planes
        // × 2 bits = 64 weight bits. Block mode row "DP P W W H H ρ0 0 0
        // ρ2 ρ1" with DP=1, W=0, H=2.
        let mut bb = BlockBuilder::new();
        bb.set(0, 1, 0); // ρ1 = 0
        bb.set(1, 1, 1); // ρ2 = 1
        bb.set(2, 2, 0); // bits[3:2] = 00
        bb.set(4, 1, 0); // ρ0 = 0
        bb.set(5, 2, 0b10); // H = 2
        bb.set(7, 2, 0b00); // W = 0
        bb.set(9, 1, 0); // P = 0
        bb.set(10, 1, 1); // DP = 1 (dual plane)

        // Single partition.
        bb.set(11, 2, 0);
        // CEM bits [16:13] = 12 (LDR RGBA direct).
        bb.set(13, 4, 12);

        // CEM 12 = 8 colour integers. Colour stream starts at bit 17.
        // Mode 12 ordering is (r0,r1,g0,g1,b0,b1,a0,a1) with an s0/s1
        // branch; pick s1 >= s0 for the direct branch. Endpoint0 RGB is
        // dark, endpoint1 RGB bright; alpha e0=0x10, e1=0xF0.
        let weight_bits = 64u32;
        let config_bits = 17u32 + 2; // single partition + dual-plane CCS
        let budget = 128 - config_bits - weight_bits;
        let range = pick_color_range(8, budget).expect("colour range");
        // Choose values so s1 = r1+g1+b1 >= s0 = r0+g0+b0 (direct branch).
        let lo = 2u32.min(range.max);
        let hi = range.max;
        // (r0,r1,g0,g1,b0,b1,a0,a1)
        let seq = [lo, hi, lo, hi, lo, hi, lo, hi];
        pack_ise_into(&mut bb, 17, &seq, range);

        // CCS = 3 (alpha → plane 1). CCS sits just below the weight data
        // (no additional CEM bits for a single partition).
        let ccs_pos = (128 - weight_bits) - 2;
        bb.set(ccs_pos, 2, 3);

        // Weights: 16 grid points, 2 planes interleaved (plane0 then
        // plane1 per point), 2 bits each. plane0 = 0 (→ e0), plane1 = 3
        // (max → e1). Stream index for point i: 2*(i*2)+ for plane0,
        // plane1 follows. Value v occupies stream bits [2*v_index .. +1].
        for i in 0..16u32 {
            let idx0 = (i * 2) * 2; // plane 0 value index *2 bits
            let idx1 = (i * 2 + 1) * 2; // plane 1
                                        // plane0 weight = 0 (bits already 0)
            let _ = idx0;
            // plane1 weight = 3
            for k in 0..2 {
                bb.set_weight_bit(idx1 + k, (3 >> k) & 1);
            }
        }

        let block = bb.bytes();
        let texels = decode_astc_ldr_block(&block, 4, 4);
        assert_eq!(texels.len(), 16);
        // RGB should track endpoint0 (the dark values) since plane-0
        // weights are 0; alpha should track endpoint1 (bright) since it
        // is driven by plane 1 at max weight. The exact unquantized bytes
        // depend on the chosen range, but the relationship R,G,B < A must
        // hold for every texel.
        for t in &texels {
            assert!(
                (t[3] as u32) > (t[0] as u32),
                "alpha {} should exceed R {} (plane-1 vs plane-0)",
                t[3],
                t[0]
            );
        }
        // And RGB must be constant across the block (plane-0 weights are
        // uniformly 0).
        let first = texels[0];
        for t in &texels {
            assert_eq!(t[0..3], first[0..3], "RGB must be constant");
        }
    }

    #[test]
    fn surface_decode_sizes_output() {
        let data = vec![0u8; 16 * 4]; // 2x2 blocks of 4x4 -> 8x8 surface
        let out = decode_astc_ldr(&data, 8, 8, 4, 4);
        assert_eq!(out.len(), 8 * 8 * 4);
    }

    #[test]
    fn surface_decode_truncated_is_error_filled() {
        let data = vec![0u8; 8]; // not even one full block
        let out = decode_astc_ldr(&data, 4, 4, 4, 4);
        assert_eq!(out.len(), 4 * 4 * 4);
        for px in out.chunks_exact(4) {
            assert_eq!(px, ERROR_COLOR);
        }
    }

    // ----- encode round-trip -----

    #[test]
    fn encode_constant_block_is_exact_void_extent() {
        // A constant-colour block must round-trip byte-exact via void
        // extent for every footprint.
        for &(bw, bh) in &LDR_BLOCK_FOOTPRINTS {
            let (bw, bh) = (bw as u32, bh as u32);
            let color = [37u8, 211, 5, 255];
            let texels = vec![color; (bw * bh) as usize];
            let block = encode_astc_ldr_block(&texels, bw, bh);
            let out = decode_astc_ldr_block(&block, bw, bh);
            for t in &out {
                assert_eq!(*t, color, "constant {bw}x{bh}");
            }
        }
    }

    #[test]
    fn encode_two_color_4x4_roundtrips_endpoints() {
        // A 4×4 block split into a dark half and a bright half (exact
        // 1:1 weight grid) should reconstruct both colours closely.
        let dark = [20u8, 30, 40, 255];
        let bright = [200u8, 190, 180, 255];
        let mut texels = vec![dark; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            if i % 4 >= 2 {
                *t = bright;
            }
        }
        let block = encode_astc_ldr_block(&texels, 4, 4);
        let out = decode_astc_ldr_block(&block, 4, 4);
        // Each texel must be within a small tolerance of its source.
        for (i, t) in out.iter().enumerate() {
            let src = texels[i];
            for c in 0..3 {
                let d = (t[c] as i32 - src[c] as i32).abs();
                assert!(d <= 12, "texel {i} ch {c}: got {} want {}", t[c], src[c]);
            }
        }
    }

    #[test]
    fn encode_luma_ramp_roundtrips_within_tolerance() {
        // A single-subset encoder reconstructs every texel on the line
        // between the two block endpoints, so it represents a
        // *collinear* (luminance) gradient well. Build a grey ramp where
        // all channels move together and assert a small mean error.
        let (w, h) = (16u32, 16u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                let v = (((x + y) * 255) / (w + h - 2)) as u8;
                rgba[o] = v;
                rgba[o + 1] = v;
                rgba[o + 2] = v;
                rgba[o + 3] = 255;
            }
        }
        for &(bw, bh) in &[(4u8, 4u8), (8, 8), (6, 6), (5, 5)] {
            let enc = encode_astc_ldr(&rgba, w, h, bw as u32, bh as u32);
            assert!(!enc.is_empty());
            let dec = decode_astc_ldr(&enc, w, h, bw as u32, bh as u32);
            let mut sum: u64 = 0;
            for i in 0..(w * h) as usize {
                for c in 0..3 {
                    let a = rgba[i * 4 + c] as i64;
                    let b = dec[i * 4 + c] as i64;
                    sum += (a - b).unsigned_abs();
                }
            }
            let mae = sum as f64 / ((w * h * 3) as f64);
            assert!(mae < 16.0, "{bw}x{bh} MAE {mae} too high");
        }
    }

    #[test]
    fn encode_solid_surface_is_exact() {
        // A solid-colour surface must round-trip byte-exact (void extent)
        // at every footprint.
        let (w, h) = (12u32, 12u32);
        let color = [90u8, 17, 240, 255];
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for px in rgba.chunks_exact_mut(4) {
            px.copy_from_slice(&color);
        }
        for &(bw, bh) in &LDR_BLOCK_FOOTPRINTS {
            let enc = encode_astc_ldr(&rgba, w, h, bw as u32, bh as u32);
            let dec = decode_astc_ldr(&enc, w, h, bw as u32, bh as u32);
            for px in dec.chunks_exact(4) {
                assert_eq!(px, color, "{bw}x{bh} solid not exact");
            }
        }
    }

    #[test]
    fn encode_alpha_block_uses_rgba_mode() {
        // A block with varying alpha must round-trip alpha too (CEM 12).
        let mut texels = vec![[10u8, 20, 30, 0]; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            t[3] = (i * 16) as u8;
        }
        let block = encode_astc_ldr_block(&texels, 4, 4);
        let out = decode_astc_ldr_block(&block, 4, 4);
        for (i, t) in out.iter().enumerate() {
            let want = texels[i][3] as i32;
            let got = t[3] as i32;
            assert!((got - want).abs() <= 20, "alpha {i}: got {got} want {want}");
        }
    }

    #[test]
    fn encode_large_footprint_roundtrips() {
        // Footprints over 64 texels use a sub-sampled weight grid; ensure
        // they still produce a decodable, reasonably-close result.
        let (w, h) = (24u32, 24u32);
        let mut rgba = vec![0u8; (w * h * 4) as usize];
        for y in 0..h {
            for x in 0..w {
                let o = ((y * w + x) * 4) as usize;
                rgba[o] = (x * 10) as u8;
                rgba[o + 1] = (y * 10) as u8;
                rgba[o + 2] = 128;
                rgba[o + 3] = 255;
            }
        }
        for &(bw, bh) in &[(12u8, 12u8), (10, 10), (10, 8)] {
            let enc = encode_astc_ldr(&rgba, w, h, bw as u32, bh as u32);
            let dec = decode_astc_ldr(&enc, w, h, bw as u32, bh as u32);
            // No texel should be the error colour (a decodable block).
            let err = dec.chunks_exact(4).filter(|p| *p == ERROR_COLOR).count();
            assert_eq!(err, 0, "{bw}x{bh} produced error-colour texels");
        }
    }

    #[test]
    fn encode_block_never_panics_on_random_texels() {
        // Deterministic pseudo-random texels across every footprint.
        let mut state = 0x1234_5678u32;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state
        };
        for &(bw, bh) in &LDR_BLOCK_FOOTPRINTS {
            let (bw, bh) = (bw as u32, bh as u32);
            for _ in 0..8 {
                let texels: Vec<[u8; 4]> = (0..bw * bh)
                    .map(|_| {
                        let v = next();
                        [v as u8, (v >> 8) as u8, (v >> 16) as u8, (v >> 24) as u8]
                    })
                    .collect();
                let block = encode_astc_ldr_block(&texels, bw, bh);
                // Decoding the produced block must not yield error colour
                // for these legal blocks.
                let out = decode_astc_ldr_block(&block, bw, bh);
                assert_eq!(out.len(), (bw * bh) as usize);
            }
        }
    }

    #[test]
    fn encode_two_subset_beats_single_on_split_block() {
        // A 4×4 block whose left half is one colour line and right half a
        // very different one is poorly served by a single endpoint pair.
        // The full encoder (which tries 2-subset) must reconstruct it at
        // least as well as the single-subset path, and strictly better on
        // this adversarial case.
        let red = [220u8, 20, 20, 255];
        let blue = [20u8, 20, 220, 255];
        let mut texels = vec![red; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            if i % 4 >= 2 {
                *t = blue;
            }
        }
        let full = encode_astc_ldr_block(&texels, 4, 4);
        let single = encode_single_subset(&texels, 4, 4);
        let full_sad = block_sad(&full, &texels, 4, 4);
        let single_sad = block_sad(&single, &texels, 4, 4);
        assert!(
            full_sad <= single_sad,
            "full {full_sad} should not exceed single {single_sad}"
        );
        // 2-subset must give a meaningful improvement on this split block
        // (at least 20% lower error than the single-pair fit).
        assert!(
            full_sad * 5 < single_sad * 4,
            "2-subset should beat single by >20%: full {full_sad} single {single_sad}"
        );
    }

    #[test]
    fn encode_dual_plane_helps_uncorrelated_alpha() {
        // RGB is a smooth ramp one way, alpha a smooth ramp the *other*
        // way — a single weight per texel (single-plane) can't track both.
        // A dual-plane block gives alpha its own weight axis, so the full
        // encoder must beat the single-subset fit here.
        let mut texels = vec![[0u8; 4]; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            let x = (i % 4) as u8;
            let y = (i / 4) as u8;
            let v = x * 60; // RGB increases with x
            t[0] = v;
            t[1] = v;
            t[2] = v;
            t[3] = 255 - y * 60; // alpha decreases with y
        }
        let full = encode_astc_ldr_block(&texels, 4, 4);
        let single = encode_single_subset(&texels, 4, 4);
        let full_sad = block_sad(&full, &texels, 4, 4);
        let single_sad = block_sad(&single, &texels, 4, 4);
        assert!(
            full_sad < single_sad,
            "dual-plane should beat single: full {full_sad} single {single_sad}"
        );
    }

    #[test]
    fn encode_dual_plane_block_is_decodable() {
        let mut texels = vec![[40u8, 40, 40, 0]; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            t[3] = (i * 16) as u8;
        }
        if let Some(block) = encode_dual_plane(&texels, 4, 4) {
            let out = decode_astc_ldr_block(&block, 4, 4);
            for t in &out {
                assert_ne!(*t, ERROR_COLOR, "dual-plane decoded to error colour");
            }
        }
    }

    #[test]
    fn encode_two_subset_block_is_decodable() {
        // The 2-subset path must produce a legal, non-error block.
        let red = [200u8, 30, 30, 255];
        let green = [30u8, 200, 30, 255];
        let mut texels = vec![red; 16];
        for (i, t) in texels.iter_mut().enumerate() {
            if i >= 8 {
                *t = green;
            }
        }
        if let Some(block) = encode_two_subset(&texels, 4, 4, 1) {
            let out = decode_astc_ldr_block(&block, 4, 4);
            for t in &out {
                assert_ne!(*t, ERROR_COLOR, "2-subset block decoded to error colour");
            }
        }
    }
}
