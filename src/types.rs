//! On-disk DDS header layout constants and helpers.
//!
//! Reference: Microsoft's public "DDS file layout for textures" guide
//! (learn.microsoft.com/windows/win32/direct3ddds/dx-graphics-dds-pguide).
//! Field names below match the public docs (which are themselves the
//! header file Microsoft has published as part of the DirectX docs).
//!
//! The on-disk file is exactly:
//!
//! ```text
//!   bytes 0..=3      ASCII magic "DDS "  (0x20534444 little-endian)
//!   bytes 4..=127    DDS_HEADER         (124 bytes)
//!     - if header.pixel_format.flags & DDPF_FOURCC and
//!       header.pixel_format.four_cc == "DX10":
//!     bytes 128..=147  DDS_HEADER_DXT10 (20 bytes)
//!   pixel data       (rest of file; per-format size)
//! ```

/// ASCII magic that opens every DDS file: `"DDS "` little-endian.
pub const DDS_MAGIC: u32 = 0x2053_4444;

/// Size in bytes of the fixed `DDS_HEADER` struct (Microsoft fixes
/// this at 124 — the value is also stored in `header.size` and the
/// reader rejects mismatches).
pub const DDS_HEADER_SIZE: usize = 124;

/// Size in bytes of the optional `DDS_HEADER_DXT10` struct that
/// follows the main header when `pixel_format.four_cc == "DX10"`.
pub const DDS_HEADER_DXT10_SIZE: usize = 20;

/// Size in bytes of the embedded `DDS_PIXELFORMAT` struct (32 bytes,
/// stored at offset 76 inside the main header). Microsoft fixes this
/// at 32 — also written into `pixel_format.size`.
pub const DDS_PIXELFORMAT_SIZE: usize = 32;

// --- DDS_HEADER.flags bit values -----------------------------------------

/// `DDSD_CAPS` — `caps` field is valid (always required).
pub const DDSD_CAPS: u32 = 0x0000_0001;
/// `DDSD_HEIGHT` — `height` field is valid (always required).
pub const DDSD_HEIGHT: u32 = 0x0000_0002;
/// `DDSD_WIDTH` — `width` field is valid (always required).
pub const DDSD_WIDTH: u32 = 0x0000_0004;
/// `DDSD_PITCH` — `pitch_or_linear_size` is the per-row pitch (uncompressed).
pub const DDSD_PITCH: u32 = 0x0000_0008;
/// `DDSD_PIXELFORMAT` — `pixel_format` is valid (always required).
pub const DDSD_PIXELFORMAT: u32 = 0x0000_1000;
/// `DDSD_MIPMAPCOUNT` — `mip_map_count` is valid.
pub const DDSD_MIPMAPCOUNT: u32 = 0x0002_0000;
/// `DDSD_LINEARSIZE` — `pitch_or_linear_size` is the total surface size in bytes (compressed).
pub const DDSD_LINEARSIZE: u32 = 0x0008_0000;
/// `DDSD_DEPTH` — `depth` is valid (volume textures).
pub const DDSD_DEPTH: u32 = 0x0080_0000;

/// The four flag bits Microsoft requires every DDS file to set in
/// `DDS_HEADER.flags` (caps, height, width, pixel_format).
pub const DDSD_REQUIRED: u32 = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;

// --- DDS_PIXELFORMAT.flags bit values ------------------------------------

/// `DDPF_ALPHAPIXELS` — `rgb_bit_count` includes alpha (`a_bit_mask` valid).
pub const DDPF_ALPHAPIXELS: u32 = 0x0000_0001;
/// `DDPF_ALPHA` — alpha-only surface (`a_bit_mask` valid, no RGB masks).
pub const DDPF_ALPHA: u32 = 0x0000_0002;
/// `DDPF_FOURCC` — `four_cc` carries a compressed-format identifier.
pub const DDPF_FOURCC: u32 = 0x0000_0004;
/// `DDPF_RGB` — `r_bit_mask` / `g_bit_mask` / `b_bit_mask` valid (uncompressed).
pub const DDPF_RGB: u32 = 0x0000_0040;
/// `DDPF_YUV` — RGB masks carry YUV plane channel masks (rare).
pub const DDPF_YUV: u32 = 0x0000_0200;
/// `DDPF_LUMINANCE` — `r_bit_mask` carries the luminance channel mask.
pub const DDPF_LUMINANCE: u32 = 0x0002_0000;

// --- DDS_HEADER.caps bit values ------------------------------------------

/// `DDSCAPS_COMPLEX` — surface has child surfaces (mipmaps, cubemap faces).
pub const DDSCAPS_COMPLEX: u32 = 0x0000_0008;
/// `DDSCAPS_TEXTURE` — every DDS file has this set.
pub const DDSCAPS_TEXTURE: u32 = 0x0000_1000;
/// `DDSCAPS_MIPMAP` — surface carries mipmap levels.
pub const DDSCAPS_MIPMAP: u32 = 0x0040_0000;

// --- DDS_HEADER.caps2 bit values -----------------------------------------

/// `DDSCAPS2_CUBEMAP` — surface is a cubemap (6 faces).
pub const DDSCAPS2_CUBEMAP: u32 = 0x0000_0200;
/// `DDSCAPS2_CUBEMAP_POSITIVEX`.
pub const DDSCAPS2_CUBEMAP_POSITIVEX: u32 = 0x0000_0400;
/// `DDSCAPS2_CUBEMAP_NEGATIVEX`.
pub const DDSCAPS2_CUBEMAP_NEGATIVEX: u32 = 0x0000_0800;
/// `DDSCAPS2_CUBEMAP_POSITIVEY`.
pub const DDSCAPS2_CUBEMAP_POSITIVEY: u32 = 0x0000_1000;
/// `DDSCAPS2_CUBEMAP_NEGATIVEY`.
pub const DDSCAPS2_CUBEMAP_NEGATIVEY: u32 = 0x0000_2000;
/// `DDSCAPS2_CUBEMAP_POSITIVEZ`.
pub const DDSCAPS2_CUBEMAP_POSITIVEZ: u32 = 0x0000_4000;
/// `DDSCAPS2_CUBEMAP_NEGATIVEZ`.
pub const DDSCAPS2_CUBEMAP_NEGATIVEZ: u32 = 0x0000_8000;
/// All six cubemap-face presence bits OR'd together (positive X..negative Z).
pub const DDSCAPS2_CUBEMAP_ALL_FACES: u32 = DDSCAPS2_CUBEMAP_POSITIVEX
    | DDSCAPS2_CUBEMAP_NEGATIVEX
    | DDSCAPS2_CUBEMAP_POSITIVEY
    | DDSCAPS2_CUBEMAP_NEGATIVEY
    | DDSCAPS2_CUBEMAP_POSITIVEZ
    | DDSCAPS2_CUBEMAP_NEGATIVEZ;
/// `DDSCAPS2_VOLUME` — surface is a volume texture (`depth` is the slice count).
pub const DDSCAPS2_VOLUME: u32 = 0x0020_0000;

// --- DDS_HEADER_DXT10.resource_dimension values --------------------------

/// `DDS_DIMENSION_TEXTURE1D` — DX10 1D texture.
pub const DDS_DIMENSION_TEXTURE1D: u32 = 2;
/// `DDS_DIMENSION_TEXTURE2D` — DX10 2D texture (the common case for DDS files).
pub const DDS_DIMENSION_TEXTURE2D: u32 = 3;
/// `DDS_DIMENSION_TEXTURE3D` — DX10 3D / volume texture.
pub const DDS_DIMENSION_TEXTURE3D: u32 = 4;

// --- DDS_HEADER_DXT10.misc_flag bit values -------------------------------

/// `DDS_RESOURCE_MISC_TEXTURECUBE` — DX10 surface is a cubemap.
pub const DDS_RESOURCE_MISC_TEXTURECUBE: u32 = 0x0000_0004;

// --- FourCC constants for legacy block-compressed formats ----------------

/// FourCC `'DXT1'` — BC1 (4 bpp, 8 bytes per 4×4 block, 1-bit alpha).
pub const FOURCC_DXT1: u32 = make_fourcc(b"DXT1");
/// FourCC `'DXT2'` — BC2 with premultiplied alpha (rare).
pub const FOURCC_DXT2: u32 = make_fourcc(b"DXT2");
/// FourCC `'DXT3'` — BC2 (8 bpp, 16 bytes per block, 4-bit explicit alpha).
pub const FOURCC_DXT3: u32 = make_fourcc(b"DXT3");
/// FourCC `'DXT4'` — BC3 with premultiplied alpha (rare).
pub const FOURCC_DXT4: u32 = make_fourcc(b"DXT4");
/// FourCC `'DXT5'` — BC3 (8 bpp, 16 bytes per block, interpolated alpha).
pub const FOURCC_DXT5: u32 = make_fourcc(b"DXT5");
/// FourCC `'BC4U'` — BC4 unorm (4 bpp, 8 bytes per block, single channel).
pub const FOURCC_BC4U: u32 = make_fourcc(b"BC4U");
/// FourCC `'BC4S'` — BC4 snorm.
pub const FOURCC_BC4S: u32 = make_fourcc(b"BC4S");
/// FourCC `'ATI1'` — alias for BC4 unorm (legacy name).
pub const FOURCC_ATI1: u32 = make_fourcc(b"ATI1");
/// FourCC `'BC5U'` — BC5 unorm (8 bpp, 16 bytes per block, two channels).
pub const FOURCC_BC5U: u32 = make_fourcc(b"BC5U");
/// FourCC `'BC5S'` — BC5 snorm.
pub const FOURCC_BC5S: u32 = make_fourcc(b"BC5S");
/// FourCC `'ATI2'` — alias for BC5 unorm (legacy name).
pub const FOURCC_ATI2: u32 = make_fourcc(b"ATI2");
/// FourCC `'DX10'` — directs the reader to consume a `DDS_HEADER_DXT10`
/// extension (which then carries the real DXGI format identifier).
pub const FOURCC_DX10: u32 = make_fourcc(b"DX10");
/// FourCC `'RGBG'` — `D3DFMT_R8G8_B8G8` / DXGI `R8G8_B8G8_UNORM` (value
/// 68). The horizontally sub-sampled packed RGB layout where each
/// 32-bit block describes a pixel pair sharing R/B with independent G.
pub const FOURCC_RGBG: u32 = make_fourcc(b"RGBG");
/// FourCC `'GRGB'` — `D3DFMT_G8R8_G8B8` / DXGI `G8R8_G8B8_UNORM` (value
/// 69). The channel-reordered sibling of [`FOURCC_RGBG`].
pub const FOURCC_GRGB: u32 = make_fourcc(b"GRGB");
/// FourCC `'YUY2'` — `D3DFMT_YUY2` / DXGI `YUY2` 4:2:2 packed luma /
/// chroma. Each 32-bit block carries `[Y0, U, Y1, V]` for a horizontal
/// pixel pair sharing the chroma samples.
pub const FOURCC_YUY2: u32 = make_fourcc(b"YUY2");
/// FourCC `'UYVY'` — `D3DFMT_UYVY` 4:2:2 packed luma / chroma. The
/// byte-swizzled sibling of [`FOURCC_YUY2`]: each 32-bit block carries
/// `[U, Y0, V, Y1]` for a horizontal pixel pair sharing the chroma.
pub const FOURCC_UYVY: u32 = make_fourcc(b"UYVY");

// --- Legacy `D3DFMT` numeric FourCC codes -------------------------------
//
// Microsoft's "DDS pixel format" + programming-guide pages list a set of
// extended uncompressed layouts whose `four_cc` field holds a numeric
// `D3DFORMAT` enumeration value (rather than a four-character ASCII tag).
// A robust DDS reader must accept these legacy codes; the values below
// are the ones Microsoft tabulates for the high-bit-depth and
// floating-point formats.

/// `D3DFMT_A16B16G16R16` (numeric FourCC 36) — DXGI `R16G16B16A16_UNORM`.
pub const D3DFMT_A16B16G16R16: u32 = 36;
/// `D3DFMT_Q16W16V16U16` (numeric FourCC 110) — DXGI `R16G16B16A16_SNORM`.
pub const D3DFMT_Q16W16V16U16: u32 = 110;
/// `D3DFMT_R16F` (numeric FourCC 111) — DXGI `R16_FLOAT`.
pub const D3DFMT_R16F: u32 = 111;
/// `D3DFMT_G16R16F` (numeric FourCC 112) — DXGI `R16G16_FLOAT`.
pub const D3DFMT_G16R16F: u32 = 112;
/// `D3DFMT_A16B16G16R16F` (numeric FourCC 113) — DXGI `R16G16B16A16_FLOAT`.
pub const D3DFMT_A16B16G16R16F: u32 = 113;
/// `D3DFMT_R32F` (numeric FourCC 114) — DXGI `R32_FLOAT`.
pub const D3DFMT_R32F: u32 = 114;
/// `D3DFMT_G32R32F` (numeric FourCC 115) — DXGI `R32G32_FLOAT`.
pub const D3DFMT_G32R32F: u32 = 115;
/// `D3DFMT_A32B32G32R32F` (numeric FourCC 116) — DXGI `R32G32B32A32_FLOAT`.
pub const D3DFMT_A32B32G32R32F: u32 = 116;

/// Pack a four-byte little-endian ASCII tag into the 32-bit FourCC layout
/// Microsoft uses (`DDS_PIXELFORMAT.four_cc`).
pub const fn make_fourcc(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes([s[0], s[1], s[2], s[3]])
}

/// In-memory mirror of the on-disk `DDS_PIXELFORMAT` (32 bytes).
///
/// All fields are little-endian on disk; this struct stores the
/// already-decoded host-endian values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsPixelFormatHeader {
    /// Always 32 (matches [`DDS_PIXELFORMAT_SIZE`]).
    pub size: u32,
    /// `DDPF_*` flag bits (alpha / RGB / fourcc / luminance).
    pub flags: u32,
    /// Compressed-format tag (valid only when `flags & DDPF_FOURCC != 0`).
    pub four_cc: u32,
    /// Total pixel size in bits (uncompressed).
    pub rgb_bit_count: u32,
    /// Red / luminance channel mask (uncompressed).
    pub r_bit_mask: u32,
    /// Green channel mask (uncompressed).
    pub g_bit_mask: u32,
    /// Blue channel mask (uncompressed).
    pub b_bit_mask: u32,
    /// Alpha channel mask (when `flags & DDPF_ALPHAPIXELS` or `DDPF_ALPHA`).
    pub a_bit_mask: u32,
}

/// In-memory mirror of the on-disk fixed-layout `DDS_HEADER` (124 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsHeader {
    /// Always 124 (matches [`DDS_HEADER_SIZE`]).
    pub size: u32,
    /// `DDSD_*` flag bits indicating which fields are valid.
    pub flags: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// Picture width in pixels.
    pub width: u32,
    /// Either per-row pitch (uncompressed) or total surface size (compressed).
    pub pitch_or_linear_size: u32,
    /// Slice count for volume textures (`flags & DDSD_DEPTH`).
    pub depth: u32,
    /// Mipmap-level count (`flags & DDSD_MIPMAPCOUNT`).
    pub mip_map_count: u32,
    /// Reserved — 11 × `u32`. Preserved on round-trip but otherwise ignored.
    pub reserved1: [u32; 11],
    /// Embedded pixel-format descriptor.
    pub pixel_format: DdsPixelFormatHeader,
    /// `DDSCAPS_*` capability bits.
    pub caps: u32,
    /// `DDSCAPS2_*` capability bits (cubemap face presence + volume).
    pub caps2: u32,
    /// Reserved (`caps3`, `caps4`, `reserved2`).
    pub caps3: u32,
    /// Reserved.
    pub caps4: u32,
    /// Reserved.
    pub reserved2: u32,
}

/// In-memory mirror of the optional `DDS_HEADER_DXT10` (20 bytes).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DdsHeaderDxt10 {
    /// `DXGI_FORMAT` identifier — see [`DxgiFormat`].
    pub dxgi_format: u32,
    /// Resource dimension (`DDS_DIMENSION_TEXTURE{1,2,3}D`).
    pub resource_dimension: u32,
    /// Misc flags (`DDS_RESOURCE_MISC_TEXTURECUBE`, ...).
    pub misc_flag: u32,
    /// Array element count (1 unless this is a texture array).
    pub array_size: u32,
    /// Misc flags 2 — alpha-mode bits (straight / premultiplied / opaque / custom).
    pub misc_flags2: u32,
}

/// Round-2 enumeration of the public Microsoft `DXGI_FORMAT` table.
///
/// Numeric values are taken from the public DXGI reference (every
/// value mirrored verbatim from
/// `learn.microsoft.com/.../dxgi_format`). Anything not enumerated
/// below is preserved as a raw `u32` in
/// [`crate::image::DdsImage::dxgi_format`] and reported as an
/// `Unknown(u32)` value via [`DxgiFormat::from_u32`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
#[allow(missing_docs)]
pub enum DxgiFormat {
    /// `DXGI_FORMAT_UNKNOWN` (0) when the raw value is 0; otherwise
    /// any value the round-2 reader does not enumerate by name.
    Unknown(u32),

    R32G32B32A32Typeless,
    R32G32B32A32Float,
    R32G32B32A32Uint,
    R32G32B32A32Sint,
    R32G32B32Typeless,
    R32G32B32Float,
    R32G32B32Uint,
    R32G32B32Sint,
    R16G16B16A16Typeless,
    R16G16B16A16Float,
    R16G16B16A16Unorm,
    R16G16B16A16Uint,
    R16G16B16A16Snorm,
    R16G16B16A16Sint,
    R32G32Typeless,
    R32G32Float,
    R32G32Uint,
    R32G32Sint,
    R32G8X24Typeless,
    D32FloatS8X24Uint,
    R32FloatX8X24Typeless,
    X32TypelessG8X24Uint,
    R10G10B10A2Typeless,
    R10G10B10A2Unorm,
    R10G10B10A2Uint,
    R11G11B10Float,
    R8G8B8A8Typeless,
    R8G8B8A8Unorm,
    R8G8B8A8UnormSrgb,
    R8G8B8A8Uint,
    R8G8B8A8Snorm,
    R8G8B8A8Sint,
    R16G16Typeless,
    R16G16Float,
    R16G16Unorm,
    R16G16Uint,
    R16G16Snorm,
    R16G16Sint,
    R32Typeless,
    D32Float,
    R32Float,
    R32Uint,
    R32Sint,
    R24G8Typeless,
    D24UnormS8Uint,
    R24UnormX8Typeless,
    X24TypelessG8Uint,
    R8G8Typeless,
    R8G8Unorm,
    R8G8Uint,
    R8G8Snorm,
    R8G8Sint,
    R16Typeless,
    R16Float,
    D16Unorm,
    R16Unorm,
    R16Uint,
    R16Snorm,
    R16Sint,
    R8Typeless,
    R8Unorm,
    R8Uint,
    R8Snorm,
    R8Sint,
    A8Unorm,
    R1Unorm,
    R9G9B9E5Sharedexp,
    R8G8B8G8Unorm,
    G8R8G8B8Unorm,
    Bc1Typeless,
    Bc1Unorm,
    Bc1UnormSrgb,
    Bc2Typeless,
    Bc2Unorm,
    Bc2UnormSrgb,
    Bc3Typeless,
    Bc3Unorm,
    Bc3UnormSrgb,
    Bc4Typeless,
    Bc4Unorm,
    Bc4Snorm,
    Bc5Typeless,
    Bc5Unorm,
    Bc5Snorm,
    B5G6R5Unorm,
    B5G5R5A1Unorm,
    B8G8R8A8Unorm,
    B8G8R8X8Unorm,
    R10G10B10XrBiasA2Unorm,
    B8G8R8A8Typeless,
    B8G8R8A8UnormSrgb,
    B8G8R8X8Typeless,
    B8G8R8X8UnormSrgb,
    Bc6hTypeless,
    Bc6hUf16,
    Bc6hSf16,
    Bc7Typeless,
    Bc7Unorm,
    Bc7UnormSrgb,
    Ayuv,
    Y410,
    Y416,
    Nv12,
    P010,
    P016,
    Opaque420,
    Yuy2,
    Y210,
    Y216,
    Nv11,
    Ai44,
    Ia44,
    P8,
    A8P8,
    B4G4R4A4Unorm,
    P208,
    V208,
    V408,
    /// `DXGI_FORMAT_SAMPLER_FEEDBACK_MIN_MIP_OPAQUE` (value 189). An
    /// opaque sampler-feedback map whose byte layout Microsoft leaves
    /// unspecified ("TBD" in the public enumeration); recognised for
    /// lossless round-trip of the format code only, no decode.
    SamplerFeedbackMinMipOpaque,
    /// `DXGI_FORMAT_SAMPLER_FEEDBACK_MIP_REGION_USED_OPAQUE` (value 190).
    /// The second opaque sampler-feedback map; layout likewise "TBD",
    /// recognised for round-trip only.
    SamplerFeedbackMipRegionUsedOpaque,
    /// `DXGI_FORMAT_A4B4G4R4_UNORM` (value 191). A four-component,
    /// 16-bit unsigned-normalised layout with 4 bits per channel
    /// including alpha. Following the "first named component in the
    /// least-significant bits" rule the packed little-endian word is
    /// `RRRR GGGG BBBB AAAA` (alpha in bits 0..=3, blue 4..=7, green
    /// 8..=11, red 12..=15). Decode to RGBA8 with
    /// [`crate::decode_a4b4g4r4_unorm_surface`].
    A4B4G4R4Unorm,

    // --- ASTC LDR block-compressed formats (DXGI 133..=187) -------------
    //
    // These `DXGI_FORMAT_ASTC_*` codes are NOT in the public Microsoft
    // DXGI enum page staged under `docs/image/dds/`; they are the
    // Windows 8.1 / D3D11.2-era `dxgiformat.h` values (range 133..=187)
    // recorded in `docs/image/astc/astc-ldr-notes.md` §11. The range is
    // laid out as 4-slot groups per ASTC block footprint —
    // `{TYPELESS, UNORM, UNORM_SRGB, (reserved)}` — starting at 133.
    // Each footprint's `_UNORM` / `_UNORM_SRGB` decode through the ASTC
    // LDR block decoder; `_TYPELESS` is recognised for round-trip but
    // decodes the same payload.
    Astc4x4Typeless,
    Astc4x4Unorm,
    Astc4x4UnormSrgb,
    Astc5x4Typeless,
    Astc5x4Unorm,
    Astc5x4UnormSrgb,
    Astc5x5Typeless,
    Astc5x5Unorm,
    Astc5x5UnormSrgb,
    Astc6x5Typeless,
    Astc6x5Unorm,
    Astc6x5UnormSrgb,
    Astc6x6Typeless,
    Astc6x6Unorm,
    Astc6x6UnormSrgb,
    Astc8x5Typeless,
    Astc8x5Unorm,
    Astc8x5UnormSrgb,
    Astc8x6Typeless,
    Astc8x6Unorm,
    Astc8x6UnormSrgb,
    Astc8x8Typeless,
    Astc8x8Unorm,
    Astc8x8UnormSrgb,
    Astc10x5Typeless,
    Astc10x5Unorm,
    Astc10x5UnormSrgb,
    Astc10x6Typeless,
    Astc10x6Unorm,
    Astc10x6UnormSrgb,
    Astc10x8Typeless,
    Astc10x8Unorm,
    Astc10x8UnormSrgb,
    Astc10x10Typeless,
    Astc10x10Unorm,
    Astc10x10UnormSrgb,
    Astc12x10Typeless,
    Astc12x10Unorm,
    Astc12x10UnormSrgb,
    Astc12x12Typeless,
    Astc12x12Unorm,
    Astc12x12UnormSrgb,
}

impl DxgiFormat {
    /// Decode a raw on-disk `DXGI_FORMAT` value.
    pub fn from_u32(v: u32) -> Self {
        match v {
            1 => Self::R32G32B32A32Typeless,
            2 => Self::R32G32B32A32Float,
            3 => Self::R32G32B32A32Uint,
            4 => Self::R32G32B32A32Sint,
            5 => Self::R32G32B32Typeless,
            6 => Self::R32G32B32Float,
            7 => Self::R32G32B32Uint,
            8 => Self::R32G32B32Sint,
            9 => Self::R16G16B16A16Typeless,
            10 => Self::R16G16B16A16Float,
            11 => Self::R16G16B16A16Unorm,
            12 => Self::R16G16B16A16Uint,
            13 => Self::R16G16B16A16Snorm,
            14 => Self::R16G16B16A16Sint,
            15 => Self::R32G32Typeless,
            16 => Self::R32G32Float,
            17 => Self::R32G32Uint,
            18 => Self::R32G32Sint,
            19 => Self::R32G8X24Typeless,
            20 => Self::D32FloatS8X24Uint,
            21 => Self::R32FloatX8X24Typeless,
            22 => Self::X32TypelessG8X24Uint,
            23 => Self::R10G10B10A2Typeless,
            24 => Self::R10G10B10A2Unorm,
            25 => Self::R10G10B10A2Uint,
            26 => Self::R11G11B10Float,
            27 => Self::R8G8B8A8Typeless,
            28 => Self::R8G8B8A8Unorm,
            29 => Self::R8G8B8A8UnormSrgb,
            30 => Self::R8G8B8A8Uint,
            31 => Self::R8G8B8A8Snorm,
            32 => Self::R8G8B8A8Sint,
            33 => Self::R16G16Typeless,
            34 => Self::R16G16Float,
            35 => Self::R16G16Unorm,
            36 => Self::R16G16Uint,
            37 => Self::R16G16Snorm,
            38 => Self::R16G16Sint,
            39 => Self::R32Typeless,
            40 => Self::D32Float,
            41 => Self::R32Float,
            42 => Self::R32Uint,
            43 => Self::R32Sint,
            44 => Self::R24G8Typeless,
            45 => Self::D24UnormS8Uint,
            46 => Self::R24UnormX8Typeless,
            47 => Self::X24TypelessG8Uint,
            48 => Self::R8G8Typeless,
            49 => Self::R8G8Unorm,
            50 => Self::R8G8Uint,
            51 => Self::R8G8Snorm,
            52 => Self::R8G8Sint,
            53 => Self::R16Typeless,
            54 => Self::R16Float,
            55 => Self::D16Unorm,
            56 => Self::R16Unorm,
            57 => Self::R16Uint,
            58 => Self::R16Snorm,
            59 => Self::R16Sint,
            60 => Self::R8Typeless,
            61 => Self::R8Unorm,
            62 => Self::R8Uint,
            63 => Self::R8Snorm,
            64 => Self::R8Sint,
            65 => Self::A8Unorm,
            66 => Self::R1Unorm,
            67 => Self::R9G9B9E5Sharedexp,
            68 => Self::R8G8B8G8Unorm,
            69 => Self::G8R8G8B8Unorm,
            70 => Self::Bc1Typeless,
            71 => Self::Bc1Unorm,
            72 => Self::Bc1UnormSrgb,
            73 => Self::Bc2Typeless,
            74 => Self::Bc2Unorm,
            75 => Self::Bc2UnormSrgb,
            76 => Self::Bc3Typeless,
            77 => Self::Bc3Unorm,
            78 => Self::Bc3UnormSrgb,
            79 => Self::Bc4Typeless,
            80 => Self::Bc4Unorm,
            81 => Self::Bc4Snorm,
            82 => Self::Bc5Typeless,
            83 => Self::Bc5Unorm,
            84 => Self::Bc5Snorm,
            85 => Self::B5G6R5Unorm,
            86 => Self::B5G5R5A1Unorm,
            87 => Self::B8G8R8A8Unorm,
            88 => Self::B8G8R8X8Unorm,
            89 => Self::R10G10B10XrBiasA2Unorm,
            90 => Self::B8G8R8A8Typeless,
            91 => Self::B8G8R8A8UnormSrgb,
            92 => Self::B8G8R8X8Typeless,
            93 => Self::B8G8R8X8UnormSrgb,
            94 => Self::Bc6hTypeless,
            95 => Self::Bc6hUf16,
            96 => Self::Bc6hSf16,
            97 => Self::Bc7Typeless,
            98 => Self::Bc7Unorm,
            99 => Self::Bc7UnormSrgb,
            100 => Self::Ayuv,
            101 => Self::Y410,
            102 => Self::Y416,
            103 => Self::Nv12,
            104 => Self::P010,
            105 => Self::P016,
            106 => Self::Opaque420,
            107 => Self::Yuy2,
            108 => Self::Y210,
            109 => Self::Y216,
            110 => Self::Nv11,
            111 => Self::Ai44,
            112 => Self::Ia44,
            113 => Self::P8,
            114 => Self::A8P8,
            115 => Self::B4G4R4A4Unorm,
            130 => Self::P208,
            131 => Self::V208,
            132 => Self::V408,
            189 => Self::SamplerFeedbackMinMipOpaque,
            190 => Self::SamplerFeedbackMipRegionUsedOpaque,
            191 => Self::A4B4G4R4Unorm,
            // ASTC LDR (Windows 8.1-era dxgiformat.h, notes §11).
            133 => Self::Astc4x4Typeless,
            134 => Self::Astc4x4Unorm,
            135 => Self::Astc4x4UnormSrgb,
            137 => Self::Astc5x4Typeless,
            138 => Self::Astc5x4Unorm,
            139 => Self::Astc5x4UnormSrgb,
            141 => Self::Astc5x5Typeless,
            142 => Self::Astc5x5Unorm,
            143 => Self::Astc5x5UnormSrgb,
            145 => Self::Astc6x5Typeless,
            146 => Self::Astc6x5Unorm,
            147 => Self::Astc6x5UnormSrgb,
            149 => Self::Astc6x6Typeless,
            150 => Self::Astc6x6Unorm,
            151 => Self::Astc6x6UnormSrgb,
            153 => Self::Astc8x5Typeless,
            154 => Self::Astc8x5Unorm,
            155 => Self::Astc8x5UnormSrgb,
            157 => Self::Astc8x6Typeless,
            158 => Self::Astc8x6Unorm,
            159 => Self::Astc8x6UnormSrgb,
            161 => Self::Astc8x8Typeless,
            162 => Self::Astc8x8Unorm,
            163 => Self::Astc8x8UnormSrgb,
            165 => Self::Astc10x5Typeless,
            166 => Self::Astc10x5Unorm,
            167 => Self::Astc10x5UnormSrgb,
            169 => Self::Astc10x6Typeless,
            170 => Self::Astc10x6Unorm,
            171 => Self::Astc10x6UnormSrgb,
            173 => Self::Astc10x8Typeless,
            174 => Self::Astc10x8Unorm,
            175 => Self::Astc10x8UnormSrgb,
            177 => Self::Astc10x10Typeless,
            178 => Self::Astc10x10Unorm,
            179 => Self::Astc10x10UnormSrgb,
            181 => Self::Astc12x10Typeless,
            182 => Self::Astc12x10Unorm,
            183 => Self::Astc12x10UnormSrgb,
            185 => Self::Astc12x12Typeless,
            186 => Self::Astc12x12Unorm,
            187 => Self::Astc12x12UnormSrgb,
            other => Self::Unknown(other),
        }
    }

    /// Encode this enum back into the raw on-disk `DXGI_FORMAT` value.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Unknown(v) => v,
            Self::R32G32B32A32Typeless => 1,
            Self::R32G32B32A32Float => 2,
            Self::R32G32B32A32Uint => 3,
            Self::R32G32B32A32Sint => 4,
            Self::R32G32B32Typeless => 5,
            Self::R32G32B32Float => 6,
            Self::R32G32B32Uint => 7,
            Self::R32G32B32Sint => 8,
            Self::R16G16B16A16Typeless => 9,
            Self::R16G16B16A16Float => 10,
            Self::R16G16B16A16Unorm => 11,
            Self::R16G16B16A16Uint => 12,
            Self::R16G16B16A16Snorm => 13,
            Self::R16G16B16A16Sint => 14,
            Self::R32G32Typeless => 15,
            Self::R32G32Float => 16,
            Self::R32G32Uint => 17,
            Self::R32G32Sint => 18,
            Self::R32G8X24Typeless => 19,
            Self::D32FloatS8X24Uint => 20,
            Self::R32FloatX8X24Typeless => 21,
            Self::X32TypelessG8X24Uint => 22,
            Self::R10G10B10A2Typeless => 23,
            Self::R10G10B10A2Unorm => 24,
            Self::R10G10B10A2Uint => 25,
            Self::R11G11B10Float => 26,
            Self::R8G8B8A8Typeless => 27,
            Self::R8G8B8A8Unorm => 28,
            Self::R8G8B8A8UnormSrgb => 29,
            Self::R8G8B8A8Uint => 30,
            Self::R8G8B8A8Snorm => 31,
            Self::R8G8B8A8Sint => 32,
            Self::R16G16Typeless => 33,
            Self::R16G16Float => 34,
            Self::R16G16Unorm => 35,
            Self::R16G16Uint => 36,
            Self::R16G16Snorm => 37,
            Self::R16G16Sint => 38,
            Self::R32Typeless => 39,
            Self::D32Float => 40,
            Self::R32Float => 41,
            Self::R32Uint => 42,
            Self::R32Sint => 43,
            Self::R24G8Typeless => 44,
            Self::D24UnormS8Uint => 45,
            Self::R24UnormX8Typeless => 46,
            Self::X24TypelessG8Uint => 47,
            Self::R8G8Typeless => 48,
            Self::R8G8Unorm => 49,
            Self::R8G8Uint => 50,
            Self::R8G8Snorm => 51,
            Self::R8G8Sint => 52,
            Self::R16Typeless => 53,
            Self::R16Float => 54,
            Self::D16Unorm => 55,
            Self::R16Unorm => 56,
            Self::R16Uint => 57,
            Self::R16Snorm => 58,
            Self::R16Sint => 59,
            Self::R8Typeless => 60,
            Self::R8Unorm => 61,
            Self::R8Uint => 62,
            Self::R8Snorm => 63,
            Self::R8Sint => 64,
            Self::A8Unorm => 65,
            Self::R1Unorm => 66,
            Self::R9G9B9E5Sharedexp => 67,
            Self::R8G8B8G8Unorm => 68,
            Self::G8R8G8B8Unorm => 69,
            Self::Bc1Typeless => 70,
            Self::Bc1Unorm => 71,
            Self::Bc1UnormSrgb => 72,
            Self::Bc2Typeless => 73,
            Self::Bc2Unorm => 74,
            Self::Bc2UnormSrgb => 75,
            Self::Bc3Typeless => 76,
            Self::Bc3Unorm => 77,
            Self::Bc3UnormSrgb => 78,
            Self::Bc4Typeless => 79,
            Self::Bc4Unorm => 80,
            Self::Bc4Snorm => 81,
            Self::Bc5Typeless => 82,
            Self::Bc5Unorm => 83,
            Self::Bc5Snorm => 84,
            Self::B5G6R5Unorm => 85,
            Self::B5G5R5A1Unorm => 86,
            Self::B8G8R8A8Unorm => 87,
            Self::B8G8R8X8Unorm => 88,
            Self::R10G10B10XrBiasA2Unorm => 89,
            Self::B8G8R8A8Typeless => 90,
            Self::B8G8R8A8UnormSrgb => 91,
            Self::B8G8R8X8Typeless => 92,
            Self::B8G8R8X8UnormSrgb => 93,
            Self::Bc6hTypeless => 94,
            Self::Bc6hUf16 => 95,
            Self::Bc6hSf16 => 96,
            Self::Bc7Typeless => 97,
            Self::Bc7Unorm => 98,
            Self::Bc7UnormSrgb => 99,
            Self::Ayuv => 100,
            Self::Y410 => 101,
            Self::Y416 => 102,
            Self::Nv12 => 103,
            Self::P010 => 104,
            Self::P016 => 105,
            Self::Opaque420 => 106,
            Self::Yuy2 => 107,
            Self::Y210 => 108,
            Self::Y216 => 109,
            Self::Nv11 => 110,
            Self::Ai44 => 111,
            Self::Ia44 => 112,
            Self::P8 => 113,
            Self::A8P8 => 114,
            Self::B4G4R4A4Unorm => 115,
            Self::P208 => 130,
            Self::V208 => 131,
            Self::V408 => 132,
            Self::SamplerFeedbackMinMipOpaque => 189,
            Self::SamplerFeedbackMipRegionUsedOpaque => 190,
            Self::A4B4G4R4Unorm => 191,
            Self::Astc4x4Typeless => 133,
            Self::Astc4x4Unorm => 134,
            Self::Astc4x4UnormSrgb => 135,
            Self::Astc5x4Typeless => 137,
            Self::Astc5x4Unorm => 138,
            Self::Astc5x4UnormSrgb => 139,
            Self::Astc5x5Typeless => 141,
            Self::Astc5x5Unorm => 142,
            Self::Astc5x5UnormSrgb => 143,
            Self::Astc6x5Typeless => 145,
            Self::Astc6x5Unorm => 146,
            Self::Astc6x5UnormSrgb => 147,
            Self::Astc6x6Typeless => 149,
            Self::Astc6x6Unorm => 150,
            Self::Astc6x6UnormSrgb => 151,
            Self::Astc8x5Typeless => 153,
            Self::Astc8x5Unorm => 154,
            Self::Astc8x5UnormSrgb => 155,
            Self::Astc8x6Typeless => 157,
            Self::Astc8x6Unorm => 158,
            Self::Astc8x6UnormSrgb => 159,
            Self::Astc8x8Typeless => 161,
            Self::Astc8x8Unorm => 162,
            Self::Astc8x8UnormSrgb => 163,
            Self::Astc10x5Typeless => 165,
            Self::Astc10x5Unorm => 166,
            Self::Astc10x5UnormSrgb => 167,
            Self::Astc10x6Typeless => 169,
            Self::Astc10x6Unorm => 170,
            Self::Astc10x6UnormSrgb => 171,
            Self::Astc10x8Typeless => 173,
            Self::Astc10x8Unorm => 174,
            Self::Astc10x8UnormSrgb => 175,
            Self::Astc10x10Typeless => 177,
            Self::Astc10x10Unorm => 178,
            Self::Astc10x10UnormSrgb => 179,
            Self::Astc12x10Typeless => 181,
            Self::Astc12x10Unorm => 182,
            Self::Astc12x10UnormSrgb => 183,
            Self::Astc12x12Typeless => 185,
            Self::Astc12x12Unorm => 186,
            Self::Astc12x12UnormSrgb => 187,
        }
    }

    /// For an ASTC `DXGI_FORMAT_ASTC_*` value, the `(block_w, block_h)`
    /// footprint; `None` for any non-ASTC format. Per `astc-ldr-notes.md`
    /// §11.
    pub fn astc_footprint(self) -> Option<(u32, u32)> {
        Some(match self {
            Self::Astc4x4Typeless | Self::Astc4x4Unorm | Self::Astc4x4UnormSrgb => (4, 4),
            Self::Astc5x4Typeless | Self::Astc5x4Unorm | Self::Astc5x4UnormSrgb => (5, 4),
            Self::Astc5x5Typeless | Self::Astc5x5Unorm | Self::Astc5x5UnormSrgb => (5, 5),
            Self::Astc6x5Typeless | Self::Astc6x5Unorm | Self::Astc6x5UnormSrgb => (6, 5),
            Self::Astc6x6Typeless | Self::Astc6x6Unorm | Self::Astc6x6UnormSrgb => (6, 6),
            Self::Astc8x5Typeless | Self::Astc8x5Unorm | Self::Astc8x5UnormSrgb => (8, 5),
            Self::Astc8x6Typeless | Self::Astc8x6Unorm | Self::Astc8x6UnormSrgb => (8, 6),
            Self::Astc8x8Typeless | Self::Astc8x8Unorm | Self::Astc8x8UnormSrgb => (8, 8),
            Self::Astc10x5Typeless | Self::Astc10x5Unorm | Self::Astc10x5UnormSrgb => (10, 5),
            Self::Astc10x6Typeless | Self::Astc10x6Unorm | Self::Astc10x6UnormSrgb => (10, 6),
            Self::Astc10x8Typeless | Self::Astc10x8Unorm | Self::Astc10x8UnormSrgb => (10, 8),
            Self::Astc10x10Typeless | Self::Astc10x10Unorm | Self::Astc10x10UnormSrgb => (10, 10),
            Self::Astc12x10Typeless | Self::Astc12x10Unorm | Self::Astc12x10UnormSrgb => (12, 10),
            Self::Astc12x12Typeless | Self::Astc12x12Unorm | Self::Astc12x12UnormSrgb => (12, 12),
            _ => return None,
        })
    }

    /// The `_UNORM` (or `_UNORM_SRGB`) `DXGI_FORMAT_ASTC_*` value for a
    /// `(block_w, block_h)` footprint, used when writing an ASTC surface.
    /// `None` if the footprint is not one of the 14 LDR 2D footprints.
    /// Per `astc-ldr-notes.md` §11.
    pub fn astc_unorm(block_w: u32, block_h: u32, srgb: bool) -> Option<Self> {
        let unorm = match (block_w, block_h) {
            (4, 4) => Self::Astc4x4Unorm,
            (5, 4) => Self::Astc5x4Unorm,
            (5, 5) => Self::Astc5x5Unorm,
            (6, 5) => Self::Astc6x5Unorm,
            (6, 6) => Self::Astc6x6Unorm,
            (8, 5) => Self::Astc8x5Unorm,
            (8, 6) => Self::Astc8x6Unorm,
            (8, 8) => Self::Astc8x8Unorm,
            (10, 5) => Self::Astc10x5Unorm,
            (10, 6) => Self::Astc10x6Unorm,
            (10, 8) => Self::Astc10x8Unorm,
            (10, 10) => Self::Astc10x10Unorm,
            (12, 10) => Self::Astc12x10Unorm,
            (12, 12) => Self::Astc12x12Unorm,
            _ => return None,
        };
        if !srgb {
            return Some(unorm);
        }
        // The `_UNORM_SRGB` value sits one slot above the `_UNORM` one
        // in the 133..=187 range (TYPELESS, UNORM, UNORM_SRGB triples).
        Some(Self::from_u32(unorm.to_u32() + 1))
    }

    /// True for any format whose pixel data is laid out in 4×4 blocks
    /// of fixed byte size (BC1..BC7 plus the typeless variants).
    pub fn is_block_compressed(self) -> bool {
        matches!(
            self,
            Self::Bc1Typeless
                | Self::Bc1Unorm
                | Self::Bc1UnormSrgb
                | Self::Bc2Typeless
                | Self::Bc2Unorm
                | Self::Bc2UnormSrgb
                | Self::Bc3Typeless
                | Self::Bc3Unorm
                | Self::Bc3UnormSrgb
                | Self::Bc4Typeless
                | Self::Bc4Unorm
                | Self::Bc4Snorm
                | Self::Bc5Typeless
                | Self::Bc5Unorm
                | Self::Bc5Snorm
                | Self::Bc6hTypeless
                | Self::Bc6hUf16
                | Self::Bc6hSf16
                | Self::Bc7Typeless
                | Self::Bc7Unorm
                | Self::Bc7UnormSrgb
        )
    }

    /// Bytes per 4×4 block for the BC* family. `None` for non-block formats.
    /// BC1 / BC4 are 8 bytes/block; BC2 / BC3 / BC5 / BC6H / BC7 are
    /// 16 bytes/block.
    pub fn block_size_bytes(self) -> Option<u32> {
        Some(match self {
            Self::Bc1Typeless
            | Self::Bc1Unorm
            | Self::Bc1UnormSrgb
            | Self::Bc4Typeless
            | Self::Bc4Unorm
            | Self::Bc4Snorm => 8,
            Self::Bc2Typeless
            | Self::Bc2Unorm
            | Self::Bc2UnormSrgb
            | Self::Bc3Typeless
            | Self::Bc3Unorm
            | Self::Bc3UnormSrgb
            | Self::Bc5Typeless
            | Self::Bc5Unorm
            | Self::Bc5Snorm
            | Self::Bc6hTypeless
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::Bc7Typeless
            | Self::Bc7Unorm
            | Self::Bc7UnormSrgb => 16,
            _ => return None,
        })
    }
}

/// Total stored size in bytes of a single mip-0 surface for a
/// block-compressed format. Width and height are rounded up to the
/// nearest multiple of 4 before the block count is computed (Microsoft's
/// public formula in the DDS programming guide's "Compressed texture
/// resources" section).
pub fn block_compressed_surface_size(width: u32, height: u32, block_bytes: u32) -> u64 {
    let bw = width.max(1).div_ceil(4) as u64;
    let bh = height.max(1).div_ceil(4) as u64;
    // Saturating so a forged (u32::MAX, u32::MAX) returns `u64::MAX` rather
    // than panicking in a debug build (callers compare against buffer
    // length and reject anyway).
    bw.saturating_mul(bh).saturating_mul(block_bytes as u64)
}
