//! Criterion benchmarks for the DDS block-compressed decoder hot
//! paths.
//!
//! Round 192 (depth-mode benchmarks): paired with `encode.rs` and
//! `roundtrip.rs`. Each scenario synthesises a deterministic RGBA8
//! source surface, encodes it via the crate's own production
//! `encode_bc*` entry point in a setup step, and then iterates the
//! decoder on the resulting block payload. The encode work is
//! **outside** the timed region — only the decoder's per-surface
//! block walk + endpoint expansion + index plane unpack is measured.
//!
//! Scenarios:
//!
//!   - **decode_bc1_512x512**: 512×512 RGBA8 source → BC1 blocks →
//!     `decode_bc1`. Exercises the RGB565 endpoint expand + 2-bit
//!     index plane unpack.
//!   - **decode_bc3_512x512**: 512×512 RGBA8 source → BC3 blocks →
//!     `decode_bc3`. Adds the 8-value interpolated alpha plane on
//!     top of the BC1 colour path.
//!   - **decode_bc4_512x512**: 512×512 R8 source → BC4 blocks →
//!     `decode_bc4_unorm`. Single-channel interpolated-endpoint
//!     path.
//!   - **decode_bc5_512x512**: 512×512 RG8 source → BC5 blocks →
//!     `decode_bc5_unorm`. Two BC4-style channels stacked.
//!   - **decode_bc6h_256x256**: 256×256 RGBA half-float source →
//!     BC6H blocks → `decode_bc6h`. All 14 modes are dispatched
//!     by the per-block 5-bit mode prefix.
//!   - **decode_bc7_256x256**: 256×256 RGBA8 source → BC7 blocks →
//!     `decode_bc7`. All 8 modes are dispatched by the per-block
//!     1..8-bit mode prefix; covers partition-table lookup and
//!     index-plane unpack for 1- / 2- / 3-subset blocks.
//!
//! Run with:
//!     cargo bench -p oxideav-dds --bench decode

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_dds::{
    decode_bc1, decode_bc3, decode_bc4_unorm, decode_bc5_unorm, decode_bc6h, decode_bc7,
    encode_bc1, encode_bc3, encode_bc4_unorm, encode_bc5_unorm, encode_bc6h, encode_bc7,
};

fn xorshift32(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

/// Smooth gradient + small additive noise — the regime where BC*
/// endpoints are well-defined and the index plane is fully used
/// (the most realistic decode workload).
fn build_rgba8(width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    let mut state: u32 = seed;
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 4;
            // Diagonal gradient + low-amplitude noise per channel.
            let r = ((x as u32 * 255 / width.max(1) as u32) & 0xff) as u8;
            let g = ((y as u32 * 255 / height.max(1) as u32) & 0xff) as u8;
            let b = (((x + y) as u32 * 255 / (width + height).max(1) as u32) & 0xff) as u8;
            let a = ((xorshift32(&mut state) >> 24) | 0x80) as u8;
            out[i] = r.saturating_add((xorshift32(&mut state) >> 28) as u8);
            out[i + 1] = g.saturating_add((xorshift32(&mut state) >> 28) as u8);
            out[i + 2] = b.saturating_add((xorshift32(&mut state) >> 28) as u8);
            out[i + 3] = a;
        }
    }
    out
}

fn build_r8(width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut out = vec![0u8; width * height];
    let mut state: u32 = seed;
    for y in 0..height {
        for x in 0..width {
            let v = ((x + y) as u32 * 255 / (width + height).max(1) as u32) as u8;
            out[y * width + x] = v.saturating_add((xorshift32(&mut state) >> 28) as u8);
        }
    }
    out
}

fn build_rg8(width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 2];
    let mut state: u32 = seed;
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 2;
            let r = ((x as u32 * 255 / width.max(1) as u32) & 0xff) as u8;
            let g = ((y as u32 * 255 / height.max(1) as u32) & 0xff) as u8;
            out[i] = r.saturating_add((xorshift32(&mut state) >> 28) as u8);
            out[i + 1] = g.saturating_add((xorshift32(&mut state) >> 28) as u8);
        }
    }
    out
}

/// Build a deterministic RGBA half-float source for BC6H. Half-floats
/// are little-endian IEEE-754 binary16; encode a smooth radial
/// gradient in [0, 1] so each block has well-defined endpoints.
fn build_rgba_half(width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 8];
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 8;
            // u/v in [0, 1].
            let u = x as f32 / width.max(1) as f32;
            let v = y as f32 / height.max(1) as f32;
            // Smooth gradients per channel.
            let r = u;
            let g = v;
            let b = 0.5 * (u + v);
            let a = 1.0f32;
            for (ch, val) in [r, g, b, a].iter().enumerate() {
                let h = f32_to_half(*val);
                out[i + ch * 2] = (h & 0xff) as u8;
                out[i + ch * 2 + 1] = (h >> 8) as u8;
            }
        }
    }
    out
}

/// Tiny f32 → IEEE-754 binary16 converter (round-to-zero, no
/// subnormal / NaN handling — adequate for the [0, 1] inputs
/// produced by [`build_rgba_half`]).
fn f32_to_half(x: f32) -> u16 {
    let bits = x.to_bits();
    let sign = ((bits >> 31) & 0x1) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let mant = bits & 0x7f_ffff;
    if exp == 0 {
        return sign << 15;
    }
    let new_exp = exp - 127 + 15;
    if new_exp <= 0 {
        return sign << 15;
    }
    if new_exp >= 31 {
        return (sign << 15) | 0x7c00;
    }
    let new_mant = (mant >> 13) as u16;
    (sign << 15) | ((new_exp as u16) << 10) | new_mant
}

fn bench_decode_bc1(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc1");
    let w = 512usize;
    let h = 512usize;
    let rgba = build_rgba8(w, h, 0xCAFE_F00D);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 8];
    encode_bc1(&rgba, w as u32, h as u32, false, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.bench_with_input(BenchmarkId::new("512x512", "BC1"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h * 4];
        b.iter(|| {
            decode_bc1(blocks, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_decode_bc3(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc3");
    let w = 512usize;
    let h = 512usize;
    let rgba = build_rgba8(w, h, 0xDEAD_BEEF);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 16];
    encode_bc3(&rgba, w as u32, h as u32, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.bench_with_input(BenchmarkId::new("512x512", "BC3"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h * 4];
        b.iter(|| {
            decode_bc3(blocks, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_decode_bc4(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc4");
    let w = 512usize;
    let h = 512usize;
    let r = build_r8(w, h, 0xABAD_1DEA);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 8];
    encode_bc4_unorm(&r, w as u32, h as u32, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h) as u64));
    group.bench_with_input(BenchmarkId::new("512x512", "BC4"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h];
        b.iter(|| {
            decode_bc4_unorm(blocks, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_decode_bc5(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc5");
    let w = 512usize;
    let h = 512usize;
    let rg = build_rg8(w, h, 0xFEED_FACE);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 16];
    encode_bc5_unorm(&rg, w as u32, h as u32, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h * 2) as u64));
    group.bench_with_input(BenchmarkId::new("512x512", "BC5"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h * 2];
        b.iter(|| {
            decode_bc5_unorm(blocks, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_decode_bc6h(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc6h");
    let w = 256usize;
    let h = 256usize;
    let half = build_rgba_half(w, h);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 16];
    encode_bc6h(&half, w as u32, h as u32, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h * 8) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC6H"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h * 8];
        b.iter(|| {
            decode_bc6h(blocks, w as u32, h as u32, false, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_decode_bc7(c: &mut Criterion) {
    let mut group = c.benchmark_group("decode_bc7");
    let w = 256usize;
    let h = 256usize;
    let rgba = build_rgba8(w, h, 0x1357_9BDF);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    let mut blocks = vec![0u8; bw * bh * 16];
    encode_bc7(&rgba, w as u32, h as u32, &mut blocks).unwrap();
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC7"), &blocks, |b, blocks| {
        let mut out = vec![0u8; w * h * 4];
        b.iter(|| {
            decode_bc7(blocks, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_decode_bc1,
    bench_decode_bc3,
    bench_decode_bc4,
    bench_decode_bc5,
    bench_decode_bc6h,
    bench_decode_bc7,
);
criterion_main!(benches);
