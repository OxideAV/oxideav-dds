# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **BC7 2-subset modes (round 4)** — the encoder now also tries modes
  1 (6-bit RGB + shared p-bits, opaque), 3 (7-bit RGB + per-endpoint
  p-bits, opaque) and 7 (5-bit RGBA + per-endpoint p-bits, translucent)
  per block, sweeping the full 64-entry Microsoft / Khronos partition
  table with two iterations of least-squares endpoint refinement per
  candidate. The block-level encoder picks the candidate with lowest
  SSE. Lifts multi-axis natural-image PSNR-RGB from the ~22 dB
  single-subset mode-6 ceiling to ~28 dB on 3-axis content and ≥30 dB
  on rank-2 (two-region) content. Mode 6 remains the always-tried
  baseline.
- **BC* mip chain emission** via new public entry point
  `encode_dds_block_compressed`. The caller supplies a `DdsImage` with
  a block-compressed `pixel_format` and `surfaces` holding pre-encoded
  per-mip block bytes (one entry per mip level in declaration order).
  The encoder writes a legacy FourCC header for BC1..BC5 and a DX10
  extension header for BC6H + BC7 (or for any format when
  `image.has_dxt10_header == true`), then concatenates the per-mip
  block streams. Cubemap / DX10-array variants remain rejected for
  this round.
- **BC6H mode-10 encoder** via new public entry points `encode_bc6h`
  and `encode_bc6h_from_f32`. Compresses an RGBA half-float (or f32-
  RGB) surface to BC6H mode 10 (1-subset, 10.10.10 absolute endpoint
  precision per channel, 4-bit indices) — the simplest 1-subset BC6H
  layout, no delta-encoding overflow risk. Furthest-point endpoint
  search in f32-RGB space; nearest-palette index quantisation;
  Microsoft's `(31/64)` finalise step matches the decoder pipeline so
  the round-trip is bit-accurate against the decoder. Solid blocks
  round-trip; grayscale HDR gradients ≥30 dB PSNR (peak 1.0).
- **BC7 mode-6 encoder** via new public entry point `encode_bc7`.
  Compresses an RGBA8 surface to BC7 mode 6 (1-subset, 7-bit RGB +
  7-bit alpha + 2 per-endpoint p-bits + 4-bit indices) — the
  canonical opaque-and-translucent BC7 layout used by virtually
  every modern texture-compression pipeline for general RGBA
  content. Furthest-point endpoint search; per-endpoint p-bit
  selection by majority-LSB vote; nearest-palette index
  quantisation; anchor swap to keep pixel 0's index in the low half.
  Solid blocks round-trip with up to 1-bit LSB error per channel
  (intrinsic to mode 6's shared-per-endpoint p-bit); grayscale
  gradients ≥30 dB PSNR-RGB.
- **Mipmap-chain emission** in `encode_dds_uncompressed`. When
  `DdsImage::mip_map_count > 1` the encoder now emits a full mipmap
  chain. Pre-supplied surfaces (`image.surfaces` carrying the right
  count of levels in mip order) are written verbatim; otherwise the
  encoder fabricates each level beyond mip 0 with a 2×2 box-filter
  downsample. Each level halves dimensions floored to 1 per
  Microsoft's mip-dimension rule.

- **BC6H decompression — all 14 modes**. Every BC6H mode (0..13) now
  decodes to RGBA half-float via `decode_bc6h`. Round-1 had only
  modes 1 and 11 (the 10-bit anchors); round-2 transcribes the
  per-mode bit-allocation tables for the remaining 12 modes
  (0, 2..10, 12, 13) — the 7-bit / 9-bit / asymmetric-delta variants
  plus the 16.4 ONE-subset mode — from the public Intel Open Source
  PRM Vol. 5 (BC6H section, 0BSD-licensed) and Microsoft's public
  "BC6H Format" reference. Reserved 5-bit prefixes (10011, 10111,
  11011, 11111) decode to zero RGB per spec without producing an
  error. The four `delta`-encoded ONE-subset modes (10, 11, 12, 13)
  use the `w + x` transform-inversion rule with prec-width wrap;
  unquantize / interpolate / finalise pipeline matches Microsoft's
  bit-accurate reference pseudocode. Full pipeline supports both
  `BC6H_UF16` (unsigned) and `BC6H_SF16` (signed) finalisation.
- **BC7 decompression** to RGBA8 via new public entry point
  `decode_bc7`. Covers all 8 modes (single-, dual- and three-subset
  partitions; 2/3/4-bit primary indices + optional 2/3-bit secondary
  alpha index plane; channel rotation in modes 4 and 5; per-endpoint
  and shared-per-subset p-bits). Partition tables for the 64 two-
  subset and 64 three-subset patterns plus the per-partition anchor
  index tables are clean-room transcribed from the public Khronos
  Data Format specification (the same numeric tables Microsoft
  mandates Direct3D 11 hardware to use); no DirectXTex / NVTT /
  bc7enc / ISPC / basisu source was consulted.
- **BC1 (DXT1) encoder** via new public entry point `encode_bc1`.
  Compresses an RGBA8 surface into 8-byte / 4×4-block BC1 with a
  furthest-point endpoint heuristic (no PCA, no cluster fit, no
  endpoint refinement). Supports the 4-colour layout (opaque) and
  the 3-colour-plus-transparent layout (1-bit punchthrough alpha,
  enabled per-call). Bit-exact roundtrip on solid blocks; "good
  enough" approximation on photographic content.
- **`.dds` still-image container demuxer + muxer**. Round-3 lift
  over the round-2 extension-only registration: the framework-side
  `ContainerRegistry` now installs probe + demuxer + muxer +
  extension entries via `register_containers`, so CLI tools (like
  `cli-convert`) can open / write `.dds` files end-to-end without
  touching the codec API directly. Probe scores `MAX_PROBE_SCORE`
  (100) on the `"DDS "` magic.
- **BC1..BC5 decompression** to RGBA8 / R8 / RG8 via new public
  entry points `decode_bc1`, `decode_bc2`, `decode_bc3`,
  `decode_bc4_unorm`, `decode_bc4_snorm`, `decode_bc5_unorm`,
  `decode_bc5_snorm`. Implementations follow Microsoft's public
  "BC1, BC2 and BC3" / "BC4" / "BC5" pages on learn.microsoft.com;
  no DirectXTex / NVTT / libsquish source consulted. Cross-validated
  against ImageMagick 7.1.2's DXT1 decoder via baked-in fixture
  files under `tests/fixtures/`.
- **Mipmap chain + cubemap face + DX10 texture array surface
  exposure.** `DdsImage` now carries a `surfaces: Vec<DdsSurface>`
  field that holds every (array_slice, face, mip_level) triple in
  the on-disk order Microsoft mandates (outer slice → middle face →
  inner mip). Each `DdsSurface` is tagged with its own
  `(width, height, mip_level, array_slice, face)` so callers can
  pick the level they want. Legacy callers still see
  `planes[0]` mirroring `surfaces[0].plane.data`.
- `CubemapFace` enum (`PositiveX..NegativeZ`) with a `::ALL`
  constant for the standard PX/NX/PY/NY/PZ/NZ ordering.
- `DdsSurface` struct exposing one (face, slice, mip) entry from
  the new `DdsImage::surfaces` field.
- `DdsImage::is_cubemap` and `DdsImage::array_size` fields.
- Per-face cubemap presence-bit constants
  (`DDSCAPS2_CUBEMAP_POSITIVEX`, ..., `DDSCAPS2_CUBEMAP_NEGATIVEZ`).
- **Full DXGI format table.** `DxgiFormat` now enumerates every
  value Microsoft assigns under `DXGI_FORMAT` (1..=132), covering
  HDR floats (R32G32B32A32_FLOAT, BC6H_UF16/SF16), integer formats
  (R8_UINT/SINT, R16_UINT, ...), depth/stencil (D32_FLOAT,
  D24_UNORM_S8_UINT, ...), YUV planar (NV12, P010, YUY2, ...), and
  the typeless variants (`Bc1Typeless`, `R8G8B8A8Typeless`, ...).
  Round-trip through `DxgiFormat::from_u32` ↔ `to_u32` is lossless;
  formats without a layout this crate can interpret produce
  `DdsError::Unsupported` rather than `Unknown`.
- `register_containers(&mut ContainerRegistry)` now installs the
  full demuxer + muxer + probe + extension surface for the `.dds`
  still-image container (round-3 lift over round-2's extension-only
  entry).

## [0.0.2](https://github.com/OxideAV/oxideav-dds/compare/v0.0.1...v0.0.2) - 2026-05-05

### Other

- replace manual div_ceil with .div_ceil() (clippy 1.95)

## [0.0.1] - 2026-05-04

### Added

- Initial round-1 reader / writer for Microsoft DirectDraw Surface
  (DDS) textures.
- `parse_dds(&[u8]) -> Result<DdsImage, DdsError>` parses the magic +
  `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10` (20 bytes) and
  hands the mip-0 surface back as a single `DdsPlane`.
- `encode_dds_uncompressed(&DdsImage) -> Result<Vec<u8>>` round-trips
  every legacy uncompressed pixel format the parser recognises:
  A8R8G8B8, X8R8G8B8, A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5,
  A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
- Block-compressed pass-through. The reader recognises BC1 / BC2 / BC3
  (the classic DXT1 / DXT3 / DXT5), BC4 unorm + snorm (`BC4U` /
  `ATI1` / `BC4S`), BC5 unorm + snorm (`BC5U` / `ATI2` / `BC5S`),
  BC6H (UF16 + SF16), and BC7 (UNORM + SRGB) from either the legacy
  four-cc or the DX10 `dxgi_format`. The raw block bytes are exposed
  through `DdsImage::planes` but not decompressed in round 1 — that's
  round 2.
- Default-on `registry` Cargo feature gates the `oxideav-core`
  dependency, the `Decoder` / `Encoder` trait implementations, and
  the `register` / `register_codecs` entry points. Image-library
  consumers can depend on `oxideav-dds` with `default-features = false`
  and skip the `oxideav-core` dep tree entirely; the standalone path
  exposes `parse_dds` / `encode_dds_uncompressed` plus crate-local
  `DdsImage` / `DdsPixelFormat` / `DdsError` types built only on
  `std`.
- Inline `ci-standalone` CI job verifies `cargo build --lib
  --no-default-features` and `cargo test --no-default-features` stay
  green on every change.
- Hard-asserted self-roundtrip test for every uncompressed format,
  plus pass-through tests for every BC* family member from both the
  legacy four-cc and the DX10 `dxgi_format` paths.
