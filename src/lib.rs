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
//! Coverage as of round 5:
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
//! * **BC6H multi-mode encoder** via [`encode_bc6h`] (and the f32-input
//!   convenience [`encode_bc6h_from_f32`]). Round-3 shipped mode 10
//!   (1-subset, 10.10 absolute endpoint precision, 4-bit indices). Round
//!   6 closes the BC6H encoder gap with a partition + mode picker:
//!   * **2-subset modes 0..9** — sweep the 32 BC6H 2-subset partition
//!     table for each candidate mode, seed per-subset endpoints with
//!     furthest-point + iterative LSQ refinement, pick the partition ×
//!     mode tuple with lowest SSE. Modes 0/2/3/4 (10.5 / 11.4-family)
//!     reject blocks where any cross-subset delta exceeds 5 bits;
//!     modes 6/7/8 (8-bit base) accept wider spreads; mode 9 (6.6.6.6
//!     absolute, no delta) is the universal fallback.
//!   * **1-subset delta-encoded modes 11/12/13** — mode 11 (10-bit
//!     base + 9-bit delta) gives one extra base bit over mode 10 when
//!     both endpoints are within ±256 in 10-bit q-space; modes 12 / 13
//!     trade base precision for ever-smaller delta range.
//! * **BC7 multi-mode encoder** via [`encode_bc7`]. Round-3 shipped
//!   mode 6 only (1-subset baseline); round 4 added the three 2-subset
//!   modes (1 / 3 / 7); round 5 added the two 3-subset modes (0 / 2)
//!   for genuine rank-3 colour content; round 7 closes encoder
//!   coverage with the two channel-rotation modes (4 / 5) — 1-subset
//!   modes with separate RGB / alpha index planes that swap A with
//!   one of R/G/B post-decode. Mode 4 uses 5/5/5 RGB + 6-bit alpha +
//!   1-bit `idx_sel` (selects whether the 2-bit primary plane drives
//!   RGB or alpha); mode 5 uses 7/7/7 RGB + 8-bit alpha + 2-bit on
//!   both planes. Encoder pre-rotates the input pixels by the chosen
//!   rotation, fits RGB and alpha endpoints separately by least-
//!   squares, then packs — closing the encoder gap.
//! * **BC6H_SF16 (signed) encoder** via [`encode_bc6h_sf16`] (and the
//!   f32-input convenience [`encode_bc6h_sf16_from_f32`]). Mirrors the
//!   decoder's signed-magnitude pipeline (signed unquantize + signed
//!   finalize per Microsoft) for content with negative radiance or
//!   signed-displacement maps. Currently emits mode 10 (1-subset,
//!   10-bit signed absolute, 4-bit indices); multi-mode SF16 is a
//!   follow-on round.
//! * **Mipmap chain emission** for both uncompressed
//!   ([`encode_dds_uncompressed`]) and block-compressed
//!   ([`encode_dds_block_compressed`]) surfaces. The uncompressed path
//!   either copies a pre-computed chain from `image.surfaces` or
//!   fabricates levels by box-filter downsampling mip 0; the
//!   pre-encoded BC* path takes per-mip block bytes from
//!   `image.surfaces` and concatenates them with a legacy FourCC
//!   header (BC1..BC5) or DX10 extension header (BC6H, BC7).
//!   [`encode_dds_block_compressed_from_rgba8`] (round 5) closes the
//!   mip-chain emission story: it accepts an RGBA8 source, generates
//!   the chain by box-filter downsampling, and encodes each level to
//!   BC* blocks in one call, so callers no longer have to pre-encode
//!   each mip themselves. Cubemap (`is_cubemap`) and DX10 texture-
//!   array (`array_size > 1`) shapes are also supported on this path.
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
//! * BC6H_SF16 multi-mode (delta-encoded modes 11/12/13 signed +
//!   2-subset modes 0..9 signed) — currently [`encode_bc6h_sf16`]
//!   emits mode 10 only.
//! * LSQ refinement metric — current pixel-space LSQ is approximate;
//!   fitting in unq-space could push 1-2 dB more on multi-axis HDR
//!   content.
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
pub use bc6h_enc::{
    encode_bc6h, encode_bc6h_from_f32, encode_bc6h_sf16, encode_bc6h_sf16_from_f32,
};
pub use bc7::decode_bc7;
pub use bc7_enc::encode_bc7;
pub use bcn::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_unorm, decode_bc5_snorm,
    decode_bc5_unorm,
};
pub use bcn_enc::{encode_bc1, encode_bc2, encode_bc3, encode_bc4_unorm, encode_bc5_unorm};
pub use decoder::parse_dds;
pub use encoder::{
    encode_dds_block_compressed, encode_dds_block_compressed_from_rgba8, encode_dds_uncompressed,
};
pub use error::{DdsError, Result};
pub use image::{CubemapFace, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
pub use types::{
    DdsHeader, DdsHeaderDxt10, DdsPixelFormatHeader, DxgiFormat, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
};

#[cfg(feature = "registry")]
pub use registry::{__oxideav_entry, register, register_codecs, register_containers};
