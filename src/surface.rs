//! The on-disk surface vocabulary — the depth API behind the contract
//! records in [`crate::image`].
//!
//! * [`SurfaceFormat`] names every `D3DFMT` / `DXGI_FORMAT` layout the
//!   parser recognises (~70 variants: legacy mask formats, BC1..BC7,
//!   ASTC, float / normalised / plain-integer, depth-stencil, YUV).
//! * [`DdsSurface`] is one stored (array slice, cubemap face, mip level,
//!   depth slice) surface with its raw bytes.
//! * [`DdsFile`] is the whole parsed file: header facts plus every
//!   surface in Microsoft's on-disk order. [`crate::parse_dds`] produces
//!   it; the depth writers in [`crate::encoder`] consume it.
//!
//! The contract layer ([`crate::decode`] → [`crate::DdsImage`]) expands
//! one surface into a layout `oxideav_core::PixelFormat` names; see
//! [`SurfaceFormat::contract_format`] for the mapping.
//!
//! Everything here builds without `oxideav-core`.

pub use crate::image::Plane;

/// The pre-contract name of [`Plane`] (same fields).
#[deprecated(note = "use oxideav_dds::Plane (IMAGE_CRATE_API)")]
pub type DdsPlane = Plane;

/// Stored pixel layout of a DDS surface — the `D3DFMT` (legacy header
/// masks / FourCC) or `DXGI_FORMAT` (DX10 header) family the file
/// declares.
///
/// Uncompressed variants list the channel order as it appears in the
/// stored plane — the parser does NOT swap BGR → RGB on read; the
/// contract layer ([`crate::decode`]) expands the surface to a
/// [`crate::DdsPixelFormat`] instead (see [`Self::contract_format`]).
/// Block-compressed variants carry the raw block bytes in
/// [`DdsSurface::plane`]; the matching `decode_bc*` / `decode_astc_ldr`
/// functions expand them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SurfaceFormat {
    /// 32 bpp, on-disk `[B, G, R, A]` per pixel
    /// (legacy `D3DFMT_A8R8G8B8` / DXGI `B8G8R8A8_UNORM`).
    A8R8G8B8,
    /// 32 bpp, on-disk `[B, G, R, X]` per pixel — alpha byte unused
    /// (`D3DFMT_X8R8G8B8` / DXGI `B8G8R8X8_UNORM`).
    X8R8G8B8,
    /// 32 bpp, on-disk `[R, G, B, A]` per pixel
    /// (DXGI `R8G8B8A8_UNORM`).
    A8B8G8R8,
    /// 32 bpp, on-disk `[R, G, B, X]` per pixel — alpha byte unused
    /// (`D3DFMT_X8B8G8R8`). The RGB sibling of [`Self::A8B8G8R8`]: red at
    /// the lowest address (mask `0x000000ff`), no alpha channel.
    X8B8G8R8,
    /// 16 bpp, packed `RRRRR GGGGGG BBBBB` little-endian
    /// (`D3DFMT_R5G6B5` / DXGI `B5G6R5_UNORM`).
    R5G6B5,
    /// 16 bpp, packed `A RRRRR GGGGG BBBBB` little-endian
    /// (`D3DFMT_A1R5G5B5` / DXGI `B5G5R5A1_UNORM`).
    A1R5G5B5,
    /// 16 bpp, packed `X RRRRR GGGGG BBBBB` little-endian — top bit unused
    /// (`D3DFMT_X1R5G5B5`). The RGB sibling of [`Self::A1R5G5B5`]: same
    /// 5:5:5 colour masks (R `0x7c00`, G `0x03e0`, B `0x001f`), no alpha.
    X1R5G5B5,
    /// 16 bpp, packed `AAAA RRRR GGGG BBBB` little-endian
    /// (`D3DFMT_A4R4G4B4` / DXGI `B4G4R4A4_UNORM`).
    A4R4G4B4,
    /// 16 bpp, packed `XXXX RRRR GGGG BBBB` little-endian — top nibble
    /// unused (`D3DFMT_X4R4G4B4`). The RGB sibling of [`Self::A4R4G4B4`]:
    /// same 4:4:4 colour masks (R `0x0f00`, G `0x00f0`, B `0x000f`), no
    /// alpha.
    X4R4G4B4,
    /// 24 bpp, on-disk `[B, G, R]` per pixel
    /// (`D3DFMT_R8G8B8`).
    R8G8B8,
    /// 16 bpp, on-disk `[L, A]` per pixel
    /// (`D3DFMT_A8L8`).
    A8L8,
    /// 16 bpp single-channel luminance, one little-endian `u16` per pixel
    /// (`D3DFMT_L16`). Luminance mask `0xffff`, no alpha; the 16-bit
    /// sibling of [`Self::L8`].
    L16,
    /// 8 bpp packed 4:4 luminance + alpha, one byte per pixel
    /// (`D3DFMT_A4L4`). Luminance in the low nibble (mask `0x0f`), alpha
    /// in the high nibble (mask `0xf0`).
    A4L4,
    /// 8 bpp single-channel luminance (`D3DFMT_L8` / DXGI `R8_UNORM`).
    L8,
    /// 8 bpp single-channel alpha (`D3DFMT_A8`).
    A8,
    /// 16 bpp packed 3:3:2 RGB plus 8-bit alpha, one little-endian `u16`
    /// per pixel (`D3DFMT_A8R3G3B2`). Per Microsoft's "Common DDS File
    /// Resource Formats" table the masks are R=`0x00e0`, G=`0x001c`,
    /// B=`0x0003`, A=`0xff00`: the colour channels occupy the low byte
    /// (red bits 5..=7, green bits 2..=4, blue bits 0..=1) and the alpha
    /// channel the high byte. Expand to RGBA8 with
    /// [`crate::decode_a8r3g3b2_surface`].
    A8R3G3B2,
    /// 16 bpp packed 4:4:4:4 with alpha, one little-endian `u16` per
    /// pixel (DXGI `A4B4G4R4_UNORM`, value 191). The DX10-only 4-bit
    /// sibling of [`Self::A4R4G4B4`] with the reverse colour order:
    /// following the "first named component in the least-significant
    /// bits" DXGI rule the word is `RRRR GGGG BBBB AAAA` — alpha in
    /// bits 0..=3, blue 4..=7, green 8..=11, red 12..=15 (the mirror of
    /// the legacy `A4R4G4B4`, whose alpha sits in the top nibble). No
    /// legacy `DDS_PIXELFORMAT` mask carries this order, so it is
    /// recognised only via the DX10 `DXGI_FORMAT` code. Expand to RGBA8
    /// (each nibble widened 4→8 by bit-replication) with
    /// [`crate::decode_a4b4g4r4_unorm_surface`].
    A4B4G4R4Unorm,

    // --- Block-compressed pass-through (raw block bytes; not decoded
    //     in round 1) ----------------------------------------------------
    /// BC1 (`DXT1`) — 4 bpp, 8 bytes per 4×4 block, 1-bit alpha.
    Bc1,
    /// BC2 (`DXT3`) — 8 bpp, 16 bytes per 4×4 block, 4-bit explicit alpha.
    Bc2,
    /// BC3 (`DXT5`) — 8 bpp, 16 bytes per 4×4 block, interpolated alpha.
    Bc3,
    /// BC4 (unsigned, `BC4U` / `ATI1`) — 4 bpp, 8 bytes/block, single channel.
    Bc4Unorm,
    /// BC4 (signed, `BC4S`).
    Bc4Snorm,
    /// BC5 (unsigned, `BC5U` / `ATI2`) — 8 bpp, 16 bytes/block, two channels.
    Bc5Unorm,
    /// BC5 (signed, `BC5S`).
    Bc5Snorm,
    /// BC6H (unsigned-float, `BC6H_UF16`) — 8 bpp, 16 bytes/block, HDR RGB.
    Bc6hUf16,
    /// BC6H (signed-float, `BC6H_SF16`).
    Bc6hSf16,
    /// BC7 (`BC7_UNORM`) — 8 bpp, 16 bytes/block, RGBA.
    Bc7Unorm,
    /// BC7 sRGB variant.
    Bc7UnormSrgb,

    // --- Extended high-bit-depth / floating-point uncompressed layouts ---
    //
    // Microsoft assigns these to the legacy `D3DFMT` numeric FourCC
    // codes 36 / 110..=116 (programming guide "DDS pixel format")
    // and to the matching `DXGI_FORMAT` values. Each carries one or more
    // 16-bit or 32-bit channels, little-endian, in the channel order the
    // DXGI name lists (lowest memory address first).
    /// 64 bpp, on-disk `[R, G, B, A]` × `u16` unsigned-normalised
    /// (`D3DFMT_A16B16G16R16`, FourCC 36 / DXGI `R16G16B16A16_UNORM`).
    R16G16B16A16Unorm,
    /// 64 bpp, on-disk `[R, G, B, A]` × `i16` signed-normalised
    /// (`D3DFMT_Q16W16V16U16`, FourCC 110 / DXGI `R16G16B16A16_SNORM`).
    R16G16B16A16Snorm,
    /// 16 bpp, on-disk `[R]` × half-float (binary16)
    /// (`D3DFMT_R16F`, FourCC 111 / DXGI `R16_FLOAT`).
    R16Float,
    /// 32 bpp, on-disk `[R, G]` × half-float (binary16)
    /// (`D3DFMT_G16R16F`, FourCC 112 / DXGI `R16G16_FLOAT`).
    R16G16Float,
    /// 64 bpp, on-disk `[R, G, B, A]` × half-float (binary16)
    /// (`D3DFMT_A16B16G16R16F`, FourCC 113 / DXGI `R16G16B16A16_FLOAT`).
    R16G16B16A16Float,
    /// 32 bpp, on-disk `[R]` × `f32` (binary32)
    /// (`D3DFMT_R32F`, FourCC 114 / DXGI `R32_FLOAT`).
    R32Float,
    /// 64 bpp, on-disk `[R, G]` × `f32` (binary32)
    /// (`D3DFMT_G32R32F`, FourCC 115 / DXGI `R32G32_FLOAT`).
    R32G32Float,
    /// 128 bpp, on-disk `[R, G, B, A]` × `f32` (binary32)
    /// (`D3DFMT_A32B32G32R32F`, FourCC 116 / DXGI `R32G32B32A32_FLOAT`).
    R32G32B32A32Float,
    /// 32 bpp packed 10:10:10:2, one little-endian `u32` per pixel
    /// (`D3DFMT_A2B10G10R10` / DXGI `R10G10B10A2_UNORM`). R occupies
    /// bits 0..=9, G bits 10..=19, B bits 20..=29, A bits 30..=31 —
    /// the canonical Direct3D 10 packing where the first named component
    /// sits in the least-significant bits.
    R10G10B10A2Unorm,
    /// 32 bpp packed 10:10:10:2, one little-endian `u32` per pixel
    /// (DXGI `R10G10B10A2_UINT`, value 25). Identical bit packing to
    /// [`Self::R10G10B10A2Unorm`] (R in bits 0..=9, G in 10..=19, B in
    /// 20..=29, A in 30..=31), but the stored values are plain unsigned
    /// integers — there is no `[0, 1]` normalisation, so the decoded
    /// `0..=1023` colour and `0..=3` alpha samples ARE the values.
    R10G10B10A2Uint,
    /// 32 bpp packed HDR, one little-endian `u32` per pixel holding
    /// three sign-less partial-precision floats (DXGI `R11G11B10_FLOAT`,
    /// value 26): R in bits 0..=10, G in 11..=21, B in 22..=31. Widen
    /// with [`crate::decode_r11g11b10_float_surface`].
    R11G11B10Float,
    /// 32 bpp packed HDR, one little-endian `u32` per pixel holding
    /// three 9-bit mantissas and a shared 5-bit exponent (DXGI
    /// `R9G9B9E5_SHAREDEXP`, value 67). Widen with
    /// [`crate::decode_r9g9b9e5_sharedexp_surface`].
    R9G9B9E5SharedExp,
    /// 32 bpp packed 10:10:10:2 in BGR channel order, one little-endian
    /// `u32` per pixel (legacy `D3DFMT_A2R10G10B10`). The BGR-ordered
    /// sibling of [`Self::R10G10B10A2Unorm`]: blue occupies bits 0..=9,
    /// green bits 10..=19, red bits 20..=29, alpha bits 30..=31 — the
    /// reverse colour order. This layout has no DX10 `DXGI_FORMAT`
    /// counterpart (Direct3D 10 dropped the BGR 10:10:10:2 packing), so
    /// it is recognised only via its legacy `DDS_PIXELFORMAT` masks.
    /// Decode the stored channels with
    /// [`crate::decode_a2r10g10b10_surface`].
    A2R10G10B10,
    /// 32 bpp per pixel-PAIR packed, horizontally sub-sampled RGB
    /// (DXGI `R8G8_B8G8_UNORM`, value 68). Each little-endian 32-bit
    /// block `[R, G0, B, G1]` describes two adjacent pixels that share
    /// the red and blue bytes but carry an independent green byte each
    /// (pixel 0 = `(R, G0, B)`, pixel 1 = `(R, G1, B)`). Width must be
    /// even; expand to RGBA8 with
    /// [`crate::decode_r8g8_b8g8_unorm_surface`].
    R8G8B8G8Unorm,
    /// 32 bpp per pixel-PAIR packed, horizontally sub-sampled RGB
    /// (DXGI `G8R8_G8B8_UNORM`, value 69). The sibling of
    /// [`Self::R8G8B8G8Unorm`] with the channels reordered inside the
    /// block to `[G0, R, G1, B]`; same pixel-pair reconstruction. Width
    /// must be even; expand to RGBA8 with
    /// [`crate::decode_g8r8_g8b8_unorm_surface`].
    G8R8G8B8Unorm,

    // --- 16-bit-per-channel integer uncompressed layouts ----------------
    //
    // Plain tightly-packed little-endian integer samples, one, two or
    // four 16-bit channels per pixel, in the named channel order with the
    // first-named component at the lowest memory address (DXGI puts the
    // first listed component in the lowest address, matching the float and
    // UNORM 16-bit families above). Unlike the `_UNORM` / `_SNORM`
    // siblings there is no `[0, 1]` / `[-1, 1]` normalisation: the stored
    // integers ARE the decoded values. UINT decodes to `u16`, SINT to
    // `i16` via [`crate::decode_uint16_surface`] /
    // [`crate::decode_sint16_surface`].
    /// 16 bpp, on-disk `[R]` × `u16` unsigned-integer
    /// (DXGI `R16_UINT`, value 57).
    R16Uint,
    /// 16 bpp, on-disk `[R]` × `i16` signed-integer
    /// (DXGI `R16_SINT`, value 59).
    R16Sint,
    /// 32 bpp, on-disk `[R, G]` × `u16` unsigned-integer
    /// (DXGI `R16G16_UINT`, value 36).
    R16G16Uint,
    /// 32 bpp, on-disk `[R, G]` × `i16` signed-integer
    /// (DXGI `R16G16_SINT`, value 38).
    R16G16Sint,
    /// 64 bpp, on-disk `[R, G, B, A]` × `u16` unsigned-integer
    /// (DXGI `R16G16B16A16_UINT`, value 12).
    R16G16B16A16Uint,
    /// 64 bpp, on-disk `[R, G, B, A]` × `i16` signed-integer
    /// (DXGI `R16G16B16A16_SINT`, value 14).
    R16G16B16A16Sint,

    // --- 8-bit-per-channel integer uncompressed layouts -----------------
    //
    // Plain tightly-packed `u8` / `i8` samples, one, two or four 8-bit
    // channels per pixel, in the named channel order with the first-named
    // component at the lowest memory address. As with the 16-bit and
    // 32-bit integer families there is no `[0, 1]` / `[-1, 1]`
    // normalisation: the stored bytes ARE the decoded values. UINT decodes
    // to `u8`, SINT to `i8` via [`crate::decode_uint8_surface`] /
    // [`crate::decode_sint8_surface`].
    /// 8 bpp, on-disk `[R]` × `u8` unsigned-integer
    /// (DXGI `R8_UINT`, value 62).
    R8Uint,
    /// 8 bpp, on-disk `[R]` × `i8` signed-integer
    /// (DXGI `R8_SINT`, value 64).
    R8Sint,
    /// 16 bpp, on-disk `[R, G]` × `u8` unsigned-integer
    /// (DXGI `R8G8_UINT`, value 50).
    R8G8Uint,
    /// 16 bpp, on-disk `[R, G]` × `i8` signed-integer
    /// (DXGI `R8G8_SINT`, value 52).
    R8G8Sint,
    /// 32 bpp, on-disk `[R, G, B, A]` × `u8` unsigned-integer
    /// (DXGI `R8G8B8A8_UINT`, value 30).
    R8G8B8A8Uint,
    /// 32 bpp, on-disk `[R, G, B, A]` × `i8` signed-integer
    /// (DXGI `R8G8B8A8_SINT`, value 32).
    R8G8B8A8Sint,

    // --- 32-bit-per-channel integer uncompressed layouts ----------------
    //
    // Plain tightly-packed little-endian `u32` / `i32` samples, one, two,
    // three or four 32-bit channels per pixel, in the named channel order
    // with the first-named component at the lowest memory address. No
    // normalisation (the `_UINT` / `_SINT` families are plain integers):
    // the stored words ARE the decoded values. UINT decodes to `u32`, SINT
    // to `i32` via [`crate::decode_uint32_surface`] /
    // [`crate::decode_sint32_surface`]. The three-channel `R32G32B32`
    // layouts have no 8-bit or 16-bit analogue (DXGI carries a 96-bit
    // three-component family only at 32-bit channel width).
    /// 32 bpp, on-disk `[R]` × `u32` unsigned-integer
    /// (DXGI `R32_UINT`, value 42).
    R32Uint,
    /// 32 bpp, on-disk `[R]` × `i32` signed-integer
    /// (DXGI `R32_SINT`, value 43).
    R32Sint,
    /// 64 bpp, on-disk `[R, G]` × `u32` unsigned-integer
    /// (DXGI `R32G32_UINT`, value 17).
    R32G32Uint,
    /// 64 bpp, on-disk `[R, G]` × `i32` signed-integer
    /// (DXGI `R32G32_SINT`, value 18).
    R32G32Sint,
    /// 96 bpp, on-disk `[R, G, B]` × `u32` unsigned-integer
    /// (DXGI `R32G32B32_UINT`, value 7).
    R32G32B32Uint,
    /// 96 bpp, on-disk `[R, G, B]` × `i32` signed-integer
    /// (DXGI `R32G32B32_SINT`, value 8).
    R32G32B32Sint,
    /// 128 bpp, on-disk `[R, G, B, A]` × `u32` unsigned-integer
    /// (DXGI `R32G32B32A32_UINT`, value 3).
    R32G32B32A32Uint,
    /// 128 bpp, on-disk `[R, G, B, A]` × `i32` signed-integer
    /// (DXGI `R32G32B32A32_SINT`, value 4).
    R32G32B32A32Sint,

    // --- Normalised 8-bit / 16-bit single- and dual-channel layouts -----
    //
    // Tightly-packed little-endian integer samples that the shader
    // interprets as normalised floating-point values: `_UNORM` maps the
    // unsigned integer range `[0, MAX]` onto `[0, 1]` (divide by
    // `2^n - 1`), `_SNORM` maps the two's-complement range onto `[-1, 1]`
    // (divide by `2^(n-1) - 1`, with the extra clamp so both the minimum
    // and second-minimum encodings map to `-1.0`). One or two 8-bit or
    // 16-bit channels per pixel, in the named order with the first-named
    // component at the lowest memory address. Decode to interleaved `f32`
    // via [`crate::decode_unorm_surface`] / [`crate::decode_snorm_surface`].
    // (The four-channel 16-bit `R16G16B16A16` UNORM/SNORM siblings keep
    // their existing `u16` / `i16` verbatim decoders; the single-channel
    // 8-bit `R8_UNORM` keeps its byte-identical [`Self::L8`] container
    // mapping but is also accepted by `decode_unorm_surface`.)
    /// 8 bpp, on-disk `[R]` × `u8` unsigned-normalised onto `[0, 1]`
    /// (DXGI `R8_UNORM`, value 61).
    R8Unorm,
    /// 8 bpp, on-disk `[R]` × `i8` signed-normalised onto `[-1, 1]`
    /// (DXGI `R8_SNORM`, value 63).
    R8Snorm,
    /// 16 bpp, on-disk `[R, G]` × `i8` signed-normalised onto `[-1, 1]`
    /// (DXGI `R8G8_SNORM`, value 51) — the classic two-channel
    /// tangent-space normal-map layout.
    R8G8Snorm,
    /// 32 bpp, on-disk `[R, G, B, A]` × `i8` signed-normalised onto
    /// `[-1, 1]` (DXGI `R8G8B8A8_SNORM`, value 31).
    R8G8B8A8Snorm,
    /// 16 bpp, on-disk `[R]` × `u16` unsigned-normalised onto `[0, 1]`
    /// (DXGI `R16_UNORM`, value 56) — common single-channel height map.
    R16Unorm,
    /// 16 bpp, on-disk `[R]` × `i16` signed-normalised onto `[-1, 1]`
    /// (DXGI `R16_SNORM`, value 58).
    R16Snorm,
    /// 32 bpp, on-disk `[R, G]` × `u16` unsigned-normalised onto `[0, 1]`
    /// (DXGI `R16G16_UNORM`, value 35).
    R16G16Unorm,
    /// 32 bpp, on-disk `[R, G]` × `i16` signed-normalised onto `[-1, 1]`
    /// (DXGI `R16G16_SNORM`, value 37) — high-precision two-channel
    /// tangent-space normal map.
    R16G16Snorm,

    /// 16 bpp depth surface — one `u16` per texel holding a
    /// single-component unsigned-normalised depth value onto `[0, 1]`
    /// (DXGI `D16_UNORM`, value 55). Same on-disk packing as `R16_UNORM`;
    /// decode with [`crate::decode_depth_d16_surface`].
    D16Unorm,
    /// 32 bpp depth surface — one little-endian `f32` per texel holding a
    /// single-component floating-point depth value (DXGI `D32_FLOAT`,
    /// value 40). Same on-disk packing as `R32_FLOAT`; decode with
    /// [`crate::decode_depth_d32_surface`].
    D32Float,
    /// 32 bpp packed depth+stencil surface — one little-endian `u32` per
    /// texel, the low 24 bits an unsigned-normalised depth onto `[0, 1]`
    /// (÷ `2^24 − 1`) and the high 8 bits a `u8` stencil index (DXGI
    /// `D24_UNORM_S8_UINT`, value 45; the typeless view `R24G8_TYPELESS`
    /// shares the packing). Decode with
    /// [`crate::decode_depth_d24s8_surface`].
    D24UnormS8Uint,
    /// 64 bpp packed depth+stencil surface — two little-endian `u32`
    /// words per texel: the first an `f32` floating-point depth, the
    /// second a `u8` stencil index in its low 8 bits with the upper 24
    /// bits unused (DXGI `D32_FLOAT_S8X24_UINT`, value 20; the typeless
    /// view `R32G8X24_TYPELESS` shares the packing). Decode with
    /// [`crate::decode_depth_d32s8_surface`].
    D32FloatS8X24Uint,

    /// 32 bpp depth-only **view** over `D24_UNORM_S8_UINT` memory — one
    /// little-endian `u32` per texel whose low 24 bits are the
    /// unsigned-normalised depth onto `[0, 1]` (÷ `2^24 − 1`) and whose
    /// high 8 bits are typeless padding that this view ignores (DXGI
    /// `R24_UNORM_X8_TYPELESS`, value 46 — "24 bits red channel and 8
    /// bits unused"). Decode with
    /// [`crate::decode_depth_r24_unorm_x8_surface`].
    R24UnormX8Typeless,
    /// 32 bpp stencil-only **view** over `D24_UNORM_S8_UINT` memory — one
    /// little-endian `u32` per texel whose low 24 bits are typeless
    /// padding this view ignores and whose high 8 bits are the `u8`
    /// stencil index (DXGI `X24_TYPELESS_G8_UINT`, value 47 — "24 bits
    /// unused and 8 bits green channel"). Decode with
    /// [`crate::decode_depth_x24_g8_uint_surface`].
    X24TypelessG8Uint,
    /// 64 bpp depth-only **view** over `D32_FLOAT_S8X24_UINT` memory —
    /// two little-endian `u32` words per texel: the first is the
    /// floating-point depth (returned verbatim as `f32`), the second is
    /// 8 bits stencil + 24 bits padding that this view ignores entirely
    /// (DXGI `R32_FLOAT_X8X24_TYPELESS`, value 21 — "32-bit red channel,
    /// 8 bits are unused, and 24 bits are unused"). Decode with
    /// [`crate::decode_depth_r32_float_x8x24_surface`].
    R32FloatX8X24Typeless,
    /// 64 bpp stencil-only **view** over `D32_FLOAT_S8X24_UINT` memory —
    /// two little-endian `u32` words per texel: the first (32-bit depth)
    /// is typeless padding this view ignores, the second holds the `u8`
    /// stencil index in its low 8 bits with the upper 24 bits unused
    /// (DXGI `X32_TYPELESS_G8X24_UINT`, value 22 — "32 bits unused, 8
    /// bits for green channel, and 24 bits are unused"). Decode with
    /// [`crate::decode_depth_x32_g8x24_uint_surface`].
    X32TypelessG8X24Uint,

    /// ASTC LDR block-compressed surface. Every ASTC block is a fixed
    /// 128 bits (16 bytes) and covers a `block_w × block_h` texel
    /// footprint (one of the 14 LDR 2D footprints, 4×4 … 12×12). The
    /// `srgb` flag mirrors the `_UNORM` vs `_UNORM_SRGB` DXGI variant
    /// (round-trip metadata only; the LDR block decoder produces the
    /// stored unorm bytes either way). Decode with
    /// [`crate::decode_astc_ldr`].
    Astc {
        /// Block footprint width (4, 5, 6, 8, 10, or 12).
        block_w: u32,
        /// Block footprint height (4, 5, 6, 8, or 10/12).
        block_h: u32,
        /// True for the `_UNORM_SRGB` DXGI variants.
        srgb: bool,
    },

    /// A YUV (video) DXGI surface — one of the eleven luma/chroma
    /// `DXGI_FORMAT` layouts Microsoft fully specifies (AYUV, Y410,
    /// Y416, NV12, P010, P016, 420_OPAQUE, YUY2, Y210, Y216, NV11).
    /// The on-disk bytes are carried verbatim (planar or packed,
    /// per-format); call the matching `crate::yuv::decode_*_surface`
    /// helper to expand the surface to interleaved `[Y, U, V, A]`
    /// samples. Surface sizing follows [`crate::yuv::YuvFormat::surface_size_bytes`].
    Yuv(crate::yuv::YuvFormat),
}

impl SurfaceFormat {
    /// Bits per pixel for uncompressed formats; for block-compressed
    /// formats this is the *amortised* rate (4 bpp for BC1/BC4, 8 bpp
    /// for BC2/BC3/BC5/BC6H/BC7). Matches Microsoft's "bits per pixel"
    /// figure in the public DDS programming guide.
    pub fn bits_per_pixel(self) -> u32 {
        match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 | Self::A8B8G8R8 | Self::X8B8G8R8 => 32,
            Self::R8G8B8 => 24,
            Self::R5G6B5
            | Self::A1R5G5B5
            | Self::X1R5G5B5
            | Self::A4R4G4B4
            | Self::X4R4G4B4
            | Self::A8R3G3B2
            | Self::A4B4G4R4Unorm
            | Self::A8L8
            | Self::L16 => 16,
            Self::L8 | Self::A8 | Self::A4L4 | Self::R8Uint | Self::R8Sint => 8,
            Self::R8Unorm | Self::R8Snorm => 8,
            // Sub-sampled packed RGB: 32 bits per pixel PAIR = 16 bpp
            // amortised over the two pixels each block encodes.
            Self::R8G8B8G8Unorm | Self::G8R8G8B8Unorm => 16,
            Self::R16Float | Self::R16Uint | Self::R16Sint | Self::R8G8Uint | Self::R8G8Sint => 16,
            Self::R8G8Snorm | Self::R16Unorm | Self::R16Snorm => 16,
            Self::R16G16Float
            | Self::R32Float
            | Self::R10G10B10A2Unorm
            | Self::R10G10B10A2Uint
            | Self::R11G11B10Float
            | Self::R9G9B9E5SharedExp
            | Self::A2R10G10B10
            | Self::R16G16Uint
            | Self::R16G16Sint
            | Self::R8G8B8A8Uint
            | Self::R8G8B8A8Sint
            | Self::R32Uint
            | Self::R32Sint => 32,
            Self::R8G8B8A8Snorm | Self::R16G16Unorm | Self::R16G16Snorm => 32,
            // Depth / depth-stencil surfaces.
            Self::D16Unorm => 16,
            Self::D32Float | Self::D24UnormS8Uint => 32,
            Self::D32FloatS8X24Uint => 64,
            // Single-aspect depth/stencil views share the combined
            // surface's footprint: the D24S8 views are 32-bit, the
            // D32S8X24 views are 64-bit (one stored stencil byte + 24
            // unused bits per the documented padding).
            Self::R24UnormX8Typeless | Self::X24TypelessG8Uint => 32,
            Self::R32FloatX8X24Typeless | Self::X32TypelessG8X24Uint => 64,
            Self::R16G16B16A16Unorm
            | Self::R16G16B16A16Snorm
            | Self::R16G16B16A16Float
            | Self::R16G16B16A16Uint
            | Self::R16G16B16A16Sint
            | Self::R32G32Float
            | Self::R32G32Uint
            | Self::R32G32Sint => 64,
            Self::R32G32B32Uint | Self::R32G32B32Sint => 96,
            Self::R32G32B32A32Float | Self::R32G32B32A32Uint | Self::R32G32B32A32Sint => 128,
            Self::Bc1 | Self::Bc4Unorm | Self::Bc4Snorm => 4,
            Self::Bc2
            | Self::Bc3
            | Self::Bc5Unorm
            | Self::Bc5Snorm
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::Bc7Unorm
            | Self::Bc7UnormSrgb => 8,
            // ASTC: amortised bits/pixel = 128 / (block_w*block_h).
            Self::Astc {
                block_w, block_h, ..
            } => 128 / (block_w * block_h).max(1),
            // YUV: amortised bits/pixel from the per-format surface size.
            // 4:4:4 8-bit = 32, 10/16-bit 4:4:4 = 32/64; 4:2:2 = 16/32;
            // 4:2:0 = 12/24; 4:1:1 (padded) = 16.
            Self::Yuv(f) => {
                use crate::yuv::YuvFormat::*;
                match f {
                    Ayuv => 32,
                    Y410 => 32,
                    Y416 => 64,
                    Yuy2 | Uyvy => 16,
                    Y210 | Y216 => 32,
                    Nv12 | Opaque420 => 12,
                    P010 | P016 => 24,
                    Nv11 => 16,
                }
            }
        }
    }

    /// Bytes per pixel for uncompressed formats. Returns `None` for
    /// block-compressed formats — use [`Self::block_bytes`] instead.
    pub fn bytes_per_pixel(self) -> Option<u32> {
        Some(match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 | Self::A8B8G8R8 | Self::X8B8G8R8 => 4,
            Self::R8G8B8 => 3,
            Self::R5G6B5
            | Self::A1R5G5B5
            | Self::X1R5G5B5
            | Self::A4R4G4B4
            | Self::X4R4G4B4
            | Self::A8R3G3B2
            | Self::A4B4G4R4Unorm
            | Self::A8L8
            | Self::L16 => 2,
            Self::L8 | Self::A8 | Self::A4L4 | Self::R8Uint | Self::R8Sint => 1,
            Self::R8Unorm | Self::R8Snorm => 1,
            // Sub-sampled packed RGB stores 4 bytes per 2-pixel block,
            // i.e. 2 bytes per pixel — exact only for an even width,
            // which the layout requires anyway.
            Self::R8G8B8G8Unorm | Self::G8R8G8B8Unorm => 2,
            Self::R16Float | Self::R16Uint | Self::R16Sint | Self::R8G8Uint | Self::R8G8Sint => 2,
            Self::R8G8Snorm | Self::R16Unorm | Self::R16Snorm => 2,
            Self::R16G16Float
            | Self::R32Float
            | Self::R10G10B10A2Unorm
            | Self::R10G10B10A2Uint
            | Self::R11G11B10Float
            | Self::R9G9B9E5SharedExp
            | Self::A2R10G10B10
            | Self::R16G16Uint
            | Self::R16G16Sint
            | Self::R8G8B8A8Uint
            | Self::R8G8B8A8Sint
            | Self::R32Uint
            | Self::R32Sint => 4,
            Self::R8G8B8A8Snorm | Self::R16G16Unorm | Self::R16G16Snorm => 4,
            // Depth / depth-stencil surfaces.
            Self::D16Unorm => 2,
            Self::D32Float | Self::D24UnormS8Uint => 4,
            Self::D32FloatS8X24Uint => 8,
            // Single-aspect depth/stencil views over the same memory.
            Self::R24UnormX8Typeless | Self::X24TypelessG8Uint => 4,
            Self::R32FloatX8X24Typeless | Self::X32TypelessG8X24Uint => 8,
            Self::R16G16B16A16Unorm
            | Self::R16G16B16A16Snorm
            | Self::R16G16B16A16Float
            | Self::R16G16B16A16Uint
            | Self::R16G16B16A16Sint
            | Self::R32G32Float
            | Self::R32G32Uint
            | Self::R32G32Sint => 8,
            Self::R32G32B32Uint | Self::R32G32B32Sint => 12,
            Self::R32G32B32A32Float | Self::R32G32B32A32Uint | Self::R32G32B32A32Sint => 16,
            _ => return None,
        })
    }

    /// Bytes per 4×4 block for block-compressed formats. Returns `None`
    /// for uncompressed formats.
    pub fn block_bytes(self) -> Option<u32> {
        Some(match self {
            Self::Bc1 | Self::Bc4Unorm | Self::Bc4Snorm => 8,
            Self::Bc2
            | Self::Bc3
            | Self::Bc5Unorm
            | Self::Bc5Snorm
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::Bc7Unorm
            | Self::Bc7UnormSrgb => 16,
            _ => return None,
        })
    }

    /// True for the BC1..BC7 family (legacy DXT* aliases included).
    /// ASTC is reported separately by [`Self::astc_footprint`] because
    /// its blocks are not the 4×4 footprint the BC* sizing assumes.
    pub fn is_block_compressed(self) -> bool {
        self.block_bytes().is_some()
    }

    /// For an ASTC format, the `(block_w, block_h)` texel footprint;
    /// `None` for every other format. Each ASTC block is a fixed 16
    /// bytes regardless of footprint.
    pub fn astc_footprint(self) -> Option<(u32, u32)> {
        match self {
            Self::Astc {
                block_w, block_h, ..
            } => Some((block_w, block_h)),
            _ => None,
        }
    }

    /// Short human-readable name (used in error messages).
    pub fn name(self) -> &'static str {
        match self {
            Self::A8R8G8B8 => "A8R8G8B8",
            Self::X8R8G8B8 => "X8R8G8B8",
            Self::A8B8G8R8 => "A8B8G8R8",
            Self::X8B8G8R8 => "X8B8G8R8",
            Self::R5G6B5 => "R5G6B5",
            Self::A1R5G5B5 => "A1R5G5B5",
            Self::X1R5G5B5 => "X1R5G5B5",
            Self::A4R4G4B4 => "A4R4G4B4",
            Self::X4R4G4B4 => "X4R4G4B4",
            Self::R8G8B8 => "R8G8B8",
            Self::A8L8 => "A8L8",
            Self::L16 => "L16",
            Self::A4L4 => "A4L4",
            Self::L8 => "L8",
            Self::A8 => "A8",
            Self::A8R3G3B2 => "A8R3G3B2",
            Self::A4B4G4R4Unorm => "A4B4G4R4_UNORM",
            Self::Bc1 => "BC1",
            Self::Bc2 => "BC2",
            Self::Bc3 => "BC3",
            Self::Bc4Unorm => "BC4_UNORM",
            Self::Bc4Snorm => "BC4_SNORM",
            Self::Bc5Unorm => "BC5_UNORM",
            Self::Bc5Snorm => "BC5_SNORM",
            Self::Bc6hUf16 => "BC6H_UF16",
            Self::Bc6hSf16 => "BC6H_SF16",
            Self::Bc7Unorm => "BC7_UNORM",
            Self::Bc7UnormSrgb => "BC7_UNORM_SRGB",
            Self::R16G16B16A16Unorm => "R16G16B16A16_UNORM",
            Self::R16G16B16A16Snorm => "R16G16B16A16_SNORM",
            Self::R16Float => "R16_FLOAT",
            Self::R16G16Float => "R16G16_FLOAT",
            Self::R16G16B16A16Float => "R16G16B16A16_FLOAT",
            Self::R32Float => "R32_FLOAT",
            Self::R32G32Float => "R32G32_FLOAT",
            Self::R32G32B32A32Float => "R32G32B32A32_FLOAT",
            Self::R10G10B10A2Unorm => "R10G10B10A2_UNORM",
            Self::R10G10B10A2Uint => "R10G10B10A2_UINT",
            Self::R11G11B10Float => "R11G11B10_FLOAT",
            Self::R9G9B9E5SharedExp => "R9G9B9E5_SHAREDEXP",
            Self::A2R10G10B10 => "A2R10G10B10",
            Self::R8G8B8G8Unorm => "R8G8_B8G8_UNORM",
            Self::G8R8G8B8Unorm => "G8R8_G8B8_UNORM",
            Self::R16Uint => "R16_UINT",
            Self::R16Sint => "R16_SINT",
            Self::R16G16Uint => "R16G16_UINT",
            Self::R16G16Sint => "R16G16_SINT",
            Self::R16G16B16A16Uint => "R16G16B16A16_UINT",
            Self::R16G16B16A16Sint => "R16G16B16A16_SINT",
            Self::R8Uint => "R8_UINT",
            Self::R8Sint => "R8_SINT",
            Self::R8G8Uint => "R8G8_UINT",
            Self::R8G8Sint => "R8G8_SINT",
            Self::R8G8B8A8Uint => "R8G8B8A8_UINT",
            Self::R8G8B8A8Sint => "R8G8B8A8_SINT",
            Self::R32Uint => "R32_UINT",
            Self::R32Sint => "R32_SINT",
            Self::R32G32Uint => "R32G32_UINT",
            Self::R32G32Sint => "R32G32_SINT",
            Self::R32G32B32Uint => "R32G32B32_UINT",
            Self::R32G32B32Sint => "R32G32B32_SINT",
            Self::R32G32B32A32Uint => "R32G32B32A32_UINT",
            Self::R32G32B32A32Sint => "R32G32B32A32_SINT",
            Self::R8Unorm => "R8_UNORM",
            Self::R8Snorm => "R8_SNORM",
            Self::R8G8Snorm => "R8G8_SNORM",
            Self::R8G8B8A8Snorm => "R8G8B8A8_SNORM",
            Self::R16Unorm => "R16_UNORM",
            Self::R16Snorm => "R16_SNORM",
            Self::R16G16Unorm => "R16G16_UNORM",
            Self::R16G16Snorm => "R16G16_SNORM",
            Self::D16Unorm => "D16_UNORM",
            Self::D32Float => "D32_FLOAT",
            Self::D24UnormS8Uint => "D24_UNORM_S8_UINT",
            Self::D32FloatS8X24Uint => "D32_FLOAT_S8X24_UINT",
            Self::R24UnormX8Typeless => "R24_UNORM_X8_TYPELESS",
            Self::X24TypelessG8Uint => "X24_TYPELESS_G8_UINT",
            Self::R32FloatX8X24Typeless => "R32_FLOAT_X8X24_TYPELESS",
            Self::X32TypelessG8X24Uint => "X32_TYPELESS_G8X24_UINT",
            Self::Astc { srgb, .. } => {
                if srgb {
                    "ASTC_LDR_SRGB"
                } else {
                    "ASTC_LDR"
                }
            }
            Self::Yuv(f) => match f {
                crate::yuv::YuvFormat::Ayuv => "AYUV",
                crate::yuv::YuvFormat::Y410 => "Y410",
                crate::yuv::YuvFormat::Y416 => "Y416",
                crate::yuv::YuvFormat::Nv12 => "NV12",
                crate::yuv::YuvFormat::P010 => "P010",
                crate::yuv::YuvFormat::P016 => "P016",
                crate::yuv::YuvFormat::Opaque420 => "420_OPAQUE",
                crate::yuv::YuvFormat::Yuy2 => "YUY2",
                crate::yuv::YuvFormat::Uyvy => "UYVY",
                crate::yuv::YuvFormat::Y210 => "Y210",
                crate::yuv::YuvFormat::Y216 => "Y216",
                crate::yuv::YuvFormat::Nv11 => "NV11",
            },
        }
    }

    /// Number of channels (1, 2, or 4) for the extended high-bit-depth /
    /// floating-point uncompressed layouts; `None` for any other format.
    pub fn channel_count(self) -> Option<u32> {
        Some(match self {
            Self::R16Float | Self::R32Float => 1,
            Self::R16G16Float | Self::R32G32Float => 2,
            Self::R16G16B16A16Unorm
            | Self::R16G16B16A16Snorm
            | Self::R16G16B16A16Float
            | Self::R32G32B32A32Float
            | Self::R10G10B10A2Unorm
            | Self::R10G10B10A2Uint => 4,
            _ => return None,
        })
    }
}

impl SurfaceFormat {
    /// The contract layout [`crate::decode`] expands a surface of this
    /// format into, or `None` for the formats that stay depth-API-only
    /// (plain-integer `_UINT` / `_SINT`, depth-stencil and the typeless
    /// views over them, the YUV family). For those,
    /// [`crate::decode`] / [`crate::info`] return
    /// [`crate::DdsError::Unsupported`] and the per-format
    /// `decode_*_surface` helpers remain the way in.
    ///
    /// | Stored layout | Contract layout | Expansion |
    /// |---|---|---|
    /// | `A8R8G8B8`, `X8R8G8B8` | `Bgra` | byte copy; the unused `X` byte reads as `0xFF` |
    /// | `A8B8G8R8`, `X8B8G8R8` | `Rgba` | byte copy; `X` → `0xFF` |
    /// | `R8G8B8` | `Bgr24` | byte copy |
    /// | `R5G6B5`, `A1R5G5B5`, `X1R5G5B5`, `A4R4G4B4`, `X4R4G4B4`, `A8R3G3B2`, `A4B4G4R4_UNORM` | `Rgba` | bit replication to 8 bits; 1-bit alpha → `0` / `0xFF`; `X` → `0xFF` |
    /// | `A8` | `Rgba` | `[0, 0, 0, a]` (a colourless alpha mask) |
    /// | `L8`, `R8_UNORM`, decoded BC4 (unsigned) | `Gray8` | byte copy |
    /// | `A8L8` | `Ya8` | byte copy; `A4L4` by nibble replication |
    /// | `L16`, `R16_UNORM` | `Gray16Le` | byte copy |
    /// | `R16G16B16A16_UNORM` | `Rgba64Le` | byte copy; `R16G16_UNORM` with `B = 0`, `A = 0xFFFF` |
    /// | `R10G10B10A2_UNORM`, `A2R10G10B10` | `Rgba64Le` | 10 → 16 and 2 → 16 bits by bit replication |
    /// | `R8G8_B8G8_UNORM`, `G8R8_G8B8_UNORM` | `Rgba` | shared R / B duplicated across the pixel pair |
    /// | BC1, BC2, BC3, BC7 (`_UNORM` and `_UNORM_SRGB`), ASTC LDR | `Rgba` | block decode |
    /// | BC5 (unsigned) | `Rgba` | `[r, g, 0, 0xFF]` |
    /// | `R16_FLOAT`, `R32_FLOAT` | `GrayF32Le` | `half` → `f32` |
    /// | `R11G11B10_FLOAT`, `R9G9B9E5_SHAREDEXP` | `RgbF32Le` | packed float → `f32` |
    /// | `R16G16B16A16_FLOAT`, `R32G32B32A32_FLOAT`, BC6H (UF16 / SF16) | `RgbaF32Le` | `half` → `f32` |
    /// | `R16G16_FLOAT`, `R32G32_FLOAT` | `RgbaF32Le` | `[r, g, 0.0, 1.0]` |
    /// | `R8_SNORM`, `R16_SNORM`, BC4 (signed) | `GrayF32Le` | `v / (2^(n-1) - 1)`, clamped to `-1` |
    /// | `R8G8_SNORM`, `R16G16_SNORM`, BC5 (signed) | `RgbaF32Le` | as above, `[r, g, 0.0, 1.0]` |
    /// | `R8G8B8A8_SNORM`, `R16G16B16A16_SNORM` | `RgbaF32Le` | as above |
    ///
    /// Missing colour channels read as `0` and a missing alpha channel
    /// as fully opaque, the Direct3D sampling convention.
    pub fn contract_format(self) -> Option<crate::image::DdsPixelFormat> {
        use crate::image::DdsPixelFormat as P;
        Some(match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 => P::Bgra,
            Self::A8B8G8R8
            | Self::X8B8G8R8
            | Self::R5G6B5
            | Self::A1R5G5B5
            | Self::X1R5G5B5
            | Self::A4R4G4B4
            | Self::X4R4G4B4
            | Self::A8R3G3B2
            | Self::A4B4G4R4Unorm
            | Self::A8
            | Self::R8G8B8G8Unorm
            | Self::G8R8G8B8Unorm
            | Self::Bc1
            | Self::Bc2
            | Self::Bc3
            | Self::Bc5Unorm
            | Self::Bc7Unorm
            | Self::Bc7UnormSrgb
            | Self::Astc { .. } => P::Rgba,
            Self::R8G8B8 => P::Bgr24,
            Self::L8 | Self::R8Unorm | Self::Bc4Unorm => P::Gray8,
            Self::A8L8 | Self::A4L4 => P::Ya8,
            Self::L16 | Self::R16Unorm => P::Gray16Le,
            Self::R16G16B16A16Unorm
            | Self::R16G16Unorm
            | Self::R10G10B10A2Unorm
            | Self::A2R10G10B10 => P::Rgba64Le,
            Self::R16Float | Self::R32Float | Self::R8Snorm | Self::R16Snorm | Self::Bc4Snorm => {
                P::GrayF32Le
            }
            Self::R11G11B10Float | Self::R9G9B9E5SharedExp => P::RgbF32Le,
            Self::R16G16Float
            | Self::R32G32Float
            | Self::R16G16B16A16Float
            | Self::R32G32B32A32Float
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::R8G8Snorm
            | Self::R16G16Snorm
            | Self::Bc5Snorm
            | Self::R8G8B8A8Snorm
            | Self::R16G16B16A16Snorm => P::RgbaF32Le,
            _ => return None,
        })
    }

    /// `true` when the stored layout carries an alpha channel that the
    /// contract layout preserves (`X`-padded and alpha-less layouts are
    /// `false` even though they expand to a four-channel layout).
    pub fn has_alpha(self) -> bool {
        matches!(
            self,
            Self::A8R8G8B8
                | Self::A8B8G8R8
                | Self::A1R5G5B5
                | Self::A4R4G4B4
                | Self::A8R3G3B2
                | Self::A4B4G4R4Unorm
                | Self::A8
                | Self::A8L8
                | Self::A4L4
                | Self::Bc1
                | Self::Bc2
                | Self::Bc3
                | Self::Bc7Unorm
                | Self::Bc7UnormSrgb
                | Self::Astc { .. }
                | Self::R16G16B16A16Unorm
                | Self::R16G16B16A16Snorm
                | Self::R10G10B10A2Unorm
                | Self::A2R10G10B10
                | Self::R16G16B16A16Float
                | Self::R32G32B32A32Float
                | Self::R8G8B8A8Snorm
        )
    }
}

impl SurfaceFormat {
    /// Every unit variant (all layouts except the parameterised
    /// [`Self::Astc`] and [`Self::Yuv`]), in declaration order.
    pub const ALL: &'static [SurfaceFormat] = &[
        Self::A8R8G8B8,
        Self::X8R8G8B8,
        Self::A8B8G8R8,
        Self::X8B8G8R8,
        Self::R5G6B5,
        Self::A1R5G5B5,
        Self::X1R5G5B5,
        Self::A4R4G4B4,
        Self::X4R4G4B4,
        Self::R8G8B8,
        Self::A8L8,
        Self::L16,
        Self::A4L4,
        Self::L8,
        Self::A8,
        Self::A8R3G3B2,
        Self::A4B4G4R4Unorm,
        Self::Bc1,
        Self::Bc2,
        Self::Bc3,
        Self::Bc4Unorm,
        Self::Bc4Snorm,
        Self::Bc5Unorm,
        Self::Bc5Snorm,
        Self::Bc6hUf16,
        Self::Bc6hSf16,
        Self::Bc7Unorm,
        Self::Bc7UnormSrgb,
        Self::R16G16B16A16Unorm,
        Self::R16G16B16A16Snorm,
        Self::R16Float,
        Self::R16G16Float,
        Self::R16G16B16A16Float,
        Self::R32Float,
        Self::R32G32Float,
        Self::R32G32B32A32Float,
        Self::R10G10B10A2Unorm,
        Self::R10G10B10A2Uint,
        Self::R11G11B10Float,
        Self::R9G9B9E5SharedExp,
        Self::A2R10G10B10,
        Self::R8G8B8G8Unorm,
        Self::G8R8G8B8Unorm,
        Self::R16Uint,
        Self::R16Sint,
        Self::R16G16Uint,
        Self::R16G16Sint,
        Self::R16G16B16A16Uint,
        Self::R16G16B16A16Sint,
        Self::R8Uint,
        Self::R8Sint,
        Self::R8G8Uint,
        Self::R8G8Sint,
        Self::R8G8B8A8Uint,
        Self::R8G8B8A8Sint,
        Self::R32Uint,
        Self::R32Sint,
        Self::R32G32Uint,
        Self::R32G32Sint,
        Self::R32G32B32Uint,
        Self::R32G32B32Sint,
        Self::R32G32B32A32Uint,
        Self::R32G32B32A32Sint,
        Self::R8Unorm,
        Self::R8Snorm,
        Self::R8G8Snorm,
        Self::R8G8B8A8Snorm,
        Self::R16Unorm,
        Self::R16Snorm,
        Self::R16G16Unorm,
        Self::R16G16Snorm,
        Self::D16Unorm,
        Self::D32Float,
        Self::D24UnormS8Uint,
        Self::D32FloatS8X24Uint,
        Self::R24UnormX8Typeless,
        Self::X24TypelessG8Uint,
        Self::R32FloatX8X24Typeless,
        Self::X32TypelessG8X24Uint,
    ];

    /// The inverse of [`Self::name`]: parse a layout from its short
    /// name (`"A8R8G8B8"`, `"BC7_UNORM"`, `"R32G32B32A32_FLOAT"`, …).
    /// ASTC footprints spell as `"ASTC_4x4"` / `"ASTC_4x4_SRGB"`; the
    /// YUV layouts use their DXGI names (`"NV12"`, `"YUY2"`, …).
    /// Case-sensitive; `None` for an unknown name.
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(f) = Self::ALL.iter().find(|f| f.name() == name) {
            return Some(*f);
        }
        if let Some(rest) = name.strip_prefix("ASTC_") {
            let (fp, srgb) = match rest.strip_suffix("_SRGB") {
                Some(fp) => (fp, true),
                None => (rest, false),
            };
            let (w, h) = fp.split_once('x')?;
            let block_w: u32 = w.parse().ok()?;
            let block_h: u32 = h.parse().ok()?;
            if !crate::astc::is_valid_footprint(block_w as u8, block_h as u8) {
                return None;
            }
            return Some(Self::Astc {
                block_w,
                block_h,
                srgb,
            });
        }
        use crate::yuv::YuvFormat as Y;
        let y = match name {
            "AYUV" => Y::Ayuv,
            "Y410" => Y::Y410,
            "Y416" => Y::Y416,
            "NV12" => Y::Nv12,
            "P010" => Y::P010,
            "P016" => Y::P016,
            "420_OPAQUE" => Y::Opaque420,
            "YUY2" => Y::Yuy2,
            "UYVY" => Y::Uyvy,
            "Y210" => Y::Y210,
            "Y216" => Y::Y216,
            "NV11" => Y::Nv11,
            _ => return None,
        };
        Some(Self::Yuv(y))
    }
}

/// Cubemap face identifier for a [`DdsSurface`]. Order mirrors
/// Microsoft's `DDS_CUBEMAP_*` flag bit positions: +X / -X / +Y / -Y /
/// +Z / -Z.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CubemapFace {
    /// `DDSCAPS2_CUBEMAP_POSITIVEX` — +X (right).
    PositiveX,
    /// `DDSCAPS2_CUBEMAP_NEGATIVEX` — -X (left).
    NegativeX,
    /// `DDSCAPS2_CUBEMAP_POSITIVEY` — +Y (top).
    PositiveY,
    /// `DDSCAPS2_CUBEMAP_NEGATIVEY` — -Y (bottom).
    NegativeY,
    /// `DDSCAPS2_CUBEMAP_POSITIVEZ` — +Z (front).
    PositiveZ,
    /// `DDSCAPS2_CUBEMAP_NEGATIVEZ` — -Z (back).
    NegativeZ,
}

impl CubemapFace {
    /// All six cubemap faces, in the same order Microsoft writes them
    /// to disk (PX, NX, PY, NY, PZ, NZ).
    pub const ALL: [Self; 6] = [
        Self::PositiveX,
        Self::NegativeX,
        Self::PositiveY,
        Self::NegativeY,
        Self::PositiveZ,
        Self::NegativeZ,
    ];

    /// Short two-character name (e.g. `"+X"`, `"-Z"`).
    pub fn short_name(self) -> &'static str {
        match self {
            Self::PositiveX => "+X",
            Self::NegativeX => "-X",
            Self::PositiveY => "+Y",
            Self::NegativeY => "-Y",
            Self::PositiveZ => "+Z",
            Self::NegativeZ => "-Z",
        }
    }

    /// Position of this face in [`Self::ALL`] (the on-disk order).
    pub fn index(self) -> usize {
        match self {
            Self::PositiveX => 0,
            Self::NegativeX => 1,
            Self::PositiveY => 2,
            Self::NegativeY => 3,
            Self::PositiveZ => 4,
            Self::NegativeZ => 5,
        }
    }
}

/// One stored surface — i.e. one (array_slice, face, mip_level,
/// depth_slice) tuple with its raw bytes in the file's
/// [`SurfaceFormat`]. For a plain 2D texture there is exactly one
/// [`DdsSurface`] in [`DdsFile::surfaces`]; for a mipmapped cubemap with
/// N array slices there are `N × 6 × mip_count`.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DdsSurface {
    /// Width of this surface in pixels (= `file.width >> mip_level`,
    /// floored to 1).
    pub width: u32,
    /// Height of this surface in pixels.
    pub height: u32,
    /// Mip level — 0 for the base level, 1 for half-res, etc.
    pub mip_level: u32,
    /// DX10-array slice index (0 for non-array textures).
    pub array_slice: u32,
    /// Cubemap face — `None` for non-cubemap textures.
    pub face: Option<CubemapFace>,
    /// Volume-texture depth (z) slice index. `0` for 1D / 2D / cubemap
    /// textures. For a volume texture the parser emits one
    /// [`DdsSurface`] per depth slice; `depth_slice` runs `0 ..
    /// depth_at(mip_level)` where `depth_at(m) = max(1, base_depth >> m)`
    /// (Microsoft halves the depth at each mip level alongside width and
    /// height, flooring to 1).
    pub depth_slice: u32,
    /// Raw stored bytes of this surface (one plane; for block-compressed
    /// formats `stride` is one row of blocks).
    pub plane: Plane,
}

impl DdsSurface {
    /// A top-level (mip 0), first-slice, non-cubemap, depth-slice-0
    /// surface; use the `with_*` builders for its position in the tree.
    pub fn new(width: u32, height: u32, plane: Plane) -> Self {
        Self {
            width,
            height,
            mip_level: 0,
            array_slice: 0,
            face: None,
            depth_slice: 0,
            plane,
        }
    }

    /// Set the mip level.
    pub fn with_mip_level(mut self, mip_level: u32) -> Self {
        self.mip_level = mip_level;
        self
    }

    /// Set the array slice.
    pub fn with_array_slice(mut self, array_slice: u32) -> Self {
        self.array_slice = array_slice;
        self
    }

    /// Set (or clear) the cubemap face.
    pub fn with_face(mut self, face: impl Into<Option<CubemapFace>>) -> Self {
        self.face = face.into();
        self
    }

    /// Set the depth slice.
    pub fn with_depth_slice(mut self, depth_slice: u32) -> Self {
        self.depth_slice = depth_slice;
        self
    }
}

/// One parsed DDS file — header facts plus every (array, face, mip,
/// depth) surface the file carries, in its stored [`SurfaceFormat`].
///
/// This is the depth record behind the contract's [`crate::DdsImage`]:
/// [`crate::parse_dds`] produces it, the `encode_dds_*` depth writers
/// consume it, and [`DdsFile::surface`] / [`DdsFile::to_image`] expand
/// any one surface into the contract layout.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct DdsFile {
    /// Picture width in pixels (mip-0).
    pub width: u32,
    /// Picture height in pixels (mip-0).
    pub height: u32,
    /// On-disk pixel layout the surfaces carry.
    pub pixel_format: SurfaceFormat,
    /// Mip-0 / first-face / first-array-slice plane. Kept as a
    /// convenience for callers that don't care about mipmaps,
    /// cubemaps, or texture arrays — mirrors `surfaces[0].plane`. New
    /// code should prefer iterating [`Self::surfaces`].
    pub planes: Vec<Plane>,
    /// Every surface the file carries, in the on-disk order Microsoft
    /// mandates (outer loop over array slice, then over cubemap face,
    /// then over mip level; volume textures: mip, then depth slice).
    ///
    /// For a non-mipmapped 2D texture this is a single-element vector
    /// equivalent to `planes[0]`. For a mipmapped cubemap with N array
    /// slices the length is `N × 6 × mip_map_count`.
    pub surfaces: Vec<DdsSurface>,
    /// Mipmap-level count as declared in the DDS header (1 for
    /// non-mipmapped surfaces).
    pub mip_map_count: u32,
    /// True when the source file used the `DDS_HEADER_DXT10` extension.
    /// Round-trip preserved by the encoder.
    pub has_dxt10_header: bool,
    /// `DXGI_FORMAT` value carried in the DXT10 extension. `None` for
    /// legacy headers. Useful for callers that want to know the BC*
    /// sRGB / unorm / snorm variant precisely.
    pub dxgi_format: Option<crate::types::DxgiFormat>,
    /// True when the source file is a cubemap (`DDSCAPS2_CUBEMAP` set).
    pub is_cubemap: bool,
    /// DX10 texture-array element count (1 for non-array textures, 6
    /// for the per-face slices of a DX10 cubemap, etc.).
    pub array_size: u32,
    /// Volume-texture depth (z) slice count at mip 0. `1` for 1D / 2D /
    /// cubemap textures. When `> 1` the file is a volume (3D) texture:
    /// the legacy header sets `DDSCAPS2_VOLUME` (and `DDSD_DEPTH` in
    /// `flags`), or the DX10 header sets
    /// `resource_dimension == DDS_DIMENSION_TEXTURE3D`. Each mip level
    /// stores `max(1, depth >> mip_level)` consecutive 2D slices, and
    /// [`Self::surfaces`] carries one entry per `(mip_level,
    /// depth_slice)` pair in on-disk order (outer loop over mip, inner
    /// over depth slice).
    pub depth: u32,
}

impl DdsFile {
    /// Assemble a file record from its top-level geometry, stored
    /// layout and surfaces (at least one, in on-disk order; `planes`
    /// mirrors the first). Mip count, cubemap / array / volume shape
    /// and the DX10 facts default to a plain 2D texture — set them with
    /// the `with_*` builders. Rejects zero dimensions and an empty
    /// surface list with [`crate::DdsError::InvalidData`].
    pub fn new(
        width: u32,
        height: u32,
        pixel_format: SurfaceFormat,
        surfaces: Vec<DdsSurface>,
    ) -> crate::error::Result<Self> {
        if width == 0 || height == 0 {
            return Err(crate::DdsError::invalid(format!(
                "DdsFile: zero-sized surface {width}x{height}"
            )));
        }
        let Some(first) = surfaces.first() else {
            return Err(crate::DdsError::invalid(
                "DdsFile: at least one surface is required",
            ));
        };
        Ok(Self {
            width,
            height,
            pixel_format,
            planes: vec![first.plane.clone()],
            surfaces,
            mip_map_count: 1,
            has_dxt10_header: false,
            dxgi_format: None,
            is_cubemap: false,
            array_size: 1,
            depth: 1,
        })
    }

    /// A single-surface 2D texture from one plane.
    pub fn single(
        width: u32,
        height: u32,
        pixel_format: SurfaceFormat,
        plane: Plane,
    ) -> crate::error::Result<Self> {
        Self::new(
            width,
            height,
            pixel_format,
            vec![DdsSurface::new(width, height, plane)],
        )
    }

    /// Set the declared mip-level count.
    pub fn with_mip_map_count(mut self, mip_map_count: u32) -> Self {
        self.mip_map_count = mip_map_count;
        self
    }

    /// Mark the file as a cubemap (six faces per array slice).
    pub fn with_cubemap(mut self, is_cubemap: bool) -> Self {
        self.is_cubemap = is_cubemap;
        self
    }

    /// Set the DX10 texture-array slice count.
    pub fn with_array_size(mut self, array_size: u32) -> Self {
        self.array_size = array_size;
        self
    }

    /// Set the volume-texture depth at mip 0.
    pub fn with_depth(mut self, depth: u32) -> Self {
        self.depth = depth;
        self
    }

    /// Force (or clear) the `DDS_HEADER_DXT10` extension on write.
    pub fn with_dxt10_header(mut self, has_dxt10_header: bool) -> Self {
        self.has_dxt10_header = has_dxt10_header;
        self
    }

    /// Set (or clear) the `DXGI_FORMAT` to write in the DX10 header.
    pub fn with_dxgi_format(
        mut self,
        dxgi_format: impl Into<Option<crate::types::DxgiFormat>>,
    ) -> Self {
        self.dxgi_format = dxgi_format.into();
        self
    }

    /// Find the surface at `(array_slice, face, mip_level, depth_slice)`.
    pub fn surface(
        &self,
        array_slice: u32,
        face: Option<CubemapFace>,
        mip_level: u32,
        depth_slice: u32,
    ) -> Option<&DdsSurface> {
        self.surfaces.iter().find(|s| {
            s.array_slice == array_slice
                && s.face == face
                && s.mip_level == mip_level
                && s.depth_slice == depth_slice
        })
    }

    /// `true` when the file's `DXGI_FORMAT` is one of the
    /// `_UNORM_SRGB` codes (the only colour information DDS carries).
    pub fn is_srgb(&self) -> bool {
        match self.pixel_format {
            SurfaceFormat::Astc { srgb, .. } => srgb,
            SurfaceFormat::Bc7UnormSrgb => true,
            _ => self.dxgi_format.is_some_and(|d| d.is_srgb()),
        }
    }

    /// Colour signalling as [`crate::decode`] reports it: sRGB for the
    /// `_UNORM_SRGB` codes, [`crate::ColorInfo::dds_default`] otherwise.
    pub fn color_info(&self) -> crate::image::ColorInfo {
        if self.is_srgb() {
            crate::image::ColorInfo::srgb()
        } else {
            crate::image::ColorInfo::dds_default()
        }
    }

    /// Expand one stored surface into the contract layout (see
    /// [`SurfaceFormat::contract_format`] for the mapping;
    /// [`crate::DdsError::Unsupported`] for the depth-API-only formats).
    pub fn to_image(&self, surface: &DdsSurface) -> crate::error::Result<crate::image::DdsImage> {
        crate::convert::surface_to_image(self, surface)
    }

    /// The top-level surface (mip 0, first face, first slice) as a
    /// contract image — what [`crate::decode`] returns.
    pub fn primary_image(&self) -> crate::error::Result<crate::image::DdsImage> {
        let first = self
            .surfaces
            .first()
            .ok_or_else(|| crate::DdsError::invalid("DdsFile: no surfaces"))?;
        self.to_image(first)
    }
}
