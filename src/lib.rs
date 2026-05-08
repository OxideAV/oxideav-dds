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
//! * **BC6H decompression** — all 14 modes (0..13) — to RGBA half-float
//!   via [`decode_bc6h`]. Reserved 5-bit prefixes (10011, 10111, 11011,
//!   11111) decode to zero RGB per spec. Both `BC6H_UF16` (unsigned)
//!   and `BC6H_SF16` (signed) finalisation paths are supported.
//! * **BC1 + BC2 + BC3 + BC4 + BC5 encoders** via
//!   [`encode_bc1`], [`encode_bc2`], [`encode_bc3`],
//!   [`encode_bc4_unorm`], [`encode_bc5_unorm`] — RGBA8 / R8 / RG8 in,
//!   block bytes out, furthest-point endpoint heuristic, bit-exact
//!   roundtrip on solid blocks.
//! * **BC6H mode-11 encoder** via [`encode_bc6h`] (and the f32-input
//!   convenience [`encode_bc6h_from_f32`]). Round-3 baseline encoder
//!   ships only mode 11 (1-subset, 11.9 endpoint precision, 4-bit
//!   indices) — the highest-precision 1-subset BC6H mode and the
//!   round-trip target for HDR gradient content.
//! * **BC7 mode-6 encoder** via [`encode_bc7`]. Round-3 baseline
//!   encoder ships only mode 6 (1-subset, 7-bit RGB + 7-bit alpha + 2
//!   per-endpoint p-bits + 4-bit indices) — the canonical opaque-and-
//!   translucent BC7 layout used by virtually every modern texture-
//!   compression pipeline for general RGBA content.
//! * **Mipmap chain emission** from [`encode_dds_uncompressed`].
//!   When `image.mip_map_count > 1` the encoder either copies a pre-
//!   computed chain from `image.surfaces` verbatim or fabricates the
//!   levels by box-filter downsampling mip 0.
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
//! * BC6H modes other than mode 11 — the round-3 encoder ships only
//!   the 1-subset 11.9 baseline. Modes 0..10 + 12..13 (2-subset
//!   partitions, asymmetric-precision modes, 16.4 ONE-subset) are
//!   decoded but not encoded.
//! * BC7 modes other than mode 6 — the round-3 encoder ships only
//!   the 1-subset 7777-mode baseline. Modes 0/1/2/3 (2- and 3-subset
//!   partitions for natural-image quality), 4/5 (channel rotation),
//!   and 7 (2-subset opaque-alpha) are decoded but not encoded.
//! * BC*-format mip-chain emission via the dedicated encoders — round
//!   3 lifts mipmap-chain emission for uncompressed surfaces only.
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
pub mod bc6h_enc;
pub mod bc7;
pub mod bc7_enc;
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
pub use bc6h_enc::{encode_bc6h, encode_bc6h_from_f32};
pub use bc7::decode_bc7;
pub use bc7_enc::encode_bc7;
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
pub use registry::{__oxideav_entry, register, register_codecs, register_containers};
