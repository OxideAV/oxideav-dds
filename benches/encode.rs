//! Criterion benchmarks for the DDS block-compressed encoder hot
//! paths.
//!
//! Round 192 (depth-mode benchmarks): paired with `decode.rs` and
//! `roundtrip.rs`. Each scenario synthesises a deterministic
//! pixel-space source in a setup step and times the encoder pass
//! that produces the BC* block payload. The PCM / RGBA synthesis
//! work is **outside** the timed region — only the encoder's
//! per-block endpoint search + index quantisation is measured.
//!
//! The BC6H and BC7 picker is the most expensive path in the crate
//! (sweeping every mode × partition table entry per block), so its
//! bench is run on a smaller surface (128×128) than the simpler
//! BCn encoders (256×256).
//!
//! Scenarios:
//!
//!   - **encode_bc1_256x256**: 256×256 RGBA8 → BC1. Furthest-point
//!     RGB565 endpoint heuristic + 2-bit index plane assignment.
//!   - **encode_bc3_256x256**: 256×256 RGBA8 → BC3. Adds 8-value
//!     interpolated alpha endpoint search on top of BC1's RGB path.
//!   - **encode_bc4_256x256**: 256×256 R8 → BC4. Single-channel
//!     endpoint min/max + index quantisation.
//!   - **encode_bc5_256x256**: 256×256 RG8 → BC5. Two BC4 channels.
//!   - **encode_bc6h_128x128**: 128×128 RGBA half-float → BC6H. The
//!     mode picker sweeps mode 10 (1-subset absolute), modes 11/12/13
//!     (1-subset delta-encoded) and modes 0..9 (2-subset over the
//!     32-entry BC6H partition table) for every block.
//!   - **encode_bc7_128x128**: 128×128 RGBA8 → BC7. The mode picker
//!     sweeps 1-subset modes 6/4/5 (channel-rotation), 2-subset modes
//!     1/3/7 (over the 64-entry BC7 2-subset partition table) and
//!     3-subset modes 0/2 (over the 64-entry BC7 3-subset partition
//!     table) for every block.
//!
//! Run with:
//!     cargo bench -p oxideav-dds --bench encode

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_dds::{
    encode_bc1, encode_bc3, encode_bc4_unorm, encode_bc5_unorm, encode_bc6h, encode_bc7,
};

fn xorshift32(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn build_rgba8(width: usize, height: usize, seed: u32) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 4];
    let mut state: u32 = seed;
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 4;
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

fn build_rgba_half(width: usize, height: usize) -> Vec<u8> {
    let mut out = vec![0u8; width * height * 8];
    for y in 0..height {
        for x in 0..width {
            let i = (y * width + x) * 8;
            let u = x as f32 / width.max(1) as f32;
            let v = y as f32 / height.max(1) as f32;
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

fn bench_encode_bc1(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc1");
    let w = 256usize;
    let h = 256usize;
    let rgba = build_rgba8(w, h, 0xCAFE_F00D);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC1"), &rgba, |b, rgba| {
        let mut out = vec![0u8; bw * bh * 8];
        b.iter(|| {
            encode_bc1(rgba, w as u32, h as u32, false, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_encode_bc3(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc3");
    let w = 256usize;
    let h = 256usize;
    let rgba = build_rgba8(w, h, 0xDEAD_BEEF);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC3"), &rgba, |b, rgba| {
        let mut out = vec![0u8; bw * bh * 16];
        b.iter(|| {
            encode_bc3(rgba, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_encode_bc4(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc4");
    let w = 256usize;
    let h = 256usize;
    let r = build_r8(w, h, 0xABAD_1DEA);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC4"), &r, |b, src| {
        let mut out = vec![0u8; bw * bh * 8];
        b.iter(|| {
            encode_bc4_unorm(src, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_encode_bc5(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc5");
    let w = 256usize;
    let h = 256usize;
    let rg = build_rg8(w, h, 0xFEED_FACE);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h * 2) as u64));
    group.bench_with_input(BenchmarkId::new("256x256", "BC5"), &rg, |b, src| {
        let mut out = vec![0u8; bw * bh * 16];
        b.iter(|| {
            encode_bc5_unorm(src, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_encode_bc6h(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc6h");
    let w = 128usize;
    let h = 128usize;
    let half = build_rgba_half(w, h);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h * 8) as u64));
    group.sample_size(10);
    group.bench_with_input(BenchmarkId::new("128x128", "BC6H"), &half, |b, src| {
        let mut out = vec![0u8; bw * bh * 16];
        b.iter(|| {
            encode_bc6h(src, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

fn bench_encode_bc7(c: &mut Criterion) {
    let mut group = c.benchmark_group("encode_bc7");
    let w = 128usize;
    let h = 128usize;
    let rgba = build_rgba8(w, h, 0x1357_9BDF);
    let bw = w.div_ceil(4);
    let bh = h.div_ceil(4);
    group.throughput(Throughput::Bytes((w * h * 4) as u64));
    group.sample_size(10);
    group.bench_with_input(BenchmarkId::new("128x128", "BC7"), &rgba, |b, src| {
        let mut out = vec![0u8; bw * bh * 16];
        b.iter(|| {
            encode_bc7(src, w as u32, h as u32, &mut out).unwrap();
            criterion::black_box(&out[0]);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_encode_bc1,
    bench_encode_bc3,
    bench_encode_bc4,
    bench_encode_bc5,
    bench_encode_bc6h,
    bench_encode_bc7,
);
criterion_main!(benches);
