//! DDS reader.
//!
//! Parses a complete in-memory DDS byte slice into a [`DdsImage`].
//!
//! Reference: Microsoft's public "DDS file layout for textures" + "DDS
//! pixel format" + "DDS programming guide" pages on learn.microsoft.com.
//!
//! Round 2 covers:
//!
//! * Magic + `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10`
//!   (20 bytes) parsing.
//! * Uncompressed RGB / RGBA / luminance / alpha layouts: A8R8G8B8,
//!   X8R8G8B8, R5G6B5, A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
//! * Block-compressed pass-through: DXT1/3/5 + BC4/5/6H/7. The reader
//!   recognises the format (legacy FourCC or DX10 dxgiFormat), computes
//!   the surface size from `ceil(w/4) × ceil(h/4) × block_bytes`, and
//!   hands the raw block bytes back via [`DdsImage::surfaces`].
//! * **Mipmap chain + cubemap faces + DX10 texture arrays** — every
//!   on-disk surface is parsed in Microsoft's mandated order
//!   (array slice → face → mip) and surfaced via
//!   [`DdsImage::surfaces`].
//! * Full DXGI format table — `DXGI_FORMAT` values 1..=132 are
//!   enumerated by name in [`DxgiFormat`] for lossless round-trip.

use crate::error::{DdsError, Result};
use crate::image::{CubemapFace, DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
use crate::types::*;

// ---- Byte readers --------------------------------------------------------

#[inline]
fn read_u32_le(buf: &[u8], off: usize) -> Result<u32> {
    if off + 4 > buf.len() {
        return Err(DdsError::invalid(format!(
            "read u32 at {off} runs past end of buffer ({} bytes)",
            buf.len()
        )));
    }
    Ok(u32::from_le_bytes([
        buf[off],
        buf[off + 1],
        buf[off + 2],
        buf[off + 3],
    ]))
}

/// Parse the embedded `DDS_PIXELFORMAT` (32 bytes) starting at `off`.
fn parse_pixel_format(buf: &[u8], off: usize) -> Result<DdsPixelFormatHeader> {
    let size = read_u32_le(buf, off)?;
    if size != DDS_PIXELFORMAT_SIZE as u32 {
        return Err(DdsError::invalid(format!(
            "DDS_PIXELFORMAT.size = {size}, expected {DDS_PIXELFORMAT_SIZE}"
        )));
    }
    Ok(DdsPixelFormatHeader {
        size,
        flags: read_u32_le(buf, off + 4)?,
        four_cc: read_u32_le(buf, off + 8)?,
        rgb_bit_count: read_u32_le(buf, off + 12)?,
        r_bit_mask: read_u32_le(buf, off + 16)?,
        g_bit_mask: read_u32_le(buf, off + 20)?,
        b_bit_mask: read_u32_le(buf, off + 24)?,
        a_bit_mask: read_u32_le(buf, off + 28)?,
    })
}

/// Parse the fixed-layout `DDS_HEADER` (124 bytes) starting at `off`.
fn parse_header(buf: &[u8], off: usize) -> Result<DdsHeader> {
    let size = read_u32_le(buf, off)?;
    if size != DDS_HEADER_SIZE as u32 {
        return Err(DdsError::invalid(format!(
            "DDS_HEADER.size = {size}, expected {DDS_HEADER_SIZE}"
        )));
    }
    let flags = read_u32_le(buf, off + 4)?;
    let height = read_u32_le(buf, off + 8)?;
    let width = read_u32_le(buf, off + 12)?;
    let pitch_or_linear_size = read_u32_le(buf, off + 16)?;
    let depth = read_u32_le(buf, off + 20)?;
    let mip_map_count = read_u32_le(buf, off + 24)?;

    let mut reserved1 = [0u32; 11];
    for (i, slot) in reserved1.iter_mut().enumerate() {
        *slot = read_u32_le(buf, off + 28 + i * 4)?;
    }

    // Microsoft `DDS_HEADER` layout (relative to the start of the
    // header, i.e. relative to `off`):
    //   00  dwSize               4
    //   04  dwFlags              4
    //   08  dwHeight             4
    //   0c  dwWidth              4
    //   10  dwPitchOrLinearSize  4
    //   14  dwDepth              4
    //   18  dwMipMapCount        4
    //   1c  dwReserved1[11]     44
    //   48  ddspf               32   (DDS_PIXELFORMAT, embedded)
    //   68  dwCaps               4
    //   6c  dwCaps2              4
    //   70  dwCaps3              4
    //   74  dwCaps4              4
    //   78  dwReserved2          4
    //   7c  end (124 bytes)
    let pixel_format = parse_pixel_format(buf, off + 72)?;

    Ok(DdsHeader {
        size,
        flags,
        height,
        width,
        pitch_or_linear_size,
        depth,
        mip_map_count,
        reserved1,
        pixel_format,
        caps: read_u32_le(buf, off + 104)?,
        caps2: read_u32_le(buf, off + 108)?,
        caps3: read_u32_le(buf, off + 112)?,
        caps4: read_u32_le(buf, off + 116)?,
        reserved2: read_u32_le(buf, off + 120)?,
    })
}

/// Parse the optional `DDS_HEADER_DXT10` (20 bytes) starting at `off`.
fn parse_dxt10(buf: &[u8], off: usize) -> Result<DdsHeaderDxt10> {
    Ok(DdsHeaderDxt10 {
        dxgi_format: read_u32_le(buf, off)?,
        resource_dimension: read_u32_le(buf, off + 4)?,
        misc_flag: read_u32_le(buf, off + 8)?,
        array_size: read_u32_le(buf, off + 12)?,
        misc_flags2: read_u32_le(buf, off + 16)?,
    })
}

/// Resolve the legacy (non-DX10) `DDS_PIXELFORMAT` into a
/// [`DdsPixelFormat`]. Returns `None` if the layout doesn't match any
/// of the round-1 supported formats.
fn pixel_format_from_legacy(p: &DdsPixelFormatHeader) -> Option<DdsPixelFormat> {
    if p.flags & DDPF_FOURCC != 0 {
        return match p.four_cc {
            FOURCC_DXT1 => Some(DdsPixelFormat::Bc1),
            FOURCC_DXT2 | FOURCC_DXT3 => Some(DdsPixelFormat::Bc2),
            FOURCC_DXT4 | FOURCC_DXT5 => Some(DdsPixelFormat::Bc3),
            FOURCC_BC4U | FOURCC_ATI1 => Some(DdsPixelFormat::Bc4Unorm),
            FOURCC_BC4S => Some(DdsPixelFormat::Bc4Snorm),
            FOURCC_BC5U | FOURCC_ATI2 => Some(DdsPixelFormat::Bc5Unorm),
            FOURCC_BC5S => Some(DdsPixelFormat::Bc5Snorm),
            // Horizontally sub-sampled packed RGB carried under their
            // legacy ASCII FourCC tags (the DX10 path routes the same two
            // layouts via DXGI values 68 / 69).
            FOURCC_RGBG => Some(DdsPixelFormat::R8G8B8G8Unorm),
            FOURCC_GRGB => Some(DdsPixelFormat::G8R8G8B8Unorm),
            // Legacy 4:2:2 packed luma/chroma FourCCs. YUY2 has a DX10
            // DXGI counterpart (value 107); UYVY is FourCC-only.
            FOURCC_YUY2 => Some(DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Yuy2)),
            FOURCC_UYVY => Some(DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Uyvy)),
            // Legacy `D3DFMT` numeric FourCC codes for the extended
            // high-bit-depth / floating-point uncompressed layouts. The
            // mapping below is the one Microsoft tabulates in the
            // programming guide's "DDS pixel format" section.
            D3DFMT_A16B16G16R16 => Some(DdsPixelFormat::R16G16B16A16Unorm),
            D3DFMT_Q16W16V16U16 => Some(DdsPixelFormat::R16G16B16A16Snorm),
            D3DFMT_R16F => Some(DdsPixelFormat::R16Float),
            D3DFMT_G16R16F => Some(DdsPixelFormat::R16G16Float),
            D3DFMT_A16B16G16R16F => Some(DdsPixelFormat::R16G16B16A16Float),
            D3DFMT_R32F => Some(DdsPixelFormat::R32Float),
            D3DFMT_G32R32F => Some(DdsPixelFormat::R32G32Float),
            D3DFMT_A32B32G32R32F => Some(DdsPixelFormat::R32G32B32A32Float),
            _ => None,
        };
    }

    let rgb = p.flags & DDPF_RGB != 0;
    let alpha_pixels = p.flags & DDPF_ALPHAPIXELS != 0;
    let alpha_only = p.flags & DDPF_ALPHA != 0;
    let luminance = p.flags & DDPF_LUMINANCE != 0;

    if rgb && p.rgb_bit_count == 32 && alpha_pixels {
        // A8R8G8B8 (BGRA on disk): A=ff000000, R=00ff0000, G=0000ff00, B=000000ff.
        if p.r_bit_mask == 0x00ff_0000
            && p.g_bit_mask == 0x0000_ff00
            && p.b_bit_mask == 0x0000_00ff
            && p.a_bit_mask == 0xff00_0000
        {
            return Some(DdsPixelFormat::A8R8G8B8);
        }
        // A8B8G8R8 (RGBA on disk): R=000000ff, G=0000ff00, B=00ff0000, A=ff000000.
        if p.r_bit_mask == 0x0000_00ff
            && p.g_bit_mask == 0x0000_ff00
            && p.b_bit_mask == 0x00ff_0000
            && p.a_bit_mask == 0xff00_0000
        {
            return Some(DdsPixelFormat::A8B8G8R8);
        }
    }
    if rgb && p.rgb_bit_count == 32 && alpha_pixels {
        // A2B10G10R10 (D3DFMT_A2B10G10R10 / DXGI R10G10B10A2_UNORM):
        // R=0x000003ff, G=0x000ffc00, B=0x3ff00000, A=0xc0000000. This is
        // the canonical Direct3D 10 10:10:10:2 packing — the first named
        // component (R) occupies the least-significant bits. (Masks per
        // Microsoft's "Programming guide for DDS" pixel-format table.)
        if p.r_bit_mask == 0x0000_03ff
            && p.g_bit_mask == 0x000f_fc00
            && p.b_bit_mask == 0x3ff0_0000
            && p.a_bit_mask == 0xc000_0000
        {
            return Some(DdsPixelFormat::R10G10B10A2Unorm);
        }
    }
    if rgb && p.rgb_bit_count == 32 {
        // G16R16 (D3DFMT_G16R16 / DXGI R16G16_UNORM): a two-channel
        // 16:16 layout sharing the bytes of R16G16_UNORM. The red
        // channel occupies the low 16 bits (mask 0x0000ffff), green the
        // high 16 bits (mask 0xffff0000), no blue, no alpha. Microsoft's
        // "Common DDS File Resource Formats" table lists it under both
        // DDS_RGBA and DDS_RGB flag flavours (the spurious alpha flag in
        // the RGBA row carries no alpha mask), so accept either.
        if p.r_bit_mask == 0x0000_ffff
            && p.g_bit_mask == 0xffff_0000
            && p.b_bit_mask == 0
            && p.a_bit_mask == 0
        {
            return Some(DdsPixelFormat::R16G16Unorm);
        }
    }
    if rgb && p.rgb_bit_count == 32 && alpha_pixels {
        // A2R10G10B10 (D3DFMT_A2R10G10B10): the BGR-ordered sibling of
        // A2B10G10R10. The first named component (R) occupies the
        // *most*-significant 10 bits here, blue the least-significant —
        // the reverse channel order of R10G10B10A2_UNORM. Masks per the
        // "Common DDS File Resource Formats" table: R=0x3ff00000,
        // G=0x000ffc00, B=0x000003ff, A=0xc0000000.
        if p.r_bit_mask == 0x3ff0_0000
            && p.g_bit_mask == 0x000f_fc00
            && p.b_bit_mask == 0x0000_03ff
            && p.a_bit_mask == 0xc000_0000
        {
            return Some(DdsPixelFormat::A2R10G10B10);
        }
    }
    if rgb && p.rgb_bit_count == 32 && !alpha_pixels {
        // X8R8G8B8: same masks as A8R8G8B8 but no alpha.
        if p.r_bit_mask == 0x00ff_0000 && p.g_bit_mask == 0x0000_ff00 && p.b_bit_mask == 0x0000_00ff
        {
            return Some(DdsPixelFormat::X8R8G8B8);
        }
        // X8B8G8R8: same RGBA byte order as A8B8G8R8 but no alpha
        // (R=000000ff, G=0000ff00, B=00ff0000). Microsoft's "Common DDS
        // File Resource Formats" table (DDS_RGB, 32 bpp).
        if p.r_bit_mask == 0x0000_00ff && p.g_bit_mask == 0x0000_ff00 && p.b_bit_mask == 0x00ff_0000
        {
            return Some(DdsPixelFormat::X8B8G8R8);
        }
    }
    if rgb && p.rgb_bit_count == 24 {
        // R8G8B8 (BGR on disk): R=ff0000, G=00ff00, B=0000ff.
        if p.r_bit_mask == 0x00ff_0000 && p.g_bit_mask == 0x0000_ff00 && p.b_bit_mask == 0x0000_00ff
        {
            return Some(DdsPixelFormat::R8G8B8);
        }
    }
    if rgb && p.rgb_bit_count == 16 {
        // R5G6B5: R=f800, G=07e0, B=001f.
        if !alpha_pixels
            && p.r_bit_mask == 0xf800
            && p.g_bit_mask == 0x07e0
            && p.b_bit_mask == 0x001f
        {
            return Some(DdsPixelFormat::R5G6B5);
        }
        // A1R5G5B5: A=8000, R=7c00, G=03e0, B=001f.
        if alpha_pixels
            && p.r_bit_mask == 0x7c00
            && p.g_bit_mask == 0x03e0
            && p.b_bit_mask == 0x001f
            && p.a_bit_mask == 0x8000
        {
            return Some(DdsPixelFormat::A1R5G5B5);
        }
        // X1R5G5B5: same 5:5:5 colour masks as A1R5G5B5 but no alpha
        // (R=7c00, G=03e0, B=001f). Microsoft's "Common DDS File Resource
        // Formats" table (DDS_RGB, 16 bpp).
        if !alpha_pixels
            && p.r_bit_mask == 0x7c00
            && p.g_bit_mask == 0x03e0
            && p.b_bit_mask == 0x001f
        {
            return Some(DdsPixelFormat::X1R5G5B5);
        }
        // A4R4G4B4: A=f000, R=0f00, G=00f0, B=000f.
        if alpha_pixels
            && p.r_bit_mask == 0x0f00
            && p.g_bit_mask == 0x00f0
            && p.b_bit_mask == 0x000f
            && p.a_bit_mask == 0xf000
        {
            return Some(DdsPixelFormat::A4R4G4B4);
        }
        // X4R4G4B4: same 4:4:4 colour masks as A4R4G4B4 but no alpha
        // (R=0f00, G=00f0, B=000f). Microsoft's "Common DDS File Resource
        // Formats" table (DDS_RGB, 16 bpp).
        if !alpha_pixels
            && p.r_bit_mask == 0x0f00
            && p.g_bit_mask == 0x00f0
            && p.b_bit_mask == 0x000f
        {
            return Some(DdsPixelFormat::X4R4G4B4);
        }
        // A8R3G3B2: packed 3:3:2 RGB in the low byte plus an 8-bit alpha
        // in the high byte (R=0x00e0, G=0x001c, B=0x0003, A=0xff00).
        // Microsoft's "Common DDS File Resource Formats" table
        // (DDS_RGBA, 16 bpp).
        if alpha_pixels
            && p.r_bit_mask == 0x00e0
            && p.g_bit_mask == 0x001c
            && p.b_bit_mask == 0x0003
            && p.a_bit_mask == 0xff00
        {
            return Some(DdsPixelFormat::A8R3G3B2);
        }
    }

    if luminance {
        if p.rgb_bit_count == 8 && p.r_bit_mask == 0x00ff && !alpha_pixels {
            return Some(DdsPixelFormat::L8);
        }
        // L16: single 16-bit luminance channel (R=ffff, no alpha).
        // Microsoft's "Common DDS File Resource Formats" table
        // (DDS_LUMINANCE, 16 bpp).
        if p.rgb_bit_count == 16 && !alpha_pixels && p.r_bit_mask == 0xffff {
            return Some(DdsPixelFormat::L16);
        }
        if p.rgb_bit_count == 16 && alpha_pixels && p.r_bit_mask == 0x00ff && p.a_bit_mask == 0xff00
        {
            return Some(DdsPixelFormat::A8L8);
        }
        // A4L4: packed 4:4 luminance + alpha in a single byte (L=0f,
        // A=f0). Microsoft's "Common DDS File Resource Formats" table
        // (DDS_LUMINANCE, 8 bpp).
        if p.rgb_bit_count == 8 && alpha_pixels && p.r_bit_mask == 0x0f && p.a_bit_mask == 0xf0 {
            return Some(DdsPixelFormat::A4L4);
        }
    }
    if alpha_only && p.rgb_bit_count == 8 && p.a_bit_mask == 0x00ff {
        return Some(DdsPixelFormat::A8);
    }

    None
}

/// Resolve a DX10 `DXGI_FORMAT` into a [`DdsPixelFormat`]. Returns
/// `None` for any DXGI format the round-2 reader does not know how to
/// lay out as one of the [`DdsPixelFormat`] variants (HDR floats,
/// integer formats, depth/stencil, YUV planar, palette-8, ...).
fn pixel_format_from_dxgi(d: DxgiFormat) -> Option<DdsPixelFormat> {
    // The DXGI naming convention swaps the apparent channel order
    // relative to the legacy D3D9 names: `R8G8B8A8_UNORM` is RGBA
    // bytes on disk, `B8G8R8A8_UNORM` is BGRA on disk. So R8G8B8A8
    // matches our crate-local A8B8G8R8 (RGBA on disk) and B8G8R8A8
    // matches A8R8G8B8 (BGRA on disk).
    Some(match d {
        DxgiFormat::R8G8B8A8Unorm
        | DxgiFormat::R8G8B8A8UnormSrgb
        | DxgiFormat::R8G8B8A8Typeless => DdsPixelFormat::A8B8G8R8,
        DxgiFormat::B8G8R8A8Unorm
        | DxgiFormat::B8G8R8A8Typeless
        | DxgiFormat::B8G8R8A8UnormSrgb => DdsPixelFormat::A8R8G8B8,
        DxgiFormat::B8G8R8X8Unorm
        | DxgiFormat::B8G8R8X8Typeless
        | DxgiFormat::B8G8R8X8UnormSrgb => DdsPixelFormat::X8R8G8B8,
        DxgiFormat::B5G6R5Unorm => DdsPixelFormat::R5G6B5,
        DxgiFormat::B5G5R5A1Unorm => DdsPixelFormat::A1R5G5B5,
        DxgiFormat::B4G4R4A4Unorm => DdsPixelFormat::A4R4G4B4,
        // A4B4G4R4_UNORM (value 191): the DX10-only 4:4:4:4 layout with
        // alpha in the least-significant nibble and red in the most —
        // the reverse channel order of the legacy A4R4G4B4, so it needs
        // its own variant rather than aliasing to A4R4G4B4.
        DxgiFormat::A4B4G4R4Unorm => DdsPixelFormat::A4B4G4R4Unorm,
        DxgiFormat::R8Unorm | DxgiFormat::R8Typeless => DdsPixelFormat::L8,
        DxgiFormat::A8Unorm => DdsPixelFormat::A8,
        DxgiFormat::R8G8Unorm | DxgiFormat::R8G8Typeless => DdsPixelFormat::A8L8,
        DxgiFormat::Bc1Unorm | DxgiFormat::Bc1UnormSrgb | DxgiFormat::Bc1Typeless => {
            DdsPixelFormat::Bc1
        }
        DxgiFormat::Bc2Unorm | DxgiFormat::Bc2UnormSrgb | DxgiFormat::Bc2Typeless => {
            DdsPixelFormat::Bc2
        }
        DxgiFormat::Bc3Unorm | DxgiFormat::Bc3UnormSrgb | DxgiFormat::Bc3Typeless => {
            DdsPixelFormat::Bc3
        }
        DxgiFormat::Bc4Unorm | DxgiFormat::Bc4Typeless => DdsPixelFormat::Bc4Unorm,
        DxgiFormat::Bc4Snorm => DdsPixelFormat::Bc4Snorm,
        DxgiFormat::Bc5Unorm | DxgiFormat::Bc5Typeless => DdsPixelFormat::Bc5Unorm,
        DxgiFormat::Bc5Snorm => DdsPixelFormat::Bc5Snorm,
        DxgiFormat::Bc6hUf16 | DxgiFormat::Bc6hTypeless => DdsPixelFormat::Bc6hUf16,
        DxgiFormat::Bc6hSf16 => DdsPixelFormat::Bc6hSf16,
        DxgiFormat::Bc7Unorm | DxgiFormat::Bc7Typeless => DdsPixelFormat::Bc7Unorm,
        DxgiFormat::Bc7UnormSrgb => DdsPixelFormat::Bc7UnormSrgb,
        // Extended high-bit-depth / floating-point uncompressed layouts.
        // These are the DX10-header DXGI counterparts of the legacy
        // numeric FourCC codes 36 / 110..=116.
        DxgiFormat::R16G16B16A16Unorm => DdsPixelFormat::R16G16B16A16Unorm,
        DxgiFormat::R16G16B16A16Snorm => DdsPixelFormat::R16G16B16A16Snorm,
        DxgiFormat::R16Float => DdsPixelFormat::R16Float,
        DxgiFormat::R16G16Float => DdsPixelFormat::R16G16Float,
        DxgiFormat::R16G16B16A16Float => DdsPixelFormat::R16G16B16A16Float,
        DxgiFormat::R32Float => DdsPixelFormat::R32Float,
        DxgiFormat::R32G32Float => DdsPixelFormat::R32G32Float,
        DxgiFormat::R32G32B32A32Float => DdsPixelFormat::R32G32B32A32Float,
        // Packed 10:10:10:2 — one 32-bit word per pixel, R in the
        // least-significant 10 bits. The UINT variant (value 25) shares
        // the UNORM packing but stores plain integers (no normalisation);
        // it is DX10-only (no legacy D3DFMT four-cc).
        DxgiFormat::R10G10B10A2Unorm => DdsPixelFormat::R10G10B10A2Unorm,
        DxgiFormat::R10G10B10A2Uint => DdsPixelFormat::R10G10B10A2Uint,
        // Horizontally sub-sampled packed RGB (values 68 / 69). One
        // 32-bit block per pixel pair: R/B shared, G unique per pixel.
        DxgiFormat::R8G8B8G8Unorm => DdsPixelFormat::R8G8B8G8Unorm,
        DxgiFormat::G8R8G8B8Unorm => DdsPixelFormat::G8R8G8B8Unorm,
        // 16-bit-per-channel plain-integer layouts (DX10-only). Tightly
        // packed little-endian samples, named channel order, no
        // normalisation: UINT → u16, SINT → i16.
        DxgiFormat::R16Uint => DdsPixelFormat::R16Uint,
        DxgiFormat::R16Sint => DdsPixelFormat::R16Sint,
        DxgiFormat::R16G16Uint => DdsPixelFormat::R16G16Uint,
        DxgiFormat::R16G16Sint => DdsPixelFormat::R16G16Sint,
        DxgiFormat::R16G16B16A16Uint => DdsPixelFormat::R16G16B16A16Uint,
        DxgiFormat::R16G16B16A16Sint => DdsPixelFormat::R16G16B16A16Sint,
        // 8-bit-per-channel plain-integer layouts (DX10-only). Tightly
        // packed `u8` / `i8` samples, named channel order, no
        // normalisation: UINT → u8, SINT → i8.
        DxgiFormat::R8Uint => DdsPixelFormat::R8Uint,
        DxgiFormat::R8Sint => DdsPixelFormat::R8Sint,
        DxgiFormat::R8G8Uint => DdsPixelFormat::R8G8Uint,
        DxgiFormat::R8G8Sint => DdsPixelFormat::R8G8Sint,
        DxgiFormat::R8G8B8A8Uint => DdsPixelFormat::R8G8B8A8Uint,
        DxgiFormat::R8G8B8A8Sint => DdsPixelFormat::R8G8B8A8Sint,
        // 32-bit-per-channel plain-integer layouts (DX10-only). Tightly
        // packed little-endian `u32` / `i32` samples, named channel order,
        // no normalisation: UINT → u32, SINT → i32. The three-channel
        // `R32G32B32` family is 96-bit (12 bytes per pixel).
        DxgiFormat::R32Uint => DdsPixelFormat::R32Uint,
        DxgiFormat::R32Sint => DdsPixelFormat::R32Sint,
        DxgiFormat::R32G32Uint => DdsPixelFormat::R32G32Uint,
        DxgiFormat::R32G32Sint => DdsPixelFormat::R32G32Sint,
        DxgiFormat::R32G32B32Uint => DdsPixelFormat::R32G32B32Uint,
        DxgiFormat::R32G32B32Sint => DdsPixelFormat::R32G32B32Sint,
        DxgiFormat::R32G32B32A32Uint => DdsPixelFormat::R32G32B32A32Uint,
        DxgiFormat::R32G32B32A32Sint => DdsPixelFormat::R32G32B32A32Sint,
        // Normalised single- / dual-channel 8-bit and 16-bit layouts.
        // Tightly-packed little-endian integers the shader reads as
        // normalised floats: UNORM → [0, 1], SNORM → [-1, 1]. Decode with
        // `decode_unorm_surface` / `decode_snorm_surface`. (`R8_UNORM`
        // keeps its byte-identical `L8` container mapping above.)
        DxgiFormat::R8Snorm => DdsPixelFormat::R8Snorm,
        DxgiFormat::R8G8Snorm => DdsPixelFormat::R8G8Snorm,
        DxgiFormat::R8G8B8A8Snorm => DdsPixelFormat::R8G8B8A8Snorm,
        DxgiFormat::R16Unorm => DdsPixelFormat::R16Unorm,
        DxgiFormat::R16Snorm => DdsPixelFormat::R16Snorm,
        DxgiFormat::R16G16Unorm => DdsPixelFormat::R16G16Unorm,
        DxgiFormat::R16G16Snorm => DdsPixelFormat::R16G16Snorm,
        // Depth / depth-stencil surfaces. The byte packing is fully
        // documented in the DXGI format enumeration; the typeless "view"
        // formats over the same memory (R32G8X24 / R24G8) share the
        // packing and route to the typed depth-stencil variant.
        DxgiFormat::D16Unorm => DdsPixelFormat::D16Unorm,
        DxgiFormat::D32Float => DdsPixelFormat::D32Float,
        DxgiFormat::D24UnormS8Uint | DxgiFormat::R24G8Typeless => DdsPixelFormat::D24UnormS8Uint,
        DxgiFormat::D32FloatS8X24Uint | DxgiFormat::R32G8X24Typeless => {
            DdsPixelFormat::D32FloatS8X24Uint
        }
        // Single-aspect depth/stencil "view" formats over the same
        // depth-stencil memory: the documented bit packing exposes only
        // the depth (R…) or only the stencil (X…G8…) component, leaving
        // the other aspect as typeless padding. Same byte footprint as
        // the combined D24S8 / D32S8X24 surfaces above.
        DxgiFormat::R24UnormX8Typeless => DdsPixelFormat::R24UnormX8Typeless,
        DxgiFormat::X24TypelessG8Uint => DdsPixelFormat::X24TypelessG8Uint,
        DxgiFormat::R32FloatX8X24Typeless => DdsPixelFormat::R32FloatX8X24Typeless,
        DxgiFormat::X32TypelessG8X24Uint => DdsPixelFormat::X32TypelessG8X24Uint,
        // ASTC LDR block-compressed surfaces (DXGI 133..=187). The
        // footprint is carried on the variant so the non-4×4 block
        // geometry sizes correctly; `_UNORM_SRGB` sets the `srgb` flag.
        astc if astc.astc_footprint().is_some() => {
            let (block_w, block_h) = astc.astc_footprint().expect("checked by guard");
            let srgb = matches!(
                astc,
                DxgiFormat::Astc4x4UnormSrgb
                    | DxgiFormat::Astc5x4UnormSrgb
                    | DxgiFormat::Astc5x5UnormSrgb
                    | DxgiFormat::Astc6x5UnormSrgb
                    | DxgiFormat::Astc6x6UnormSrgb
                    | DxgiFormat::Astc8x5UnormSrgb
                    | DxgiFormat::Astc8x6UnormSrgb
                    | DxgiFormat::Astc8x8UnormSrgb
                    | DxgiFormat::Astc10x5UnormSrgb
                    | DxgiFormat::Astc10x6UnormSrgb
                    | DxgiFormat::Astc10x8UnormSrgb
                    | DxgiFormat::Astc10x10UnormSrgb
                    | DxgiFormat::Astc12x10UnormSrgb
                    | DxgiFormat::Astc12x12UnormSrgb
            );
            DdsPixelFormat::Astc {
                block_w,
                block_h,
                srgb,
            }
        }
        // YUV (video) DXGI formats — packed and planar luma/chroma
        // surfaces. The bytes are carried verbatim; the matching
        // `crate::yuv::decode_*_surface` helper expands them.
        DxgiFormat::Ayuv => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Ayuv),
        DxgiFormat::Y410 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Y410),
        DxgiFormat::Y416 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Y416),
        DxgiFormat::Nv12 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Nv12),
        DxgiFormat::P010 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::P010),
        DxgiFormat::P016 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::P016),
        DxgiFormat::Opaque420 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Opaque420),
        DxgiFormat::Yuy2 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Yuy2),
        DxgiFormat::Y210 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Y210),
        DxgiFormat::Y216 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Y216),
        DxgiFormat::Nv11 => DdsPixelFormat::Yuv(crate::yuv::YuvFormat::Nv11),
        // Plain colour `_TYPELESS` formats. A typeless surface stores the
        // exact same per-channel bytes as its typed siblings but assigns
        // no interpretation — the runtime view (UINT / SINT / UNORM /
        // FLOAT) is chosen later. We size and carry the bytes verbatim by
        // routing each to its byte-identical `_UINT` variant, whose
        // decoder returns the stored words uninterpreted — exactly the
        // "uninterpreted bytes" semantics of a typeless surface. (The
        // already-routed `R8_TYPELESS` → L8, `R8G8_TYPELESS` → A8L8,
        // `R8G8B8A8_TYPELESS` → A8B8G8R8 and the `B8G8R8*` views above
        // follow the same byte-pass-through convention.) The per-channel
        // bit counts are from Microsoft's `DXGI_FORMAT` enumeration.
        DxgiFormat::R16Typeless => DdsPixelFormat::R16Uint,
        DxgiFormat::R16G16Typeless => DdsPixelFormat::R16G16Uint,
        DxgiFormat::R16G16B16A16Typeless => DdsPixelFormat::R16G16B16A16Uint,
        DxgiFormat::R32Typeless => DdsPixelFormat::R32Uint,
        DxgiFormat::R32G32Typeless => DdsPixelFormat::R32G32Uint,
        DxgiFormat::R32G32B32Typeless => DdsPixelFormat::R32G32B32Uint,
        DxgiFormat::R32G32B32A32Typeless => DdsPixelFormat::R32G32B32A32Uint,
        // R10G10B10A2_TYPELESS shares the 32-bit packed word of its
        // UINT/UNORM siblings; route to the UINT decoder (stored integers,
        // no normalisation).
        DxgiFormat::R10G10B10A2Typeless => DdsPixelFormat::R10G10B10A2Uint,
        // R10G10B10_XR_BIAS_A2_UNORM (value 89) shares the exact same
        // 32-bit 10:10:10:2 packing (R in the least-significant 10 bits).
        // Microsoft documents it only as a "2.8-biased fixed-point"
        // format; the enumeration page does not state the bias→float
        // arithmetic (the offset / scale constants), so we cannot decode
        // it to normalised floats clean-room. We route it to the UINT
        // decoder so the stored 10-bit fixed-point codes and 2-bit alpha
        // are recovered verbatim; the caller applies the display-side
        // bias transform (mirroring how the 16-bit UNORM/SNORM surfaces
        // hand back raw channels pending a documented scaling rule). The
        // `dxgi_format` field still carries the exact code 89 for
        // round-trip.
        DxgiFormat::R10G10B10XrBiasA2Unorm => DdsPixelFormat::R10G10B10A2Uint,
        // Everything else (the depth/colour-ambiguous views without a
        // byte-identical typed sibling, the packed/planar P208/V208/V408
        // video formats whose byte layout Microsoft does not document,
        // palette-8) has no [`DdsPixelFormat`] mapping yet.
        _ => return None,
    })
}

/// Compute the in-memory mip-0 surface size in bytes for `pix` at the
/// given dimensions. Returns `Err(DdsError::invalid)` if the multiplication
/// would overflow `u64`, so adversarial header values cannot panic this
/// path in a debug build.
fn surface_size_bytes(pix: DdsPixelFormat, width: u32, height: u32) -> Result<u64> {
    if let DdsPixelFormat::Yuv(f) = pix {
        // YUV surfaces are planar / packed / subsampled; the exact
        // on-disk byte count is computed per-format (Microsoft's
        // staging-resource sizing rules), not via the bpp model. The
        // documented width/height divisibility constraints are enforced
        // here so a malformed (odd-dimension) YUV surface is rejected at
        // parse time rather than only when the decoder is later called.
        f.check_dims(width, height)?;
        return f.surface_size_bytes(width, height);
    }
    if let Some((bw, bh)) = pix.astc_footprint() {
        // ASTC: ceil(w/bw) × ceil(h/bh) blocks of a fixed 16 bytes.
        let bx = (width.max(1).div_ceil(bw)) as u64;
        let by = (height.max(1).div_ceil(bh)) as u64;
        return bx
            .checked_mul(by)
            .and_then(|n| n.checked_mul(16))
            .ok_or_else(|| {
                DdsError::invalid(format!(
                    "ASTC surface size overflow for {width}x{height} × {bw}x{bh} blocks"
                ))
            });
    }
    if let Some(bb) = pix.block_bytes() {
        let bw = (width.max(1).div_ceil(4)) as u64;
        let bh = (height.max(1).div_ceil(4)) as u64;
        return bw
            .checked_mul(bh)
            .and_then(|n| n.checked_mul(bb as u64))
            .ok_or_else(|| {
                DdsError::invalid(format!(
                    "BC surface size overflow for {width}x{height} × {bb} byte blocks"
                ))
            });
    }
    let bpp = pix.bytes_per_pixel().expect("uncompressed format") as u64;
    (width as u64)
        .checked_mul(height as u64)
        .and_then(|n| n.checked_mul(bpp))
        .ok_or_else(|| {
            DdsError::invalid(format!(
                "uncompressed surface size overflow for {width}x{height} × {bpp} bpp"
            ))
        })
}

/// Per-row stride for `pix` at the given width. For block-compressed
/// formats this is `ceil(width/4) × block_bytes`.
fn surface_stride_bytes(pix: DdsPixelFormat, width: u32) -> usize {
    if let DdsPixelFormat::Yuv(f) = pix {
        // Report the first (luma / packed) plane's row pitch. For the
        // planar 4:2:0 / 4:1:1 formats this is the Y-plane row; for the
        // packed formats it is one row of the packed groups. The exact
        // per-plane geometry is reconstructed by the matching
        // `crate::yuv::decode_*_surface` helper from the full payload.
        use crate::yuv::YuvFormat::*;
        let w = width as usize;
        return match f {
            Ayuv | Y410 => w * 4,
            Y416 => w * 8,
            Yuy2 | Uyvy => w * 2,
            Y210 | Y216 => w * 4,
            // Planar luma row pitch: 1 byte/sample (8-bit) or 2 (16-bit).
            Nv12 | Opaque420 | Nv11 => w,
            P010 | P016 => w * 2,
        };
    }
    if let Some((bw, _bh)) = pix.astc_footprint() {
        // One row of ASTC blocks = ceil(width/block_w) × 16 bytes.
        return (width.max(1).div_ceil(bw) * 16) as usize;
    }
    if let Some(bb) = pix.block_bytes() {
        return (width.max(1).div_ceil(4) * bb) as usize;
    }
    (width as u64 * pix.bytes_per_pixel().expect("uncompressed format") as u64) as usize
}

/// Parse a complete DDS byte stream.
///
/// The returned [`DdsImage`] carries every (array_slice, face,
/// mip_level) surface declared by the file in [`DdsImage::surfaces`].
/// `planes[0]` mirrors `surfaces[0].plane` for callers that just want
/// the base level of a non-array, non-cubemap texture.
pub fn parse_dds(bytes: &[u8]) -> Result<DdsImage> {
    if bytes.len() < 4 + DDS_HEADER_SIZE {
        return Err(DdsError::invalid(format!(
            "buffer too small for DDS magic + header ({} bytes)",
            bytes.len()
        )));
    }

    let magic = read_u32_le(bytes, 0)?;
    if magic != DDS_MAGIC {
        return Err(DdsError::invalid(format!(
            "bad DDS magic: 0x{magic:08x}, expected 0x{DDS_MAGIC:08x} (\"DDS \")"
        )));
    }

    let header = parse_header(bytes, 4)?;
    if header.flags & DDSD_REQUIRED != DDSD_REQUIRED {
        return Err(DdsError::invalid(format!(
            "DDS_HEADER.flags = 0x{:08x} missing required bits (caps|height|width|pixel_format)",
            header.flags
        )));
    }

    let width = header.width;
    let height = header.height;
    if width == 0 || height == 0 {
        return Err(DdsError::invalid(format!(
            "zero-sized surface: {width}x{height}"
        )));
    }

    let mut pixel_data_off = 4 + DDS_HEADER_SIZE;
    let mut has_dxt10 = false;
    let mut dxgi: Option<DxgiFormat> = None;
    let pix: DdsPixelFormat;

    if header.pixel_format.flags & DDPF_FOURCC != 0 && header.pixel_format.four_cc == FOURCC_DX10 {
        has_dxt10 = true;
        if bytes.len() < pixel_data_off + DDS_HEADER_DXT10_SIZE {
            return Err(DdsError::invalid("buffer too small for DDS_HEADER_DXT10"));
        }
        let dxt10 = parse_dxt10(bytes, pixel_data_off)?;
        pixel_data_off += DDS_HEADER_DXT10_SIZE;
        let dxgi_fmt = DxgiFormat::from_u32(dxt10.dxgi_format);
        dxgi = Some(dxgi_fmt);
        pix = pixel_format_from_dxgi(dxgi_fmt).ok_or_else(|| {
            DdsError::unsupported(format!(
                "unsupported DXGI_FORMAT = {} (raw)",
                dxt10.dxgi_format
            ))
        })?;
    } else {
        pix = pixel_format_from_legacy(&header.pixel_format).ok_or_else(|| {
            DdsError::unsupported(format!(
                "unsupported legacy DDS_PIXELFORMAT (flags=0x{:08x}, fourCC=0x{:08x}, bpp={}, R={:08x} G={:08x} B={:08x} A={:08x})",
                header.pixel_format.flags,
                header.pixel_format.four_cc,
                header.pixel_format.rgb_bit_count,
                header.pixel_format.r_bit_mask,
                header.pixel_format.g_bit_mask,
                header.pixel_format.b_bit_mask,
                header.pixel_format.a_bit_mask,
            ))
        })?;
    }

    let mip_count = if header.flags & DDSD_MIPMAPCOUNT != 0 {
        header.mip_map_count.max(1)
    } else {
        1
    };

    // Reject mip counts the on-disk dimensions cannot possibly justify.
    // The maximum useful mip level for a w×h surface is
    // `1 + floor(log2(max(w, h)))` (the chain bottoms out at 1×1). A
    // forged `u32::MAX` or any value greater than the dim-implied cap is
    // either malicious (over-allocation / shift-overflow attempt) or
    // malformed; either way we surface InvalidData rather than walk the
    // shift loop and panic at `width >> 32`. Depth is folded in below
    // once the volume-texture detection has run.
    let max_dim = width.max(height);
    let max_mip_for_2d = if max_dim == 0 {
        1
    } else {
        // 32 - leading_zeros gives `1 + floor(log2(max_dim))` for max_dim ≥ 1.
        (32 - max_dim.leading_zeros()).max(1)
    };
    if mip_count > max_mip_for_2d {
        return Err(DdsError::invalid(format!(
            "DDS mip_map_count = {mip_count} exceeds dimension-implied cap of {max_mip_for_2d} for {width}x{height}",
        )));
    }

    // Cubemap detection: legacy header sets DDSCAPS2_CUBEMAP plus the
    // six per-face presence bits; DX10 header sets
    // DDS_RESOURCE_MISC_TEXTURECUBE in misc_flag.
    let mut is_cubemap = header.caps2 & DDSCAPS2_CUBEMAP != 0;
    let mut present_faces_mask = if is_cubemap {
        // Microsoft notes that since D3D9 every cubemap face must be
        // present; a `DDSCAPS2_CUBEMAP` header without per-face bits
        // is interpreted as "all six faces present".
        if header.caps2 & DDSCAPS2_CUBEMAP_ALL_FACES == 0 {
            DDSCAPS2_CUBEMAP_ALL_FACES
        } else {
            header.caps2 & DDSCAPS2_CUBEMAP_ALL_FACES
        }
    } else {
        0
    };

    // Volume (3D) texture detection: legacy header sets DDSCAPS2_VOLUME
    // (paired with DDSD_DEPTH in flags); DX10 header sets
    // resource_dimension == DDS_DIMENSION_TEXTURE3D. The mip-0 slice
    // count lives in header.depth.
    let mut is_volume = header.caps2 & DDSCAPS2_VOLUME != 0;

    let mut array_size: u32 = 1;
    if has_dxt10 {
        // Re-read the parsed dxt10 to apply cubemap / array adjustments.
        let dxt10 = parse_dxt10(bytes, 4 + DDS_HEADER_SIZE).expect("already parsed once above");
        if dxt10.misc_flag & DDS_RESOURCE_MISC_TEXTURECUBE != 0 {
            is_cubemap = true;
            present_faces_mask = DDSCAPS2_CUBEMAP_ALL_FACES;
        }
        if dxt10.resource_dimension == DDS_DIMENSION_TEXTURE3D {
            is_volume = true;
        }
        array_size = dxt10.array_size.max(1);
    }

    // mip-0 depth (z) slice count. Only meaningful for volume textures;
    // 1 otherwise. Microsoft notes that DX10 3D textures cannot be
    // arrays, so a volume texture always has array_size == 1.
    let base_depth = if is_volume { header.depth.max(1) } else { 1 };
    if is_volume && (is_cubemap || array_size > 1) {
        return Err(DdsError::invalid(
            "volume (3D) texture cannot also be a cubemap or texture array",
        ));
    }

    // Bound the per-axis dimensions one more time now that depth is known.
    // A forged `header.depth = u32::MAX` with a legitimate w×h would let an
    // attacker request multi-billion slice loops; we cap at the dimension-
    // implied mip count again (volume mips halve depth alongside w/h).
    if is_volume {
        let max_dim_3d = width.max(height).max(base_depth);
        let max_mip_for_3d = if max_dim_3d == 0 {
            1
        } else {
            (32 - max_dim_3d.leading_zeros()).max(1)
        };
        if mip_count > max_mip_for_3d {
            return Err(DdsError::invalid(format!(
                "DDS volume mip_map_count = {mip_count} exceeds dimension-implied cap of {max_mip_for_3d} for {width}x{height}x{base_depth}",
            )));
        }
        // Also bound the mip-0 slice count itself: it must not exceed the
        // dimension-implied cap on the smaller axes (otherwise the parser
        // tries to walk a 4-billion-slice loop at mip 0 alone).
        let slice_cap = 1u32 << (max_mip_for_3d - 1);
        if base_depth > slice_cap.saturating_mul(2) {
            return Err(DdsError::invalid(format!(
                "DDS volume base_depth = {base_depth} exceeds 2 × dimension-implied cap ({})",
                slice_cap.saturating_mul(2),
            )));
        }
    }

    // Order Microsoft mandates for cubemap faces (PX, NX, PY, NY,
    // PZ, NZ). Skip faces whose presence bit is clear.
    let face_indices: Vec<CubemapFace> = if is_cubemap {
        let bits_in_order = [
            (DDSCAPS2_CUBEMAP_POSITIVEX, CubemapFace::PositiveX),
            (DDSCAPS2_CUBEMAP_NEGATIVEX, CubemapFace::NegativeX),
            (DDSCAPS2_CUBEMAP_POSITIVEY, CubemapFace::PositiveY),
            (DDSCAPS2_CUBEMAP_NEGATIVEY, CubemapFace::NegativeY),
            (DDSCAPS2_CUBEMAP_POSITIVEZ, CubemapFace::PositiveZ),
            (DDSCAPS2_CUBEMAP_NEGATIVEZ, CubemapFace::NegativeZ),
        ];
        bits_in_order
            .iter()
            .filter(|(b, _)| present_faces_mask & b != 0)
            .map(|(_, f)| *f)
            .collect()
    } else {
        vec![]
    };

    // Pre-compute (width, height) for each mip level.
    let mip_dims: Vec<(u32, u32)> = (0..mip_count)
        .map(|m| ((width >> m).max(1), (height >> m).max(1)))
        .collect();

    let surfaces_per_slice: usize = if !face_indices.is_empty() {
        face_indices
            .len()
            .checked_mul(mip_count as usize)
            .ok_or_else(|| {
                DdsError::invalid(format!(
                    "DDS surfaces_per_slice overflow: {} faces × {} mips",
                    face_indices.len(),
                    mip_count,
                ))
            })?
    } else if is_volume {
        // One surface per (mip, depth_slice). Depth halves each mip
        // level (floored to 1). All summands are bounded by `base_depth`
        // which has been clamped above, so the running sum fits in a
        // `usize`.
        (0..mip_count)
            .map(|m| (base_depth >> m.min(31)).max(1) as usize)
            .sum()
    } else {
        mip_count as usize
    };
    let total_surfaces = (array_size as usize)
        .checked_mul(surfaces_per_slice)
        .ok_or_else(|| {
            DdsError::invalid(format!(
                "DDS total_surfaces overflow: array_size {} × {} surfaces/slice",
                array_size, surfaces_per_slice,
            ))
        })?;
    // Even if the multiplication fits in a `usize`, a Vec::with_capacity
    // for billions of entries is itself a panic risk on most allocators;
    // reject anything larger than a generous practical cap.
    const SURFACE_HARD_CAP: usize = 1 << 20; // 1M surfaces
    if total_surfaces > SURFACE_HARD_CAP {
        return Err(DdsError::invalid(format!(
            "DDS total_surfaces = {total_surfaces} exceeds hard cap of {SURFACE_HARD_CAP}",
        )));
    }

    let mut surfaces: Vec<DdsSurface> = Vec::with_capacity(total_surfaces);
    let mut cursor = pixel_data_off;

    if is_volume {
        // Volume (3D) texture: mip-major on-disk order. For each mip
        // level, `max(1, base_depth >> mip)` consecutive 2D slices are
        // stored back to back (each at that mip's width × height). The
        // depth shrinks alongside width/height per Microsoft's volume
        // mip rule.
        for (mi, &(mw, mh)) in mip_dims.iter().enumerate() {
            let mip_depth = (base_depth >> mi).max(1);
            for z in 0..mip_depth {
                let sb = surface_size_bytes(pix, mw, mh)? as usize;
                if bytes.len() < cursor + sb {
                    return Err(DdsError::invalid(format!(
                        "DDS pixel data truncated at mip={mi} slice={z} ({mw}x{mh}): need {sb} bytes, have {}",
                        bytes.len() - cursor,
                    )));
                }
                let stride = surface_stride_bytes(pix, mw);
                surfaces.push(DdsSurface {
                    width: mw,
                    height: mh,
                    mip_level: mi as u32,
                    array_slice: 0,
                    face: None,
                    depth_slice: z,
                    plane: DdsPlane {
                        stride,
                        data: bytes[cursor..cursor + sb].to_vec(),
                    },
                });
                cursor += sb;
            }
        }
    } else {
        for ai in 0..array_size {
            if face_indices.is_empty() {
                for (mi, &(mw, mh)) in mip_dims.iter().enumerate() {
                    let sb = surface_size_bytes(pix, mw, mh)? as usize;
                    if bytes.len() < cursor + sb {
                        return Err(DdsError::invalid(format!(
                            "DDS pixel data truncated at array={ai} mip={mi} ({mw}x{mh}): need {sb} bytes, have {}",
                            bytes.len() - cursor,
                        )));
                    }
                    let stride = surface_stride_bytes(pix, mw);
                    surfaces.push(DdsSurface {
                        width: mw,
                        height: mh,
                        mip_level: mi as u32,
                        array_slice: ai,
                        face: None,
                        depth_slice: 0,
                        plane: DdsPlane {
                            stride,
                            data: bytes[cursor..cursor + sb].to_vec(),
                        },
                    });
                    cursor += sb;
                }
            } else {
                for face in face_indices.iter() {
                    for (mi, &(mw, mh)) in mip_dims.iter().enumerate() {
                        let sb = surface_size_bytes(pix, mw, mh)? as usize;
                        if bytes.len() < cursor + sb {
                            return Err(DdsError::invalid(format!(
                                "DDS pixel data truncated at array={ai} face={} mip={mi} ({mw}x{mh}): need {sb} bytes, have {}",
                                face.short_name(),
                                bytes.len() - cursor,
                            )));
                        }
                        let stride = surface_stride_bytes(pix, mw);
                        surfaces.push(DdsSurface {
                            width: mw,
                            height: mh,
                            mip_level: mi as u32,
                            array_slice: ai,
                            face: Some(*face),
                            depth_slice: 0,
                            plane: DdsPlane {
                                stride,
                                data: bytes[cursor..cursor + sb].to_vec(),
                            },
                        });
                        cursor += sb;
                    }
                }
            }
        }
    }

    if surfaces.is_empty() {
        return Err(DdsError::invalid("DDS file produced zero surfaces"));
    }

    // Mirror surface[0] into `planes[0]` for the legacy single-surface
    // API surface that round-1 callers relied on.
    let primary_plane = surfaces[0].plane.clone();

    Ok(DdsImage {
        width,
        height,
        pixel_format: pix,
        planes: vec![primary_plane],
        surfaces,
        pts: None,
        mip_map_count: mip_count,
        has_dxt10_header: has_dxt10,
        dxgi_format: dxgi,
        is_cubemap,
        array_size,
        depth: base_depth,
    })
}

#[cfg(feature = "registry")]
pub(crate) fn make_decoder(
    params: &oxideav_core::CodecParameters,
) -> oxideav_core::Result<Box<dyn oxideav_core::Decoder>> {
    use crate::registry::DdsDecoder;
    let codec_id = params.codec_id.clone();
    Ok(Box::new(DdsDecoder::new(codec_id)))
}
