# oxideav-dds

Pure-Rust reader / writer for Microsoft's DirectDraw Surface (DDS) texture
container, the format every Direct3D game ships its baked block-compressed
art in. Part of the [oxideav workspace][oxideav-workspace] family of
single-format codec crates.

[oxideav-workspace]: https://github.com/OxideAV/oxideav-workspace

## Status

Coverage as of round 4:

- `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10` (20 bytes) parser.
- Bit-exact round-trip of every common uncompressed surface layout:
  A8R8G8B8, X8R8G8B8, A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5,
  A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
- **BC1..BC5 + BC7 decompression** to RGBA8 / R8 / RG8 via
  `decode_bc1`, `decode_bc2`, `decode_bc3`, `decode_bc4_unorm`,
  `decode_bc4_snorm`, `decode_bc5_unorm`, `decode_bc5_snorm`,
  `decode_bc7`. BC7 covers all 8 modes (single-, dual- and
  three-subset partitions, p-bits, channel rotation, secondary
  alpha index plane).
- **BC6H decompression — all 14 modes — to RGBA half-float** via
  `decode_bc6h`. Modes 0..13 are implemented per the per-mode
  bit-allocation tables Microsoft mandates for Direct3D 11 hardware
  (covering 10.5.5.5, 7.6.6.6, 11.5.5.5 / 11.4.4.4 / 11.4.5.4 /
  11.4.4.5, 9.5.5.5, 8.6.6.6 / 8.5.6.5 / 8.5.5.6, 6.6.6.6 absolute
  TWO-subset variants and 10.10 / 11.9 / 12.8 / 16.4 ONE-subset
  variants). Both `BC6H_UF16` (unsigned) and `BC6H_SF16` (signed)
  finalisation paths are supported. Reserved 5-bit prefixes
  (10011, 10111, 11011, 11111) decode to zero RGB per spec.
- **BC1 / BC2 / BC3 / BC4 / BC5 encoders.** Round 4 lifts encoding
  beyond BC1: `encode_bc1`, `encode_bc2`, `encode_bc3`,
  `encode_bc4_unorm`, `encode_bc5_unorm` all emit valid block-
  compressed surfaces from RGBA8 / R8 / RG8 input. Each uses a
  furthest-point endpoint heuristic (no PCA / cluster fit / RDO);
  bit-exact roundtrip on solid blocks, ~19 dB PSNR-RGB on small
  natural-image gradients (>25 dB on the 16×16 test), 8-value
  interpolated alpha throughout BC3 / BC4 / BC5. BC1 honours
  punchthrough alpha when requested.
- **`.dds` container demuxer + muxer.** Round-3 lift over the
  round-2 extension-only entry: the framework-side `ContainerRegistry`
  now installs probe + demuxer + muxer + extension table entries via
  `register_containers`, so CLI tools (such as `cli-convert`) can
  open / write `.dds` files without touching the codec API directly.
- **Mipmap chain + cubemap faces + DX10 texture arrays.** Every
  on-disk surface is parsed into `DdsImage::surfaces` in Microsoft's
  mandated order (array slice → face → mip), tagged with
  `mip_level` / `array_slice` / `face`. `DdsImage::planes[0]` still
  mirrors the base level for callers that don't care.
- **Full DXGI format table** — every `DXGI_FORMAT` value Microsoft
  assigns (1..=132) is enumerated by name in `DxgiFormat` for
  lossless round-trip; HDR-float, integer, depth/stencil, YUV, and
  palette formats are recognised but produce
  `DdsError::Unsupported` from the layout resolver.
- Block-compressed pass-through. BC1..BC7 raw block bytes are
  surfaced through `DdsImage::surfaces[i].plane.data`; BC1..BC5 +
  BC7 also decompress to RGBA / R / RG via the dedicated `decode_bc*`
  entry points; BC6H decompresses to RGBA half-float for all 14
  modes via `decode_bc6h`.
- Standalone-friendly via the default-on `registry` Cargo feature.
  Disable it (`default-features = false`) to drop the `oxideav-core`
  dependency tree entirely; the crate then exposes only the
  framework-free `parse_dds` / `encode_dds_uncompressed` /
  `decode_bc1..bc7` / `decode_bc6h` / `encode_bc1..bc5` API plus
  crate-local `DdsImage` / `DdsPixelFormat` / `DdsError` types built on `std`.

Still deferred (followups):

- BC6H + BC7 encoders — the crate ships decoders only for the HDR /
  high-quality-LDR formats.
- Mipmap-chain emission from the encoder (still a single-level
  surface; round-trip across `parse_dds` -> `encode_dds_uncompressed`
  preserves only mip 0).

## Quickstart

```rust
use oxideav_dds::{parse_dds, encode_dds_uncompressed, DdsImage, DdsPixelFormat, DdsPlane};

// Parse a DDS file.
let bytes: Vec<u8> = std::fs::read("input.dds").unwrap();
let img = parse_dds(&bytes).unwrap();
println!(
    "{}x{} {} (mip levels: {})",
    img.width, img.height, img.pixel_format.name(), img.mip_map_count,
);

// Build + write a 4x3 A8R8G8B8 surface.
let mut data = vec![0u8; 4 * 3 * 4];
for (i, b) in data.iter_mut().enumerate() {
    *b = (i & 0xff) as u8;
}
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

For block-compressed input the same `parse_dds` returns an image whose
`pixel_format` is one of the `Bc*` variants and whose
`surfaces[i].plane.data` holds the raw 4x4-block byte array. For
BC1..BC5 + BC7 you can call the matching `decode_bc*` helper to expand
it into RGBA8 / R8 / RG8:

```rust
use oxideav_dds::{decode_bc1, decode_bc7, parse_dds, DdsPixelFormat};

let dds = std::fs::read("texture.dds").unwrap();
let img = parse_dds(&dds).unwrap();
let mut rgba = vec![0u8; (img.width * img.height * 4) as usize];
match img.pixel_format {
    DdsPixelFormat::Bc1 => {
        decode_bc1(&img.surfaces[0].plane.data, img.width, img.height, &mut rgba).unwrap();
    }
    DdsPixelFormat::Bc7Unorm | DdsPixelFormat::Bc7UnormSrgb => {
        decode_bc7(&img.surfaces[0].plane.data, img.width, img.height, &mut rgba).unwrap();
    }
    _ => { /* see decode_bc2..bc5 helpers */ }
}
```

To encode an RGBA8 surface to BC1 (DXT1) on disk:

```rust
use oxideav_dds::encode_bc1;

let rgba: Vec<u8> = vec![0xff; 16 * 16 * 4];
let mut bc1 = vec![0u8; (16 / 4) * (16 / 4) * 8];
encode_bc1(&rgba, 16, 16, /* punchthrough_alpha = */ false, &mut bc1).unwrap();
// `bc1` now holds the raw block bytes; wrap them in a DDS file with
// FOURCC_DXT1 to write a valid texture.
```

For BC2 (DXT3 explicit alpha), BC3 (DXT5 interpolated alpha), BC4
(single-channel) or BC5 (two-channel) the entry points mirror the BC1
encoder:

```rust
use oxideav_dds::{encode_bc2, encode_bc3, encode_bc4_unorm, encode_bc5_unorm};

let rgba = vec![0u8; 16 * 16 * 4];
let mut bc3 = vec![0u8; (16 / 4) * (16 / 4) * 16];
encode_bc3(&rgba, 16, 16, &mut bc3).unwrap();
```

For mipmapped or cubemap textures iterate `img.surfaces` directly:
each entry carries its own `mip_level`, `array_slice`, `face`, and
`(width, height)`.

## Clean-room provenance

Every byte of the parser was written from Microsoft's public DDS
programming-guide pages on [learn.microsoft.com][ms-dds-pguide] (the
"DDS file layout for textures", "DDS pixel format", and "Programming
guide for DDS" articles plus the public DXGI format reference). No
DirectXTex, D3DX, NVTT, squish, or other DDS-handling source code was
consulted, paraphrased, or cross-referenced. Binaries (`magick`,
`texconv`) are used only as black-box validators when generating
test fixtures, not as a source of constants or layout.

[ms-dds-pguide]: https://learn.microsoft.com/en-us/windows/win32/direct3ddds/dx-graphics-dds-pguide

## Cargo features

| Feature    | Default | Effect                                                                                                                                |
|------------|---------|---------------------------------------------------------------------------------------------------------------------------------------|
| `registry` | yes     | Pulls in `oxideav-core`, exposes the `Decoder` / `Encoder` trait surface, registers the codec with the framework via `register`.      |

## License

MIT — see [LICENSE](LICENSE).
