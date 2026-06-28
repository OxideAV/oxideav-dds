# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Signed-output `BC4_SNORM` / `BC5_SNORM` decoders (round 379).**
  `decode_bc4_snorm_i8` and `decode_bc5_snorm_i8` return a true `Vec<i8>`
  (one / two interleaved signed channels per texel) rather than the
  `i8`-reinterpreted-`u8` written by the existing `decode_bc4_snorm` /
  `decode_bc5_snorm`, so signed displacement / signed-distance-field and
  tangent-space normal-map content recovers its `[-127, 127]` channels
  directly. The two APIs agree byte-for-byte under reinterpretation.

- **Legacy `D3DFMT_A8R3G3B2` packed 3:3:2 + alpha layout (round 379).**
  The 16-bit-per-pixel layout from the "Common DDS File Resource Formats"
  table whose low byte packs 3:3:2 RGB (red bits 5..=7, green 2..=4, blue
  0..=1) and high byte an 8-bit alpha. New `DdsPixelFormat::A8R3G3B2`
  variant resolves from its masks, the legacy encoder writes them
  (byte-exact round-trip), and `decode_a8r3g3b2_surface` widens it to
  RGBA8 with the standard bit-replication rule (an all-ones colour field
  maps to `0xff`).

- **Legacy ASCII-FourCC packed RGB / 4:2:2 layouts (round 379).** Four
  more "Common DDS File Resource Formats" entries that older `.dds`
  writers carry under an ASCII FourCC tag now resolve at parse time:
  `RGBG` (`D3DFMT_R8G8_B8G8`) and `GRGB` (`D3DFMT_G8R8_G8B8`) route to
  the existing `R8G8_B8G8_UNORM` / `G8R8_G8B8_UNORM` byte layouts;
  `YUY2` (`D3DFMT_YUY2`) routes to the existing 4:2:2 packed decoder; and
  `UYVY` (`D3DFMT_UYVY`) — the byte-swizzled `[U, Y0, V, Y1]` sibling of
  YUY2, with no DX10 `DXGI_FORMAT` — is a new `YuvFormat` variant decoded
  by `decode_uyvy_surface` to interleaved `[Y, U, V, A]` samples (chroma
  replicated across the pixel pair, alpha `0xff`). New
  `tests/legacy_fourcc_formats.rs` covers all four.

- **Legacy `D3DFMT_G16R16` + `D3DFMT_A2R10G10B10` mask layouts (round
  379).** Two more entries from Microsoft's "Common DDS File Resource
  Formats" table now resolve from their legacy `DDS_PIXELFORMAT` masks.
  `G16R16` (a 16:16 two-channel layout, red in the low 16 bits, green in
  the high 16 bits) routes to the existing `R16G16_UNORM` byte layout
  under both the DDS_RGB and DDS_RGBA flag flavours the table lists.
  `A2R10G10B10` — the BGR-ordered sibling of the already-supported
  `A2B10G10R10` (red in the most-significant 10 colour bits, blue in the
  least, alpha in the top two bits) — is a new `DdsPixelFormat` variant
  with no DX10 `DXGI_FORMAT` counterpart; `decode_a2r10g10b10_surface`
  expands its packed words to interleaved `[R, G, B, A]` `u16` samples
  and the legacy encoder writes its masks, so a hand-built image
  round-trips byte-for-byte through `parse_dds`.

- **`encode_round375` fuzz target (round 375).** A tenth `cargo-fuzz`
  panic-free target feeds every parser-accepted `DdsImage` through
  whichever round-375 encoder its shape matches
  (`encode_dds_uncompressed_dx10`,
  `encode_dds_volume_block_compressed`,
  `encode_dds_uncompressed_cubemap_array`) and re-parses any successful
  output, so unchecked arithmetic, slice bounds, or a non-round-trippable
  header in the new writers surfaces as a finding.

- **ASTC LDR three-subset (three-partition) encode (round 375).**
  `encode_astc_ldr_block` now also evaluates three-subset candidates for
  opaque-alpha blocks. The single-CEM colour-value cap is 18 integers
  (§23.11 of the Khronos Data Format Specification 1.4), so a
  three-partition block can only use CEM 8 (LDR RGB direct, 6 values per
  partition → 18 total); CEM 12 (24 values) overflows it. The encoder
  therefore restricts the three-subset path to blocks whose alpha is
  uniformly opaque, routes each texel to its partition via the decoder's
  own `select_partition` over a fixed seed set, fits each partition with
  its own RGB endpoint line, and keeps the result only when it decodes
  strictly closer than the best single-/two-subset/dual-plane candidate.
  This lets a block with three distinct opaque colour regions — which two
  endpoint lines cannot separate — reconstruct from three independent
  lines. Three new tests cover non-regression on a three-region block,
  panic-free encode across all 14 footprints, and the solid-block fast
  path.

- **Uncompressed cubemap / texture-array encode (round 375).**
  `encode_dds_uncompressed_cubemap_array` writes a cubemap or DX10
  texture array from a pre-populated `DdsImage::surfaces` list (slice →
  face → mip on-disk order). A legacy-mask single cubemap uses the legacy
  header with all six `DDSCAPS2_CUBEMAP_*` face-presence bits (every face
  present since Direct3D 9, per the cubic-environment-map layout page);
  any texture array, cube array, or DX10-only format uses a
  `DDS_HEADER_DXT10` extension with the `DDS_RESOURCE_MISC_TEXTURECUBE`
  misc flag (cubemaps) and the `array_size` element count. The legacy
  32-bit RGBA / 16-bit packed / L8 / A8 mask formats gain a DXGI-code
  fallback so they can be carried in the array path. Both shapes
  round-trip byte-for-byte through `parse_dds`. Seven new tests cover
  single-mip / mipmapped legacy cubemaps, DX10 texture arrays, DX10-only
  cubemaps, cube arrays, and the plain-2D / block-compressed rejection
  paths.

- **DX10-header uncompressed encode (round 375).**
  `encode_dds_uncompressed_dx10` serialises the uncompressed formats that
  have no legacy `DDS_PIXELFORMAT` mask layout: the high-bit-depth
  16-bit-per-channel UNORM/SNORM, the half-float and `f32` families, the
  packed `R10G10B10A2_UNORM` / `_UINT` and sub-sampled
  `R8G8_B8G8` / `G8R8_G8B8`, the plain-integer 8/16/32-bit `_UINT`/`_SINT`
  families, the normalised single-/dual-channel 8/16-bit `_UNORM`/`_SNORM`
  layouts, and the four depth / depth-stencil surfaces. Each stores its
  little-endian channels verbatim on disk, so the encoder copies the
  plane bytes unchanged under a `DDS_HEADER_DXT10` extension carrying the
  matching `DXGI_FORMAT` code — the byte stream round-trips through
  `parse_dds` back to the same `DdsPixelFormat`. A box-filter mip chain is
  fabricated unless a canonical `image.surfaces` chain is supplied (use
  the latter for the >8-bit / packed channels, where byte-domain
  filtering is approximate). Legacy-mask, block-compressed, ASTC, YUV,
  cubemap and array inputs are rejected with a clear error. Ten new
  round-trip tests cover every supported format plus the rejection and
  `dxgi_format`-override paths.

- **Block-compressed volume (3D) encode (round 375).**
  `encode_dds_volume_block_compressed` writes a BC1..BC7 volume texture to
  a `.dds` file: each depth slice is an independent 4×4-block surface, the
  slices laid out in the same mip-major / depth-major order Microsoft
  mandates for uncompressed volumes (per the "DDS file layout for volume
  textures" page) but storing `ceil(w/4) × ceil(h/4) × block_bytes`
  compressed bytes per slice. The file always uses a `DDS_HEADER_DXT10`
  extension with `resource_dimension == DDS_DIMENSION_TEXTURE3D` and
  `array_size == 1` (Microsoft requires `arraySize == 1` for a 3D
  texture), plus the legacy `DDSD_DEPTH` flag, `header.depth` slice count,
  and `DDSCAPS2_VOLUME` so a legacy reader still recognises the shape. The
  per-mip depth-halving rule (`max(1, depth >> mip)` slices at each level)
  matches the uncompressed `encode_dds_volume`. Six new round-trip tests
  cover single-mip, mip-chain depth-halving, non-power-of-two
  width/height, and the uncompressed-format / `depth == 1` rejection
  paths. Decode already round-trips BC volumes via `parse_dds`.

- **ASTC LDR dual-plane encode (round 372).** When a block's alpha varies
  independently of its RGB (the single-plane fit forces one shared weight
  per texel onto both), the encoder now also tries a dual-plane block:
  CEM 12 (RGBA direct) with CCS = 3, RGB driven by weight plane 0 and
  alpha by weight plane 1, each fitted on its own interpolation axis. The
  candidate joins the single-/two-subset error-driven selection, so it is
  kept only when it decodes closer to the source. Two new tests cover the
  win on an RGB-vs-alpha cross-gradient and dual-plane decodability.

### Changed

- **ASTC encoder block-mode lookup cached (round 372).** `find_block_mode`
  previously re-scanned all 2048 block-mode fields through
  `decode_block_mode` on every query (and the encoder queries it many
  times per block). The single-plane normal modes are now derived once
  into a `OnceLock` table and looked up, cutting ~18 % off the
  `encode_astc` benchmark (128×128 4×4: 14.7 ms → 12.1 ms) with identical
  output — the table is still built from the decoder, so encode and
  decode stay in lockstep.

### Added

- **ASTC encode fuzz target + benchmark (round 372).** A ninth
  `cargo-fuzz` target `encode_astc` drives arbitrary RGBA8 texels /
  surfaces through `encode_astc_ldr_block` / `encode_astc_ldr`,
  re-decodes the output, and asserts the byte counts round-trip — the
  encoder must be panic-free and only ever emit blocks it can itself
  decode. A 30-second local run logged 22k+ executions with no crash. A
  paired `encode_astc` criterion scenario (128×128 → ASTC 4×4) is added
  to `benches/encode.rs`.

- **ASTC LDR two-subset (partition) encode (round 372).** The block
  encoder now also tries two-partition blocks: it routes the texels
  through the decoder's own `select_partition` for a handful of fixed
  seeds, fits each partition with its own min/max endpoint line (single
  shared CEM, colour stream at bit 29), and keeps whichever block — the
  single-subset fit or the best two-subset candidate — decodes closest to
  the source (measured against this crate's decoder, so the choice is
  exact). Non-collinear blocks that a single endpoint pair can't
  represent (e.g. a red region beside a blue one) drop > 30 % in
  round-trip error. Two new unit tests assert the two-subset win on an
  adversarial split block and that the two-subset path is always
  decodable.

- **End-to-end ASTC `.dds` emitter — `encode_dds_astc` (round 372).** A
  new top-level writer takes an RGBA8 surface plus a
  `DdsPixelFormat::Astc { block_w, block_h, srgb }` and emits a complete
  DX10-header `.dds` file: the matching `DXGI_FORMAT_ASTC_*` code (via
  the new `DxgiFormat::astc_unorm` footprint → UNORM/UNORM_SRGB helper),
  a fabricated box-filter mipmap chain when `mip_map_count > 1`, and the
  per-level ASTC blocks from `encode_astc_ldr`. The file round-trips
  through `parse_dds` + `decode_astc_ldr_surface`: solid surfaces
  byte-exact, collinear gradients within tolerance. Five new
  `tests/astc_encode.rs` end-to-end tests cover the 14-footprint solid
  round-trip, the luma-ramp tolerance, the DX10 header + mip-chain shape,
  the sRGB code path, and bad-input rejection.

- **ASTC LDR encoder — `src/astc.rs` (round 372).** The decoder gains a
  matching single-partition, single-plane LDR encoder:
  `encode_astc_ldr_block` (one 128-bit block from `block_w × block_h`
  RGBA8 texels), `encode_astc_ldr` (a tiled surface at any of the 14 2D
  footprints, 4×4 … 12×12) and `encode_astc_ldr_surface` (footprint
  pulled from a `DxgiFormat::Astc` value). Constant-colour blocks emit a
  **void-extent** block and round-trip byte-exact at every footprint;
  other blocks use colour endpoint mode 8 (LDR RGB direct) when alpha is
  uniformly opaque, else mode 12 (LDR RGBA direct), with per-channel
  min/max endpoints and a per-texel weight chosen by projecting onto the
  endpoint axis. The weight grid is the footprint itself for ≤ 36-texel
  blocks (an exact 1:1 mapping with no bilinear-infill loss) and a
  sub-sampled grid for the larger footprints. Block-mode fields, colour
  values and weights are all derived by **inverting the decoder's own**
  `decode_block_mode` / `unquant_color` / `unquant_weight` model, so the
  encoder and decoder can never disagree about a block's meaning — the
  encoder consults nothing but this crate's existing decode code. The
  production ISE / block-builder helpers were promoted out of the
  `#[cfg(test)]` module so both halves share one packing implementation.
  Sourced from the Khronos Data Format Specification 1.4, chapter 23
  (the same source the decoder was written from). Round-trip is exact for
  solid blocks, within a small tolerance for collinear (luminance)
  gradients, and decodable (no error-colour texels) for arbitrary input.
  Single-subset only — non-collinear 2D detail is approximated, since one
  endpoint pair reconstructs texels along a single RGB line. Eight new
  `astc` unit tests cover void-extent exactness, two-colour and alpha
  round-trips, the luma ramp tolerance, large-footprint decodability, and
  a per-footprint random-block panic sweep.

### Fixed

- **`encode_dds_uncompressed` / `encode_dds_volume` no longer panic on a
  format with no flat bytes-per-pixel.** Both emitters guarded against
  ASTC and block-compressed inputs but then `expect`-ed
  `DdsPixelFormat::bytes_per_pixel()` to be `Some`, which panicked for
  YUV (planar / packed / sub-sampled) surfaces that report `None`. They
  now return `DdsError::Unsupported` instead. Caught by the `roundtrip`
  cargo-fuzz target.

### Added

- **Plain colour `_TYPELESS` DXGI formats now sized + carried verbatim
  (round 367).** Eight `_TYPELESS` colour formats that previously
  resolved to `DdsError::Unsupported` are now resolved by `parse_dds`:
  `R16_TYPELESS` (53), `R16G16_TYPELESS` (33), `R16G16B16A16_TYPELESS`
  (9), `R32_TYPELESS` (39), `R32G32_TYPELESS` (15), `R32G32B32_TYPELESS`
  (5), `R32G32B32A32_TYPELESS` (1) and `R10G10B10A2_TYPELESS` (23). A
  typeless surface stores the exact same per-channel bytes as its typed
  siblings but assigns no runtime interpretation, so each routes to its
  byte-identical `_UINT` variant — whose decoder returns the stored words
  uninterpreted, exactly matching the "uninterpreted bytes" semantics of
  a typeless surface (the same byte-pass-through convention the already-
  routed `R8_TYPELESS` → L8, `R8G8_TYPELESS` → A8L8, `R8G8B8A8_TYPELESS`
  and `B8G8R8A8/X8_TYPELESS` views follow). `parse_dds` sizes them at the
  documented 2 / 4 / 8 / 12 / 16 bytes per pixel and carries the bytes
  verbatim instead of rejecting the file; the caller expands them with
  the matching `decode_uint16_surface` / `decode_uint32_surface` /
  `decode_r10g10b10a2_uint_surface` helper. The per-channel bit counts
  come from Microsoft's public `DXGI_FORMAT` enumeration page. Nine new
  end-to-end `tests/hdr_surfaces.rs` tests (one per format plus a 2×2
  sizing check) confirm the resolved variant, the surface byte size, and
  byte-exact decode.

- **Single-aspect depth/stencil view-format decode — `src/depth.rs`
  (round 367).** Four more `DXGI_FORMAT` values that previously resolved
  to `DdsError::Unsupported` now decode: the depth-only / stencil-only
  "view" formats over the same memory the combined depth-stencil
  surfaces occupy. `R24_UNORM_X8_TYPELESS` (value 46) and
  `X24_TYPELESS_G8_UINT` (value 47) are the depth and stencil aspects of
  `D24_UNORM_S8_UINT` memory (one 32-bit word per texel); `R32_FLOAT_X8X24_TYPELESS`
  (value 21) and `X32_TYPELESS_G8X24_UINT` (value 22) are the depth and
  stencil aspects of `D32_FLOAT_S8X24_UINT` memory (two 32-bit words per
  texel). New `decode_depth_r24_unorm_x8_surface` / `decode_depth_r32_float_x8x24_surface`
  expand the depth aspect to a flat `Vec<f32>` (the D24 view normalises
  `÷ (2^24 − 1)` onto `[0, 1]`, the D32 view returns the verbatim `f32`),
  ignoring the typeless other-aspect bits per Microsoft's documented "N
  bits unused" wording; `decode_depth_x24_g8_uint_surface` /
  `decode_depth_x32_g8x24_uint_surface` expand the stencil aspect to a
  flat `Vec<u8>`, masking off the typeless padding. `parse_dds` resolves
  all four from the `DDS_HEADER_DXT10` `dxgi_format`, sizing them at the
  same 4 / 8 bytes per texel as the combined surfaces they view. Four new
  `DdsPixelFormat` variants carry them with their bits-/bytes-per-pixel
  and name entries. The bit fields and aspect semantics come from
  Microsoft's public `DXGI_FORMAT` enumeration page. These views are
  decode-only (no encoder). Twelve new tests: eight `depth`-module unit
  tests (each view extracting its aspect while ignoring the other,
  combined-vs-view agreement for both pairs, short-buffer rejection) plus
  four `tests/depth_surfaces.rs` end-to-end DX10-parse tests and one
  64-bit-view sizing test. The `decode_depth` cargo-fuzz target now
  drives all eight depth decoders (its format selector widened from 4 to
  8) so the new views are exercised panic-free on arbitrary input.

- **Depth / depth-stencil surface decode — `src/depth.rs` (round 363).**
  New `depth` module decoding the four depth `DXGI_FORMAT` values whose
  byte packing Microsoft fully specifies and that previously had no
  `DdsPixelFormat` mapping: `D16_UNORM` (value 55), `D32_FLOAT` (40),
  `D24_UNORM_S8_UINT` (45) and `D32_FLOAT_S8X24_UINT` (20). `parse_dds`
  now recognises them from the DX10 `dxgi_format` (the typeless views
  `R24G8_TYPELESS` / `R32G8X24_TYPELESS` over the same memory route to
  the typed depth-stencil variant), sizes the surfaces, and carries the
  bytes verbatim. `decode_depth_d16_surface` / `decode_depth_d32_surface`
  expand the single-component depths to a flat `Vec<f32>` (the UNORM
  depth normalised `÷ (2^16 − 1)` onto `[0, 1]`, the float depth
  verbatim); `decode_depth_d24s8_surface` / `decode_depth_d32s8_surface`
  expand the combined depth+stencil layouts to `Vec<DepthStencil>`
  (24-bit depth `÷ (2^24 − 1)` + 8-bit stencil for D24S8; verbatim `f32`
  depth + 8-bit stencil for D32S8, the upper 24 bits of the second word
  ignored). The bit fields and packing all come from Microsoft's public
  `DXGI_FORMAT` enumeration page. Depth surfaces are decode-only (no
  encoder). All four decoders use `checked_` arithmetic and return
  `DdsError::InvalidData` rather than panicking on a short buffer.

- **YUV (video) DXGI surface decode — `src/yuv.rs` (round 354).** New
  `yuv` module decoding the eleven well-documented YUV `DXGI_FORMAT`
  values that previously resolved to `DdsError::Unsupported`: the 4:4:4
  packed `AYUV` / `Y410` / `Y416`, the 4:2:2 packed `YUY2` / `Y210` /
  `Y216`, the 4:2:0 planar `NV12` / `P010` / `P016` / `420_OPAQUE`, and
  the 4:1:1 planar `NV11`. Each surface decoder (`decode_ayuv_surface`,
  `decode_yuy2_surface`, `decode_nv12_surface`, `decode_nv11_surface`,
  `decode_y410_surface`, `decode_y416_surface`, `decode_y210_surface`,
  `decode_y216_surface`, `decode_p010_surface`, `decode_p016_surface`,
  `decode_420_opaque_surface`) expands its packed or planar layout to
  interleaved full-resolution `[Y, U, V, A]` samples (`u8` for the 8-bit
  formats, `u16` for the 10/16-bit formats), replicating chroma across
  the subsampled neighbourhood and forcing opaque alpha where the format
  carries none. A `YuvFormat` descriptor exposes per-format `sampling`,
  `stored_bits`, `has_alpha`, exact `surface_size_bytes`, and the
  width/height divisibility constraints; the channel mappings, plane
  structure, subsampling, byte sizing and dimension rules all come from
  Microsoft's public `DXGI_FORMAT` enumeration page. Decode is
  matrix-agnostic (no YUV→RGB conversion — the colour matrix is not part
  of the DDS container spec), mirroring how `hdr` returns stored channel
  values. The decoders are recognised end-to-end by `parse_dds`, which
  now sizes and carries these surfaces' raw bytes instead of rejecting
  the file.

- **`decode_yuv` fuzz target (round 354).** A seventh `cargo-fuzz`
  panic-free target (`fuzz/fuzz_targets/decode_yuv.rs`) drives arbitrary
  bytes through every YUV surface decoder across all eleven formats and
  both the natural and a zero-padded payload, asserting the success path
  returns exactly `width × height × 4` samples and the short path returns
  `Err` without panicking.

- **ASTC LDR multi-partition + dual-plane decode tests (round 348).**
  Two hand-built block tests now exercise the harder decode paths
  end-to-end against spec-constructed inputs: `two_partition_routes_texels_by_pattern`
  builds a two-partition block (selector-00 single-CEM path, `hash52`
  partition pattern) and confirms every texel decodes to its partition's
  colour with the two partitions distinct; `dual_plane_alpha_uses_second_plane`
  builds a dual-plane CEM-12 block with CCS = 3 and confirms RGB tracks
  weight plane 0 while alpha tracks plane 1 independently. Both drive the
  quint/3-bit colour ISE range via a new in-test BISE encoder
  (`encode_ise_into` + exhaustive-search `pack_trits`/`pack_quints`
  inverses of the decoder's unpackers).

### Fixed

- **ASTC LDR high-precision weight ranges + dual-plane colour budget
  (round 348).** Three `P = 1` rows of the ASTC weight-range table
  (Khronos DFS 1.4 Table 23.9, §23.10) decoded with the wrong number of
  trailing bits: ρ = 011 (0..11) used 1 bit instead of 2, ρ = 101
  (0..19) used a 1-bit quint instead of 2-bit, and ρ = 110 (0..23) used
  2 bits instead of 3 — so every high-precision weight grid produced
  wrong weights and mis-sized the weight ISE. The colour-endpoint ISE
  budget for single-partition **dual-plane** blocks also failed to
  reserve the two colour-component-selector (CCS) bits per §23.21 (Data
  Size Determination), letting `pick_color_range` overshoot by two bits.
  Both are now spec-exact, covered by a new `weight_range_table_matches_spec`
  unit test.

### Added

- **ASTC LDR block decode (round 341).** A from-scratch ASTC LDR-Profile
  decoder (`src/astc.rs`) sourced from the Khronos Data Format
  Specification 1.4 chapter 23. `decode_astc_ldr_block` decodes one
  128-bit block; `decode_astc_ldr` / `decode_astc_ldr_surface` tile the
  blocks across a surface. Covers all 14 LDR 2D block footprints
  (4×4 … 12×12), BISE trit/quint/bit integer-sequence unpacking, the LDR
  colour endpoint modes (0/1/4/5/6/8/9/10/12/13), endpoint + weight
  unquantization, bilinear weight infill, multi-partition pattern
  generation (`hash52`), dual-plane mode, and void-extent constant-colour
  blocks; HDR endpoints and illegal blocks emit the spec error colour
  (opaque magenta). `DxgiFormat` gains the Windows 8.1-era ASTC codes
  (133..=187) as named per-footprint TYPELESS/UNORM/UNORM_SRGB variants
  with an `astc_footprint()` accessor; `DdsPixelFormat` gains a
  footprint-carrying `Astc` variant. The container parser sizes ASTC
  surfaces as `ceil(w/bw) × ceil(h/bh) × 16`-byte blocks across the
  mip / array / cubemap walk. A `decode_astc` cargo-fuzz target plus
  `tests/astc_robustness.rs` (70k random-block sweep + exhaustive 2^11
  block-mode-field sweep) confirm the decoders are panic-free and
  bounds-safe on arbitrary input. ASTC is decode-only this round.

- **Normalised single- / dual-channel 8-bit and 16-bit surface decoders
  (round 336).** New `decode_unorm_surface` / `decode_snorm_surface`
  decode the DX10-header normalised layouts that the legacy verbatim
  integer decoders did not cover, expanding each stored integer to an
  interleaved `f32`. UNORM (`R8_UNORM` value 61, `R16_UNORM` value 56,
  `R16G16_UNORM` value 35) maps `[0, MAX]` onto `[0, 1]` by dividing by
  `2^bits − 1`; SNORM (`R8_SNORM` value 63, `R8G8_SNORM` value 51,
  `R8G8B8A8_SNORM` value 31, `R16_SNORM` value 58, `R16G16_SNORM` value
  37) maps the two's-complement range onto `[-1, 1]` by dividing by
  `2^(bits−1) − 1`, clamped so both the minimum and second-minimum
  encodings give exactly `-1.0` (the DXGI signed-normalised rule). The
  two-channel `R8G8_SNORM` / `R16G16_SNORM` layouts are the classic
  tangent-space normal-map encodings; `R16_UNORM` is a common
  single-channel height map. Eight new `DdsPixelFormat` variants carry
  the formats (sized 1 / 2 / 4 bytes per pixel) and `parse_dds` resolves
  them from the `DDS_HEADER_DXT10` `dxgi_format` (`R8_UNORM` keeps its
  byte-identical `L8` container mapping, and `decode_unorm_surface`
  accepts `L8` too). Thirteen new end-to-end tests assert the resolved
  pixel format, surface sizing, and decoded `f32` values including the
  SNORM clamp endpoints.

- **Legacy `DDS_PIXELFORMAT` mask layouts X8B8G8R8 / X1R5G5B5 / X4R4G4B4
  / L16 / A4L4 (round 331).** Five more uncompressed layouts from
  Microsoft's "Common DDS File Resource Formats" table are now resolved
  by `parse_dds` and written by `encode_dds_uncompressed`: `X8B8G8R8`
  (32 bpp, R `0x000000ff` / G `0x0000ff00` / B `0x00ff0000`, no alpha —
  the RGB sibling of `A8B8G8R8`), `X1R5G5B5` (16 bpp, R `0x7c00` /
  G `0x03e0` / B `0x001f`, top bit unused — the RGB sibling of
  `A1R5G5B5`), `X4R4G4B4` (16 bpp, R `0x0f00` / G `0x00f0` / B `0x000f`,
  top nibble unused — the RGB sibling of `A4R4G4B4`), `L16` (16 bpp
  single-channel luminance, mask `0xffff`), and `A4L4` (8 bpp packed
  4:4 luminance + alpha, L `0x0f` / A `0xf0`). Each is a pass-through
  container layout (raw bytes preserved verbatim in the plane); five new
  `DdsPixelFormat` variants carry them with their bits-per-pixel /
  bytes-per-pixel / name entries, and five new self-roundtrip tests
  (`roundtrip_x8b8g8r8` / `_x1r5g5b5` / `_x4r4g4b4` / `_l16` / `_a4l4`)
  assert encode → parse byte-exactness.

- **8-bit and 32-bit plain-integer surface decoders (round 326).** New
  `decode_uint8_surface` / `decode_sint8_surface` decode the
  tightly-packed 8-bit integer layouts `R8_UINT` / `R8G8_UINT` /
  `R8G8B8A8_UINT` (`DXGI_FORMAT` values 62 / 50 / 30) and their signed
  siblings `R8_SINT` / `R8G8_SINT` / `R8G8B8A8_SINT` (values 64 / 52 / 32)
  into flat interleaved `Vec<u8>` / `Vec<i8>`; new `decode_uint32_surface`
  / `decode_sint32_surface` decode the 32-bit integer layouts `R32_UINT`
  / `R32G32_UINT` / `R32G32B32_UINT` / `R32G32B32A32_UINT` (`DXGI_FORMAT`
  values 42 / 17 / 7 / 3) and their `_SINT` siblings (43 / 18 / 8 / 4)
  into flat interleaved `Vec<u32>` / `Vec<i32>`. All channels are
  little-endian in the named order, returned verbatim with no `[0, 1]` /
  `[-1, 1]` normalisation. Fourteen new `DdsPixelFormat` variants carry
  the formats (sized 1 / 2 / 4 / 8 / 12 / 16 bytes per pixel, including
  the 96-bit three-channel `R32G32B32` family) and `parse_dds` resolves
  them from the `DDS_HEADER_DXT10` `dxgi_format`. The
  `unsupported_dxgi_format_errors` robustness test now points at the
  depth/stencil `D32_FLOAT_S8X24_UINT` layout (still unsupported by
  design) in place of the now-supported `R32G32B32A32_UINT`.

- **16-bit plain-integer surface decoders (round 319).** New
  `decode_uint16_surface` / `decode_sint16_surface` decode the
  tightly-packed 16-bit integer layouts `R16_UINT` / `R16G16_UINT` /
  `R16G16B16A16_UINT` (`DXGI_FORMAT` values 57 / 36 / 12) and their
  signed siblings `R16_SINT` / `R16G16_SINT` / `R16G16B16A16_SINT`
  (values 59 / 38 / 14) into flat, interleaved, row-major `Vec<u16>` /
  `Vec<i16>` — one, two or four little-endian channels per pixel in the
  named order, returned verbatim with no `[0, 1]` / `[-1, 1]`
  normalisation. Six new `DdsPixelFormat` variants carry the formats
  (sized 2 / 4 / 8 bytes per pixel) and `parse_dds` resolves them from
  the `DDS_HEADER_DXT10` `dxgi_format`.

- **Sub-sampled packed RGB decoders `R8G8_B8G8_UNORM` /
  `G8R8_G8B8_UNORM` (round 314).** New `decode_r8g8_b8g8_unorm_surface`
  and `decode_g8r8_g8b8_unorm_surface` expand the two horizontally
  sub-sampled packed RGB layouts (`DXGI_FORMAT` values 68 and 69) into
  interleaved RGBA8. Each 32-bit on-disk block encodes an adjacent pixel
  pair that shares its red and blue bytes but carries an independent
  green byte for each pixel; the two formats differ only in the byte
  order within the block (`[R, G0, B, G1]` for value 68,
  `[G0, R, G1, B]` for value 69). Both require an even width and force
  the decoded alpha to `0xff`. New `DdsPixelFormat::R8G8B8G8Unorm` /
  `G8R8G8B8Unorm` variants carry them, sized at two bytes per pixel, and
  `parse_dds` resolves them from the `DDS_HEADER_DXT10` `dxgi_format`.

## [0.0.5](https://github.com/OxideAV/oxideav-dds/compare/v0.0.4...v0.0.5) - 2026-06-15

### Other

- decode packed R10G10B10A2_UINT surface (round 309)
- decode packed R10G10B10A2_UNORM surface (round 305)
- decode shared-exponent R9G9B9E5_SHAREDEXP HDR surface (round 299)
- decode packed R11G11B10_FLOAT HDR surface (round 293)
- decode extended high-bit-depth / float uncompressed surfaces (round 289)
- remove enumerated-denial prose from src/ module headers
- drop release-plz.toml — use release-plz defaults across the workspace
- scrub pre-existing enumerated-denial prose across 8 src/ files
- unq-space LSQ refinement pass (round 207)

### Added

- **Packed `R10G10B10A2_UINT` surface decoder (round 309).** New
  `decode_r10g10b10a2_uint_surface` widens the `DXGI_FORMAT` value 25
  layout — the integer sibling of value 24 (`R10G10B10A2_UNORM`). It
  shares the exact same little-endian 32-bit word packing (R in bits
  0..=9, G in 10..=19, B in 20..=29, A in 30..=31), but Microsoft's
  `DXGI_FORMAT` reference describes value 25 as a "four-component,
  32-bit unsigned-integer format" rather than "unsigned-normalized-
  integer", so the returned `Vec<u16>` carries the stored integers
  (R / G / B in `0..=1023`, A in `0..=3`) as the values themselves —
  there is no `[0, 1]` normalisation step at all (the caller does not
  divide). The format has no legacy `D3DFMT` four-cc — it is
  DX10-header only — so `parse_dds` resolves it solely from the
  `DDS_HEADER_DXT10` `dxgi_format == 25`, sizing the surface at four
  bytes per pixel. A new `DdsPixelFormat::R10G10B10A2Uint` variant
  carries it; the UNORM and UINT decoders share one private bit-
  extraction helper. Six `hdr` unit tests (channel order / widths,
  all-zero word, all-ones word, UNORM/UINT bit-extraction parity,
  row-major multi-pixel, truncated-input rejection) plus two
  `tests/hdr_surfaces.rs` integration tests (DX10 path, 2×2 sizing).
  Exported at the crate root.
- **Packed `R10G10B10A2_UNORM` surface decoder (round 305).** New
  `decode_r10g10b10a2_unorm_surface` widens the `DXGI_FORMAT` value 24
  layout (legacy `D3DFMT_A2B10G10R10`) — three 10-bit colour channels
  plus one 2-bit alpha channel packed into a single little-endian 32-bit
  word — to an interleaved `Vec<u16>` of `width × height × 4` stored
  samples (R / G / B in `0..=1023`, A in `0..=3`). The bit masks
  (R = `0x000003ff`, G = `0x000ffc00`, B = `0x3ff00000`,
  A = `0xc0000000`) come from the programming guide's pixel-format
  table, fixing R in bits 0..=9, G in bits 10..=19, B in bits 20..=29,
  and A in bits 30..=31. As with the `R16G16B16A16_UNORM` path the
  returned values are the raw stored unsigned-normalised integers; the
  caller divides colour by `1023` and alpha by `3` to normalise onto
  `[0, 1]`. `parse_dds` now resolves the format from both the DX10
  `DDS_HEADER_DXT10` `dxgi_format == 24` and the legacy
  `D3DFMT_A2B10G10R10` DDPF_RGB mask layout, sizing the surface at four
  bytes per pixel. A new `DdsPixelFormat::R10G10B10A2Unorm` variant
  carries it. Five `hdr` unit tests (channel order / widths, all-zero
  word, all-ones word, row-major multi-pixel, truncated-input rejection)
  plus three `tests/hdr_surfaces.rs` integration tests (DX10 path,
  legacy-mask path, 2×2 sizing). Exported at the crate root.
- **Shared-exponent `R9G9B9E5_SHAREDEXP` HDR surface decoder (round
  299).** New `decode_r9g9b9e5_sharedexp_surface` widens the
  `DXGI_FORMAT` value 67 layout — three sign-less channels packed into
  one little-endian 32-bit word that *share* a single 5-bit
  biased-by-15 exponent, each with its own 9-bit mantissa (R in bits
  0..=8, G in 9..=17, B in 18..=26, shared exponent in 27..=31) — to an
  interleaved `Vec<f32>` of `width × height × 3` samples. The format's
  `DXGI_FORMAT` table entry carries footnotes 6 and 7 (no implied
  leading one on the mantissa; denormal support), so each channel
  reconstructs with the single linear expression
  `mantissa × 2^(exp − 15 − 9)` = `mantissa × 2^(exp − 24)`, uniform
  across every exponent — there is no normal / subnormal split and the
  all-zero word decodes to `+0`. Bit packing, the shared-exponent
  semantics and the least-significant-bits component ordering are taken
  from Microsoft's public `DXGI_FORMAT` reference. Exported at the crate
  root. Nine unit tests cover unity per channel, the all-zero word, the
  shared exponent scaling all three channels at once, an exponent bump
  doubling the value, the no-implied-one magnitude, the smallest
  denormal, the maximum value, row-major multi-pixel layout, and the
  truncated-input error.

- **Packed `R11G11B10_FLOAT` HDR surface decoder (round 293).** New
  `decode_r11g11b10_float_surface` widens the `DXGI_FORMAT` value 26
  layout — three sign-less partial-precision floats packed into one
  little-endian 32-bit word (R in bits 0..=10, G in 11..=21, B in
  22..=31; each with a 5-bit biased-by-15 exponent, 6-bit mantissa for
  R / G and 5-bit mantissa for B) — to an interleaved `Vec<f32>` of
  `width × height × 3` samples. Mirrors IEEE-754 half-precision rules:
  denormals are decoded rather than flushed, and the all-ones exponent
  maps to infinity / NaN. Bit packing, exponent bias, per-channel
  mantissa widths and the least-significant-bits component ordering are
  taken from Microsoft's public `DXGI_FORMAT` reference. Exported at
  the crate root. Eight unit tests cover unity per channel, zero,
  channel independence, the narrower B mantissa, a subnormal, inf/NaN,
  row-major multi-pixel layout, and the truncated-input error.

- **Extended high-bit-depth / floating-point uncompressed surfaces
  (round 289).** `parse_dds` now recognises the 16-bit-per-channel and
  32-bit-float uncompressed layouts Microsoft assigns to the legacy
  `D3DFMT` numeric FourCC codes 36 / 110..=116 and to the matching
  `DXGI_FORMAT` values: `R16G16B16A16_UNORM` (FourCC 36 /
  `D3DFMT_A16B16G16R16`), `R16G16B16A16_SNORM` (110 /
  `Q16W16V16U16`), `R16_FLOAT` (111 / `R16F`), `R16G16_FLOAT` (112 /
  `G16R16F`), `R16G16B16A16_FLOAT` (113 / `A16B16G16R16F`),
  `R32_FLOAT` (114 / `R32F`), `R32G32_FLOAT` (115 / `G32R32F`) and
  `R32G32B32A32_FLOAT` (116 / `A32B32G32R32F`). Each is resolved from
  both the legacy numeric FourCC and the DX10 `dxgi_format`, sized
  correctly, and surfaced as raw bytes via `DdsImage::surfaces`. New
  `DdsPixelFormat` variants plus three public decode helpers in the
  `hdr` module: `decode_float_surface` widens the half-float / `f32`
  layouts to an interleaved `Vec<f32>` (the half path reuses the
  crate's IEEE-754 binary16 → `f32` widening, the 32-bit path
  reinterprets the little-endian bytes as binary32), and
  `decode_rgba16_unorm_surface` / `decode_rgba16_snorm_surface`
  expose the stored 16-bit channels (`u16` / `i16`). Channel order,
  bit count, and the FourCC ↔ DXGI ↔ `D3DFMT` correspondence come
  from Microsoft's public DDS / DXGI programming-guide pages. The
  real-range normalisation arithmetic for the UNORM / SNORM pair is
  not stated on those pages, so the scaling step is left to the
  caller. 11 new integration tests in `tests/hdr_surfaces.rs` plus 8
  `hdr`-module unit tests; the pre-existing
  `unsupported_dxgi_format_errors` injection test was retargeted at
  `R32G32B32A32_UINT` (a still-unsupported integer format) since its
  former target is now decoded.
- **BC6H unq-space LSQ refinement pass (round 207).** Closes the only
  remaining "Still deferred" followup the round-77 BC6H multi-mode
  encoder shipped with. After the existing pixel-`half_to_f32`-space
  LSQ pass converges in `encode_mode10` (1-subset, 10.10 absolute) and
  `try_2subset` (modes 0..9 across the 32-entry BC6H partition table),
  a second LSQ pass runs in the 17-bit unq integer space where the
  decoder's `(e0 * (64-w) + e1 * w + 32) >> 6` interpolation is
  *linear*. Pixel-space LSQ over-weights bright-exponent pixels
  proportionally to their `half_to_f32` magnitude; the unq-space LSQ
  weights every pixel uniformly in the lattice the decoder's integer
  arithmetic operates over. Two new helpers underpin the pass:
  `target_unq_uf16(half_bits)` inverts the `finish_uf16` non-linearity
  (`(unq * 31) >> 6` → `unq ≈ (half * 64 + 31) / 31`, clamped to
  `[0, 0xffff]`) to set the per-pixel LSQ target, and
  `unq_to_q_uf16(unq, prec)` inverts `unquantize_uf16` (probe ±2
  around the `((unq << prec) - 0x8000) >> 16` continuous estimate) to
  map the LSQ float endpoint back to the `prec`-bit lattice. Both
  helpers carry round-trip-validation tests. Acceptance is
  SSE-guarded — the unq-space iteration only commits when its re-snap
  improves SSE, mirroring the existing pixel-space pass. A new
  `bc6h_encode_mixed_dynamic_range_unq_lsq` test (4×4 block with R
  sweeping 0.02 → 1.0 against an anti-ramp B) measures the headline
  uplift: 28.00 → 29.75 dB PSNR (+1.75 dB), within the "1-2 dB"
  followup target. All 232 pre-existing tests continue to pass; both
  `default` and `--no-default-features` test sweeps are clean.

## [0.0.4](https://github.com/OxideAV/oxideav-dds/compare/v0.0.3...v0.0.4) - 2026-05-30

### Other

- criterion harnesses for decode + encode + roundtrip (round 192)
- add encode_bc4_snorm + encode_bc5_snorm (round 182)
- saturating block-grid math to clear three fuzz crashes (round 176)
- 40 hard-asserted tests + 4 panic fixes (round 162)
- cargo-fuzz harness with five panic-free targets (round 156)
- decode + encode 3D textures (round 123)
- multi-mode encoder (round 77)

### Added

- **Criterion benchmark harnesses (round 192).** Three new benches
  under `benches/`: `decode`, `encode`, `roundtrip`. Each is
  self-contained — every input surface is synthesised in-bench from
  a deterministic xorshift seed, then fed through the crate's own
  public standalone entry points. Wired into `Cargo.toml` under a
  new `[dev-dependencies] criterion = "0.5"` (pinned to the line
  the other OxideAV crates with benches track) plus three
  `[[bench]] harness = false` declarations. Run with
  `cargo bench -p oxideav-dds --bench {decode,encode,roundtrip}`.
  Scenarios — `decode`: BC1 / BC3 / BC4 / BC5 at 512×512, BC6H /
  BC7 at 256×256 (block-decode hot path on a pre-encoded payload).
  `encode`: BC1 / BC3 / BC4 / BC5 at 256×256, BC6H / BC7 at 128×128
  (the mode-picker sweep is the most expensive crate path so the
  surface is smaller — `sample_size(10)` on the BC6H / BC7 groups).
  `roundtrip`: end-to-end `parse_dds` ↔ `encode_dds_uncompressed`
  on A8R8G8B8 (512×512 single-mip + 256×256 mip-9), R8G8B8A8_UNORM
  via DXT10 extension (128×128) and L8 (64×64) — measures
  container-level header / surface-table walking + DX10-header
  emit cost separately from the per-block BCn hot path. The
  harness is paired with the round-156 fuzz harness (panic-free
  surfaces) and the round-162 / round-176 injection-robustness
  property tests: fuzz fixes broken paths, the robustness suite
  hard-asserts hostile-input rejection, the benches give future
  encoder algorithm rounds (LSQ-in-unq-space, partition-table
  prune, endpoint-search prune) an A/B baseline to land against.

- **`encode_bc4_snorm` + `encode_bc5_snorm` (round 182).** Signed-
  channel encoders mirroring the existing `encode_bc4_unorm` /
  `encode_bc5_unorm` paths. Inputs are treated as `i8` per Microsoft's
  `BC4_SNORM` / `BC5_SNORM` convention and the reserved -128
  codepoint is clamped to -127 so it never appears as an endpoint
  or palette entry (matches the decoder's `clamp(-127, 127)` on the
  palette side). Endpoint selection still uses the
  furthest-point heuristic; the 8-value interpolation mode is
  selected whenever `max > min` (i.e. on every non-degenerate block).
  Encoder uses `i16` arithmetic + `div_euclid` to match the decoder's
  signed-division behaviour on negative palette entries. Ten new
  unit tests in `src/bcn_enc.rs` cover solid-zero, ±127 saturation,
  reserved-`-128` clamping, two-value bit-exact roundtrip, signed
  gradient (max absolute error ≤ 22 over a 16-pixel uniform range),
  endpoint-ordering (`a0 > a1` confirms 8-value mode), BC5 independent-
  channel roundtrip (R varies, G constant), 5×3 non-aligned dimensions
  and short-buffer rejection for both `encode_bc4_snorm` and
  `encode_bc5_snorm`. Closes the encoder gap relative to the existing
  `decode_bc4_snorm` / `decode_bc5_snorm` decoders.

### Fixed

- **Panic-on-overflow regressions in `decode_bc6h` / `decode_bc7` /
  `decode_bc{1..=5}` (round 176).** The daily `cargo-fuzz` workflow
  surfaced three crashes simultaneously on three targets
  (`decode_bcn` / `decode_bc6h` / `decode_bc7`): every BC-block decoder
  computed its required-input length as a `usize × usize × 16`
  product, which trips `panic_const_mul_overflow` when the caller
  supplies `width = height = u32::MAX` (a deliberate adversarial probe
  in each fuzz harness). The same shape was present in the four
  surface-size helpers (`rgba8_surface_bytes` / `rgba_half_surface_bytes`
  / `r8_surface_bytes` / `rg8_surface_bytes`) and the `block_input_bytes`
  helper. All six paths now use `saturating_mul`, so the
  pre-existing `input.len() < want_in` / `output.len() < want_out`
  length checks reject the surface rather than triggering a panic.
  Thirteen regression tests added to `tests/injection_robustness.rs`
  (one `does_not_panic` test per `decode_bc*` entry, plus three
  verbatim-byte reproductions of the fuzz crash artifacts:
  `decode_bc6h/crash-ebc0c3370c…`,
  `decode_bc7/crash-c382ab7c10…`,
  `decode_bcn/crash-3d19281e55…`). The three crash inputs are also
  committed to the corpus directories under
  `fuzz/corpus/decode_*/regression-r176-mul-overflow` so the daily
  workflow re-validates the fix on every run.

### Added

- **Injection-robustness property tests for `parse_dds` + every
  `decode_bc*` entry (round 162).** New `tests/injection_robustness.rs`
  carries 40 hard-asserted cases that build a known-good DDS byte
  stream, mutate a single field (bad magic, bad header size, bad pixel-
  format size, zero width / height, missing required flags, DXT10
  fourCC without extension bytes, unsupported legacy / DXGI format,
  truncated payload, forged `mip_map_count = u32::MAX`, forged
  `array_size = u32::MAX`, forged cubemap × array overflow, forged
  volume `depth = u32::MAX`, volume + cubemap combined,
  `width = height = u32::MAX`, etc.) and assert `parse_dds` returns
  `Err(DdsError::…)` rather than panicking. Also covers
  `decode_bc1` / `decode_bc2` / `decode_bc3` / `decode_bc4_unorm` /
  `decode_bc4_snorm` / `decode_bc5_unorm` / `decode_bc5_snorm` /
  `decode_bc6h` / `decode_bc7` short-input + short-output paths.

### Fixed

- **Panic-on-overflow regressions in `parse_dds` (round 162).** The
  injection tests above caught four real panic paths that a hostile
  DDS file could trigger:
  * `surface_size_bytes` multiplied `width × height × bpp` in `u64`
    without checked arithmetic; `u32::MAX × u32::MAX × 4` overflowed
    `u64` in a debug build. Now uses `checked_mul` and surfaces an
    `InvalidData("uncompressed surface size overflow …")` error.
  * `(width >> m).max(1)` panicked when `m >= 32` (e.g.,
    `mip_map_count = u32::MAX`). The parser now rejects any
    `mip_map_count` greater than `1 + floor(log2(max(width, height)))`
    — the dimension-implied cap. Same check on the volume path with
    depth folded in.
  * `array_size as usize * surfaces_per_slice` could overflow `usize`
    on 64-bit targets when both factors carry attacker-controlled
    `u32::MAX` values. Now uses `checked_mul` and additionally rejects
    a `total_surfaces` above a 1 M hard cap before calling
    `Vec::with_capacity`, so a forged header can never request a
    multi-billion-entry surface vector.
  * `block_compressed_surface_size` is now saturating rather than
    wrapping, mirroring the `surface_size_bytes` change.

- **`cargo-fuzz` harness with five panic-free targets (round 156)** —
  new `fuzz/` directory carrying a sibling `Cargo.toml` and five
  fuzz targets exercising every attacker-controlled entry point:
  - `parse_dds` — full container parse off arbitrary bytes (4-byte
    magic + 124-byte `DDS_HEADER` + optional 20-byte
    `DDS_HEADER_DXT10` + mip / array / face / depth-slice surface
    tail; every length / count / format-code field is fuzzed).
  - `decode_bcn` — every BC1..BC5 entry point
    (`decode_bc1` / `decode_bc2` / `decode_bc3` / `decode_bc4_unorm` /
    `decode_bc4_snorm` / `decode_bc5_unorm` / `decode_bc5_snorm`)
    with fuzzed `(width, height)` + block stream, including an
    adversarial `u32::MAX × u32::MAX` block-grid sweep and a
    zero-length-output sweep.
  - `decode_bc6h` — 14-mode signed + unsigned BC6H decoder with
    the same dimension / buffer-size adversarial sweep.
  - `decode_bc7` — 8-mode BC7 decoder + reserved-mode (eight leading
    zero bits) handling.
  - `roundtrip` — `parse_dds` → `encode_dds_uncompressed` →
    `parse_dds` idempotency on every parser-accepted uncompressed
    single-plane non-cubemap non-volume input.
  Plus a daily `Fuzz` GitHub Actions workflow that runs the org
  reusable `crate-fuzz.yml` (30-minute total budget split across
  the five targets, cron `53 7 * * *`). Corpus seeded with the
  two existing crate fixtures (`grad8.dds`, `red16.dds`) and six
  hand-crafted BC1 / BC3 / BC6H / BC7 single-block blobs. The
  harness is built with `default-features = false` so it
  exercises the framework-free standalone decode path.

- **Volume (3D) texture support (round 123)** — the parser now decodes
  volume textures from both the legacy header (`DDSCAPS2_VOLUME` +
  `DDSD_DEPTH`, with the slice count in `header.depth`) and the DX10
  header (`resource_dimension == DDS_DIMENSION_TEXTURE3D`). Each mip
  level stores `max(1, depth >> mip)` consecutive 2D slices in
  mip-major on-disk order; the depth halves alongside width / height
  per Microsoft's volume mip rule. `DdsImage` gains a `depth` field
  (mip-0 slice count) and `DdsSurface` gains a `depth_slice` field
  (the z index of each emitted surface). A new `encode_dds_volume`
  writer round-trips an uncompressed volume back to disk with the
  matching legacy header. Volume textures are validated to not also be
  cubemaps or texture arrays. Seven new tests cover legacy / DX10
  decode, the depth-halving mip chain, a truncated-payload error, and
  single-mip / mipped round-trips.

- **BC6H_SF16 multi-mode encoder (round 77)** — `encode_bc6h_sf16`
  now sweeps every BC6H mode for signed-format output. Previous
  round-7 dispatch shipped mode 10 only; this round closes the
  gap with:
  - **1-subset signed-delta modes 11/12/13** via
    `encode_mode_delta_1subset_signed`. Each candidate quantises
    pixel endpoints to signed two's-complement integers in
    `prec`-bit space, encodes the second endpoint as a signed
    delta in `delta_bits` two's-complement space, and rejects
    when the per-channel signed delta overflows
    `[-2^(d-1), 2^(d-1) - 1]`.
  - **2-subset signed modes 0..9** via `try_2subset_signed`.
    Same 32-entry BC6H partition sweep as the unsigned 2-subset
    path; per-subset furthest-point seed + 2 LSQ refinement
    passes against the signed unquantize / finish pipeline.
    Cross-subset deltas that exceed `delta_bits` signed range
    cause the candidate to bail.
  - **New helpers**: `furthest_pair_subset_signed`,
    `refine_endpoints_1subset_signed`,
    `refine_endpoints_2subset_signed`,
    `snap_indices_2subset_signed`, `f32_to_signed_q`. All built
    on the existing `quantize_half_sf16` / `unquantize_sf16` /
    `finish_sf16` primitives.
  - **PSNR lift**: signed two-cluster content (left half = -0.4,
    right half = +0.4) reaches ≥30 dB PSNR (peak 1.0) via the
    2-subset signed modes; tight-range signed gradients
    ([-0.05, 0.05]) reach ≥35 dB via the delta modes; sign-
    spanning gradients clear the round-7 mode-10-only 19 dB
    threshold by >3 dB. Pixel-rotated solid negative blocks
    bit-identical to the round-7 mode-10 baseline.

## [0.0.3](https://github.com/OxideAV/oxideav-dds/compare/v0.0.2...v0.0.3) - 2026-05-08

### Other

- add mode 4/5 channel-rotation encoders + BC6H_SF16 (round 7)
- add 2-subset modes 0..9 + delta-encoded 1-subset modes 11/12/13 (round 6)
- add 3-subset modes (0/2) + BC*-from-RGBA8 mip emitter (round 5)
- add 2-subset modes (1/3/7) + BC* mip chain emission (round 4)
- add baseline encoders + mipmap-chain emission (round 3)
- implement all 14 modes (round 2)
- drop stale REGISTRARS / with_all_features intra-doc links
- drop dead `linkme` dep
- re-export __oxideav_entry from registry sub-module
- round 4: BC6H decompression (modes 1+11) + BC2/3/4/5 encoders
- deduplicate rgb565_to_rgb888 + drop hot-path heap allocations
- round 3: BC7 decompression + BC1 encoder + .dds container demuxer/muxer
- round 2: BC1-5 decompression + mipmaps + cubemap faces + texture arrays + full DXGI table
- auto-register via oxideav_core::register! macro (linkme distributed slice)
- unify entry point on register(&mut RuntimeContext) ([#502](https://github.com/OxideAV/oxideav-dds/pull/502))
- add register_containers for .dds extension lookup

### Added

- **BC7 mode 4/5 channel-rotation encoders (round 7)** — the encoder
  now also tries the two 1-subset channel-rotation modes per block,
  sweeping all 4 rotation values × (mode 4: 2 idx_sel choices) × mode 5.
  Mode 4 = 1-subset 5/5/5 RGB + 6-bit alpha + 1-bit `idx_sel` selecting
  whether the 2-bit primary index plane drives RGB or alpha (and the
  3-bit secondary plane drives the other). Mode 5 = 1-subset 7/7/7 RGB
  + 8-bit alpha + 2-bit indices on both planes. The 2-bit `rotation`
  field swaps A with R/G/B post-decode, letting content where one
  channel varies independently from the other three use the higher
  alpha precision. Encoder pre-rotates the input pixels by the chosen
  rotation, fits RGB and alpha endpoints separately by least-squares,
  picks per-plane indices, and packs the bitstream — closing the BC7
  encoder coverage gap (decoder already supported these).
- **BC6H_SF16 (signed half-float) encoder (round 7)** — new
  `encode_bc6h_sf16` and `encode_bc6h_sf16_from_f32` entry points emit
  BC6H blocks for the signed-format DXGI variant (`BC6H_SF16` =
  format-id 96). Signed format preserves negative values (sign bit at
  half-bit position 15), useful for HDR content with negative radiance
  or signed-displacement maps. The encoder mirrors the decoder's
  signed-pipeline math: signed-magnitude quantisation, signed
  unquantize (`((c << 15) + 0x4000) >> (bits-1)` per Microsoft), and
  signed finalize (`(|c| * 31) >> 5` with sign re-attached). Currently
  emits mode 10 (1-subset, 10/10 absolute, 4-bit indices) for SF16;
  multi-mode SF16 (delta-encoded modes 11/12/13 + 2-subset modes 0..9
  signed) is a follow-on. Decoder already supported `signed=true`.
- **BC6H 2-subset modes 0..9 + 1-subset delta modes 11/12/13 (round 6)**
  — the BC6H encoder now sweeps all 14 BC6H modes per block. For
  2-subset modes (0/1/2/3/4/5/6/7/8/9), the encoder iterates over the
  32-entry BC6H partition table, seeds per-subset endpoints with
  furthest-point + iterative LSQ refinement, and rejects partitions
  where any cross-subset delta exceeds the mode's per-channel delta
  width (5–6 bits). For 1-subset delta modes (11/12/13), the encoder
  encodes the second endpoint as a signed delta from the first base
  endpoint and rejects when overflow forces the delta out of the
  per-mode range (9 / 8 / 4 bits respectively). The block-level
  picker selects the lowest-SSE candidate across all modes; this
  closes the BC6H encoder gap and lets the encoder pick higher-
  precision modes (e.g. 11 = 10-bit base + 9-bit delta) for tight
  gradients and lower-precision modes (9 = 6.6.6.6 absolute) for
  cross-subset spreads that exceed the delta range. Round 5 mode
  10 (1-subset, 10.10 absolute) remains the SSE reference baseline.

- **BC7 3-subset modes (round 5)** — the encoder now also tries modes
  0 (3-subset, 4-bit partition, 4-bit RGB + per-endpoint p-bits,
  3-bit indices) and 2 (3-subset, 6-bit partition, 5-bit RGB, no
  p-bits, 2-bit indices) per opaque block, sweeping the 16 / 64-entry
  Microsoft / Khronos 3-subset partition tables with the same
  least-squares refinement loop as the 2-subset modes. Lifts
  rank-3 natural-image PSNR-RGB from the round-4 ~28 dB ceiling to
  ≥30 dB (measured: 30.44 dB on the standard 8×8 three-axis fixture).
- **`encode_dds_block_compressed_from_rgba8`** (round 5) closes the
  BC* mip-chain emission story: takes an RGBA8 source plus
  destination format + dimensions + mip count + cubemap / array_size
  flags and returns a fully-formed DDS file. The encoder generates
  each mip level by 2×2 box-filter downsampling the previous level's
  RGBA8, then encodes that level to BC* blocks. Supports BC1, BC2,
  BC3, BC4_UNORM, BC5_UNORM, BC7_UNORM and BC7_UNORM_SRGB; rejects
  BC6H (HDR — callers must use `encode_bc6h_from_f32` +
  `encode_dds_block_compressed`). Cubemap (`is_cubemap = true`,
  6-face RGBA8 source) and DX10 texture-array (`array_size > 1`,
  N-slice RGBA8 source) shapes are also supported on this path —
  they previously hit the "cubemap / DX10 texture-array
  block-compressed emission is not yet supported" error.

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
