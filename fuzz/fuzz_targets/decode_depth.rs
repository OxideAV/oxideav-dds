#![no_main]

//! Drive arbitrary fuzz-supplied bytes through every depth /
//! depth-stencil surface decoder. Each entry point must be panic-free
//! and bounds-safe on every input: a too-short payload must return
//! `Err` (never index out of bounds), and a sufficient payload must
//! return exactly `width * height` decoded texels.
//!
//! Fuzz strategy: the first byte selects one of the four depth formats;
//! the next two bytes steer a bounded width / height (each kept small so
//! the harness's own output allocation stays bounded); the remaining
//! bytes are the surface payload, fed both at its natural length
//! (usually too short → `Err`) and zero-padded to the exact required
//! length (must succeed and return `width * height` texels).

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{
    decode_depth_d16_surface, decode_depth_d24s8_surface, decode_depth_d32_surface,
    decode_depth_d32s8_surface, decode_depth_r24_unorm_x8_surface,
    decode_depth_r32_float_x8x24_surface, decode_depth_x24_g8_uint_surface,
    decode_depth_x32_g8x24_uint_surface,
};

const MAX_DIM: u32 = 64;

#[derive(Clone, Copy)]
enum DepthFmt {
    D16,
    D32,
    D24S8,
    D32S8,
    R24X8,
    X24G8,
    R32X8X24,
    X32G8X24,
}

impl DepthFmt {
    /// Bytes per texel.
    fn bpp(self) -> usize {
        match self {
            DepthFmt::D16 => 2,
            DepthFmt::D32 | DepthFmt::D24S8 | DepthFmt::R24X8 | DepthFmt::X24G8 => 4,
            DepthFmt::D32S8 | DepthFmt::R32X8X24 | DepthFmt::X32G8X24 => 8,
        }
    }
}

fuzz_target!(|data: &[u8]| {
    if data.len() < 3 {
        return;
    }

    let fmt = match data[0] % 8 {
        0 => DepthFmt::D16,
        1 => DepthFmt::D32,
        2 => DepthFmt::D24S8,
        3 => DepthFmt::D32S8,
        4 => DepthFmt::R24X8,
        5 => DepthFmt::X24G8,
        6 => DepthFmt::R32X8X24,
        _ => DepthFmt::X32G8X24,
    };

    let width = (u32::from(data[1]) % MAX_DIM) + 1;
    let height = (u32::from(data[2]) % MAX_DIM) + 1;
    let payload = &data[3..];

    let want = (width as usize) * (height as usize);

    // Run against the raw payload (likely too short → Err) and against a
    // correctly-sized zero-padded payload (must succeed and return
    // width*height texels).
    let need = want * fmt.bpp();
    let mut padded = vec![0u8; need];
    let n = payload.len().min(need);
    padded[..n].copy_from_slice(&payload[..n]);

    match fmt {
        DepthFmt::D16 => {
            let _ = decode_depth_d16_surface(width, height, payload);
            if let Ok(v) = decode_depth_d16_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::D32 => {
            let _ = decode_depth_d32_surface(width, height, payload);
            if let Ok(v) = decode_depth_d32_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::D24S8 => {
            let _ = decode_depth_d24s8_surface(width, height, payload);
            if let Ok(v) = decode_depth_d24s8_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::D32S8 => {
            let _ = decode_depth_d32s8_surface(width, height, payload);
            if let Ok(v) = decode_depth_d32s8_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::R24X8 => {
            let _ = decode_depth_r24_unorm_x8_surface(width, height, payload);
            if let Ok(v) = decode_depth_r24_unorm_x8_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::X24G8 => {
            let _ = decode_depth_x24_g8_uint_surface(width, height, payload);
            if let Ok(v) = decode_depth_x24_g8_uint_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::R32X8X24 => {
            let _ = decode_depth_r32_float_x8x24_surface(width, height, payload);
            if let Ok(v) = decode_depth_r32_float_x8x24_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
        DepthFmt::X32G8X24 => {
            let _ = decode_depth_x32_g8x24_uint_surface(width, height, payload);
            if let Ok(v) = decode_depth_x32_g8x24_uint_surface(width, height, &padded) {
                assert_eq!(v.len(), want);
            }
        }
    }
});
