# oxideav-dds

Pure-Rust reader / writer for Microsoft's DirectDraw Surface (DDS) texture
container, the format every Direct3D game ships its baked block-compressed
art in. Part of the [oxideav workspace][oxideav-workspace] family of
single-format codec crates.

[oxideav-workspace]: https://github.com/OxideAV/oxideav-workspace

## Status

Round 1 (this release):

- `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10` (20 bytes) parser.
- Bit-exact round-trip of every common uncompressed surface layout:
  A8R8G8B8, X8R8G8B8, A8B8G8R8 (DXGI `R8G8B8A8_UNORM`), R5G6B5,
  A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
- Block-compressed pass-through. The reader recognises BC1 / BC2 / BC3
  (the classic DXT1 / DXT3 / DXT5), BC4 unorm + snorm (`BC4U` / `ATI1` /
  `BC4S`), BC5 unorm + snorm (`BC5U` / `ATI2` / `BC5S`), BC6H (UF16 +
  SF16), and BC7 (UNORM + SRGB) from either the legacy four-cc or the
  DX10 `dxgi_format`. The raw block bytes are exposed through
  `DdsImage::planes` but the decompressed RGB(A) pixels are not produced
  yet — that is round 2.
- Standalone-friendly via the default-on `registry` Cargo feature.
  Disable it (`default-features = false`) to drop the `oxideav-core`
  dependency tree entirely; the crate then exposes only the
  framework-free `parse_dds` / `encode_dds_uncompressed` API plus
  crate-local `DdsImage` / `DdsPixelFormat` / `DdsError` types built on
  `std`.

Out of scope for round 1 (planned for round 2):

- BC1..BC7 decompression to RGB(A).
- Mipmap-chain extraction (the parser surfaces only mip-0; it reads
  `mip_map_count` from the header but does not return the higher
  levels yet).
- Cubemap face surfaces and DX10 texture arrays.
- The full DXGI `DXGI_FORMAT` table — round 1 enumerates only the BC*
  family plus the few uncompressed RGBA / luminance formats it needs
  to reconstruct from a DX10 header.
- The `.dds` still-image container demuxer / muxer (probe by magic,
  expose as a single-frame stream).

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
`pixel_format` is one of the `Bc*` variants and whose `planes[0].data`
holds the raw 4x4-block byte array. Round 2 will add helpers that
decompress those into RGB(A).

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
