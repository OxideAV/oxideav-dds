#![no_main]

//! The image-crate contract entry points over arbitrary bytes:
//! `probe` must be total, `info` / `decode` / `decode_all` must return
//! a `Result` without panicking or committing memory the limits forbid,
//! and whatever `decode` accepts must re-encode and decode back to an
//! equal image.
//!
//! The first byte selects a limit profile so the fuzzer also exercises
//! the `LimitExceeded` paths and the strict-mode checks.

use libfuzzer_sys::fuzz_target;
use oxideav_dds::{decode, decode_all_with, decode_with, encode, info_with, probe, DecodeOptions, EncodeOptions};

fuzz_target!(|data: &[u8]| {
    let Some((&sel, bytes)) = data.split_first() else {
        return;
    };
    let _ = probe(bytes);
    let opts = match sel & 3 {
        0 => DecodeOptions::default(),
        1 => DecodeOptions::default().with_strict(true),
        2 => DecodeOptions::default()
            .with_max_width(64)
            .with_max_height(64)
            .with_max_pixels(4096)
            .with_max_bytes(1 << 16),
        _ => DecodeOptions::default().with_max_bytes(1 << 20),
    };
    let _ = info_with(bytes, &opts);
    if let Ok(img) = decode_with(bytes, &opts) {
        let _ = img.to_rgba8();
        if let Ok(re) = encode(&img, &EncodeOptions::default()) {
            let back = decode(&re).expect("re-encoded image must decode");
            assert_eq!(back.format, img.format);
            assert_eq!((back.width, back.height), (img.width, img.height));
            assert_eq!(back.planes, img.planes, "lossless round trip");
        }
    }
    let _ = decode_all_with(bytes, &opts);
});
