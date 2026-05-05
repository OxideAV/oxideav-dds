//! DDS writer.
//!
//! Round 1 emits uncompressed surfaces only via
//! [`encode_dds_uncompressed`]. Block-compressed (BC*) bitstreams can
//! be wrapped on disk by callers that already have the encoded block
//! bytes, but the reader-side pass-through path is the round-1
//! contract; the round-1 encoder explicitly rejects block-compressed
//! [`DdsPixelFormat`] inputs to keep the contract symmetric.
//!
//! Reference: Microsoft's public "DDS file layout for textures" page on
//! learn.microsoft.com. No DirectXTex / D3DX / NVTT / squish source
//! consulted or paraphrased.

use crate::error::{DdsError, Result};
use crate::image::{DdsImage, DdsPixelFormat};
use crate::types::*;

/// Append a `u32` to `out` in little-endian byte order.
fn push_u32(out: &mut Vec<u8>, v: u32) {
    out.extend_from_slice(&v.to_le_bytes());
}

/// Build the 32-byte `DDS_PIXELFORMAT` block for a legacy uncompressed
/// surface. Returns the eight `u32` field values in declaration order.
fn legacy_pixel_format_fields(pix: DdsPixelFormat) -> Result<DdsPixelFormatHeader> {
    Ok(match pix {
        DdsPixelFormat::A8R8G8B8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            four_cc: 0,
            rgb_bit_count: 32,
            r_bit_mask: 0x00ff_0000,
            g_bit_mask: 0x0000_ff00,
            b_bit_mask: 0x0000_00ff,
            a_bit_mask: 0xff00_0000,
        },
        DdsPixelFormat::X8R8G8B8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB,
            four_cc: 0,
            rgb_bit_count: 32,
            r_bit_mask: 0x00ff_0000,
            g_bit_mask: 0x0000_ff00,
            b_bit_mask: 0x0000_00ff,
            a_bit_mask: 0,
        },
        DdsPixelFormat::A8B8G8R8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            four_cc: 0,
            rgb_bit_count: 32,
            r_bit_mask: 0x0000_00ff,
            g_bit_mask: 0x0000_ff00,
            b_bit_mask: 0x00ff_0000,
            a_bit_mask: 0xff00_0000,
        },
        DdsPixelFormat::R8G8B8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB,
            four_cc: 0,
            rgb_bit_count: 24,
            r_bit_mask: 0x00ff_0000,
            g_bit_mask: 0x0000_ff00,
            b_bit_mask: 0x0000_00ff,
            a_bit_mask: 0,
        },
        DdsPixelFormat::R5G6B5 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB,
            four_cc: 0,
            rgb_bit_count: 16,
            r_bit_mask: 0xf800,
            g_bit_mask: 0x07e0,
            b_bit_mask: 0x001f,
            a_bit_mask: 0,
        },
        DdsPixelFormat::A1R5G5B5 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            four_cc: 0,
            rgb_bit_count: 16,
            r_bit_mask: 0x7c00,
            g_bit_mask: 0x03e0,
            b_bit_mask: 0x001f,
            a_bit_mask: 0x8000,
        },
        DdsPixelFormat::A4R4G4B4 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_RGB | DDPF_ALPHAPIXELS,
            four_cc: 0,
            rgb_bit_count: 16,
            r_bit_mask: 0x0f00,
            g_bit_mask: 0x00f0,
            b_bit_mask: 0x000f,
            a_bit_mask: 0xf000,
        },
        DdsPixelFormat::A8L8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_LUMINANCE | DDPF_ALPHAPIXELS,
            four_cc: 0,
            rgb_bit_count: 16,
            r_bit_mask: 0x00ff,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0xff00,
        },
        DdsPixelFormat::L8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_LUMINANCE,
            four_cc: 0,
            rgb_bit_count: 8,
            r_bit_mask: 0x00ff,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0,
        },
        DdsPixelFormat::A8 => DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_ALPHA,
            four_cc: 0,
            rgb_bit_count: 8,
            r_bit_mask: 0,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0x00ff,
        },
        bc if bc.is_block_compressed() => {
            return Err(DdsError::unsupported(format!(
                "encode_dds_uncompressed cannot serialise block-compressed {} — round 1 is pass-through only",
                bc.name()
            )));
        }
        _ => {
            return Err(DdsError::unsupported(format!(
                "no legacy DDS_PIXELFORMAT for {}",
                pix.name()
            )));
        }
    })
}

/// Encode a [`DdsImage`] as an uncompressed DDS file.
///
/// `image.pixel_format` must be one of the round-1 uncompressed
/// formats (A8R8G8B8, X8R8G8B8, A8B8G8R8, R5G6B5, A1R5G5B5, A4R4G4B4,
/// R8G8B8, A8L8, L8, A8). Block-compressed formats return
/// [`DdsError::Unsupported`] — they are pass-through-only on the
/// reader side in round 1.
///
/// The plane data must already match
/// `width × height × bytes_per_pixel` bytes; the encoder copies it
/// verbatim into the file. No channel swapping or alpha-fill happens.
///
/// The emitted file always uses the legacy `DDS_HEADER` layout (no
/// DX10 extension). Mipmap count is set to `image.mip_map_count.max(1)`,
/// but the encoder does not fabricate additional mip levels — round 2
/// will add proper mipmap-chain handling.
pub fn encode_dds_uncompressed(image: &DdsImage) -> Result<Vec<u8>> {
    if image.pixel_format.is_block_compressed() {
        return Err(DdsError::unsupported(format!(
            "encode_dds_uncompressed cannot serialise block-compressed {} — round 1 is pass-through only",
            image.pixel_format.name()
        )));
    }
    if image.planes.len() != 1 {
        return Err(DdsError::invalid(format!(
            "DdsImage must carry exactly one plane (got {})",
            image.planes.len()
        )));
    }
    let plane = &image.planes[0];
    let width = image.width;
    let height = image.height;
    if width == 0 || height == 0 {
        return Err(DdsError::invalid(format!(
            "zero-sized surface: {width}x{height}"
        )));
    }
    let bpp = image
        .pixel_format
        .bytes_per_pixel()
        .expect("checked uncompressed above");
    let pitch = width as u64 * bpp as u64;
    let need = pitch * height as u64;
    if (plane.data.len() as u64) < need {
        return Err(DdsError::invalid(format!(
            "plane data {} bytes < expected {} bytes ({}x{} {} @ {} bpp)",
            plane.data.len(),
            need,
            width,
            height,
            image.pixel_format.name(),
            bpp * 8,
        )));
    }
    if plane.stride != pitch as usize {
        return Err(DdsError::invalid(format!(
            "plane.stride {} != width × bytes_per_pixel = {}",
            plane.stride, pitch
        )));
    }

    let pf = legacy_pixel_format_fields(image.pixel_format)?;
    let mip = image.mip_map_count.max(1);
    let with_mips = mip > 1;
    let flags = DDSD_REQUIRED | DDSD_PITCH | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    let caps = DDSCAPS_TEXTURE
        | if with_mips {
            DDSCAPS_COMPLEX | DDSCAPS_MIPMAP
        } else {
            0
        };

    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + need as usize);

    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    push_u32(&mut out, flags);
    push_u32(&mut out, height);
    push_u32(&mut out, width);
    push_u32(&mut out, pitch as u32);
    push_u32(&mut out, 0); // depth
    push_u32(&mut out, mip);
    for _ in 0..11 {
        push_u32(&mut out, 0); // reserved1
    }
    // pixel_format (32 bytes)
    push_u32(&mut out, pf.size);
    push_u32(&mut out, pf.flags);
    push_u32(&mut out, pf.four_cc);
    push_u32(&mut out, pf.rgb_bit_count);
    push_u32(&mut out, pf.r_bit_mask);
    push_u32(&mut out, pf.g_bit_mask);
    push_u32(&mut out, pf.b_bit_mask);
    push_u32(&mut out, pf.a_bit_mask);
    // caps + caps2..4 + reserved2 (5 × u32 = 20 bytes)
    push_u32(&mut out, caps);
    push_u32(&mut out, 0); // caps2
    push_u32(&mut out, 0); // caps3
    push_u32(&mut out, 0); // caps4
    push_u32(&mut out, 0); // reserved2

    out.extend_from_slice(&plane.data[..need as usize]);

    Ok(out)
}

#[cfg(feature = "registry")]
pub(crate) fn make_encoder(
    params: &oxideav_core::CodecParameters,
) -> oxideav_core::Result<Box<dyn oxideav_core::Encoder>> {
    use crate::registry::DdsEncoder;
    Ok(Box::new(DdsEncoder::from_params(params)?))
}
