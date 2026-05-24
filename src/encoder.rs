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
use crate::image::{DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
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

/// Per-mip depth (z) slice count for a volume texture: the base depth
/// halved once per mip level, floored to 1 — Microsoft's volume mip
/// rule mirrors the width / height rule.
fn volume_mip_depths(base_depth: u32, mip_count: u32) -> Vec<u32> {
    (0..mip_count).map(|m| (base_depth >> m).max(1)).collect()
}

/// Encode an uncompressed volume (3D) texture as a DDS file.
///
/// `image.depth` is the mip-0 depth (z) slice count and must be `> 1`
/// (use [`encode_dds_uncompressed`] for plain 2D surfaces).
/// `image.pixel_format` must be one of the uncompressed formats (the
/// block-compressed formats are rejected — symmetry with the 2D
/// uncompressed encoder).
///
/// The caller supplies every depth slice via `image.surfaces` in the
/// on-disk order Microsoft mandates for volume textures: outer loop
/// over mip level, inner loop over depth slice. At mip level `m` there
/// are `max(1, depth >> m)` slices, each at `(width >> m, height >> m)`
/// (floored to 1). The total surface count is therefore the sum of the
/// per-mip slice counts. Each surface's `plane.data` must hold exactly
/// `mip_w × mip_h × bytes_per_pixel` bytes.
///
/// The emitted file uses the legacy `DDS_HEADER` layout with
/// `DDSD_DEPTH` set in `flags`, `header.depth` carrying the slice
/// count, and `DDSCAPS2_VOLUME` set in `caps2` (plus
/// `DDSCAPS_COMPLEX` so DirectX recognises the child surfaces).
pub fn encode_dds_volume(image: &DdsImage) -> Result<Vec<u8>> {
    if image.pixel_format.is_block_compressed() {
        return Err(DdsError::unsupported(format!(
            "encode_dds_volume cannot serialise block-compressed {} (uncompressed volume only)",
            image.pixel_format.name()
        )));
    }
    let width = image.width;
    let height = image.height;
    let depth = image.depth;
    if width == 0 || height == 0 || depth == 0 {
        return Err(DdsError::invalid(format!(
            "zero-sized volume: {width}x{height}x{depth}"
        )));
    }
    if depth < 2 {
        return Err(DdsError::invalid(
            "encode_dds_volume requires depth > 1 (use encode_dds_uncompressed for 2D surfaces)",
        ));
    }
    if image.is_cubemap || image.array_size > 1 {
        return Err(DdsError::unsupported(
            "a volume texture cannot also be a cubemap or texture array".to_string(),
        ));
    }
    let bpp = image
        .pixel_format
        .bytes_per_pixel()
        .expect("checked uncompressed above");

    let mip = image.mip_map_count.max(1);
    let with_mips = mip > 1;
    let mip_dims = mip_dimensions(width, height, mip);
    let mip_depths = volume_mip_depths(depth, mip);

    // Validate the supplied surface list: one surface per (mip, slice),
    // in mip-major order, each carrying the right dimensions + byte
    // count.
    let total_surfaces: usize = mip_depths.iter().map(|&d| d as usize).sum();
    if image.surfaces.len() < total_surfaces {
        return Err(DdsError::invalid(format!(
            "volume encode requires {} surface(s) (sum of per-mip slice counts), got {}",
            total_surfaces,
            image.surfaces.len()
        )));
    }
    let mut si = 0usize;
    let mut payload = Vec::new();
    for (mi, &(mw, mh)) in mip_dims.iter().enumerate() {
        let want = (mw as usize) * (mh as usize) * bpp as usize;
        for z in 0..mip_depths[mi] {
            let s = &image.surfaces[si];
            if s.width != mw || s.height != mh {
                return Err(DdsError::invalid(format!(
                    "surface[{si}] dims {}x{} != canonical mip {mi} slice {z} ({mw}x{mh})",
                    s.width, s.height
                )));
            }
            if s.plane.data.len() < want {
                return Err(DdsError::invalid(format!(
                    "surface[{si}] has {} bytes, expected ≥ {want} ({mw}x{mh} {} @ {} bpp)",
                    s.plane.data.len(),
                    image.pixel_format.name(),
                    bpp * 8,
                )));
            }
            payload.extend_from_slice(&s.plane.data[..want]);
            si += 1;
        }
    }

    let pf = legacy_pixel_format_fields(image.pixel_format)?;
    let pitch = width as u64 * bpp as u64;
    let flags =
        DDSD_REQUIRED | DDSD_PITCH | DDSD_DEPTH | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    let mut caps = DDSCAPS_TEXTURE | DDSCAPS_COMPLEX;
    if with_mips {
        caps |= DDSCAPS_MIPMAP;
    }
    let caps2 = DDSCAPS2_VOLUME;

    let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + payload.len());
    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    push_u32(&mut out, flags);
    push_u32(&mut out, height);
    push_u32(&mut out, width);
    push_u32(&mut out, pitch as u32);
    push_u32(&mut out, depth);
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
    push_u32(&mut out, caps2);
    push_u32(&mut out, 0); // caps3
    push_u32(&mut out, 0); // caps4
    push_u32(&mut out, 0); // reserved2

    out.extend_from_slice(&payload);
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

/// Encode an RGBA8 surface as a block-compressed DDS file with an
/// optional fully-fabricated mipmap chain.
///
/// `pixel_format` selects the destination BC* format and must be one of
/// the LDR encoders (`Bc1`, `Bc2`, `Bc3`, `Bc4Unorm`, `Bc5Unorm`,
/// `Bc7Unorm`, `Bc7UnormSrgb`). For HDR (BC6H) use [`encode_bc6h`] +
/// [`encode_dds_block_compressed`] directly with a pre-encoded chain.
///
/// `rgba8` must hold `width × height × 4` bytes. When `mip_map_count > 1`
/// the encoder generates each subsequent level by 2×2 box-filter
/// downsampling the previous RGBA8 level (with right-/bottom-edge
/// replication for odd dimensions), then encodes that level to BC*
/// blocks. The on-disk byte stream is identical to what one would get
/// by manually downsampling, calling the per-format encoder for each
/// level, and feeding the surfaces into [`encode_dds_block_compressed`].
///
/// `is_cubemap` and `array_size > 1` follow the same emission path —
/// each face / slice gets its own RGBA8 source, packed in the
/// Microsoft / DX11 mandated outer-loop-first order: array slice → face
/// → mip level. The `rgba8` slice is partitioned into one mip-0 surface
/// per (slice, face) tuple, in that same order, with each tuple
/// occupying `width × height × 4` consecutive bytes.
///
/// Returns the full DDS file bytes (4-byte magic + 124-byte
/// `DDS_HEADER` + optional 20-byte `DDS_HEADER_DXT10` + concatenated
/// per-(slice, face, mip) BC* block streams).
///
/// [`encode_bc6h`]: crate::encode_bc6h
#[allow(clippy::too_many_arguments)]
pub fn encode_dds_block_compressed_from_rgba8(
    rgba8: &[u8],
    width: u32,
    height: u32,
    pixel_format: DdsPixelFormat,
    mip_map_count: u32,
    is_cubemap: bool,
    array_size: u32,
    has_dxt10_header: bool,
) -> Result<Vec<u8>> {
    if !pixel_format.is_block_compressed() {
        return Err(DdsError::unsupported(format!(
            "encode_dds_block_compressed_from_rgba8 requires a block-compressed pixel_format (got {})",
            pixel_format.name()
        )));
    }
    if matches!(
        pixel_format,
        DdsPixelFormat::Bc6hUf16 | DdsPixelFormat::Bc6hSf16
    ) {
        return Err(DdsError::unsupported(
            "BC6H encode-from-RGBA8 is not supported (BC6H is HDR — use encode_bc6h_from_f32 + encode_dds_block_compressed)"
                .to_string(),
        ));
    }
    if width == 0 || height == 0 {
        return Err(DdsError::invalid(format!(
            "zero-sized surface: {width}x{height}"
        )));
    }
    let mip = mip_map_count.max(1);
    let array_n = array_size.max(1);
    let face_count: u32 = if is_cubemap { 6 } else { 1 };
    let surfaces_per_slice = face_count * mip;
    let _ = surfaces_per_slice;

    let bytes_per_mip0 = (width as usize) * (height as usize) * 4;
    let need_in = bytes_per_mip0 * (array_n as usize) * (face_count as usize);
    if rgba8.len() < need_in {
        return Err(DdsError::invalid(format!(
            "RGBA8 input {} bytes < expected {} bytes for {}x{} × {} slice(s) × {} face(s)",
            rgba8.len(),
            need_in,
            width,
            height,
            array_n,
            face_count
        )));
    }

    // ---- For each (slice, face), generate the mip chain (box-filter)
    //      and encode each level to BC* blocks. Stash the per-level
    //      DdsSurface entries in the on-disk order (slice → face → mip).
    let mip_dims = mip_dimensions(width, height, mip);
    let mut surfaces: Vec<DdsSurface> =
        Vec::with_capacity((array_n as usize) * (face_count as usize) * (mip as usize));

    for slice in 0..array_n {
        for face_idx in 0..face_count {
            let face = if is_cubemap {
                Some(crate::image::CubemapFace::ALL[face_idx as usize])
            } else {
                None
            };
            let src_off =
                ((slice as usize) * (face_count as usize) + (face_idx as usize)) * bytes_per_mip0;
            let mip0_rgba: &[u8] = &rgba8[src_off..src_off + bytes_per_mip0];

            // Build the per-mip RGBA8 chain (box-filter).
            let mut prev_rgba: Vec<u8> = mip0_rgba.to_vec();
            let mut prev_w = width;
            let mut prev_h = height;
            for (level, &(mw, mh)) in mip_dims.iter().enumerate() {
                let level_rgba: Vec<u8> = if level == 0 {
                    mip0_rgba.to_vec()
                } else {
                    let (downsampled, nw, nh) = box_downsample(&prev_rgba, prev_w, prev_h, 4);
                    debug_assert_eq!(nw, mw);
                    debug_assert_eq!(nh, mh);
                    prev_rgba = downsampled.clone();
                    prev_w = nw;
                    prev_h = nh;
                    downsampled
                };

                // Encode this RGBA8 level to BC* blocks.
                let want = block_compressed_surface_bytes(pixel_format, mw, mh);
                let mut bc = vec![0u8; want];
                encode_rgba8_to_bc_level(&level_rgba, mw, mh, pixel_format, &mut bc)?;

                let bw = mw.max(1).div_ceil(4) as usize;
                let bb = pixel_format.block_bytes().expect("block-compressed") as usize;
                surfaces.push(DdsSurface {
                    width: mw,
                    height: mh,
                    mip_level: level as u32,
                    array_slice: slice,
                    face,
                    depth_slice: 0,
                    plane: DdsPlane {
                        stride: bw * bb,
                        data: bc,
                    },
                });
            }
        }
    }

    // ---- Compose the DdsImage and delegate to the existing emitter.
    //      The existing emitter rejects cubemap / array_size > 1, so we
    //      handle the multi-surface composition inline below for those
    //      shapes.
    if !is_cubemap && array_n == 1 {
        let img = DdsImage {
            width,
            height,
            pixel_format,
            planes: vec![surfaces[0].plane.clone()],
            surfaces,
            pts: None,
            mip_map_count: mip,
            has_dxt10_header,
            dxgi_format: None,
            is_cubemap: false,
            array_size: 1,
            depth: 1,
        };
        return encode_dds_block_compressed(&img);
    }

    // Multi-face / array path: emit DDS bytes inline.
    encode_dds_block_compressed_multi_surface(
        width,
        height,
        pixel_format,
        mip,
        is_cubemap,
        array_n,
        has_dxt10_header,
        &surfaces,
    )
}

/// Encode a single RGBA8 mip level into the destination BC* format.
fn encode_rgba8_to_bc_level(
    rgba: &[u8],
    width: u32,
    height: u32,
    pixel_format: DdsPixelFormat,
    output: &mut [u8],
) -> Result<()> {
    match pixel_format {
        DdsPixelFormat::Bc1 => crate::bcn_enc::encode_bc1(rgba, width, height, false, output),
        DdsPixelFormat::Bc2 => crate::bcn_enc::encode_bc2(rgba, width, height, output),
        DdsPixelFormat::Bc3 => crate::bcn_enc::encode_bc3(rgba, width, height, output),
        DdsPixelFormat::Bc4Unorm => {
            // BC4 takes a single channel. Extract R from RGBA.
            let r_only: Vec<u8> = rgba.chunks_exact(4).map(|p| p[0]).collect();
            crate::bcn_enc::encode_bc4_unorm(&r_only, width, height, output)
        }
        DdsPixelFormat::Bc5Unorm => {
            // BC5 takes two channels. Extract RG from RGBA.
            let mut rg = Vec::with_capacity(rgba.len() / 2);
            for p in rgba.chunks_exact(4) {
                rg.push(p[0]);
                rg.push(p[1]);
            }
            crate::bcn_enc::encode_bc5_unorm(&rg, width, height, output)
        }
        DdsPixelFormat::Bc7Unorm | DdsPixelFormat::Bc7UnormSrgb => {
            crate::bc7_enc::encode_bc7(rgba, width, height, output)
        }
        _ => Err(DdsError::unsupported(format!(
            "encode_rgba8_to_bc_level: unsupported destination format {}",
            pixel_format.name()
        ))),
    }
}

/// Multi-(slice, face) BC* DDS emission. Mirrors
/// [`encode_dds_block_compressed`] for the 2D path but writes the full
/// (slice → face → mip) surface array. Only used by
/// [`encode_dds_block_compressed_from_rgba8`] for cubemap / array_size>1
/// inputs; the plain 2D path still goes through `encode_dds_block_compressed`.
#[allow(clippy::too_many_arguments)]
fn encode_dds_block_compressed_multi_surface(
    width: u32,
    height: u32,
    pixel_format: DdsPixelFormat,
    mip: u32,
    is_cubemap: bool,
    array_size: u32,
    has_dxt10_header: bool,
    surfaces: &[DdsSurface],
) -> Result<Vec<u8>> {
    let with_mips = mip > 1;
    let four_cc = block_compressed_fourcc(pixel_format);
    // Cubemap / array always uses DX10 extension to expose the
    // arrayness; legacy headers can't encode array_size or cube-array
    // texture types.
    let use_dx10 = has_dxt10_header || four_cc.is_none() || is_cubemap || array_size > 1;

    let mip0_bytes = block_compressed_surface_bytes(pixel_format, width, height);
    let flags = DDSD_REQUIRED | DDSD_LINEARSIZE | if with_mips { DDSD_MIPMAPCOUNT } else { 0 };
    let mut caps = DDSCAPS_TEXTURE;
    if with_mips {
        caps |= DDSCAPS_COMPLEX | DDSCAPS_MIPMAP;
    }
    if is_cubemap {
        caps |= DDSCAPS_COMPLEX;
    }
    let caps2 = if is_cubemap {
        DDSCAPS2_CUBEMAP
            | DDSCAPS2_CUBEMAP_POSITIVEX
            | DDSCAPS2_CUBEMAP_NEGATIVEX
            | DDSCAPS2_CUBEMAP_POSITIVEY
            | DDSCAPS2_CUBEMAP_NEGATIVEY
            | DDSCAPS2_CUBEMAP_POSITIVEZ
            | DDSCAPS2_CUBEMAP_NEGATIVEZ
    } else {
        0
    };

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
            four_cc: four_cc.expect("legacy FourCC"),
            rgb_bit_count: 0,
            r_bit_mask: 0,
            g_bit_mask: 0,
            b_bit_mask: 0,
            a_bit_mask: 0,
        }
    };

    let total_payload: usize = surfaces.iter().map(|s| s.plane.data.len()).sum();
    let header_bytes = 4 + DDS_HEADER_SIZE + if use_dx10 { DDS_HEADER_DXT10_SIZE } else { 0 };
    let mut out = Vec::with_capacity(header_bytes + total_payload);

    push_u32(&mut out, DDS_MAGIC);
    push_u32(&mut out, DDS_HEADER_SIZE as u32);
    push_u32(&mut out, flags);
    push_u32(&mut out, height);
    push_u32(&mut out, width);
    push_u32(&mut out, mip0_bytes as u32);
    push_u32(&mut out, 0); // depth
    push_u32(&mut out, mip);
    for _ in 0..11 {
        push_u32(&mut out, 0);
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
    push_u32(&mut out, caps2);
    push_u32(&mut out, 0); // caps3
    push_u32(&mut out, 0); // caps4
    push_u32(&mut out, 0); // reserved2

    if use_dx10 {
        let dxgi = block_compressed_dxgi_code(pixel_format);
        push_u32(&mut out, dxgi);
        push_u32(&mut out, DDS_DIMENSION_TEXTURE2D);
        // misc_flag: cubemap → DDS_RESOURCE_MISC_TEXTURECUBE.
        let misc_flag: u32 = if is_cubemap { 0x4 } else { 0 };
        push_u32(&mut out, misc_flag);
        // array_size for cubemap is the number of cube arrays (i.e.
        // total faces / 6); for plain 2D arrays it's the slice count.
        // We pass through `array_size` verbatim — callers always supply
        // it as the slice count (cubemaps default to 1 = one cube).
        let store_array_size = array_size.max(1);
        push_u32(&mut out, store_array_size);
        push_u32(&mut out, 0); // misc_flags2
    }

    for s in surfaces {
        out.extend_from_slice(&s.plane.data);
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
