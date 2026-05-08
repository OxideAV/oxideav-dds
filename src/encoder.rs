//! DDS writer.
//!
//! Round 1 emits uncompressed surfaces only via
//! [`encode_dds_uncompressed`]. Block-compressed (BC*) bitstreams can
//! be wrapped on disk by callers that already have the encoded block
//! bytes, but the reader-side pass-through path is the round-1
//! contract; the round-1 encoder explicitly rejects block-compressed
//! [`DdsPixelFormat`] inputs to keep the contract symmetric.
//!
//! Round-3 / round-5 lift: when `image.mip_map_count > 1`, the encoder
//! emits a full mipmap chain. If `image.surfaces` already contains the
//! required (mip_count) entries the encoder writes them verbatim — the
//! caller pre-computed the chain. Otherwise the encoder fabricates
//! every level after mip 0 with a 2×2 box filter (with right-/bottom-
//! edge replication for odd-sized intermediate levels), respecting
//! Microsoft's mandate that each level halves dimensions (rounded down,
//! floored to 1) and the chain ends at the 1×1 surface.
//!
//! Round-4 lift: BC*-format mip chains can now be emitted via
//! [`encode_dds_block_compressed`]. The caller supplies a [`DdsImage`]
//! with a block-compressed [`DdsPixelFormat`] and `image.surfaces`
//! holding the per-mip pre-encoded block bytes (one entry per mip level
//! in declaration order). The encoder writes a DX10-extension header
//! (or a legacy FourCC header for BC1/2/3/4/5) and concatenates the
//! per-mip block streams.
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

/// Box-downsample a single uncompressed mip-level surface to the next
/// (half-size) level. `bytes_per_pixel` is the uncompressed sample
/// stride; channel arithmetic is unsigned 8-bit (the round-3 mip
/// downsampler is byte-domain — it works for all the round-1
/// uncompressed formats because their channels live in independent
/// bytes; for packed-bit formats like R5G6B5 the result is a noisy
/// approximation but still well-formed).
fn box_downsample(src: &[u8], src_w: u32, src_h: u32, bytes_per_pixel: u32) -> (Vec<u8>, u32, u32) {
    let dst_w = (src_w / 2).max(1);
    let dst_h = (src_h / 2).max(1);
    let bpp = bytes_per_pixel as usize;
    let src_stride = src_w as usize * bpp;
    let dst_stride = dst_w as usize * bpp;
    let mut dst = vec![0u8; dst_stride * dst_h as usize];
    for dy in 0..dst_h as usize {
        for dx in 0..dst_w as usize {
            // For odd dimensions, replicate the last in-bounds source
            // pixel rather than going out of bounds.
            let sx0 = (dx * 2).min(src_w as usize - 1);
            let sx1 = (dx * 2 + 1).min(src_w as usize - 1);
            let sy0 = (dy * 2).min(src_h as usize - 1);
            let sy1 = (dy * 2 + 1).min(src_h as usize - 1);
            for c in 0..bpp {
                let p00 = src[sy0 * src_stride + sx0 * bpp + c] as u32;
                let p01 = src[sy0 * src_stride + sx1 * bpp + c] as u32;
                let p10 = src[sy1 * src_stride + sx0 * bpp + c] as u32;
                let p11 = src[sy1 * src_stride + sx1 * bpp + c] as u32;
                dst[dy * dst_stride + dx * bpp + c] = ((p00 + p01 + p10 + p11 + 2) / 4) as u8;
            }
        }
    }
    (dst, dst_w, dst_h)
}

/// Compute the canonical mip-level dimensions list for `(width, height,
/// mip_count)` per Microsoft's rule: each level halves both dimensions
/// (floored to 1), and the chain has exactly `mip_count` entries
/// starting from level 0.
fn mip_dimensions(width: u32, height: u32, mip_count: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::with_capacity(mip_count as usize);
    let mut w = width;
    let mut h = height;
    for _ in 0..mip_count {
        out.push((w, h));
        w = (w / 2).max(1);
        h = (h / 2).max(1);
    }
    out
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
/// DX10 extension). When `image.mip_map_count > 1` the encoder emits
/// the full mipmap chain: if `image.surfaces` carries the required
/// number of mip levels (in declaration order, mip 0 first) those are
/// copied verbatim; otherwise the encoder fabricates every level
/// beyond mip 0 by a box-filter downsample of the previous level.
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

    // ---- Compose mip-chain payload --------------------------------
    //
    // If image.surfaces already holds `mip` entries (in mip order, no
    // cubemap / array indices), use them verbatim — caller pre-computed.
    // Otherwise box-filter mip 0 to fabricate levels 1..mip-1.
    let mip_dims = mip_dimensions(width, height, mip);
    let pre_supplied_chain: Option<Vec<&[u8]>> =
        if !with_mips || image.surfaces.len() < mip as usize {
            None
        } else if image.is_cubemap || image.array_size > 1 {
            // Multi-face / multi-slice: fall through; the round-3 mip-
            // chain emitter doesn't handle non-trivial array shapes.
            None
        } else {
            // Verify dimensions match the canonical chain.
            let mut ok = true;
            let mut chain: Vec<&[u8]> = Vec::with_capacity(mip as usize);
            for (i, (w, h)) in mip_dims.iter().enumerate() {
                let s = &image.surfaces[i];
                if s.width != *w || s.height != *h {
                    ok = false;
                    break;
                }
                let want = (*w as usize) * (*h as usize) * bpp as usize;
                if s.plane.data.len() < want {
                    ok = false;
                    break;
                }
                chain.push(&s.plane.data[..want]);
            }
            if ok {
                Some(chain)
            } else {
                None
            }
        };

    let total_payload: usize = mip_dims
        .iter()
        .map(|(w, h)| (*w as usize) * (*h as usize) * bpp as usize)
        .sum();

    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + total_payload);

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

    // ---- Emit mip 0.
    let mip0_bytes = need as usize;
    out.extend_from_slice(&plane.data[..mip0_bytes]);

    // ---- Emit mip 1..n (either pre-supplied or fabricated).
    if with_mips {
        if let Some(chain) = pre_supplied_chain {
            for level_data in chain.iter().skip(1) {
                out.extend_from_slice(level_data);
            }
        } else {
            // Fabricate by box-downsample.
            let mut prev: Vec<u8> = plane.data[..mip0_bytes].to_vec();
            let mut prev_w = width;
            let mut prev_h = height;
            for _ in 1..mip {
                let (next, nw, nh) = box_downsample(&prev, prev_w, prev_h, bpp);
                out.extend_from_slice(&next);
                prev = next;
                prev_w = nw;
                prev_h = nh;
            }
        }
    }

    Ok(out)
}

/// Map a block-compressed [`DdsPixelFormat`] to its on-disk
/// `DDS_PIXELFORMAT.four_cc`. Returns `None` for formats that have no
/// FourCC equivalent (BC6H, BC7) — those must use the DX10 extension
/// header. Per Microsoft's "DDS pixel format" page.
fn block_compressed_fourcc(pix: DdsPixelFormat) -> Option<u32> {
    match pix {
        DdsPixelFormat::Bc1 => Some(FOURCC_DXT1),
        DdsPixelFormat::Bc2 => Some(FOURCC_DXT3),
        DdsPixelFormat::Bc3 => Some(FOURCC_DXT5),
        DdsPixelFormat::Bc4Unorm => Some(FOURCC_BC4U),
        DdsPixelFormat::Bc4Snorm => Some(FOURCC_BC4S),
        DdsPixelFormat::Bc5Unorm => Some(FOURCC_BC5U),
        DdsPixelFormat::Bc5Snorm => Some(FOURCC_BC5S),
        // BC6H / BC7 require the DX10 extension header.
        DdsPixelFormat::Bc6hUf16
        | DdsPixelFormat::Bc6hSf16
        | DdsPixelFormat::Bc7Unorm
        | DdsPixelFormat::Bc7UnormSrgb => None,
        _ => None,
    }
}

/// Map a block-compressed [`DdsPixelFormat`] to its DX10 `DXGI_FORMAT`
/// integer code (per Microsoft's `DXGI_FORMAT` reference). Used when
/// emitting the DX10 extension header.
fn block_compressed_dxgi_code(pix: DdsPixelFormat) -> u32 {
    match pix {
        DdsPixelFormat::Bc1 => 71,          // BC1_UNORM
        DdsPixelFormat::Bc2 => 74,          // BC2_UNORM
        DdsPixelFormat::Bc3 => 77,          // BC3_UNORM
        DdsPixelFormat::Bc4Unorm => 80,     // BC4_UNORM
        DdsPixelFormat::Bc4Snorm => 81,     // BC4_SNORM
        DdsPixelFormat::Bc5Unorm => 83,     // BC5_UNORM
        DdsPixelFormat::Bc5Snorm => 84,     // BC5_SNORM
        DdsPixelFormat::Bc6hUf16 => 95,     // BC6H_UF16
        DdsPixelFormat::Bc6hSf16 => 96,     // BC6H_SF16
        DdsPixelFormat::Bc7Unorm => 98,     // BC7_UNORM
        DdsPixelFormat::Bc7UnormSrgb => 99, // BC7_UNORM_SRGB
        _ => 0,
    }
}

/// Compute the byte size of one block-compressed mip-level surface for
/// width × height.
fn block_compressed_surface_bytes(pix: DdsPixelFormat, width: u32, height: u32) -> usize {
    let bb = pix.block_bytes().expect("block-compressed format") as usize;
    let bw = width.max(1).div_ceil(4) as usize;
    let bh = height.max(1).div_ceil(4) as usize;
    bw * bh * bb
}

/// Encode a block-compressed [`DdsImage`] (BC1..BC7) as a DDS file.
///
/// The caller supplies pre-encoded block bytes via `image.surfaces`
/// (one entry per mip level in declaration order, mip 0 first). Each
/// surface's `plane.data` must be exactly
/// `ceil(width/4) × ceil(height/4) × block_bytes` long for that mip's
/// dimensions.
///
/// The encoder writes a legacy FourCC header for BC1..BC5 (matching
/// the on-disk layout `texconv` produces) and a DX10-extension header
/// for BC6H + BC7 (which have no legacy FourCC). When
/// `image.has_dxt10_header` is true the DX10 extension is forced for
/// every BC* format — useful for round-tripping a DXGI-tagged source.
///
/// `image.is_cubemap` and `image.array_size > 1` are not yet supported;
/// the encoder rejects those inputs.
pub fn encode_dds_block_compressed(image: &DdsImage) -> Result<Vec<u8>> {
    if !image.pixel_format.is_block_compressed() {
        return Err(DdsError::unsupported(format!(
            "encode_dds_block_compressed requires a block-compressed pixel_format (got {})",
            image.pixel_format.name()
        )));
    }
    if image.is_cubemap || image.array_size > 1 {
        return Err(DdsError::unsupported(
            "cubemap / DX10 texture-array block-compressed emission is not yet supported"
                .to_string(),
        ));
    }
    let width = image.width;
    let height = image.height;
    if width == 0 || height == 0 {
        return Err(DdsError::invalid(format!(
            "zero-sized surface: {width}x{height}"
        )));
    }
    let mip = image.mip_map_count.max(1);
    let with_mips = mip > 1;
    if image.surfaces.len() < mip as usize {
        return Err(DdsError::invalid(format!(
            "block-compressed encode requires {} pre-supplied surface(s), got {}",
            mip,
            image.surfaces.len()
        )));
    }

    // Validate per-mip dimensions match the canonical chain and size
    // matches `ceil(w/4) × ceil(h/4) × block_bytes`.
    let mip_dims = mip_dimensions(width, height, mip);
    for (i, (w, h)) in mip_dims.iter().enumerate() {
        let s = &image.surfaces[i];
        if s.width != *w || s.height != *h {
            return Err(DdsError::invalid(format!(
                "surface[{i}] dimensions {}x{} != canonical mip {} ({}x{})",
                s.width, s.height, i, *w, *h
            )));
        }
        let want = block_compressed_surface_bytes(image.pixel_format, *w, *h);
        if s.plane.data.len() < want {
            return Err(DdsError::invalid(format!(
                "surface[{i}] plane has {} bytes, expected ≥ {} ({}x{} {})",
                s.plane.data.len(),
                want,
                *w,
                *h,
                image.pixel_format.name(),
            )));
        }
    }

    // Decide DX10 vs legacy header.
    let four_cc = block_compressed_fourcc(image.pixel_format);
    let use_dx10 = image.has_dxt10_header || four_cc.is_none();

    let mip0_bytes = block_compressed_surface_bytes(image.pixel_format, width, height);
    let flags = DDSD_REQUIRED | DDSD_LINEARSIZE | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    let caps = DDSCAPS_TEXTURE
        | if with_mips {
            DDSCAPS_COMPLEX | DDSCAPS_MIPMAP
        } else {
            0
        };

    // ---- Build pixel format block.
    let pf = if use_dx10 {
        DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_FOURCC,
            four_cc: FOURCC_DX10,
            rgb_bit_count: 0,
            r_bit_mask: 0,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0,
        }
    } else {
        DdsPixelFormatHeader {
            size: DDS_PIXELFORMAT_SIZE as u32,
            flags: DDPF_FOURCC,
            four_cc: four_cc.expect("legacy FourCC available"),
            rgb_bit_count: 0,
            r_bit_mask: 0,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0,
        }
    };

    // ---- Compose output.
    let total_payload: usize = mip_dims
        .iter()
        .map(|(w, h)| block_compressed_surface_bytes(image.pixel_format, *w, *h))
        .sum();
    let header_bytes = 4 + DDS_HEADER_SIZE + if use_dx10 { DDS_HEADER_DXT10_SIZE } else { 0 };
    let mut out = Vec::with_capacity(header_bytes + total_payload);

    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    push_u32(&mut out, flags);
    push_u32(&mut out, height);
    push_u32(&mut out, width);
    push_u32(&mut out, mip0_bytes as u32); // pitch_or_linear_size = mip-0 byte count
    push_u32(&mut out, 0); // depth
    push_u32(&mut out, mip);
    for _ in 0..11 {
        push_u32(&mut out, 0); // reserved1
    }
    push_u32(&mut out, pf.size);
    push_u32(&mut out, pf.flags);
    push_u32(&mut out, pf.four_cc);
    push_u32(&mut out, pf.rgb_bit_count);
    push_u32(&mut out, pf.r_bit_mask);
    push_u32(&mut out, pf.g_bit_mask);
    push_u32(&mut out, pf.b_bit_mask);
    push_u32(&mut out, pf.a_bit_mask);
    push_u32(&mut out, caps);
    push_u32(&mut out, 0); // caps2
    push_u32(&mut out, 0); // caps3
    push_u32(&mut out, 0); // caps4
    push_u32(&mut out, 0); // reserved2

    if use_dx10 {
        let dxgi = image
            .dxgi_format
            .map(|f| f.to_u32())
            .unwrap_or_else(|| block_compressed_dxgi_code(image.pixel_format));
        push_u32(&mut out, dxgi);
        push_u32(&mut out, DDS_DIMENSION_TEXTURE2D);
        push_u32(&mut out, 0); // misc_flag
        push_u32(&mut out, image.array_size.max(1));
        push_u32(&mut out, 0); // misc_flags2
    }

    // ---- Emit per-mip block bytes.
    for (i, (w, h)) in mip_dims.iter().enumerate() {
        let want = block_compressed_surface_bytes(image.pixel_format, *w, *h);
        out.extend_from_slice(&image.surfaces[i].plane.data[..want]);
    }

    Ok(out)
}

#[cfg(feature = "registry")]
pub(crate) fn make_encoder(
    params: &oxideav_core::CodecParameters,
) -> oxideav_core::Result<Box<dyn oxideav_core::Encoder>> {
    use crate::registry::DdsEncoder;
    Ok(Box::new(DdsEncoder::from_params(params)?))
}
