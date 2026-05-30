//! Criterion benchmarks for the full DDS container round-trip
//! (`parse_dds` → `encode_dds_uncompressed` → `parse_dds`).
//!
//! Round 192 (depth-mode benchmarks): paired with `decode.rs` and
//! `encode.rs`. Where those harnesses isolate the per-block BC*
//! decode / encode hot path, this one measures the container-level
//! cost — header validation, pixel-format resolution, surface-table
//! walking, mip / array / face / depth-slice enumeration, and the
//! encoder's matching emit pass.
//!
//! Scenarios:
//!
//!   - **rt_a8r8g8b8_512x512**: Single 32-bpp surface (legacy
//!     header, no DXT10 extension). The straight-line "parse-then-
//!     emit" baseline.
//!   - **rt_a8r8g8b8_mip_256x256**: 256×256 A8R8G8B8 with a full
//!     mipmap chain (9 levels). Exercises the encoder's per-mip
//!     surface-table walk and the parser's chain-aware surface
//!     enumeration.
//!   - **rt_a8b8g8r8_dxt10_128x128**: 128×128 R8G8B8A8_UNORM via the
//!     DXT10 extension header. Exercises the DX10 path through both
//!     parser (header_dxt10 read) and encoder.
//!   - **rt_l8_64x64**: 8-bpp single-channel L8 surface — the
//!     narrowest format. Useful as a header-parse-cost baseline.
//!
//! Run with:
//!     cargo bench -p oxideav-dds --bench roundtrip

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};

use oxideav_dds::{
    encode_dds_uncompressed, parse_dds, types::DxgiFormat, DdsImage, DdsPixelFormat, DdsPlane,
};

fn xorshift32(state: &mut u32) -> u32 {
    *state ^= *state << 13;
    *state ^= *state >> 17;
    *state ^= *state << 5;
    *state
}

fn build_a8r8g8b8(width: u32, height: u32, mip_levels: u32) -> DdsImage {
    let mut state: u32 = 0xCAFE_F00D;
    let bpp = 4u32;
    let mut planes: Vec<DdsPlane> = Vec::new();
    // Encoder fabricates the mipmap chain on its own when
    // `mip_map_count > 1` and the surface list contains only mip 0,
    // so we just emit one plane here.
    let stride = (width * bpp) as usize;
    let mut data = vec![0u8; stride * height as usize];
    for v in data.iter_mut() {
        *v = (xorshift32(&mut state) >> 24) as u8;
    }
    planes.push(DdsPlane { stride, data });
    DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::A8R8G8B8,
        planes,
        pts: None,
        mip_map_count: mip_levels,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
        surfaces: Vec::new(),
    }
}

fn build_a8b8g8r8_dxt10(width: u32, height: u32) -> DdsImage {
    let mut state: u32 = 0xDEAD_BEEF;
    let bpp = 4u32;
    let stride = (width * bpp) as usize;
    let mut data = vec![0u8; stride * height as usize];
    for v in data.iter_mut() {
        *v = (xorshift32(&mut state) >> 24) as u8;
    }
    DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::A8B8G8R8,
        planes: vec![DdsPlane { stride, data }],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: true,
        dxgi_format: Some(DxgiFormat::R8G8B8A8Unorm),
        is_cubemap: false,
        array_size: 1,
        depth: 1,
        surfaces: Vec::new(),
    }
}

fn build_l8(width: u32, height: u32) -> DdsImage {
    let mut state: u32 = 0xABAD_1DEA;
    let stride = width as usize;
    let mut data = vec![0u8; stride * height as usize];
    for v in data.iter_mut() {
        *v = (xorshift32(&mut state) >> 24) as u8;
    }
    DdsImage {
        width,
        height,
        pixel_format: DdsPixelFormat::L8,
        planes: vec![DdsPlane { stride, data }],
        pts: None,
        mip_map_count: 1,
        has_dxt10_header: false,
        dxgi_format: None,
        is_cubemap: false,
        array_size: 1,
        depth: 1,
        surfaces: Vec::new(),
    }
}

fn bench_rt_a8r8g8b8_512(c: &mut Criterion) {
    let mut group = c.benchmark_group("rt_a8r8g8b8_512x512");
    let img = build_a8r8g8b8(512, 512, 1);
    let bytes = encode_dds_uncompressed(&img).unwrap();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_with_input(BenchmarkId::new("parse", "A8R8G8B8"), &bytes, |b, bytes| {
        b.iter(|| {
            let parsed = parse_dds(bytes).unwrap();
            criterion::black_box(parsed);
        });
    });
    group.bench_with_input(BenchmarkId::new("encode", "A8R8G8B8"), &img, |b, img| {
        b.iter(|| {
            let out = encode_dds_uncompressed(img).unwrap();
            criterion::black_box(out);
        });
    });
    group.finish();
}

fn bench_rt_a8r8g8b8_mip(c: &mut Criterion) {
    let mut group = c.benchmark_group("rt_a8r8g8b8_mip_256x256");
    let img = build_a8r8g8b8(256, 256, 9);
    let bytes = encode_dds_uncompressed(&img).unwrap();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_with_input(BenchmarkId::new("parse", "mip9"), &bytes, |b, bytes| {
        b.iter(|| {
            let parsed = parse_dds(bytes).unwrap();
            criterion::black_box(parsed);
        });
    });
    group.bench_with_input(BenchmarkId::new("encode", "mip9"), &img, |b, img| {
        b.iter(|| {
            let out = encode_dds_uncompressed(img).unwrap();
            criterion::black_box(out);
        });
    });
    group.finish();
}

fn bench_rt_a8b8g8r8_dxt10(c: &mut Criterion) {
    let mut group = c.benchmark_group("rt_a8b8g8r8_dxt10_128x128");
    let img = build_a8b8g8r8_dxt10(128, 128);
    let bytes = encode_dds_uncompressed(&img).unwrap();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_with_input(BenchmarkId::new("parse", "DXT10"), &bytes, |b, bytes| {
        b.iter(|| {
            let parsed = parse_dds(bytes).unwrap();
            criterion::black_box(parsed);
        });
    });
    group.bench_with_input(BenchmarkId::new("encode", "DXT10"), &img, |b, img| {
        b.iter(|| {
            let out = encode_dds_uncompressed(img).unwrap();
            criterion::black_box(out);
        });
    });
    group.finish();
}

fn bench_rt_l8(c: &mut Criterion) {
    let mut group = c.benchmark_group("rt_l8_64x64");
    let img = build_l8(64, 64);
    let bytes = encode_dds_uncompressed(&img).unwrap();
    group.throughput(Throughput::Bytes(bytes.len() as u64));
    group.bench_with_input(BenchmarkId::new("parse", "L8"), &bytes, |b, bytes| {
        b.iter(|| {
            let parsed = parse_dds(bytes).unwrap();
            criterion::black_box(parsed);
        });
    });
    group.bench_with_input(BenchmarkId::new("encode", "L8"), &img, |b, img| {
        b.iter(|| {
            let out = encode_dds_uncompressed(img).unwrap();
            criterion::black_box(out);
        });
    });
    group.finish();
}

criterion_group!(
    benches,
    bench_rt_a8r8g8b8_512,
    bench_rt_a8r8g8b8_mip,
    bench_rt_a8b8g8r8_dxt10,
    bench_rt_l8,
);
criterion_main!(benches);
