# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.0.5](https://github.com/OxideAV/oxideav-dds/compare/v0.0.4...v0.0.5) - 2026-06-01

### Other

- unq-space LSQ refinement pass (round 207)

### Added

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
