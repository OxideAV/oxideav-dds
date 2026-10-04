# oxideav-dds

[![CI](https://github.com/OxideAV/oxideav-dds/actions/workflows/ci.yml/badge.svg)](https://github.com/OxideAV/oxideav-dds/actions/workflows/ci.yml) [![crates.io](https://img.shields.io/crates/v/oxideav-dds.svg)](https://crates.io/crates/oxideav-dds) [![docs.rs](https://docs.rs/oxideav-dds/badge.svg)](https://docs.rs/oxideav-dds) [![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Pure-Rust decoder / encoder for Microsoft's DirectDraw Surface (DDS)
texture container, the format Direct3D games ship their baked
block-compressed art in. Part of the [oxideav workspace][oxideav-workspace]
family of single-format codec crates.

[oxideav-workspace]: https://github.com/OxideAV/oxideav-workspace

## Standalone use

`oxideav-dds` follows the OxideAV image-crate contract
(`IMAGE_CRATE_API`): the same small root vocabulary every
`oxideav-<format>` image crate exposes, usable with
`default-features = false` and no `oxideav-core`, returning pixels as
plain `Vec<u8>`.

```toml
[dependencies]
oxideav-dds = { version = "0.0", default-features = false }
```

```rust
let bytes = std::fs::read("in.dds")?;
if oxideav_dds::probe(&bytes) {
    let info  = oxideav_dds::info(&bytes)?;         // header only: width, height, format, frames, shape
    let img   = oxideav_dds::decode(&bytes)?;       // DdsImage: the top-level surface, native layout
    let rgba: Vec<u8> = img.to_rgba8();             // tightly packed RGBA, 4 * width bytes per row
    let (w, h) = (img.width(), img.height());

    let opts = oxideav_dds::EncodeOptions::default()
        .with_surface_format(oxideav_dds::SurfaceFormat::Bc7Unorm)
        .with_mip_levels(0);                        // BC7 blocks, full mip chain
    let out: Vec<u8> = oxideav_dds::encode_rgba8(w, h, &rgba, &opts)?;
    std::fs::write("out.dds", out)?;
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

| Item | Signature |
|---|---|
| `probe` | `fn(&[u8]) -> bool` — `DDS ` magic + `dwSize == 124`, allocation-free |
| `info` / `info_with` | `fn(&[u8][, &DecodeOptions]) -> Result<ImageInfo, Error>` — `width`, `height`, `format`, `frames` (every surface), `has_alpha`, `color`, `has_icc` / `has_exif` / `has_xmp` (always `false`), plus `surface_format`, `dxgi_format`, `dx10_header`, `mip_levels`, `cubemap`, `array_size`, `depth` |
| `decode` / `decode_with` | `fn(&[u8][, &DecodeOptions]) -> Result<DdsImage, Error>` — the top-level surface (mip 0, first face / slice) in its native contract layout |
| `decode_rgb8` / `decode_rgba8` | `-> Result<RgbImage / RgbaImage, Error>` — `{ width, height, data }`, tightly packed, 3 / 4 bytes per pixel |
| `decode_all` / `decode_all_with` | `-> Result<Vec<Frame>, Error>` — every surface as `Frame { image, delay: None, mip_level, array_slice, face, depth_slice }` in on-disk order |
| `decode_from` | `fn<R: Read>(R) -> Result<DdsImage, Error>` |
| `encode` | `fn(&DdsImage, &EncodeOptions) -> Result<Vec<u8>, Error>` — natural layout of the image format, or the requested `surface_format` |
| `encode_rgb8` / `encode_rgba8` | `fn(w, h, &[u8], &EncodeOptions)` — `R8G8B8` / `R8G8B8A8_UNORM` |
| `encode_to` | `fn<W: Write>(&DdsImage, &EncodeOptions, W) -> Result<(), Error>` |
| `encode_all` | `fn(&[Frame], &EncodeOptions) -> Result<Vec<u8>, Error>` — mip chains, cubemaps, texture arrays and volumes from the frames' extras (the mirror of `decode_all`) |
| `DdsImage` | `{ width, height, format: PixelFormat, planes: Vec<Plane>, color: ColorInfo, metadata: Metadata }` with `new` / `packed` / `from_rgb8` / `from_rgba8` (all `Result`), `width()` / `height()` / `format()` / `stride()`, `as_bytes()` / `into_raw()`, `to_rgb8()` / `to_rgba8()` (+ `try_*`) — no `palette`: DDS has no palette layout this crate decodes |
| `PixelFormat` | `= DdsPixelFormat`: `Gray8`, `Ya8`, `Gray16Le`, `Rgb24`, `Bgr24`, `Rgba`, `Bgra`, `Rgba64Le`, `GrayF32Le`, `RgbF32Le`, `RgbaF32Le` (names mirror `oxideav_core::PixelFormat`) |
| `Error` | `= DdsError`: `InvalidData`, `Unsupported`, `LimitExceeded`, `Io(std::io::Error)` |

`to_rgba8` is an exact integer kernel per layout: grey replicated, BGR
orders swizzled, 16-bit samples reduced to their high byte, `f32`
samples clamped to `[0, 1]` and scaled by 255 (round half up), alpha
`0xFF` where the layout has none. No gamma or colour management is
applied.

The stored layout (`SurfaceFormat`, ~70 `D3DFMT` / `DXGI_FORMAT`
variants) and the whole mip / face / slice / depth tree stay available
as the depth API: `parse_dds(&bytes) -> DdsFile` (`DdsFile::surfaces`,
`DdsFile::to_image(&surface)`), `write_dds_file(&DdsFile, force_dx10)`,
the `encode_dds_*` writers, and the per-layout codecs (`decode_bc1` …
`decode_bc7`, `decode_bc6h`, `decode_astc_ldr*`, `encode_bc*`,
`encode_bc6h*`, `encode_astc_ldr*`, the `decode_*_surface` helpers).
The pre-contract `DdsPlane` remains for one release as a `#[deprecated]`
alias of `Plane`; `DdsImage` and `DdsPixelFormat` changed meaning (the
old records are `DdsFile` and `SurfaceFormat`) — see the CHANGELOG.

## Framework use

With the default-on `registry` feature the crate plugs into the
`oxideav-core` registry:

```rust
# let img = oxideav_dds::DdsImage::from_rgba8(1, 1, vec![0; 4])?;
# let mut params = oxideav_core::CodecParameters::video(oxideav_core::CodecId::new("dds"));
# params.width = Some(1);
# params.height = Some(1);
# params.pixel_format = Some(oxideav_core::PixelFormat::Rgba);
let mut ctx = oxideav_core::RuntimeContext::new();
oxideav_dds::register(&mut ctx);                       // codec "dds" + the .dds container (probe / demuxer / muxer)
let dec = oxideav_dds::make_decoder(&params)?;         // / make_encoder (options: surface_format, mip_levels, dx10_header)
let frame: oxideav_core::VideoFrame = img.into();      // From<DdsImage>: one packed plane + colour signal when sRGB
let back = oxideav_dds::DdsImage::from_video_frame(&frame, &params)?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The trait-side `Decoder` / `Encoder` are thin adapters over the
standalone functions (one implementation). The framework `Decoder`
emits the **native contract layout** of the top-level surface (`Bgra`
for a `B8G8R8A8` file, `Rgba` for a decoded BC7, `RgbaF32Le` for a
float surface, …), never a pre-converted `Rgba`; the container demuxer
declares that layout on the stream from the header alone. A colour
signal is stamped on the frame only when the file carries one (an
`_UNORM_SRGB` `DXGI_FORMAT`).

## Supported layouts

Decode — stored `SurfaceFormat` → native `PixelFormat` (every
expansion is a byte copy or a lossless bit-replication widening; float
layouts widen `half` / packed floats to `f32`):

| Stored layout | `PixelFormat` | Expansion |
|---|---|---|
| `A8R8G8B8`, `X8R8G8B8` (`B8G8R8A8/X8_UNORM`) | `Bgra` | byte copy; the unused `X` byte reads as `0xFF` |
| `A8B8G8R8`, `X8B8G8R8` (`R8G8B8A8_UNORM`) | `Rgba` | byte copy; `X` → `0xFF` |
| `R8G8B8` | `Bgr24` | byte copy |
| `R5G6B5`, `A1R5G5B5`, `X1R5G5B5`, `A4R4G4B4`, `X4R4G4B4`, `A8R3G3B2`, `A4B4G4R4_UNORM` | `Rgba` | bit replication to 8 bits; 1-bit alpha → `0` / `0xFF` |
| `A8` | `Rgba` | `[0, 0, 0, a]` |
| `L8`, `R8_UNORM`; BC4 (unsigned) | `Gray8` | byte copy; block decode |
| `A8L8`; `A4L4` | `Ya8` | byte copy; nibble replication |
| `L16`, `R16_UNORM` | `Gray16Le` | byte copy |
| `R16G16B16A16_UNORM`; `R16G16_UNORM` | `Rgba64Le` | byte copy; `B = 0`, `A = 0xFFFF` |
| `R10G10B10A2_UNORM`, `A2R10G10B10` | `Rgba64Le` | 10 → 16 and 2 → 16 bits by bit replication |
| `R8G8_B8G8_UNORM`, `G8R8_G8B8_UNORM` | `Rgba` | shared R / B duplicated across the pixel pair |
| BC1, BC2, BC3, BC7 (`_UNORM` / `_UNORM_SRGB`), ASTC LDR (14 footprints) | `Rgba` | block decode |
| BC5 (unsigned) | `Rgba` | `[r, g, 0, 0xFF]` |
| `R16_FLOAT`, `R32_FLOAT` | `GrayF32Le` | `half` → `f32` |
| `R11G11B10_FLOAT`, `R9G9B9E5_SHAREDEXP` | `RgbF32Le` | packed float → `f32` |
| `R16G16B16A16_FLOAT`, `R32G32B32A32_FLOAT`; BC6H (UF16 / SF16) | `RgbaF32Le` | `half` → `f32`; block decode |
| `R16G16_FLOAT`, `R32G32_FLOAT` | `RgbaF32Le` | `[r, g, 0.0, 1.0]` |
| `R8_SNORM`, `R16_SNORM`; BC4 (signed) | `GrayF32Le` | DXGI SNORM rule `v / (2^(n-1) - 1)`, clamped to `-1` |
| `R8G8_SNORM`, `R16G16_SNORM`; BC5 (signed) | `RgbaF32Le` | as above, `[r, g, 0.0, 1.0]` |
| `R8G8B8A8_SNORM`, `R16G16B16A16_SNORM` | `RgbaF32Le` | as above |

Missing colour channels read as `0` and a missing alpha channel as
opaque. The plain-integer `_UINT` / `_SINT` layouts, the depth-stencil
surfaces and their typeless views, and the eleven YUV layouts have no
contract layout: `info` / `decode` return `Error::Unsupported` for them
and `parse_dds` + the matching `decode_*_surface` helper is the way in.

Encode — `EncodeOptions::surface_format` `None` picks the natural
layout of the image:

| `PixelFormat` | Stored layout | Header |
|---|---|---|
| `Gray8` | `L8` | legacy |
| `Ya8` | `A8L8` | legacy |
| `Gray16Le` | `L16` | legacy |
| `Rgb24` (swizzled), `Bgr24` | `R8G8B8` | legacy |
| `Rgba` | `A8B8G8R8` (`R8G8B8A8_UNORM`) | legacy |
| `Bgra` | `A8R8G8B8` (`B8G8R8A8_UNORM`) | legacy |
| `Rgba64Le` | `R16G16B16A16_UNORM` | DX10 |
| `GrayF32Le` | `R32_FLOAT` | DX10 |
| `RgbF32Le` (alpha 1.0 added), `RgbaF32Le` | `R32G32B32A32_FLOAT` | DX10 |

`decode(encode(img)) == img` for every native layout except `Rgb24`
(stored as `R8G8B8`, read back as `Bgr24`) and `RgbF32Le` (read back as
`RgbaF32Le`). With an explicit `surface_format` the image is converted:
every 8-bit colour layout and the BC1 / BC2 / BC3 / BC4 / BC5 / BC7 and
ASTC block encoders take any image through `to_rgba8()`; the 16-bit
layouts take `Gray16Le` / `Rgba64Le` exactly and 8-bit sources by
`× 257`; the float, SNORM and BC6H layouts take float sources exactly and
integer sources normalised to `[0, 1]`; the luminance layouts (`L8`,
`A8L8`, `A4L4`, `L16`, `R8/R16_UNORM`) need a grey image and `A8` an
alpha-carrying one. `Error::Unsupported` is returned for those
mismatches, for `R11G11B10_FLOAT` / `R9G9B9E5_SHAREDEXP` (decode only, no
packer) and for the plain-integer / depth / YUV / typeless layouts;
nothing is converted silently. Re-encoding a decoded image in its own
stored layout is byte-stable for every integer layout.

## Options

`DecodeOptions` (`Default` + `with_*`): `max_width`, `max_height`,
`max_pixels` (top-level geometry), `max_bytes` (stored bytes of every
surface plus the expanded top-level plane; default 1 GiB, `None` lifts
it) — all checked against the header before any surface is copied
(`Error::LimitExceeded`) — and `strict` (default `false`): lenient mode
ignores trailing bytes after the last surface and the
`dwPitchOrLinearSize` field; strict mode rejects trailing bytes and a
non-zero pitch / linear size that disagrees with the computed value.
Zero dimensions, a mip count beyond the geometry, a volume that is also
a cubemap or array, truncated surfaces, and unknown `DXGI_FORMAT` codes
are rejected in both modes.

`EncodeOptions` (`Default` + `with_*`): `surface_format:
Option<SurfaceFormat>` (`None` = natural layout), `mip_levels` (`1` =
none, `0` = full chain to 1×1, `n` = `n` levels; generated by a 2×2 box
filter in the contract layout, per channel in the sample's own domain,
before conversion), `dx10_header` (force the `DDS_HEADER_DXT10`
extension; it is written automatically for layouts and shapes without
a legacy encoding — DX10-only formats, texture arrays, BC6H / BC7 /
ASTC). `encode_all` reads the texture shape off the frames: any
`face` makes a cubemap (all six faces required), `array_slice` values
make a DX10 array, `depth_slice` values at mip 0 make a volume, and
`mip_level` values supply an explicit chain (otherwise `mip_levels`
generates one per face / slice; volume mips halve the depth too).

## Metadata and colour

DDS carries no ICC profile, Exif or XMP, and no gamma: `metadata` is
always empty and `has_icc` / `has_exif` / `has_xmp` are always
`false`. The only colour information in the format is the DX10
`DXGI_FORMAT`: an `_UNORM_SRGB` code (`R8G8B8A8`, `B8G8R8A8/X8`,
BC1 / BC2 / BC3 / BC7, ASTC) decodes to `ColorInfo::srgb()` (full
range, BT.709 primaries, sRGB transfer 13, identity matrix) and is the
only case the registry frame carries a colour signal. Every other file
decodes to `ColorInfo::dds_default()` — full-range RGB, primaries and
transfer unspecified — which is this crate's convention, not a format
rule, and is not stamped on frames. On encode, `color` has no DDS
encoding except through an `_SRGB` `surface_format`.

## Limits

Header geometry is validated before any allocation: `width × height`
and the per-surface byte counts are computed in checked `u64`
arithmetic, mip counts beyond `1 + log2(max dimension)` (depth included
for volumes), more than 2²⁰ surfaces, and a volume combined with a
cubemap / array are `Error::InvalidData`; a surface that does not fit
the file is rejected at the geometry walk, so `info` fails on a
truncated file without reading it. Geometry whose expanded plane
overflows the platform's `usize` is `Error::Unsupported`; a configured
`DecodeOptions` limit is `Error::LimitExceeded`. Hostile input never
panics (see *Robustness*).

## Format specifics — the stored-surface depth API

**Container.** `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10`
(20 bytes) parser and writer. Every on-disk surface is parsed into
`DdsFile::surfaces` in the mandated order (array slice → face → mip),
tagged with `mip_level` / `array_slice` / `face`; mipmap chains, cubemap
faces, DX10 texture arrays, and 3D (volume) textures are all surfaced.
A framework-side `ContainerRegistry` probe + demuxer + muxer is
installed via `register_containers`, so CLI tools can open / write `.dds`
files without touching the codec API directly.

**Uncompressed surfaces.** Bit-exact round-trip of the common layouts —
A8R8G8B8, X8R8G8B8, A8B8G8R8, X8B8G8R8, R5G6B5, A1R5G5B5, X1R5G5B5,
A4R4G4B4, X4R4G4B4, A8R3G3B2, R8G8B8, A8L8, L16, A4L4, L8, A8 — every
legacy `DDS_PIXELFORMAT` mask layout Microsoft tabulates in the "Common
DDS File Resource Formats" table, including the BGR-ordered 10:10:10:2
`A2R10G10B10`, the two-channel `G16R16` (routed to the `R16G16_UNORM`
byte layout), and the packed 3:3:2-plus-alpha `A8R3G3B2`.
`A8R3G3B2` expands to RGBA8 via `decode_a8r3g3b2_surface` (3-bit / 2-bit
channels widened by bit-replication) and `A2R10G10B10` to interleaved
`u16` channels via `decode_a2r10g10b10_surface`; both round-trip
byte-for-byte through `encode_dds_uncompressed`. The DX10-only packed
4:4:4:4 layout `A4B4G4R4_UNORM` (`DXGI_FORMAT` value 191 — alpha in the
low nibble, red in the high, the reverse channel order of the legacy
`A4R4G4B4`) expands to RGBA8 (each nibble widened 4→8 by
bit-replication) via `decode_a4b4g4r4_unorm_surface` and round-trips
verbatim through `encode_dds_uncompressed_dx10`. The legacy ASCII-FourCC
packed layouts `RGBG` / `GRGB` (sub-sampled RGB → `R8G8_B8G8` /
`G8R8_G8B8`) and `YUY2` / `UYVY` (4:2:2 packed luma-chroma) are resolved
from their FourCC tags. High-bit-depth and floating-point layouts (16-bit-per-channel
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
route to the corresponding depth-stencil variant. The four
**single-aspect view** formats that expose only one component over the
same memory — `R24_UNORM_X8_TYPELESS` (depth of D24S8 →
`decode_depth_r24_unorm_x8_surface` `f32`), `X24_TYPELESS_G8_UINT`
(stencil of D24S8 → `decode_depth_x24_g8_uint_surface` `u8`),
`R32_FLOAT_X8X24_TYPELESS` (depth of D32S8X24 →
`decode_depth_r32_float_x8x24_surface` `f32`) and
`X32_TYPELESS_G8X24_UINT` (stencil of D32S8X24 →
`decode_depth_x32_g8x24_uint_surface` `u8`) — decode their aspect and
ignore the typeless other-aspect bits, agreeing byte-for-byte with the
combined decoder over the same surface. No depth-range remapping is
applied — that is a viewport transform, not part of the surface
encoding. Depth surfaces are decode-only.

**Block-compressed decode.**

- `decode_bc1`..`decode_bc5` + `decode_bc7` expand to RGBA8 / R8 / RG8.
  BC7 covers all 8 modes. The signed `BC4_SNORM` / `BC5_SNORM` blocks
  decode to `[-127, 127]` either as `i8`-reinterpreted-`u8` via
  `decode_bc4_snorm` / `decode_bc5_snorm` or as a true `Vec<i8>` via
  `decode_bc4_snorm_i8` / `decode_bc5_snorm_i8`.
- `decode_bc6h` decodes all 14 BC6H modes to RGBA half-float, for both
  `BC6H_UF16` (unsigned) and `BC6H_SF16` (signed).
- Raw BC1..BC7 block bytes are always available verbatim through
  `DdsFile::surfaces[i].plane.data` for callers that want to keep the
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
formats, `u16` for the 10/16-bit ones). The legacy `D3DFMT_UYVY` 4:2:2
layout — the byte-swizzled `[U, Y0, V, Y1]` sibling of `YUY2` carried
under its own FourCC, with no DX10 `DXGI_FORMAT` — decodes the same way
via `decode_uyvy_surface`. Chroma is replicated across the
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
1.4 chapter 23.

**ASTC LDR encode.** `encode_astc_ldr` / `encode_astc_ldr_block` /
`encode_astc_ldr_surface` emit valid `DXGI_FORMAT_ASTC_*` surfaces from
RGBA8 at any of the 14 2D footprints. The encoder is single-partition,
single-plane: a constant-colour block becomes a void-extent block
(byte-exact round-trip), otherwise colour endpoint mode 8 (LDR RGB
direct, opaque alpha) or mode 12 (LDR RGBA direct) carries per-channel
min/max endpoints and each texel picks the weight that best
reconstructs it as a blend of the two endpoints. Footprints with ≤ 36
texels use a 1:1 weight grid (no bilinear-infill loss); larger ones use
a sub-sampled grid. Block-mode, colour and weight quantization are all
derived by inverting the crate's own decode model, so encode and decode
agree by construction. The encoder also tries two-subset
(partition) blocks — it splits the texels via the decoder's own
partition pattern over a few seeds, fits each subset with its own
endpoint line, and keeps whichever block (single- or two-subset) decodes
closest to the source — so a non-collinear block (e.g. two distinct
colour regions) is reconstructed far better than a single endpoint pair
allows. A three-subset (three-partition) candidate is also tried for
opaque-alpha blocks: the single-CEM 18-value colour cap admits only
CEM 8 (RGB direct) at three partitions, so a block with three distinct
opaque colour regions is fitted with three independent endpoint lines
and kept when it decodes closer. When a block's
alpha varies independently of RGB, a dual-plane candidate (CEM 12,
CCS = 3 — RGB on weight plane 0, alpha on plane 1) is also tried and
kept when it decodes closer. Round-trip is exact for solid blocks and
within a documented tolerance for collinear gradients. No HDR encode. `encode_dds_astc` wraps the
encoder in a complete DX10-header `.dds` file (correct
`DXGI_FORMAT_ASTC_*` code, optional fabricated mipmap chain), so an
RGBA8 surface round-trips to disk and back through `parse_dds`.

**Block-compressed encode.**

- `encode_bc1`..`encode_bc5` emit valid block-compressed surfaces from
  RGBA8 / R8 / RG8 (furthest-point endpoint heuristic; no PCA / RDO).
  Bit-exact on solid blocks; 8-value interpolated alpha for BC3/4/5.
- `encode_bc7` sweeps all 8 modes (single-, dual- and three-subset
  partitions, p-bits, channel rotation).
- `encode_bc6h` / `encode_bc6h_sf16` sweep every BC6H mode per block
  (1-subset absolute + delta modes, 2-subset partitions) for both
  unsigned and signed formats.

**Uncompressed encode.** `encode_dds_uncompressed` writes the legacy
`DDS_PIXELFORMAT` mask layouts (A8R8G8B8 … A8, L16, A4L4). The
DX10-only uncompressed formats — high-bit-depth 16-bit-per-channel
UNORM/SNORM, half-float / `f32`, packed `R10G10B10A2_UNORM`/`_UINT`,
sub-sampled `R8G8_B8G8`/`G8R8_G8B8`, plain-integer 8/16/32-bit
`_UINT`/`_SINT`, normalised single-/dual-channel `_UNORM`/`_SNORM`, and
the four depth/depth-stencil surfaces — are written by
`encode_dds_uncompressed_dx10` with a `DDS_HEADER_DXT10` extension
carrying the matching `DXGI_FORMAT`; the plane bytes are stored
verbatim and round-trip byte-for-byte through `parse_dds`.

**Mipmap emission.** `encode_dds_uncompressed` /
`encode_dds_uncompressed_dx10` emit a full mipmap chain (caller-supplied
surfaces verbatim, otherwise box-filter downsampled);
`encode_dds_block_compressed` writes pre-encoded per-mip block bytes;
`encode_dds_volume` round-trips an uncompressed volume and
`encode_dds_volume_block_compressed` a BC1..BC7 volume (3D) texture
(DX10 `TEXTURE3D` header, per-mip depth-halving, non-power-of-two
footprints).

**Cubemap / array emission.**
`encode_dds_uncompressed_cubemap_array` writes an uncompressed cubemap
(legacy header + six face bits, or DX10 `TEXTURECUBE`) or DX10 texture /
cube array from a pre-populated `surfaces` list;
`encode_dds_block_compressed_from_rgba8` covers the block-compressed
cubemap / array path from RGBA8.

**Format table.** Every `DXGI_FORMAT` value Microsoft assigns (1..=132),
the Windows 8.1-era ASTC range (133..=187), and the three top-of-range
codes the DXGI enumeration adds after ASTC (`SAMPLER_FEEDBACK_MIN_MIP_OPAQUE`
189, `SAMPLER_FEEDBACK_MIP_REGION_USED_OPAQUE` 190, `A4B4G4R4_UNORM` 191)
is enumerated by name in `DxgiFormat` for lossless round-trip — the two
sampler-feedback codes are opaque ("TBD" layout, round-trip only) while
`A4B4G4R4_UNORM` decodes. The biased fixed-point `R10G10B10_XR_BIAS_A2_UNORM`
(value 89) shares the 10:10:10:2 packing of `R10G10B10A2_UINT` and is
resolved to its stored fixed-point codes (the documented enumeration
does not give the bias→float constants, so the display-side transform is
left to the caller). The plain
8/16/32-bit integer colour formats (`R8`/`R8G8`/`R8G8B8A8`,
`R16`/`R16G16`/`R16G16B16A16`, `R32`/`R32G32`/`R32G32B32`/`R32G32B32A32`,
each in `_UINT` and `_SINT`) are sized and decoded, the eleven
documented YUV (video) formats (`AYUV` / `Y410` / `Y416` / `YUY2` /
`Y210` / `Y216` / `NV12` / `P010` / `P016` / `420_OPAQUE` / `NV11`) are
sized and decoded to interleaved `[Y, U, V, A]` samples, the four
documented depth / depth-stencil formats (`D16_UNORM` / `D32_FLOAT` /
`D24_UNORM_S8_UINT` / `D32_FLOAT_S8X24_UINT`, plus the combined `R24G8` /
`R32G8X24` typeless views and the four single-aspect
depth-only / stencil-only views `R24_UNORM_X8_TYPELESS` /
`X24_TYPELESS_G8_UINT` / `R32_FLOAT_X8X24_TYPELESS` /
`X32_TYPELESS_G8X24_UINT`) are sized and decoded to depth (and stencil)
values, and the plain colour `_TYPELESS` formats (`R16` / `R16G16` /
`R16G16B16A16` / `R32` / `R32G32` / `R32G32B32` / `R32G32B32A32` /
`R10G10B10A2`, joining the already-routed `R8` / `R8G8` / `R8G8B8A8` /
`B8G8R8A8` / `B8G8R8X8` views) are sized and carried verbatim by routing
to their byte-identical `_UINT` sibling, since a typeless surface stores
the same bytes with no fixed interpretation. The three under-documented
video formats (`P208` / `V208` / `V408`) and palette formats are
recognised but return `DdsError::Unsupported` from the layout resolver.
The legacy bump-derived `D3DFMT_CxV8U8` (numeric FourCC 117) is likewise
recognised but unsupported: the in-tree spec lists only its FourCC code,
not the bit layout or the channel-derivation rule, so it is a documented
docs gap rather than a guessed decode.

## Robustness

- A 40-case injection-robustness suite (`tests/injection_robustness.rs`)
  mutates one header field at a time and asserts `parse_dds` returns
  `Err` rather than panicking. Surface-size and block-grid arithmetic
  uses `checked_` / `saturating_` multiplication throughout.
- Eleven `cargo-fuzz` panic-free targets under `fuzz/` (`contract`,
  `parse_dds`, `decode_bcn`, `decode_bc6h`, `decode_bc7`, `decode_astc`,
  `decode_yuv`, `decode_depth`, `roundtrip`, `encode_astc`,
  `encode_round375`), driven daily by `.github/workflows/fuzz.yml`. The
  `contract` target drives `probe` / `info` / `decode` / `decode_all`
  under every limit profile (default, strict, tight limits) and asserts
  the lossless re-encode round trip of whatever decodes; the
  `encode_astc` target round-trips arbitrary RGBA8 through the ASTC
  encoder and re-decodes the output; `encode_round375` feeds every
  parser-accepted image through whichever round-375 encoder its shape
  matches (`encode_dds_uncompressed_dx10` /
  `encode_dds_volume_block_compressed` /
  `encode_dds_uncompressed_cubemap_array`) and re-parses the output. The ASTC
  block + surface decoders are additionally exercised by
  `tests/astc_robustness.rs` (a 70k random-block sweep over every
  footprint plus an exhaustive 2^11 block-mode-field sweep).
- Criterion benchmarks under `benches/` (`decode`, `encode`,
  `roundtrip`); run with
  `cargo bench -p oxideav-dds --bench {decode,encode,roundtrip}`.

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
| `registry` | yes     | Pulls in `oxideav-core`, exposes `register` / `make_decoder` / `make_encoder`, the `Decoder` / `Encoder` adapters, the `.dds` container and the `VideoFrame` bridge. Disable (`default-features = false`) to drop the `oxideav-core` dependency tree; the full contract API (`probe` / `info` / `decode*` / `encode*`) and the depth API stay available on `std`. |

## License

MIT — see [LICENSE](LICENSE).
