// When built without the `registry` feature, the framework-side
// trait impls and the bridging conversions in `registry.rs` are
// gated out, leaving a couple of helpers (the optional Decoder
// factory in `decoder.rs`, the Encoder factory in `encoder.rs`)
// without callers. Suppress the resulting dead-code warnings rather
// than gating every helper.
#![cfg_attr(not(feature = "registry"), allow(dead_code))]

//! Pure-Rust DDS (DirectDraw Surface) reader / writer.
//!
//! DDS is Microsoft's container for Direct3D textures: a 4-byte ASCII
//! magic, a fixed-layout 124-byte `DDS_HEADER`, an optional 20-byte
//! `DDS_HEADER_DXT10` extension (when the legacy header signals
//! `four_cc == "DX10"`), and the raw pixel array (or block-compressed
//! block array) for one or more mip levels.
//!
//! Coverage as of round 4:
//!
//! * Header parsing — magic + `DDS_HEADER` (124 bytes) + optional
//!   `DDS_HEADER_DXT10` (20 bytes).
//! * Uncompressed pixel formats with bit-exact round-trip through
//!   [`parse_dds`] + [`encode_dds_uncompressed`]: A8R8G8B8, X8R8G8B8,
//!   A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5, A1R5G5B5, A4R4G4B4,
//!   R8G8B8, A8L8, L8, A8.
//! * Block-compressed pass-through — recognises BC1..BC7 from the
//!   legacy four-cc or the DX10 `dxgi_format` and exposes the raw
//!   block bytes through `DdsImage::planes` / `DdsImage::surfaces`.
//! * **BC1..BC5 + BC7 decompression** to RGBA8 / R8 / RG8 via
//!   [`decode_bc1`], [`decode_bc2`], [`decode_bc3`],
//!   [`decode_bc4_unorm`], [`decode_bc4_snorm`], [`decode_bc5_unorm`],
//!   [`decode_bc5_snorm`], and [`decode_bc7`].
//! * **BC6H decompression** (modes 1 + 11) to RGBA half-float via
//!   [`decode_bc6h`]. Modes 1 and 11 are the two 10-bit anchor modes
//!   (single-subset no-delta + 2-subset 5-bit-delta) that encoders
//!   most often emit. The remaining 12 BC6H modes return
//!   `DdsError::Unsupported` and are followups.
//! * **BC1 + BC2 + BC3 + BC4 + BC5 encoders** via
//!   [`encode_bc1`], [`encode_bc2`], [`encode_bc3`],
//!   [`encode_bc4_unorm`], [`encode_bc5_unorm`] — RGBA8 / R8 / RG8 in,
//!   block bytes out, furthest-point endpoint heuristic, bit-exact
//!   roundtrip on solid blocks.
//! * **Mipmap chain + cubemap faces + DX10 texture arrays** — every
//!   on-disk surface is parsed into [`DdsImage::surfaces`] in
//!   Microsoft's mandated order (array slice → face → mip).
//! * **Full DXGI format table** — every value Microsoft assigns
//!   (1..=132) is enumerated in [`DxgiFormat`] for lossless
//!   round-trip; consumers can drop unsupported variants without
//!   losing the original integer code.
//! * **`.dds` still-image container demuxer + muxer** (round-3 lift
//!   over the round-2 extension-only registration). The framework
//!   `ContainerRegistry` now carries probe + demuxer + muxer entries
//!   for `.dds` so CLI tools can read / write DDS files end-to-end
//!   through the pipeline.
//!
//! Still deferred (followups):
//!
//! * BC6H modes other than 1 and 11 — the 12 delta-encoded variants
//!   need a per-mode bit-allocation table that's audit-heavy to
//!   transcribe. Modes recognised but not implemented return
//!   `DdsError::Unsupported`.
//! * BC6H + BC7 encoders — round 4 ships decoders only for the HDR
//!   and high-quality-LDR formats.
//! * Mipmap-chain emission from the encoder (still single-level).
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in
//! `oxideav-core` and exposes the framework `Decoder` / `Encoder`
//! trait surface plus a [`registry::register`] entry point. Disable
//! the feature (`default-features = false`) for an
//! `oxideav-core`-free build that still exposes the standalone
//! [`parse_dds`] / [`encode_dds_uncompressed`] API plus crate-local
//! [`DdsImage`] / [`DdsPixelFormat`] / [`DdsError`] types built only
//! on `std`.
//!
//! ## Clean-room provenance
//!
//! Every byte of the parser was written from Microsoft's public DDS
//! programming-guide pages on learn.microsoft.com (the "DDS file
//! layout for textures", "DDS pixel format", and "Programming guide
//! for DDS" articles plus the public DXGI format reference). No
//! DirectXTex, D3DX, NVTT, squish, or other DDS-handling source code
//! was consulted, paraphrased, or cross-referenced. Binaries (`magick`,
//! `texconv`) are used only as black-box validators when generating
//! test fixtures, not as a source of constants or layout.

pub mod bc6h;
pub mod bc7;
pub mod bcn;
pub mod bcn_enc;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod encoder;
pub mod error;
pub mod image;
pub mod types;

#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for DDS image frames.
pub const CODEC_ID_STR: &str = "dds";

pub use bc6h::decode_bc6h;
pub use bc7::decode_bc7;
pub use bcn::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_unorm, decode_bc5_snorm,
    decode_bc5_unorm,
};
pub use bcn_enc::{encode_bc1, encode_bc2, encode_bc3, encode_bc4_unorm, encode_bc5_unorm};
pub use decoder::parse_dds;
pub use encoder::encode_dds_uncompressed;
pub use error::{DdsError, Result};
pub use image::{CubemapFace, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
pub use types::{
    DdsHeader, DdsHeaderDxt10, DdsPixelFormatHeader, DxgiFormat, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};
