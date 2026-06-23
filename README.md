# oxideav-dds

Pure-Rust reader / writer for Microsoft's DirectDraw Surface (DDS) texture
container, the format Direct3D games ship their baked block-compressed art
in. Part of the [oxideav workspace][oxideav-workspace] family of
single-format codec crates.

[oxideav-workspace]: https://github.com/OxideAV/oxideav-workspace

## Capabilities

**Container.** `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10`
(20 bytes) parser and writer. Every on-disk surface is parsed into
`DdsImage::surfaces` in the mandated order (array slice → face → mip),
tagged with `mip_level` / `array_slice` / `face`; mipmap chains, cubemap
faces, DX10 texture arrays, and 3D (volume) textures are all surfaced.
A framework-side `ContainerRegistry` probe + demuxer + muxer is
installed via `register_containers`, so CLI tools can open / write `.dds`
files without touching the codec API directly.

**Uncompressed surfaces.** Bit-exact round-trip of the common layouts —
A8R8G8B8, X8R8G8B8, A8B8G8R8, X8B8G8R8, R5G6B5, A1R5G5B5, X1R5G5B5,
A4R4G4B4, X4R4G4B4, R8G8B8, A8L8, L16, A4L4, L8, A8 — every legacy
`DDS_PIXELFORMAT` mask layout Microsoft tabulates in the "Common DDS
File Resource Formats" table. High-bit-depth and floating-point layouts (16-bit-per-channel
UNORM / SNORM, half-float and `f32` variants) are recognised, sized, and
exposed via `decode_float_surface` / `decode_rgba16_unorm_surface` /
`decode_rgba16_snorm_surface`. Packed HDR layouts decode to interleaved
`f32` / integers: `R11G11B10_FLOAT` (`decode_r11g11b10_float_surface`),
`R9G9B9E5_SHAREDEXP` (`decode_r9g9b9e5_sharedexp_surface`),
`R10G10B10A2_UNORM` (`decode_r10g10b10a2_unorm_surface`), and
`R10G10B10A2_UINT` (`decode_r10g10b10a2_uint_surface`). The two
horizontally sub-sampled packed RGB layouts `R8G8_B8G8_UNORM`
(`decode_r8g8_b8g8_unorm_surface`) and `G8R8_G8B8_UNORM`
(`decode_g8r8_g8b8_unorm_surface`) — one 32-bit block per adjacent
pixel pair, red/blue shared and green sampled per pixel — expand to
interleaved RGBA8 (alpha `0xff`); both require an even width. The 16-bit
plain-integer layouts `R16_UINT` / `R16G16_UINT` / `R16G16B16A16_UINT`
and their signed siblings `R16_SINT` / `R16G16_SINT` /
`R16G16B16A16_SINT` — one, two or four tightly-packed little-endian
16-bit channels per pixel, no normalisation — yield the stored words as
interleaved `u16` / `i16` via `decode_uint16_surface` /
`decode_sint16_surface`. The 8-bit plain-integer layouts `R8_UINT` /
`R8G8_UINT` / `R8G8B8A8_UINT` and their signed siblings (`_SINT`) decode
to interleaved `u8` / `i8` via `decode_uint8_surface` /
`decode_sint8_surface`; the 32-bit plain-integer layouts `R32_UINT` /
`R32G32_UINT` / `R32G32B32_UINT` (96-bit, three-channel) /
`R32G32B32A32_UINT` and their `_SINT` siblings decode to interleaved
`u32` / `i32` via `decode_uint32_surface` / `decode_sint32_surface` —
again no normalisation, the stored words are the values. The
normalised single- / dual-channel layouts `R8_UNORM` / `R16_UNORM` /
`R16G16_UNORM` and the signed `R8_SNORM` / `R8G8_SNORM` /
`R8G8B8A8_SNORM` / `R16_SNORM` / `R16G16_SNORM` — the integer ranges a
shader reads as floats — expand to interleaved `f32` via
`decode_unorm_surface` (`[0, 1]`, divide by `2^bits − 1`) /
`decode_snorm_surface` (`[-1, 1]`, divide by `2^(bits−1) − 1` with the
documented min / second-min clamp to `-1.0`). `R8G8_SNORM` /
`R16G16_SNORM` are the classic tangent-space normal-map encodings.

**Depth / depth-stencil decode.** The four depth `DXGI_FORMAT` layouts
whose bit packing Microsoft fully documents decode to depth (and where
present stencil) values: `D16_UNORM` (`decode_depth_d16_surface` → `f32`
depth, `÷ (2^16 − 1)` onto `[0, 1]`), `D32_FLOAT`
(`decode_depth_d32_surface` → `f32` depth, verbatim),
`D24_UNORM_S8_UINT` (`decode_depth_d24s8_surface` → `DepthStencil`:
24-bit depth `÷ (2^24 − 1)` plus a `u8` stencil index) and
`D32_FLOAT_S8X24_UINT` (`decode_depth_d32s8_surface` → `DepthStencil`:
verbatim `f32` depth plus a `u8` stencil, the upper 24 bits of the
second 32-bit word ignored). The typeless views over the same memory
(`R24G8_TYPELESS`, `R32G8X24_TYPELESS`) are recognised at parse time and
route to the corresponding depth-stencil variant. No depth-range
remapping is applied — that is a viewport transform, not part of the
surface encoding. Depth surfaces are decode-only.

**Block-compressed decode.**

- `decode_bc1`..`decode_bc5` + `decode_bc7` expand to RGBA8 / R8 / RG8.
  BC7 covers all 8 modes.
- `decode_bc6h` decodes all 14 BC6H modes to RGBA half-float, for both
  `BC6H_UF16` (unsigned) and `BC6H_SF16` (signed).
- Raw BC1..BC7 block bytes are always available verbatim through
  `DdsImage::surfaces[i].plane.data` for callers that want to keep the
  texture compressed.

**YUV (video) decode.** The eleven luma/chroma `DXGI_FORMAT` values
Microsoft fully specifies in the DXGI enumeration page — the 4:4:4
packed `AYUV` / `Y410` / `Y416`, the 4:2:2 packed `YUY2` / `Y210` /
`Y216`, the 4:2:0 planar `NV12` / `P010` / `P016` / `420_OPAQUE`, and
the 4:1:1 planar `NV11` — are parsed (sized + carried verbatim) and
decoded to interleaved full-resolution `[Y, U, V, A]` samples via
`decode_ayuv_surface` / `decode_y410_surface` / `decode_y416_surface` /
`decode_yuy2_surface` / `decode_y210_surface` / `decode_y216_surface` /
`decode_nv12_surface` / `decode_p010_surface` / `decode_p016_surface` /
`decode_420_opaque_surface` / `decode_nv11_surface` (`u8` for the 8-bit
formats, `u16` for the 10/16-bit ones). Chroma is replicated across the
subsampled neighbourhood; opaque formats decode alpha to the channel
maximum. A `YuvFormat` descriptor exposes per-format `sampling`,
`stored_bits`, `has_alpha`, exact `surface_size_bytes`, and the
documented width/height divisibility constraints (enforced at parse
time). Decode is matrix-agnostic — no YUV→RGB conversion, since the
colour matrix is not part of the DDS container spec — mirroring how the
HDR formats decode to stored channel values. YUV is decode-only.

**ASTC LDR decode.** `decode_astc_ldr` / `decode_astc_ldr_block` /
`decode_astc_ldr_surface` decode the `DXGI_FORMAT_ASTC_*` surfaces
(codes 133..=187) to RGBA8. The LDR-Profile decoder covers all 14 2D
block footprints (4×4 … 12×12), BISE trit/quint/bit integer-sequence
unpacking, the LDR colour endpoint modes (0/1/4/5/6/8/9/10/12/13),
weight unquantization + bilinear infill, multi-partition pattern
generation, dual-plane mode, and void-extent constant-colour blocks.
HDR endpoints and illegal blocks decode to the spec error colour
(opaque magenta). Sourced from the Khronos Data Format Specification
1.4 chapter 23. ASTC is decode-only (no encoder).

**Block-compressed encode.**

- `encode_bc1`..`encode_bc5` emit valid block-compressed surfaces from
  RGBA8 / R8 / RG8 (furthest-point endpoint heuristic; no PCA / RDO).
  Bit-exact on solid blocks; 8-value interpolated alpha for BC3/4/5.
- `encode_bc7` sweeps all 8 modes (single-, dual- and three-subset
  partitions, p-bits, channel rotation).
- `encode_bc6h` / `encode_bc6h_sf16` sweep every BC6H mode per block
  (1-subset absolute + delta modes, 2-subset partitions) for both
  unsigned and signed formats.

**Mipmap emission.** `encode_dds_uncompressed` emits a full mipmap chain
(caller-supplied surfaces verbatim, otherwise box-filter downsampled);
`encode_dds_block_compressed` writes pre-encoded per-mip block bytes;
`encode_dds_volume` round-trips an uncompressed volume.

**Format table.** Every `DXGI_FORMAT` value Microsoft assigns (1..=132)
plus the Windows 8.1-era ASTC range (133..=187) is enumerated by name in
`DxgiFormat` for lossless round-trip; the plain
8/16/32-bit integer colour formats (`R8`/`R8G8`/`R8G8B8A8`,
`R16`/`R16G16`/`R16G16B16A16`, `R32`/`R32G32`/`R32G32B32`/`R32G32B32A32`,
each in `_UINT` and `_SINT`) are sized and decoded, the eleven
documented YUV (video) formats (`AYUV` / `Y410` / `Y416` / `YUY2` /
`Y210` / `Y216` / `NV12` / `P010` / `P016` / `420_OPAQUE` / `NV11`) are
sized and decoded to interleaved `[Y, U, V, A]` samples, the four
documented depth / depth-stencil formats (`D16_UNORM` / `D32_FLOAT` /
`D24_UNORM_S8_UINT` / `D32_FLOAT_S8X24_UINT`, plus the `R24G8` /
`R32G8X24` typeless views) are sized and decoded to depth (and stencil)
values, while the colour `_TYPELESS` views whose runtime interpretation
is ambiguous, the three under-documented video formats (`P208` /
`V208` / `V408`), and palette formats are recognised but return
`DdsError::Unsupported` from the layout resolver.

## Robustness

- A 40-case injection-robustness suite (`tests/injection_robustness.rs`)
  mutates one header field at a time and asserts `parse_dds` returns
  `Err` rather than panicking. Surface-size and block-grid arithmetic
  uses `checked_` / `saturating_` multiplication throughout.
- Eight `cargo-fuzz` panic-free targets under `fuzz/` (`parse_dds`,
  `decode_bcn`, `decode_bc6h`, `decode_bc7`, `decode_astc`,
  `decode_yuv`, `decode_depth`, `roundtrip`), driven daily by
  `.github/workflows/fuzz.yml`. The ASTC
  block + surface decoders are additionally exercised by
  `tests/astc_robustness.rs` (a 70k random-block sweep over every
  footprint plus an exhaustive 2^11 block-mode-field sweep).
- Criterion benchmarks under `benches/` (`decode`, `encode`,
  `roundtrip`); run with
  `cargo bench -p oxideav-dds --bench {decode,encode,roundtrip}`.

## Quickstart

```rust
use oxideav_dds::{parse_dds, encode_dds_uncompressed, DdsImage, DdsPixelFormat, DdsPlane};

// Parse a DDS file.
let bytes: Vec<u8> = std::fs::read("input.dds").unwrap();
let img = parse_dds(&bytes).unwrap();
println!("{}x{} {}", img.width, img.height, img.pixel_format.name());

// Build + write a 4x3 A8R8G8B8 surface.
let data = vec![0u8; 4 * 3 * 4];
let img = DdsImage {
    width: 4,
    height: 3,
    pixel_format: DdsPixelFormat::A8R8G8B8,
    planes: vec![DdsPlane { stride: 4 * 4, data }],
    pts: None,
    mip_map_count: 1,
    has_dxt10_header: false,
    dxgi_format: None,
};
let out: Vec<u8> = encode_dds_uncompressed(&img).unwrap();
std::fs::write("output.dds", out).unwrap();
```

For block-compressed input, `parse_dds` returns an image whose
`pixel_format` is a `Bc*` variant and whose `surfaces[i].plane.data`
holds the raw 4x4-block bytes; call the matching `decode_bc*` helper to
expand it. To encode an RGBA8 surface to BC1:

```rust
use oxideav_dds::encode_bc1;

let rgba: Vec<u8> = vec![0xff; 16 * 16 * 4];
let mut bc1 = vec![0u8; (16 / 4) * (16 / 4) * 8];
encode_bc1(&rgba, 16, 16, /* punchthrough_alpha = */ false, &mut bc1).unwrap();
```

For mipmapped or cubemap textures iterate `img.surfaces` directly; each
entry carries its own `mip_level`, `array_slice`, `face`, and `(width,
height)`.

## Clean-room provenance

Every byte of the parser was written from Microsoft's public DDS
programming-guide pages on [learn.microsoft.com][ms-dds-pguide] (the
"DDS file layout for textures", "DDS pixel format", and "Programming
guide for DDS" articles plus the public DXGI format reference). Binaries
(`magick`, `texconv`) are used only as black-box validators when
generating test fixtures, never as a source of constants or layout.

[ms-dds-pguide]: https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide

## Cargo features

| Feature    | Default | Effect                                                                                                       |
|------------|---------|------------------------------------------------------------------------------------------------------------|
| `registry` | yes     | Pulls in `oxideav-core`, exposes the `Decoder` / `Encoder` trait surface, registers the codec via `register`. Disable (`default-features = false`) to drop the `oxideav-core` dependency tree; the standalone `parse_dds` / `encode_*` / `decode_*` API plus crate-local types stay available on `std`. |

## License

MIT — see [LICENSE](LICENSE).
