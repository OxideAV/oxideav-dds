//! On-disk DDS header layout constants and helpers.
//!
//! Reference: Microsoft's public "DDS file layout for textures" guide
//! (learn.microsoft.com/windows/win32/direct3ddds/dx-graphics-dds-pguide).
//! Field names below match the public docs (which are themselves the
//! header file Microsoft has published as part of the DirectX docs);
//! no implementation source from DirectXTex / D3DX / NVTT / squish was
//! consulted, paraphrased, or cross-checked.
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
/// All six cubemap-face presence bits OR'd together (positive X..negative Z).
pub const DDSCAPS2_CUBEMAP_ALL_FACES: u32 = 0x0000_FC00;
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

/// Subset of the `DXGI_FORMAT` enum the round-1 reader recognises by
/// name. Numeric values are taken from the public Microsoft DXGI
/// reference. Anything not enumerated below is preserved as a raw
/// `u32` in [`crate::image::DdsImage::dxgi_format`] and reported as an
/// `Unknown(u32)` value via [`DxgiFormat::from_u32`].
///
/// This enum intentionally lists only the formats round 1 needs to
/// distinguish: the BC* compressed family (pass-through) plus a few
/// uncompressed RGBA variants the encoder can emit through the DX10
/// extension. Round 2 will add the full DXGI table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DxgiFormat {
    /// `DXGI_FORMAT_UNKNOWN` (0).
    Unknown(u32),
    /// `DXGI_FORMAT_R8G8B8A8_UNORM` (28).
    R8G8B8A8Unorm,
    /// `DXGI_FORMAT_R8G8B8A8_UNORM_SRGB` (29).
    R8G8B8A8UnormSrgb,
    /// `DXGI_FORMAT_B8G8R8A8_UNORM` (87).
    B8G8R8A8Unorm,
    /// `DXGI_FORMAT_B8G8R8X8_UNORM` (88).
    B8G8R8X8Unorm,
    /// `DXGI_FORMAT_B5G6R5_UNORM` (85).
    B5G6R5Unorm,
    /// `DXGI_FORMAT_B5G5R5A1_UNORM` (86).
    B5G5R5A1Unorm,
    /// `DXGI_FORMAT_B4G4R4A4_UNORM` (115).
    B4G4R4A4Unorm,
    /// `DXGI_FORMAT_R8_UNORM` (61).
    R8Unorm,
    /// `DXGI_FORMAT_R8G8_UNORM` (49).
    R8G8Unorm,
    /// `DXGI_FORMAT_BC1_UNORM` (71).
    Bc1Unorm,
    /// `DXGI_FORMAT_BC1_UNORM_SRGB` (72).
    Bc1UnormSrgb,
    /// `DXGI_FORMAT_BC2_UNORM` (74).
    Bc2Unorm,
    /// `DXGI_FORMAT_BC2_UNORM_SRGB` (75).
    Bc2UnormSrgb,
    /// `DXGI_FORMAT_BC3_UNORM` (77).
    Bc3Unorm,
    /// `DXGI_FORMAT_BC3_UNORM_SRGB` (78).
    Bc3UnormSrgb,
    /// `DXGI_FORMAT_BC4_UNORM` (80).
    Bc4Unorm,
    /// `DXGI_FORMAT_BC4_SNORM` (81).
    Bc4Snorm,
    /// `DXGI_FORMAT_BC5_UNORM` (83).
    Bc5Unorm,
    /// `DXGI_FORMAT_BC5_SNORM` (84).
    Bc5Snorm,
    /// `DXGI_FORMAT_BC6H_UF16` (95).
    Bc6hUf16,
    /// `DXGI_FORMAT_BC6H_SF16` (96).
    Bc6hSf16,
    /// `DXGI_FORMAT_BC7_UNORM` (98).
    Bc7Unorm,
    /// `DXGI_FORMAT_BC7_UNORM_SRGB` (99).
    Bc7UnormSrgb,
}

impl DxgiFormat {
    /// Decode a raw on-disk `DXGI_FORMAT` value. Returns
    /// [`DxgiFormat::Unknown`] (carrying the raw integer) for any value
    /// the round-1 reader does not enumerate by name.
    pub fn from_u32(v: u32) -> Self {
        match v {
            28 => Self::R8G8B8A8Unorm,
            29 => Self::R8G8B8A8UnormSrgb,
            49 => Self::R8G8Unorm,
            61 => Self::R8Unorm,
            71 => Self::Bc1Unorm,
            72 => Self::Bc1UnormSrgb,
            74 => Self::Bc2Unorm,
            75 => Self::Bc2UnormSrgb,
            77 => Self::Bc3Unorm,
            78 => Self::Bc3UnormSrgb,
            80 => Self::Bc4Unorm,
            81 => Self::Bc4Snorm,
            83 => Self::Bc5Unorm,
            84 => Self::Bc5Snorm,
            85 => Self::B5G6R5Unorm,
            86 => Self::B5G5R5A1Unorm,
            87 => Self::B8G8R8A8Unorm,
            88 => Self::B8G8R8X8Unorm,
            95 => Self::Bc6hUf16,
            96 => Self::Bc6hSf16,
            98 => Self::Bc7Unorm,
            99 => Self::Bc7UnormSrgb,
            115 => Self::B4G4R4A4Unorm,
            other => Self::Unknown(other),
        }
    }

    /// Encode this enum back into the raw on-disk `DXGI_FORMAT` value.
    pub fn to_u32(self) -> u32 {
        match self {
            Self::Unknown(v) => v,
            Self::R8G8B8A8Unorm => 28,
            Self::R8G8B8A8UnormSrgb => 29,
            Self::R8G8Unorm => 49,
            Self::R8Unorm => 61,
            Self::Bc1Unorm => 71,
            Self::Bc1UnormSrgb => 72,
            Self::Bc2Unorm => 74,
            Self::Bc2UnormSrgb => 75,
            Self::Bc3Unorm => 77,
            Self::Bc3UnormSrgb => 78,
            Self::Bc4Unorm => 80,
            Self::Bc4Snorm => 81,
            Self::Bc5Unorm => 83,
            Self::Bc5Snorm => 84,
            Self::B5G6R5Unorm => 85,
            Self::B5G5R5A1Unorm => 86,
            Self::B8G8R8A8Unorm => 87,
            Self::B8G8R8X8Unorm => 88,
            Self::Bc6hUf16 => 95,
            Self::Bc6hSf16 => 96,
            Self::Bc7Unorm => 98,
            Self::Bc7UnormSrgb => 99,
            Self::B4G4R4A4Unorm => 115,
        }
    }

    /// True for any format whose pixel data is laid out in 4×4 blocks
    /// of fixed byte size (BC1..BC7 plus the legacy DXTn aliases).
    pub fn is_block_compressed(self) -> bool {
        matches!(
            self,
            Self::Bc1Unorm
                | Self::Bc1UnormSrgb
                | Self::Bc2Unorm
                | Self::Bc2UnormSrgb
                | Self::Bc3Unorm
                | Self::Bc3UnormSrgb
                | Self::Bc4Unorm
                | Self::Bc4Snorm
                | Self::Bc5Unorm
                | Self::Bc5Snorm
                | Self::Bc6hUf16
                | Self::Bc6hSf16
                | Self::Bc7Unorm
                | Self::Bc7UnormSrgb
        )
    }

    /// Bytes per 4×4 block for the BC* family. `None` for non-block formats.
    /// BC1 / BC4 are 8 bytes/block; BC2 / BC3 / BC5 / BC6H / BC7 are
    /// 16 bytes/block.
    pub fn block_size_bytes(self) -> Option<u32> {
        Some(match self {
            Self::Bc1Unorm | Self::Bc1UnormSrgb | Self::Bc4Unorm | Self::Bc4Snorm => 8,
            Self::Bc2Unorm
            | Self::Bc2UnormSrgb
            | Self::Bc3Unorm
            | Self::Bc3UnormSrgb
            | Self::Bc5Unorm
            | Self::Bc5Snorm
            | Self::Bc6hUf16
            | Self::Bc6hSf16
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
    bw * bh * block_bytes as u64
}
