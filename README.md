# oxideav-dds

Pure-Rust reader / writer for Microsoft's DirectDraw Surface (DDS) texture
container, the format every Direct3D game ships its baked block-compressed
art in. Part of the [oxideav workspace][oxideav-workspace] family of
single-format codec crates.

[oxideav-workspace]: https://github.com/OxideAV/oxideav-workspace

## Status

Coverage as of round 2:

- `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10` (20 bytes) parser.
- Bit-exact round-trip of every common uncompressed surface layout:
  A8R8G8B8, X8R8G8B8, A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5,
  A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
- **BC1..BC5 decompression** to RGBA8 / R8 / RG8 via `decode_bc1`,
  `decode_bc2`, `decode_bc3`, `decode_bc4_unorm`, `decode_bc4_snorm`,
  `decode_bc5_unorm`, `decode_bc5_snorm`. Cross-validated against
  ImageMagick's DXT1 decoder.
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
  surfaced through `DdsImage::surfaces[i].plane.data`; BC1..BC5 also
  decompress to RGBA / R / RG via the dedicated `decode_bc*` entry
  points.
- Standalone-friendly via the default-on `registry` Cargo feature.
  Disable it (`default-features = false`) to drop the `oxideav-core`
  dependency tree entirely; the crate then exposes only the
  framework-free `parse_dds` / `encode_dds_uncompressed` API plus
  crate-local `DdsImage` / `DdsPixelFormat` / `DdsError` types built
  on `std`.

Still deferred (followups):

- BC6H + BC7 decompression to RGBA — recognised pass-through, not
  decompressed yet (multi-mode partition tables too large to land
  alongside BC1..BC5).
- BCn-encoder side — the encoder stays uncompressed-only.
- The `.dds` still-image container demuxer / muxer.

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
BC1..BC5 you can call the matching `decode_bc*` helper to expand it
into RGBA8 / R8 / RG8:

```rust
use oxideav_dds::{decode_bc1, parse_dds};

let dds = std::fs::read("texture.dds").unwrap();
let img = parse_dds(&dds).unwrap();
let mut rgba = vec![0u8; (img.width * img.height * 4) as usize];
decode_bc1(&img.surfaces[0].plane.data, img.width, img.height, &mut rgba)
    .unwrap();
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
