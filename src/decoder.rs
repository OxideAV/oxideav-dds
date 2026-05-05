//! DDS reader.
//!
//! Parses a complete in-memory DDS byte slice into a [`DdsImage`].
//!
//! Reference: Microsoft's public "DDS file layout for textures" + "DDS
//! pixel format" + "DDS programming guide" pages on learn.microsoft.com.
//! No DirectXTex / D3DX / NVTT / squish / `dds.h` source consulted or
//! paraphrased.
//!
//! Round 1 covers:
//!
//! * Magic + `DDS_HEADER` (124 bytes) + optional `DDS_HEADER_DXT10`
//!   (20 bytes) parsing.
//! * Uncompressed RGB / RGBA / luminance / alpha layouts: A8R8G8B8,
//!   X8R8G8B8, R5G6B5, A1R5G5B5, A4R4G4B4, R8G8B8, A8L8, L8, A8.
//! * Block-compressed pass-through: DXT1/3/5 + BC4/5/6H/7. The reader
//!   recognises the format (legacy FourCC or DX10 dxgiFormat), computes
//!   the surface size from `ceil(w/4) × ceil(h/4) × block_bytes`, and
//!   hands the raw block bytes back via [`DdsImage::planes`].

use crate::error::{DdsError, Result};
use crate::image::{DdsImage, DdsPixelFormat, DdsPlane};
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
    if rgb && p.rgb_bit_count == 32 && !alpha_pixels {
        // X8R8G8B8: same masks as A8R8G8B8 but no alpha.
        if p.r_bit_mask == 0x00ff_0000 && p.g_bit_mask == 0x0000_ff00 && p.b_bit_mask == 0x0000_00ff
        {
            return Some(DdsPixelFormat::X8R8G8B8);
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
        // A4R4G4B4: A=f000, R=0f00, G=00f0, B=000f.
        if alpha_pixels
            && p.r_bit_mask == 0x0f00
            && p.g_bit_mask == 0x00f0
            && p.b_bit_mask == 0x000f
            && p.a_bit_mask == 0xf000
        {
            return Some(DdsPixelFormat::A4R4G4B4);
        }
    }

    if luminance {
        if p.rgb_bit_count == 8 && p.r_bit_mask == 0x00ff && !alpha_pixels {
            return Some(DdsPixelFormat::L8);
        }
        if p.rgb_bit_count == 16 && alpha_pixels && p.r_bit_mask == 0x00ff && p.a_bit_mask == 0xff00
        {
            return Some(DdsPixelFormat::A8L8);
        }
    }
    if alpha_only && p.rgb_bit_count == 8 && p.a_bit_mask == 0x00ff {
        return Some(DdsPixelFormat::A8);
    }

    None
}

/// Resolve a DX10 `DXGI_FORMAT` into a [`DdsPixelFormat`].
fn pixel_format_from_dxgi(d: DxgiFormat) -> Option<DdsPixelFormat> {
    Some(match d {
        // The DXGI naming convention swaps the apparent channel order
        // relative to the legacy D3D9 names: `R8G8B8A8_UNORM` is RGBA
        // bytes on disk, `B8G8R8A8_UNORM` is BGRA on disk. So R8G8B8A8
        // matches our crate-local A8B8G8R8 (RGBA on disk) and
        // B8G8R8A8 matches A8R8G8B8 (BGRA on disk).
        DxgiFormat::R8G8B8A8Unorm | DxgiFormat::R8G8B8A8UnormSrgb => DdsPixelFormat::A8B8G8R8,
        DxgiFormat::B8G8R8A8Unorm => DdsPixelFormat::A8R8G8B8,
        DxgiFormat::B8G8R8X8Unorm => DdsPixelFormat::X8R8G8B8,
        DxgiFormat::B5G6R5Unorm => DdsPixelFormat::R5G6B5,
        DxgiFormat::B5G5R5A1Unorm => DdsPixelFormat::A1R5G5B5,
        DxgiFormat::B4G4R4A4Unorm => DdsPixelFormat::A4R4G4B4,
        DxgiFormat::R8Unorm => DdsPixelFormat::L8,
        DxgiFormat::R8G8Unorm => DdsPixelFormat::A8L8,
        DxgiFormat::Bc1Unorm | DxgiFormat::Bc1UnormSrgb => DdsPixelFormat::Bc1,
        DxgiFormat::Bc2Unorm | DxgiFormat::Bc2UnormSrgb => DdsPixelFormat::Bc2,
        DxgiFormat::Bc3Unorm | DxgiFormat::Bc3UnormSrgb => DdsPixelFormat::Bc3,
        DxgiFormat::Bc4Unorm => DdsPixelFormat::Bc4Unorm,
        DxgiFormat::Bc4Snorm => DdsPixelFormat::Bc4Snorm,
        DxgiFormat::Bc5Unorm => DdsPixelFormat::Bc5Unorm,
        DxgiFormat::Bc5Snorm => DdsPixelFormat::Bc5Snorm,
        DxgiFormat::Bc6hUf16 => DdsPixelFormat::Bc6hUf16,
        DxgiFormat::Bc6hSf16 => DdsPixelFormat::Bc6hSf16,
        DxgiFormat::Bc7Unorm => DdsPixelFormat::Bc7Unorm,
        DxgiFormat::Bc7UnormSrgb => DdsPixelFormat::Bc7UnormSrgb,
        DxgiFormat::Unknown(_) => return None,
    })
}

/// Compute the in-memory mip-0 surface size in bytes for `pix` at the
/// given dimensions.
fn surface_size_bytes(pix: DdsPixelFormat, width: u32, height: u32) -> u64 {
    if let Some(bb) = pix.block_bytes() {
        return block_compressed_surface_size(width, height, bb);
    }
    let bpp = pix.bytes_per_pixel().expect("uncompressed format");
    width as u64 * height as u64 * bpp as u64
}

/// Per-row stride for `pix` at the given width. For block-compressed
/// formats this is `ceil(width/4) × block_bytes`.
fn surface_stride_bytes(pix: DdsPixelFormat, width: u32) -> usize {
    if let Some(bb) = pix.block_bytes() {
        return (width.max(1).div_ceil(4) * bb) as usize;
    }
    (width as u64 * pix.bytes_per_pixel().expect("uncompressed format") as u64) as usize
}

/// Parse a complete DDS byte stream.
///
/// The returned [`DdsImage`] always carries the mip-0 surface in a
/// single plane. Additional mip levels, cubemap faces, and texture
/// arrays are not surfaced in round 1 (the on-disk bytes for those are
/// preserved if the caller round-trips through [`crate::encode_dds_uncompressed`],
/// but the higher-level API does not expose them yet).
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

    let surface_bytes = surface_size_bytes(pix, width, height);
    let want_end = pixel_data_off as u64 + surface_bytes;
    if (bytes.len() as u64) < want_end {
        return Err(DdsError::invalid(format!(
            "DDS pixel data truncated: file has {} bytes, mip-0 surface needs {} bytes (offset {pixel_data_off})",
            bytes.len(),
            surface_bytes,
        )));
    }

    let stride = surface_stride_bytes(pix, width);
    let plane_bytes = surface_bytes as usize;
    let plane = DdsPlane {
        stride,
        data: bytes[pixel_data_off..pixel_data_off + plane_bytes].to_vec(),
    };

    let mip = if header.flags & DDSD_MIPMAPCOUNT != 0 {
        header.mip_map_count.max(1)
    } else {
        1
    };

    Ok(DdsImage {
        width,
        height,
        pixel_format: pix,
        planes: vec![plane],
        pts: None,
        mip_map_count: mip,
        has_dxt10_header: has_dxt10,
        dxgi_format: dxgi,
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
