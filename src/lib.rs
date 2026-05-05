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
//! Round 1 covers:
//!
//! * Header parsing — magic + `DDS_HEADER` (124 bytes) + optional
//!   `DDS_HEADER_DXT10` (20 bytes).
//! * Uncompressed pixel formats with bit-exact round-trip through
//!   [`parse_dds`] + [`encode_dds_uncompressed`]: A8R8G8B8, X8R8G8B8,
//!   A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5, A1R5G5B5, A4R4G4B4,
//!   R8G8B8, A8L8, L8, A8.
//! * Block-compressed pass-through — recognises BC1 / BC2 / BC3 (the
//!   classic DXT1 / DXT3 / DXT5), BC4 unorm + snorm (`BC4U` /
//!   `ATI1` / `BC4S`), BC5 unorm + snorm (`BC5U` / `ATI2` / `BC5S`),
//!   BC6H (UF16 + SF16), and BC7 (UNORM + SRGB) from either the
//!   legacy four-cc or the DX10 `dxgi_format`. The raw block bytes are
//!   exposed through [`DdsImage::planes`] but not decoded into RGB(A)
//!   in round 1 — that's round 2, and the BC6H/BC7 decoders are
//!   substantial enough to deserve their own commits.
//!
//! Out of scope for round 1 (planned for round 2):
//!
//! * BC1..BC7 decompression.
//! * Mipmap-chain extraction (the parser surfaces only mip-0; it
//!   reads `mip_map_count` from the header but does not return the
//!   higher levels yet).
//! * Cubemap face surfaces and DX10 texture arrays.
//! * The full DXGI format table — round 1 enumerates only the BC*
//!   family plus the few uncompressed RGBA / luminance formats it
//!   needs to reconstruct from a DX10 header.
//! * The `.dds` still-image container demuxer / muxer (probe by
//!   magic, expose as a single-frame stream) — once landed it will
//!   plug into the framework via `register_containers`.
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

pub mod decoder;
pub mod encoder;
pub mod error;
pub mod image;
pub mod types;

#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for DDS image frames.
pub const CODEC_ID_STR: &str = "dds";

pub use decoder::parse_dds;
pub use encoder::encode_dds_uncompressed;
pub use error::{DdsError, Result};
pub use image::{DdsImage, DdsPixelFormat, DdsPlane};
pub use types::{
    DdsHeader, DdsHeaderDxt10, DdsPixelFormatHeader, DxgiFormat, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
};

#[cfg(feature = "registry")]
pub use registry::{register, register_codecs, register_containers};
