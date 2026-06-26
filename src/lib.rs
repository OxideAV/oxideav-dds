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
//!   [`encode_bc4_unorm`], [`encode_bc4_snorm`], [`encode_bc5_unorm`],
//!   [`encode_bc5_snorm`] — RGBA8 / R8 / RG8 in, block bytes out,
//!   furthest-point endpoint heuristic, bit-exact roundtrip on solid
//!   blocks. The SNORM pair (round 182) treats inputs as i8 and
//!   clamps the reserved -128 codepoint to -127 per Microsoft's
//!   BC4/BC5 spec.
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
//! * **BC6H_SF16 (signed) multi-mode encoder** via
//!   [`encode_bc6h_sf16`] (and the f32-input convenience
//!   [`encode_bc6h_sf16_from_f32`]). Mirrors the decoder's signed-
//!   magnitude pipeline (signed unquantize + signed finalize per
//!   Microsoft) for content with negative radiance or signed-
//!   displacement maps. Round-77 lift over round 7: the picker now
//!   sweeps mode 10 (1-subset signed absolute), modes 11/12/13
//!   (1-subset signed delta), and modes 0..9 (2-subset signed)
//!   across the 32-entry BC6H partition table. Cross-subset signed
//!   deltas that overflow the per-channel delta range bail out so
//!   the picker can fall through to a wider mode.
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
//! * **Extended high-bit-depth / floating-point uncompressed surfaces**
//!   — the 16-bit-per-channel and 32-bit-float layouts Microsoft
//!   assigns to the legacy `D3DFMT` numeric FourCC codes 36 / 110..=116
//!   and to the matching `DXGI_FORMAT` values: `R16G16B16A16_UNORM`,
//!   `R16G16B16A16_SNORM`, `R16_FLOAT`, `R16G16_FLOAT`,
//!   `R16G16B16A16_FLOAT`, `R32_FLOAT`, `R32G32_FLOAT`,
//!   `R32G32B32A32_FLOAT`. [`parse_dds`] now recognises them from both
//!   the numeric FourCC and the DX10 `dxgi_format`, sizes the surfaces
//!   correctly, and surfaces the raw bytes; [`decode_float_surface`]
//!   widens the half-float / `f32` layouts to interleaved `f32`, and
//!   [`decode_rgba16_unorm_surface`] / [`decode_rgba16_snorm_surface`]
//!   expose the stored 16-bit channels (`u16` / `i16`). The packed
//!   `R11G11B10_FLOAT` HDR layout (`DXGI_FORMAT` value 26 — three
//!   sign-less partial-precision floats sharing a 32-bit word) widens
//!   to interleaved `f32` via [`decode_r11g11b10_float_surface`]. The
//!   shared-exponent `R9G9B9E5_SHAREDEXP` HDR layout (`DXGI_FORMAT`
//!   value 67 — three sign-less 9-bit mantissas sharing a single 5-bit
//!   exponent in one 32-bit word) widens to interleaved `f32` via
//!   [`decode_r9g9b9e5_sharedexp_surface`]. The packed
//!   `R10G10B10A2_UNORM` layout (`DXGI_FORMAT` value 24 / legacy
//!   `D3DFMT_A2B10G10R10` — three 10-bit colour channels plus a 2-bit
//!   alpha channel in one 32-bit word, R in the least-significant bits)
//!   yields the stored unsigned-normalised integers via
//!   [`decode_r10g10b10a2_unorm_surface`]. Its integer sibling
//!   `R10G10B10A2_UINT` (`DXGI_FORMAT` value 25 — same packing, plain
//!   unsigned integers, no normalisation, DX10-header only) yields the
//!   stored integers via [`decode_r10g10b10a2_uint_surface`]. The two
//!   horizontally sub-sampled packed RGB layouts `R8G8_B8G8_UNORM`
//!   (`DXGI_FORMAT` value 68) and `G8R8_G8B8_UNORM` (value 69) — one
//!   32-bit block per adjacent pixel pair, the red and blue bytes shared
//!   across the pair and the green byte sampled per pixel — expand to
//!   interleaved RGBA8 (alpha forced to `0xff`) via
//!   [`decode_r8g8_b8g8_unorm_surface`] /
//!   [`decode_g8r8_g8b8_unorm_surface`]; they differ only in the byte
//!   order within each block (`[R, G0, B, G1]` vs `[G0, R, G1, B]`) and
//!   both require an even width. The 16-bit plain-integer layouts
//!   `R16_UINT` / `R16G16_UINT` / `R16G16B16A16_UINT` (`DXGI_FORMAT`
//!   values 57 / 36 / 12) and their signed siblings `R16_SINT` /
//!   `R16G16_SINT` / `R16G16B16A16_SINT` (values 59 / 38 / 14) — one,
//!   two or four tightly-packed little-endian 16-bit channels per pixel,
//!   no normalisation — yield the stored words as interleaved `u16` /
//!   `i16` via [`decode_uint16_surface`] / [`decode_sint16_surface`]. The
//!   8-bit plain-integer layouts `R8_UINT` / `R8G8_UINT` /
//!   `R8G8B8A8_UINT` (`DXGI_FORMAT` values 62 / 50 / 30) and their signed
//!   siblings `R8_SINT` / `R8G8_SINT` / `R8G8B8A8_SINT` (values
//!   64 / 52 / 32) yield interleaved `u8` / `i8` via
//!   [`decode_uint8_surface`] / [`decode_sint8_surface`], and the 32-bit
//!   plain-integer layouts `R32_UINT` / `R32G32_UINT` / `R32G32B32_UINT`
//!   (96-bit, three-channel) / `R32G32B32A32_UINT` (`DXGI_FORMAT` values
//!   42 / 17 / 7 / 3) and their `_SINT` siblings (43 / 18 / 8 / 4) yield
//!   interleaved `u32` / `i32` via [`decode_uint32_surface`] /
//!   [`decode_sint32_surface`] — again no normalisation, the stored words
//!   are the values.
//! * **`.dds` still-image container demuxer + muxer** (round-3 lift
//!   over the round-2 extension-only registration). The framework
//!   `ContainerRegistry` now carries probe + demuxer + muxer entries
//!   for `.dds` so CLI tools can read / write DDS files end-to-end
//!   through the pipeline.
//!
//! Still deferred (followups):
//!
//! * LSQ refinement metric — current pixel-space LSQ is approximate;
//!   fitting in unq-space could push 1-2 dB more on multi-axis HDR
//!   content.
//! * UNORM / SNORM real-range normalisation — the Microsoft DDS / DXGI
//!   programming-guide pages describe `R16G16B16A16_UNORM` /
//!   `_SNORM` as "unsigned-normalised-integer" / "signed-normalised-
//!   integer" but do not state the arithmetic that maps the stored
//!   16-bit integers onto `[0, 1]` / `[-1, 1]`. The crate therefore
//!   decodes these two formats to their raw `u16` / `i16` channels;
//!   the scaling step is left to the caller pending a documentation
//!   addition. The floating-point layouts have no such gap — their
//!   stored bits are the value.
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
//! for DDS" articles plus the public DXGI format reference). Binaries
//! (`magick`, `texconv`) are used only as black-box validators when
//! generating test fixtures, not as a source of constants or layout.

pub mod astc;
pub mod bc6h;
pub mod bc6h_enc;
pub mod bc7;
pub mod bc7_enc;
pub mod bcn;
pub mod bcn_enc;
#[cfg(feature = "registry")]
pub mod container;
pub mod decoder;
pub mod depth;
pub mod encoder;
pub mod error;
pub mod hdr;
pub mod image;
pub mod types;
pub mod yuv;

#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for DDS image frames.
pub const CODEC_ID_STR: &str = "dds";

pub use astc::{
    decode_astc_ldr, decode_astc_ldr_block, decode_astc_ldr_surface, encode_astc_ldr,
    encode_astc_ldr_block, encode_astc_ldr_surface, is_valid_footprint,
    ERROR_COLOR as ASTC_ERROR_COLOR, LDR_BLOCK_FOOTPRINTS,
};
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
pub use bcn_enc::{
    encode_bc1, encode_bc2, encode_bc3, encode_bc4_snorm, encode_bc4_unorm, encode_bc5_snorm,
    encode_bc5_unorm,
};
pub use decoder::parse_dds;
pub use depth::{
    decode_depth_d16_surface, decode_depth_d24s8_surface, decode_depth_d32_surface,
    decode_depth_d32s8_surface, decode_depth_r24_unorm_x8_surface,
    decode_depth_r32_float_x8x24_surface, decode_depth_x24_g8_uint_surface,
    decode_depth_x32_g8x24_uint_surface, DepthStencil,
};
pub use encoder::{
    encode_dds_astc, encode_dds_block_compressed, encode_dds_block_compressed_from_rgba8,
    encode_dds_uncompressed, encode_dds_volume, encode_dds_volume_block_compressed,
};
pub use error::{DdsError, Result};
pub use hdr::{
    decode_float_surface, decode_g8r8_g8b8_unorm_surface, decode_r10g10b10a2_uint_surface,
    decode_r10g10b10a2_unorm_surface, decode_r11g11b10_float_surface,
    decode_r8g8_b8g8_unorm_surface, decode_r9g9b9e5_sharedexp_surface, decode_rgba16_snorm_surface,
    decode_rgba16_unorm_surface, decode_sint16_surface, decode_sint32_surface,
    decode_sint8_surface, decode_snorm_surface, decode_uint16_surface, decode_uint32_surface,
    decode_uint8_surface, decode_unorm_surface,
};
pub use image::{CubemapFace, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
pub use types::{
    DdsHeader, DdsHeaderDxt10, DdsPixelFormatHeader, DxgiFormat, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
};
pub use yuv::{
    decode_420_opaque_surface, decode_ayuv_surface, decode_nv11_surface, decode_nv12_surface,
    decode_p010_surface, decode_p016_surface, decode_y210_surface, decode_y216_surface,
    decode_y410_surface, decode_y416_surface, decode_yuy2_surface, YuvFormat, YuvSampling,
};

#[cfg(feature = "registry")]
pub use registry::{__oxideav_entry, register, register_codecs, register_containers};
