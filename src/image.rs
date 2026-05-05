//! Standalone image container returned by `oxideav-dds`'s framework-free
//! decode API and accepted by the standalone encode API.
//!
//! Defined here (rather than reusing `oxideav_core::VideoFrame`) so the
//! crate can be built with the default `registry` feature off — i.e.
//! without depending on `oxideav-core` at all. When the `registry`
//! feature is on, [`crate::registry`] provides
//! `From<DdsImage> for oxideav_core::VideoFrame` so the trait-side
//! `Decoder` / `Encoder` impls can interoperate with the framework
//! pixel-format / frame surface.

/// Pixel layout of the bytes the parser produces (or the encoder
/// accepts).
///
/// Uncompressed variants list the channel order as it appears in the
/// returned plane — the parser does NOT swap BGR → RGB on read; the
/// caller does the swap if it cares. (The DX10 `DXGI_FORMAT` and the
/// legacy DDS pixel-format flags both natively describe channels in
/// "B-then-G-then-R-then-A" order for the most common Direct3D 9
/// surfaces, so keeping the on-disk layout means the round-trip is
/// trivially lossless.)
///
/// Block-compressed variants do NOT decompress in round 1; the plane
/// carries the raw on-disk block bytes (8 or 16 bytes per 4×4 block).
/// Round 2 will land BC1..BC7 decoders; this enum will then gain a
/// matching `Bc*Decoded` variant rather than mutate the existing
/// pass-through ones.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DdsPixelFormat {
    /// 32 bpp, on-disk `[B, G, R, A]` per pixel
    /// (legacy `D3DFMT_A8R8G8B8` / DXGI `B8G8R8A8_UNORM`).
    A8R8G8B8,
    /// 32 bpp, on-disk `[B, G, R, X]` per pixel — alpha byte unused
    /// (`D3DFMT_X8R8G8B8` / DXGI `B8G8R8X8_UNORM`).
    X8R8G8B8,
    /// 32 bpp, on-disk `[R, G, B, A]` per pixel
    /// (DXGI `R8G8B8A8_UNORM`).
    A8B8G8R8,
    /// 16 bpp, packed `RRRRR GGGGGG BBBBB` little-endian
    /// (`D3DFMT_R5G6B5` / DXGI `B5G6R5_UNORM`).
    R5G6B5,
    /// 16 bpp, packed `A RRRRR GGGGG BBBBB` little-endian
    /// (`D3DFMT_A1R5G5B5` / DXGI `B5G5R5A1_UNORM`).
    A1R5G5B5,
    /// 16 bpp, packed `AAAA RRRR GGGG BBBB` little-endian
    /// (`D3DFMT_A4R4G4B4` / DXGI `B4G4R4A4_UNORM`).
    A4R4G4B4,
    /// 24 bpp, on-disk `[B, G, R]` per pixel
    /// (`D3DFMT_R8G8B8`).
    R8G8B8,
    /// 16 bpp, on-disk `[L, A]` per pixel
    /// (`D3DFMT_A8L8`).
    A8L8,
    /// 8 bpp single-channel luminance (`D3DFMT_L8` / DXGI `R8_UNORM`).
    L8,
    /// 8 bpp single-channel alpha (`D3DFMT_A8`).
    A8,

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
}

impl DdsPixelFormat {
    /// Bits per pixel for uncompressed formats; for block-compressed
    /// formats this is the *amortised* rate (4 bpp for BC1/BC4, 8 bpp
    /// for BC2/BC3/BC5/BC6H/BC7). Matches Microsoft's "bits per pixel"
    /// figure in the public DDS programming guide.
    pub fn bits_per_pixel(self) -> u32 {
        match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 | Self::A8B8G8R8 => 32,
            Self::R8G8B8 => 24,
            Self::R5G6B5 | Self::A1R5G5B5 | Self::A4R4G4B4 | Self::A8L8 => 16,
            Self::L8 | Self::A8 => 8,
            Self::Bc1 | Self::Bc4Unorm | Self::Bc4Snorm => 4,
            Self::Bc2
            | Self::Bc3
            | Self::Bc5Unorm
            | Self::Bc5Snorm
            | Self::Bc6hUf16
            | Self::Bc6hSf16
            | Self::Bc7Unorm
            | Self::Bc7UnormSrgb => 8,
        }
    }

    /// Bytes per pixel for uncompressed formats. Returns `None` for
    /// block-compressed formats — use [`Self::block_bytes`] instead.
    pub fn bytes_per_pixel(self) -> Option<u32> {
        Some(match self {
            Self::A8R8G8B8 | Self::X8R8G8B8 | Self::A8B8G8R8 => 4,
            Self::R8G8B8 => 3,
            Self::R5G6B5 | Self::A1R5G5B5 | Self::A4R4G4B4 | Self::A8L8 => 2,
            Self::L8 | Self::A8 => 1,
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
    pub fn is_block_compressed(self) -> bool {
        self.block_bytes().is_some()
    }

    /// Short human-readable name (used in error messages).
    pub fn name(self) -> &'static str {
        match self {
            Self::A8R8G8B8 => "A8R8G8B8",
            Self::X8R8G8B8 => "X8R8G8B8",
            Self::A8B8G8R8 => "A8B8G8R8",
            Self::R5G6B5 => "R5G6B5",
            Self::A1R5G5B5 => "A1R5G5B5",
            Self::A4R4G4B4 => "A4R4G4B4",
            Self::R8G8B8 => "R8G8B8",
            Self::A8L8 => "A8L8",
            Self::L8 => "L8",
            Self::A8 => "A8",
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
        }
    }
}

/// One image plane: row-major bytes plus the row stride in bytes.
///
/// For block-compressed formats `stride` is the per-row stride
/// expressed in *block bytes* (i.e. one row of 4×4 blocks); the data
/// vector still holds the raw on-disk pixel array.
#[derive(Debug, Clone)]
pub struct DdsPlane {
    /// Bytes per row in `data`. For uncompressed formats this is
    /// `width × bytes_per_pixel`; for block-compressed formats it is
    /// `ceil(width/4) × block_bytes` and the row count is
    /// `ceil(height/4)`.
    pub stride: usize,
    /// Raw plane bytes, packed `stride` × number of rows.
    pub data: Vec<u8>,
}

/// One decoded DDS surface (mip level 0; round 1 ignores additional
/// mip levels and cubemap faces).
///
/// `pts` is `None` for the standalone [`crate::parse_dds`] entry
/// point. The registry-backed `Decoder` impl still passes `pts`
/// through from the surrounding `Packet`.
#[derive(Debug, Clone)]
pub struct DdsImage {
    /// Picture width in pixels.
    pub width: u32,
    /// Picture height in pixels.
    pub height: u32,
    /// On-disk pixel layout the planes carry.
    pub pixel_format: DdsPixelFormat,
    /// One [`DdsPlane`] per plane — DDS files always pack into a
    /// single contiguous plane today, so this is always `len() == 1`.
    pub planes: Vec<DdsPlane>,
    /// Optional presentation timestamp (carried through from the
    /// registry-backed decoder; always `None` for the standalone path).
    pub pts: Option<i64>,
    /// Mipmap-level count as declared in the DDS header (1 for
    /// non-mipmapped surfaces). The pixel data the parser hands back
    /// covers mip-0 only; round 2 will surface the rest.
    pub mip_map_count: u32,
    /// True when the source file used the `DDS_HEADER_DXT10` extension.
    /// Round-trip preserved by the encoder.
    pub has_dxt10_header: bool,
    /// `DXGI_FORMAT` value carried in the DXT10 extension. `None` for
    /// legacy headers. Useful for callers that want to know the BC*
    /// sRGB / unorm / snorm variant precisely.
    pub dxgi_format: Option<crate::types::DxgiFormat>,
}
