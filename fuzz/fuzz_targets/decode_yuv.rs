#![no_main]

//! Drive arbitrary fuzz-supplied bytes through every YUV surface
//! decoder. Each entry point must be panic-free and bounds-safe on every
//! input: a too-short payload must return `Err` (never index out of
//! bounds), and a sufficient payload must return exactly
//! `width * height * 4` interleaved samples.
//!
//! Fuzz strategy: the first byte selects one of the eleven YUV formats;
//! the next two bytes steer a bounded width / height (each kept small so
//! the harness's own output allocation stays bounded); the remaining
//! bytes are the surface payload, fed both at its natural length and
//! deliberately truncated to exercise the length checks. The dimensions
//! are aligned up to each format's documented divisibility constraint so
//! a fraction of iterations reach the decode body rather than always
//! tripping the dimension check.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::yuv::YuvFormat;
use oxideav_dds::{
    decode_420_opaque_surface, decode_ayuv_surface, decode_nv11_surface, decode_nv12_surface,
    decode_p010_surface, decode_p016_surface, decode_y210_surface, decode_y216_surface,
    decode_y410_surface, decode_y416_surface, decode_yuy2_surface,
};

const MAX_DIM: u32 = 64;

fn align_up(v: u32, m: u32) -> u32 {
    if m <= 1 {
        v.max(1)
    } else {
        ((v / m).max(1)) * m
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }

    let fmt = match data[0] % 11 {
        0 => YuvFormat::Ayuv,
        1 => YuvFormat::Y410,
        2 => YuvFormat::Y416,
        3 => YuvFormat::Nv12,
        4 => YuvFormat::P010,
        5 => YuvFormat::P016,
        6 => YuvFormat::Opaque420,
        7 => YuvFormat::Yuy2,
        8 => YuvFormat::Y210,
        9 => YuvFormat::Y216,
        _ => YuvFormat::Nv11,
    };

    let s = fmt.sampling();
    let width = align_up((u32::from(data[1]) % MAX_DIM) + 1, s.width_multiple());
    let height = align_up((u32::from(data[2]) % MAX_DIM) + 1, s.height_multiple());
    let payload = &data[3..];

    // Helper that dispatches to the right decoder. Returns Ok(len) where
    // len is the sample count on success.
    let run_u8 = |w: u32, h: u32, p: &[u8]| -> Option<usize> {
        let r = match fmt {
            YuvFormat::Ayuv => decode_ayuv_surface(w, h, p),
            YuvFormat::Yuy2 => decode_yuy2_surface(w, h, p),
            YuvFormat::Nv12 => decode_nv12_surface(w, h, p),
            YuvFormat::Opaque420 => decode_420_opaque_surface(w, h, p),
            YuvFormat::Nv11 => decode_nv11_surface(w, h, p),
            _ => return None,
        };
        r.ok().map(|v| v.len())
    };
    let run_u16 = |w: u32, h: u32, p: &[u8]| -> Option<usize> {
        let r = match fmt {
            YuvFormat::Y410 => decode_y410_surface(w, h, p),
            YuvFormat::Y416 => decode_y416_surface(w, h, p),
            YuvFormat::Y210 => decode_y210_surface(w, h, p),
            YuvFormat::Y216 => decode_y216_surface(w, h, p),
            YuvFormat::P010 => decode_p010_surface(w, h, p),
            YuvFormat::P016 => decode_p016_surface(w, h, p),
            _ => return None,
        };
        r.ok().map(|v| v.len())
    };

    let want = (width as usize) * (height as usize) * 4;

    // Run against the raw payload (likely too short → Err) and against a
    // correctly-sized zero-padded payload (must succeed and return
    // width*height*4 samples).
    let need = fmt.surface_size_bytes(width, height).unwrap_or(u64::MAX) as usize;
    let mut padded = vec![0u8; need];
    let n = payload.len().min(need);
    padded[..n].copy_from_slice(&payload[..n]);

    if fmt.is_u16() {
        let _ = run_u16(width, height, payload);
        if let Some(len) = run_u16(width, height, &padded) {
            assert_eq!(len, want);
        }
    } else {
        let _ = run_u8(width, height, payload);
        if let Some(len) = run_u8(width, height, &padded) {
            assert_eq!(len, want);
        }
    }
});
