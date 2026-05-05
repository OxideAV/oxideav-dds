# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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
