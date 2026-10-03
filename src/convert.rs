//! Stored-layout ↔ contract-layout conversion.
//!
//! * [`surface_to_image`] expands one stored [`DdsSurface`] (any
//!   [`SurfaceFormat`] with a [`SurfaceFormat::contract_format`]) into a
//!   contract [`DdsImage`] — the decode direction behind
//!   [`crate::decode`] / [`crate::decode_all`].
//! * [`image_to_surface`] converts a contract [`DdsImage`] into the
//!   tightly packed bytes of a target [`SurfaceFormat`] — the encode
//!   direction behind [`crate::encode`] / [`crate::encode_all`],
//!   including the block encoders.
//! * [`downsample`] is the 2×2 box filter that generates mip levels in
//!   the contract layout (per channel, in the sample's own domain, with
//!   edge replication for odd dimensions).
//!
//! The expansion rules are the ones documented on
//! [`SurfaceFormat::contract_format`]; the reverse direction quantises
//! by truncation of the replicated bits (`v >> 3` for a 5-bit channel),
//! so `decode(encode_with(sf, decode(file))) == decode(file)` for every
//! integer layout.

use crate::error::{DdsError, Result};
use crate::image::{DdsImage, DdsPixelFormat as P};
use crate::surface::{DdsFile, DdsSurface, SurfaceFormat as S};

// ---- bit-replication widening ------------------------------------------

#[inline]
const fn expand1(v: u16) -> u8 {
    if v != 0 {
        0xFF
    } else {
        0
    }
}
#[inline]
const fn expand4(v: u16) -> u8 {
    ((v & 0xF) * 0x11) as u8
}
#[inline]
const fn expand5(v: u16) -> u8 {
    let v = (v & 0x1F) as u8;
    (v << 3) | (v >> 2)
}
#[inline]
const fn expand6(v: u16) -> u8 {
    let v = (v & 0x3F) as u8;
    (v << 2) | (v >> 4)
}
/// 10 → 16 bits by bit replication (`v << 6 | v >> 4`).
#[inline]
const fn expand10(v: u16) -> u16 {
    let v = v & 0x3FF;
    (v << 6) | (v >> 4)
}
/// 2 → 16 bits by bit replication.
#[inline]
const fn expand2_16(v: u16) -> u16 {
    (v & 3) * 0x5555
}

/// The DXGI SNORM rule: `v / (2^(n-1) - 1)`, both minimum encodings → -1.
#[inline]
fn snorm8(v: i8) -> f32 {
    (v as f32 / 127.0).max(-1.0)
}
#[inline]
fn snorm16(v: i16) -> f32 {
    (v as f32 / 32767.0).max(-1.0)
}

fn push_f32s(out: &mut Vec<u8>, vals: &[f32]) {
    for v in vals {
        out.extend_from_slice(&v.to_le_bytes());
    }
}

fn pixel_count(w: u32, h: u32) -> Result<usize> {
    (w as usize)
        .checked_mul(h as usize)
        .ok_or_else(|| DdsError::unsupported(format!("{w}x{h} exceeds the platform address space")))
}

fn need(data: &[u8], want: usize, what: &str) -> Result<()> {
    if data.len() < want {
        return Err(DdsError::invalid(format!(
            "{what}: surface holds {} bytes, needs {want}",
            data.len()
        )));
    }
    Ok(())
}

/// Read `n` little-endian `u16` words.
fn le_u16s(data: &[u8], n: usize) -> Vec<u16> {
    data.chunks_exact(2)
        .take(n)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

// ---- decode direction ----------------------------------------------------

/// Expand one stored surface of `file` into the contract layout (see
/// [`SurfaceFormat::contract_format`]).
pub(crate) fn surface_to_image(file: &DdsFile, s: &DdsSurface) -> Result<DdsImage> {
    let pix = file.pixel_format;
    let target = pix.contract_format().ok_or_else(|| {
        DdsError::unsupported(format!(
            "{} surfaces have no contract layout; use parse_dds and the matching decode_*_surface helper",
            pix.name()
        ))
    })?;
    let (w, h) = (s.width, s.height);
    let n = pixel_count(w, h)?;
    let data = s.plane.data.as_slice();
    let name = pix.name();

    let out: Vec<u8> = match pix {
        // Byte-identical layouts.
        S::A8R8G8B8 | S::A8B8G8R8 => {
            need(data, n * 4, name)?;
            data[..n * 4].to_vec()
        }
        S::X8R8G8B8 | S::X8B8G8R8 => {
            need(data, n * 4, name)?;
            let mut v = data[..n * 4].to_vec();
            for px in v.chunks_exact_mut(4) {
                px[3] = 0xFF;
            }
            v
        }
        S::R8G8B8 => {
            need(data, n * 3, name)?;
            data[..n * 3].to_vec()
        }
        S::L8 | S::R8Unorm => {
            need(data, n, name)?;
            data[..n].to_vec()
        }
        S::A8L8 => {
            need(data, n * 2, name)?;
            data[..n * 2].to_vec()
        }
        S::A4L4 => {
            need(data, n, name)?;
            let mut v = Vec::with_capacity(n * 2);
            for &b in &data[..n] {
                v.push(expand4((b & 0x0F) as u16));
                v.push(expand4((b >> 4) as u16));
            }
            v
        }
        S::A8 => {
            need(data, n, name)?;
            let mut v = Vec::with_capacity(n * 4);
            for &a in &data[..n] {
                v.extend_from_slice(&[0, 0, 0, a]);
            }
            v
        }
        S::L16 | S::R16Unorm => {
            need(data, n * 2, name)?;
            data[..n * 2].to_vec()
        }
        S::R16G16B16A16Unorm => {
            need(data, n * 8, name)?;
            data[..n * 8].to_vec()
        }
        S::R16G16Unorm => {
            need(data, n * 4, name)?;
            let mut v = Vec::with_capacity(n * 8);
            for px in data[..n * 4].chunks_exact(4) {
                v.extend_from_slice(px);
                v.extend_from_slice(&[0, 0, 0xFF, 0xFF]);
            }
            v
        }
        // Packed 16-bit colour.
        S::R5G6B5 => {
            need(data, n * 2, name)?;
            let mut v = Vec::with_capacity(n * 4);
            for p in le_u16s(data, n) {
                v.extend_from_slice(&[expand5(p >> 11), expand6(p >> 5), expand5(p), 0xFF]);
            }
            v
        }
        S::A1R5G5B5 | S::X1R5G5B5 => {
            need(data, n * 2, name)?;
            let has_a = pix == S::A1R5G5B5;
            let mut v = Vec::with_capacity(n * 4);
            for p in le_u16s(data, n) {
                let a = if has_a { expand1(p >> 15) } else { 0xFF };
                v.extend_from_slice(&[expand5(p >> 10), expand5(p >> 5), expand5(p), a]);
            }
            v
        }
        S::A4R4G4B4 | S::X4R4G4B4 => {
            need(data, n * 2, name)?;
            let has_a = pix == S::A4R4G4B4;
            let mut v = Vec::with_capacity(n * 4);
            for p in le_u16s(data, n) {
                let a = if has_a { expand4(p >> 12) } else { 0xFF };
                v.extend_from_slice(&[expand4(p >> 8), expand4(p >> 4), expand4(p), a]);
            }
            v
        }
        S::A8R3G3B2 => crate::hdr::decode_a8r3g3b2_surface(w, h, data)?,
        S::A4B4G4R4Unorm => crate::hdr::decode_a4b4g4r4_unorm_surface(w, h, data)?,
        S::R8G8B8G8Unorm => crate::hdr::decode_r8g8_b8g8_unorm_surface(w, h, data)?,
        S::G8R8G8B8Unorm => crate::hdr::decode_g8r8_g8b8_unorm_surface(w, h, data)?,
        // Packed 10:10:10:2 → 16-bit.
        S::R10G10B10A2Unorm | S::A2R10G10B10 => {
            let codes = if pix == S::R10G10B10A2Unorm {
                crate::hdr::decode_r10g10b10a2_unorm_surface(w, h, data)?
            } else {
                crate::hdr::decode_a2r10g10b10_surface(w, h, data)?
            };
            let mut v = Vec::with_capacity(n * 8);
            for px in codes.chunks_exact(4) {
                for c in &px[..3] {
                    v.extend_from_slice(&expand10(*c).to_le_bytes());
                }
                v.extend_from_slice(&expand2_16(px[3]).to_le_bytes());
            }
            v
        }
        // Block-compressed → RGBA8 / R8.
        S::Bc1 | S::Bc2 | S::Bc3 | S::Bc7Unorm | S::Bc7UnormSrgb => {
            let mut v = vec![0u8; n * 4];
            match pix {
                S::Bc1 => crate::bcn::decode_bc1(data, w, h, &mut v)?,
                S::Bc2 => crate::bcn::decode_bc2(data, w, h, &mut v)?,
                S::Bc3 => crate::bcn::decode_bc3(data, w, h, &mut v)?,
                _ => crate::bc7::decode_bc7(data, w, h, &mut v)?,
            }
            v
        }
        S::Bc4Unorm => {
            let mut v = vec![0u8; n];
            crate::bcn::decode_bc4_unorm(data, w, h, &mut v)?;
            v
        }
        S::Bc5Unorm => {
            let mut rg = vec![0u8; n * 2];
            crate::bcn::decode_bc5_unorm(data, w, h, &mut rg)?;
            let mut v = Vec::with_capacity(n * 4);
            for p in rg.chunks_exact(2) {
                v.extend_from_slice(&[p[0], p[1], 0, 0xFF]);
            }
            v
        }
        S::Astc { .. } => crate::astc::decode_astc_ldr_surface(pix, data, w, h)
            .ok_or_else(|| DdsError::unsupported("ASTC footprint"))?,
        // Floating point → f32.
        S::R16Float
        | S::R32Float
        | S::R16G16Float
        | S::R32G32Float
        | S::R16G16B16A16Float
        | S::R32G32B32A32Float => {
            let samples = crate::hdr::decode_float_surface(pix, w, h, data)?;
            let ch = samples.len() / n.max(1);
            widen_channels(&samples, ch, n)
        }
        S::R11G11B10Float => {
            let samples = crate::hdr::decode_r11g11b10_float_surface(w, h, data)?;
            let mut v = Vec::with_capacity(n * 12);
            push_f32s(&mut v, &samples);
            v
        }
        S::R9G9B9E5SharedExp => {
            let samples = crate::hdr::decode_r9g9b9e5_sharedexp_surface(w, h, data)?;
            let mut v = Vec::with_capacity(n * 12);
            push_f32s(&mut v, &samples);
            v
        }
        // Signed-normalised → f32.
        S::R8Snorm | S::R16Snorm | S::R8G8Snorm | S::R16G16Snorm | S::R8G8B8A8Snorm => {
            let samples = crate::hdr::decode_snorm_surface(pix, w, h, data)?;
            let ch = samples.len() / n.max(1);
            widen_channels(&samples, ch, n)
        }
        S::R16G16B16A16Snorm => {
            let raw = crate::hdr::decode_rgba16_snorm_surface(w, h, data)?;
            let samples: Vec<f32> = raw.iter().map(|&v| snorm16(v)).collect();
            widen_channels(&samples, 4, n)
        }
        S::Bc4Snorm => {
            let raw = crate::bcn::decode_bc4_snorm_i8(data, w, h)?;
            let mut v = Vec::with_capacity(n * 4);
            for &r in raw.iter().take(n) {
                v.extend_from_slice(&snorm8(r).to_le_bytes());
            }
            v
        }
        S::Bc5Snorm => {
            let raw = crate::bcn::decode_bc5_snorm_i8(data, w, h)?;
            let samples: Vec<f32> = raw.iter().take(n * 2).map(|&v| snorm8(v)).collect();
            widen_channels(&samples, 2, n)
        }
        S::Bc6hUf16 | S::Bc6hSf16 => {
            let mut half = vec![0u8; n * 8];
            crate::bc6h::decode_bc6h(data, w, h, pix == S::Bc6hSf16, &mut half)?;
            let mut v = Vec::with_capacity(n * 16);
            for hw in half.chunks_exact(2) {
                let f = crate::bc6h::half_to_f32(u16::from_le_bytes([hw[0], hw[1]]));
                v.extend_from_slice(&f.to_le_bytes());
            }
            v
        }
        _ => {
            return Err(DdsError::unsupported(format!(
                "{name} surfaces have no contract layout"
            )))
        }
    };

    let img = DdsImage::tight(w, h, target, out)?;
    Ok(img.with_color(file.color_info()))
}

/// Lay `ch`-channel `f32` samples out as the contract float plane:
/// 1 channel → `GrayF32Le`, 2 → `RgbaF32Le` with `[r, g, 0, 1]`,
/// 4 → `RgbaF32Le`.
fn widen_channels(samples: &[f32], ch: usize, n: usize) -> Vec<u8> {
    match ch {
        1 => {
            let mut v = Vec::with_capacity(n * 4);
            push_f32s(&mut v, &samples[..n.min(samples.len())]);
            v.resize(n * 4, 0);
            v
        }
        2 => {
            let mut v = Vec::with_capacity(n * 16);
            for p in samples.chunks_exact(2).take(n) {
                push_f32s(&mut v, &[p[0], p[1], 0.0, 1.0]);
            }
            v.resize(n * 16, 0);
            v
        }
        _ => {
            let mut v = Vec::with_capacity(n * 16);
            push_f32s(&mut v, &samples[..(n * 4).min(samples.len())]);
            v.resize(n * 16, 0);
            v
        }
    }
}

// ---- encode direction ----------------------------------------------------

/// The stored layout [`crate::encode`] picks for a contract layout when
/// [`crate::EncodeOptions::surface_format`] is `None`.
pub(crate) fn natural_surface_format(f: P) -> S {
    match f {
        P::Gray8 => S::L8,
        P::Ya8 => S::A8L8,
        P::Gray16Le => S::L16,
        P::Rgb24 | P::Bgr24 => S::R8G8B8,
        P::Rgba => S::A8B8G8R8,
        P::Bgra => S::A8R8G8B8,
        P::Rgba64Le => S::R16G16B16A16Unorm,
        P::GrayF32Le => S::R32Float,
        P::RgbF32Le | P::RgbaF32Le => S::R32G32B32A32Float,
    }
}

/// Tightly packed 16-bit RGBA of the image (exact for `Gray16Le` /
/// `Rgba64Le`, `v × 257` for the 8-bit layouts, `clamp × 65535` for
/// float).
fn to_rgba16(img: &DdsImage) -> Vec<u16> {
    let n = img.width as usize * img.height as usize;
    let mut out = Vec::with_capacity(n * 4);
    match img.format {
        P::Gray16Le => {
            for y in 0..img.height as usize {
                let Some(row) = img.row(y) else { break };
                for p in row.chunks_exact(2) {
                    let v = u16::from_le_bytes([p[0], p[1]]);
                    out.extend_from_slice(&[v, v, v, 0xFFFF]);
                }
            }
        }
        P::Rgba64Le => {
            for y in 0..img.height as usize {
                let Some(row) = img.row(y) else { break };
                for p in row.chunks_exact(2) {
                    out.push(u16::from_le_bytes([p[0], p[1]]));
                }
            }
        }
        f if f.is_float() => {
            for v in to_rgba_f32(img) {
                let c = if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) };
                out.push((c * 65535.0 + 0.5) as u16);
            }
        }
        _ => {
            for b in img.to_rgba8() {
                out.push(b as u16 * 257);
            }
        }
    }
    out.resize(n * 4, 0);
    out
}

/// Tightly packed `f32` RGBA of the image (exact for the float layouts
/// with missing channels filled `0, 0, 0, 1`; `v / 255` or `v / 65535`
/// for the integer layouts).
fn to_rgba_f32(img: &DdsImage) -> Vec<f32> {
    let n = img.width as usize * img.height as usize;
    let mut out = Vec::with_capacity(n * 4);
    let rd = |p: &[u8]| f32::from_le_bytes([p[0], p[1], p[2], p[3]]);
    match img.format {
        P::GrayF32Le => {
            for y in 0..img.height as usize {
                let Some(row) = img.row(y) else { break };
                for p in row.chunks_exact(4) {
                    let v = rd(p);
                    out.extend_from_slice(&[v, v, v, 1.0]);
                }
            }
        }
        P::RgbF32Le => {
            for y in 0..img.height as usize {
                let Some(row) = img.row(y) else { break };
                for p in row.chunks_exact(12) {
                    out.extend_from_slice(&[rd(&p[0..4]), rd(&p[4..8]), rd(&p[8..12]), 1.0]);
                }
            }
        }
        P::RgbaF32Le => {
            for y in 0..img.height as usize {
                let Some(row) = img.row(y) else { break };
                for p in row.chunks_exact(4) {
                    out.push(rd(p));
                }
            }
        }
        P::Gray16Le | P::Rgba64Le => {
            for v in to_rgba16(img) {
                out.push(v as f32 / 65535.0);
            }
        }
        _ => {
            for b in img.to_rgba8() {
                out.push(b as f32 / 255.0);
            }
        }
    }
    out.resize(n * 4, 0.0);
    out
}

/// `true` when the image is a grey layout (what the luminance / single
/// channel stored layouts accept).
fn is_grey(f: P) -> bool {
    matches!(f, P::Gray8 | P::Ya8 | P::Gray16Le | P::GrayF32Le)
}

fn grey_only(img: &DdsImage, target: S) -> Result<()> {
    if !is_grey(img.format) {
        return Err(DdsError::unsupported(format!(
            "{} needs a grey image (Gray8 / Ya8 / Gray16Le / GrayF32Le), got {}; DDS luminance layouts carry no colour",
            target.name(),
            img.format.name()
        )));
    }
    Ok(())
}

/// Quantise `[-1, 1]` floats to SNORM integers of `bits` width (round
/// to nearest, clamped to `±(2^(bits-1) - 1)`).
fn snorm_q(v: f32, bits: u32) -> i32 {
    let max = ((1i32 << (bits - 1)) - 1) as f32;
    if v.is_nan() {
        return 0;
    }
    (v.clamp(-1.0, 1.0) * max).round() as i32
}

/// Convert a contract image into the tightly packed bytes of `target`
/// (one surface, no header). `Unsupported` when no conversion exists:
/// luminance layouts from colour images, alpha-only `A8` from an
/// alpha-less image, the plain-integer / depth / YUV / typeless
/// layouts, and the two packed-float layouts without a packer.
pub(crate) fn image_to_surface(img: &DdsImage, target: S) -> Result<Vec<u8>> {
    let (w, h) = (img.width, img.height);
    let n = pixel_count(w, h)?;
    Ok(match target {
        // 8-bit colour layouts from RGBA8.
        S::A8R8G8B8 | S::X8R8G8B8 => {
            let rgba = img.to_rgba8();
            let x = target == S::X8R8G8B8;
            let mut v = Vec::with_capacity(n * 4);
            for p in rgba.chunks_exact(4) {
                v.extend_from_slice(&[p[2], p[1], p[0], if x { 0xFF } else { p[3] }]);
            }
            v
        }
        S::A8B8G8R8 => img.to_rgba8(),
        S::X8B8G8R8 => {
            // The unused byte is written as 0xFF, the value it reads back as.
            let mut v = img.to_rgba8();
            for p in v.chunks_exact_mut(4) {
                p[3] = 0xFF;
            }
            v
        }
        S::R8G8B8 => {
            let rgb = img.to_rgb8();
            let mut v = Vec::with_capacity(n * 3);
            for p in rgb.chunks_exact(3) {
                v.extend_from_slice(&[p[2], p[1], p[0]]);
            }
            v
        }
        S::R5G6B5 => pack16(&img.to_rgba8(), |p| {
            ((p[0] as u16 >> 3) << 11) | ((p[1] as u16 >> 2) << 5) | (p[2] as u16 >> 3)
        }),
        S::A1R5G5B5 | S::X1R5G5B5 => pack16(&img.to_rgba8(), |p| {
            let a = if target == S::A1R5G5B5 && p[3] >= 0x80 { 0x8000 } else { 0 };
            a | ((p[0] as u16 >> 3) << 10) | ((p[1] as u16 >> 3) << 5) | (p[2] as u16 >> 3)
        }),
        S::A4R4G4B4 | S::X4R4G4B4 => pack16(&img.to_rgba8(), |p| {
            let a = if target == S::A4R4G4B4 { (p[3] as u16 >> 4) << 12 } else { 0 };
            a | ((p[0] as u16 >> 4) << 8) | ((p[1] as u16 >> 4) << 4) | (p[2] as u16 >> 4)
        }),
        S::A4B4G4R4Unorm => pack16(&img.to_rgba8(), |p| {
            ((p[0] as u16 >> 4) << 12)
                | ((p[1] as u16 >> 4) << 8)
                | ((p[2] as u16 >> 4) << 4)
                | (p[3] as u16 >> 4)
        }),
        S::A8R3G3B2 => pack16(&img.to_rgba8(), |p| {
            ((p[3] as u16) << 8)
                | ((p[0] as u16 >> 5) << 5)
                | ((p[1] as u16 >> 5) << 2)
                | (p[2] as u16 >> 6)
        }),
        S::A8 => {
            if !img.format.has_alpha() {
                return Err(DdsError::unsupported(format!(
                    "A8 stores an alpha channel; {} has none",
                    img.format.name()
                )));
            }
            img.to_rgba8().chunks_exact(4).map(|p| p[3]).collect()
        }
        // Luminance layouts from grey images.
        S::L8 | S::R8Unorm => {
            grey_only(img, target)?;
            img.to_rgba8().chunks_exact(4).map(|p| p[0]).collect()
        }
        S::A8L8 => {
            grey_only(img, target)?;
            let mut v = Vec::with_capacity(n * 2);
            for p in img.to_rgba8().chunks_exact(4) {
                v.extend_from_slice(&[p[0], p[3]]);
            }
            v
        }
        S::A4L4 => {
            grey_only(img, target)?;
            img.to_rgba8()
                .chunks_exact(4)
                .map(|p| (p[0] >> 4) | (p[3] & 0xF0))
                .collect()
        }
        S::L16 | S::R16Unorm => {
            grey_only(img, target)?;
            let mut v = Vec::with_capacity(n * 2);
            for p in to_rgba16(img).chunks_exact(4) {
                v.extend_from_slice(&p[0].to_le_bytes());
            }
            v
        }
        // 16-bit layouts.
        S::R16G16Unorm => {
            let mut v = Vec::with_capacity(n * 4);
            for p in to_rgba16(img).chunks_exact(4) {
                v.extend_from_slice(&p[0].to_le_bytes());
                v.extend_from_slice(&p[1].to_le_bytes());
            }
            v
        }
        S::R16G16B16A16Unorm => {
            let mut v = Vec::with_capacity(n * 8);
            for s in to_rgba16(img) {
                v.extend_from_slice(&s.to_le_bytes());
            }
            v
        }
        S::R10G10B10A2Unorm => {
            let mut v = Vec::with_capacity(n * 4);
            for p in to_rgba16(img).chunks_exact(4) {
                let word = (p[0] as u32 >> 6)
                    | ((p[1] as u32 >> 6) << 10)
                    | ((p[2] as u32 >> 6) << 20)
                    | ((p[3] as u32 >> 14) << 30);
                v.extend_from_slice(&word.to_le_bytes());
            }
            v
        }
        S::A2R10G10B10 => {
            let mut v = Vec::with_capacity(n * 4);
            for p in to_rgba16(img).chunks_exact(4) {
                let word = ((p[0] as u32 >> 6) << 20)
                    | ((p[1] as u32 >> 6) << 10)
                    | (p[2] as u32 >> 6)
                    | ((p[3] as u32 >> 14) << 30);
                v.extend_from_slice(&word.to_le_bytes());
            }
            v
        }
        // Block encoders from RGBA8.
        S::Bc1 | S::Bc2 | S::Bc3 | S::Bc4Unorm | S::Bc5Unorm | S::Bc7Unorm | S::Bc7UnormSrgb => {
            let rgba = img.to_rgba8();
            let bw = w.max(1).div_ceil(4) as usize;
            let bh = h.max(1).div_ceil(4) as usize;
            let mut v = vec![0u8; bw * bh * target.block_bytes().unwrap_or(16) as usize];
            crate::encoder::encode_rgba8_to_bc_level(&rgba, w, h, target, &mut v)?;
            v
        }
        S::Astc { block_w, block_h, .. } => {
            crate::astc::encode_astc_ldr(&img.to_rgba8(), w, h, block_w, block_h)
        }
        // Floating point.
        S::R16Float | S::R16G16Float | S::R16G16B16A16Float => {
            let ch = match target {
                S::R16Float => 1,
                S::R16G16Float => 2,
                _ => 4,
            };
            let mut v = Vec::with_capacity(n * 2 * ch);
            for p in to_rgba_f32(img).chunks_exact(4) {
                for c in &p[..ch] {
                    v.extend_from_slice(&crate::bc6h_enc::f32_to_half(*c).to_le_bytes());
                }
            }
            v
        }
        S::R32Float | S::R32G32Float | S::R32G32B32A32Float => {
            let ch = match target {
                S::R32Float => 1,
                S::R32G32Float => 2,
                _ => 4,
            };
            let mut v = Vec::with_capacity(n * 4 * ch);
            for p in to_rgba_f32(img).chunks_exact(4) {
                push_f32s(&mut v, &p[..ch]);
            }
            v
        }
        S::Bc6hUf16 | S::Bc6hSf16 => {
            let rgb: Vec<f32> = to_rgba_f32(img)
                .chunks_exact(4)
                .flat_map(|p| [p[0], p[1], p[2]])
                .collect();
            let bw = w.max(1).div_ceil(4) as usize;
            let bh = h.max(1).div_ceil(4) as usize;
            let mut v = vec![0u8; bw * bh * 16];
            if target == S::Bc6hUf16 {
                crate::bc6h_enc::encode_bc6h_from_f32(&rgb, w, h, &mut v)?;
            } else {
                crate::bc6h_enc::encode_bc6h_sf16_from_f32(&rgb, w, h, &mut v)?;
            }
            v
        }
        // Signed-normalised.
        S::R8Snorm | S::R8G8Snorm | S::R8G8B8A8Snorm => {
            let ch = match target {
                S::R8Snorm => 1,
                S::R8G8Snorm => 2,
                _ => 4,
            };
            let mut v = Vec::with_capacity(n * ch);
            for p in to_rgba_f32(img).chunks_exact(4) {
                for c in &p[..ch] {
                    v.push(snorm_q(*c, 8) as i8 as u8);
                }
            }
            v
        }
        S::R16Snorm | S::R16G16Snorm | S::R16G16B16A16Snorm => {
            let ch = match target {
                S::R16Snorm => 1,
                S::R16G16Snorm => 2,
                _ => 4,
            };
            let mut v = Vec::with_capacity(n * 2 * ch);
            for p in to_rgba_f32(img).chunks_exact(4) {
                for c in &p[..ch] {
                    v.extend_from_slice(&(snorm_q(*c, 16) as i16).to_le_bytes());
                }
            }
            v
        }
        S::Bc4Snorm | S::Bc5Snorm => {
            let ch = if target == S::Bc4Snorm { 1 } else { 2 };
            let mut q = Vec::with_capacity(n * ch);
            for p in to_rgba_f32(img).chunks_exact(4) {
                for c in &p[..ch] {
                    q.push(snorm_q(*c, 8) as i8 as u8);
                }
            }
            let bw = w.max(1).div_ceil(4) as usize;
            let bh = h.max(1).div_ceil(4) as usize;
            let mut v = vec![0u8; bw * bh * 8 * ch];
            if ch == 1 {
                crate::bcn_enc::encode_bc4_snorm(&q, w, h, &mut v)?;
            } else {
                crate::bcn_enc::encode_bc5_snorm(&q, w, h, &mut v)?;
            }
            v
        }
        S::R11G11B10Float | S::R9G9B9E5SharedExp => {
            return Err(DdsError::unsupported(format!(
                "{} has no packer in this crate (decode only)",
                target.name()
            )))
        }
        other => {
            return Err(DdsError::unsupported(format!(
                "{} cannot be produced from a contract image (plain-integer, depth-stencil, typeless and YUV layouts are depth-API only)",
                other.name()
            )))
        }
    })
}

fn pack16(rgba: &[u8], f: impl Fn(&[u8]) -> u16) -> Vec<u8> {
    let mut v = Vec::with_capacity(rgba.len() / 2);
    for p in rgba.chunks_exact(4) {
        v.extend_from_slice(&f(p).to_le_bytes());
    }
    v
}

// ---- mip generation ------------------------------------------------------

/// Halve a contract image with a 2×2 box filter (edge replication for
/// odd dimensions, round half up), per channel in the sample's own
/// domain: bytes for the 8-bit layouts, `u16` for `Gray16Le` /
/// `Rgba64Le`, `f32` for the float layouts. Colour and metadata are
/// carried over.
pub(crate) fn downsample(img: &DdsImage) -> Result<DdsImage> {
    let sw = img.width as usize;
    let sh = img.height as usize;
    let dw = (sw / 2).max(1);
    let dh = (sh / 2).max(1);
    let bpp = img.format.bytes_per_pixel();
    let (sample, chans) = match img.format {
        P::Gray16Le | P::Rgba64Le => (2usize, bpp / 2),
        f if f.is_float() => (4, bpp / 4),
        _ => (1, bpp),
    };
    let mut dst = vec![0u8; dw * dh * bpp];
    let src = img.as_bytes().unwrap_or(&[]);
    let stride = img.stride();
    const ZERO: [u8; 4] = [0; 4];
    let at = |x: usize, y: usize, c: usize| -> &[u8] {
        let off = y * stride + x * bpp + c * sample;
        src.get(off..off + sample).unwrap_or(&ZERO[..sample])
    };
    for dy in 0..dh {
        for dx in 0..dw {
            let sx0 = (dx * 2).min(sw - 1);
            let sx1 = (dx * 2 + 1).min(sw - 1);
            let sy0 = (dy * 2).min(sh - 1);
            let sy1 = (dy * 2 + 1).min(sh - 1);
            for c in 0..chans {
                let o = dy * dw * bpp + dx * bpp + c * sample;
                let taps = [
                    at(sx0, sy0, c),
                    at(sx1, sy0, c),
                    at(sx0, sy1, c),
                    at(sx1, sy1, c),
                ];
                match sample {
                    1 => {
                        let s: u32 = taps.iter().map(|t| t[0] as u32).sum();
                        dst[o] = ((s + 2) / 4) as u8;
                    }
                    2 => {
                        let s: u32 = taps
                            .iter()
                            .map(|t| u16::from_le_bytes([t[0], t[1]]) as u32)
                            .sum();
                        dst[o..o + 2].copy_from_slice(&(((s + 2) / 4) as u16).to_le_bytes());
                    }
                    _ => {
                        let s: f32 = taps
                            .iter()
                            .map(|t| f32::from_le_bytes([t[0], t[1], t[2], t[3]]))
                            .sum();
                        dst[o..o + 4].copy_from_slice(&(s / 4.0).to_le_bytes());
                    }
                }
            }
        }
    }
    let out = DdsImage::tight(dw as u32, dh as u32, img.format, dst)?;
    Ok(out
        .with_color(img.color)
        .with_metadata(img.metadata.clone()))
}

/// Number of levels in the full chain down to 1×1.
pub(crate) fn full_mip_chain(width: u32, height: u32) -> u32 {
    (32 - width.max(height).leading_zeros()).max(1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surface::Plane;

    fn file(pix: S, w: u32, h: u32, data: Vec<u8>) -> DdsFile {
        let stride = data.len() / h as usize;
        DdsFile::single(w, h, pix, Plane::new(stride, data)).unwrap()
    }

    #[test]
    fn packed_16bit_expand_and_requantise_round_trip() {
        for pix in [
            S::R5G6B5,
            S::A1R5G5B5,
            S::X1R5G5B5,
            S::A4R4G4B4,
            S::X4R4G4B4,
            S::A8R3G3B2,
            S::A4B4G4R4Unorm,
        ] {
            let words: Vec<u8> = (0u16..256).flat_map(|i| (i * 257).to_le_bytes()).collect();
            let f = file(pix, 16, 16, words.clone());
            let img = f.primary_image().unwrap();
            assert_eq!(img.format, P::Rgba);
            let back = image_to_surface(&img, pix).unwrap();
            // X layouts drop the unused bit(s); compare through a second decode.
            let f2 = file(pix, 16, 16, back);
            assert_eq!(f2.primary_image().unwrap(), img, "{}", pix.name());
        }
    }

    #[test]
    fn x8_layouts_force_opaque_alpha() {
        let f = file(S::X8R8G8B8, 1, 1, vec![1, 2, 3, 0]);
        let img = f.primary_image().unwrap();
        assert_eq!(img.format, P::Bgra);
        assert_eq!(img.as_bytes().unwrap(), &[1, 2, 3, 0xFF]);
        assert_eq!(img.to_rgba8(), vec![3, 2, 1, 0xFF]);
    }

    #[test]
    fn a8_is_colourless_alpha_and_round_trips() {
        let f = file(S::A8, 2, 1, vec![9, 200]);
        let img = f.primary_image().unwrap();
        assert_eq!(img.as_bytes().unwrap(), &[0, 0, 0, 9, 0, 0, 0, 200]);
        assert_eq!(image_to_surface(&img, S::A8).unwrap(), vec![9, 200]);
        let grey = DdsImage::tight(1, 1, P::Gray8, vec![5]).unwrap();
        assert!(matches!(
            image_to_surface(&grey, S::A8),
            Err(DdsError::Unsupported(_))
        ));
    }

    #[test]
    fn a4l4_nibbles() {
        let f = file(S::A4L4, 1, 1, vec![0x3A]);
        let img = f.primary_image().unwrap();
        assert_eq!(img.format, P::Ya8);
        assert_eq!(img.as_bytes().unwrap(), &[0xAA, 0x33]);
        assert_eq!(image_to_surface(&img, S::A4L4).unwrap(), vec![0x3A]);
    }

    #[test]
    fn ten_bit_widening_round_trips() {
        let word: u32 = 0x3FF | (0x200 << 10) | (0x001 << 20) | (0x2 << 30);
        let f = file(S::R10G10B10A2Unorm, 1, 1, word.to_le_bytes().to_vec());
        let img = f.primary_image().unwrap();
        assert_eq!(img.format, P::Rgba64Le);
        let v = le_u16s(img.as_bytes().unwrap(), 4);
        assert_eq!(v, vec![0xFFFF, 0x8020, 0x0040, 0xAAAA]);
        assert_eq!(
            image_to_surface(&img, S::R10G10B10A2Unorm).unwrap(),
            word.to_le_bytes()
        );
        let f2 = file(S::A2R10G10B10, 1, 1, word.to_le_bytes().to_vec());
        let img2 = f2.primary_image().unwrap();
        assert_eq!(
            image_to_surface(&img2, S::A2R10G10B10).unwrap(),
            word.to_le_bytes()
        );
    }

    #[test]
    fn luminance_targets_reject_colour() {
        let rgb = DdsImage::from_rgb8(1, 1, vec![1, 2, 3]).unwrap();
        for t in [S::L8, S::A8L8, S::L16, S::A4L4, S::R16Unorm] {
            assert!(matches!(
                image_to_surface(&rgb, t),
                Err(DdsError::Unsupported(_))
            ));
        }
        let g = DdsImage::tight(1, 1, P::Gray8, vec![0x80]).unwrap();
        assert_eq!(
            image_to_surface(&g, S::L16).unwrap(),
            0x8080u16.to_le_bytes()
        );
    }

    #[test]
    fn float_surfaces_widen_missing_channels() {
        let mut d = Vec::new();
        push_f32s(&mut d, &[0.25, 0.5]);
        let f = file(S::R32G32Float, 1, 1, d);
        let img = f.primary_image().unwrap();
        assert_eq!(img.format, P::RgbaF32Le);
        let vals: Vec<f32> = img
            .as_bytes()
            .unwrap()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(vals, vec![0.25, 0.5, 0.0, 1.0]);
        let mut back = Vec::new();
        push_f32s(&mut back, &[0.25, 0.5]);
        assert_eq!(image_to_surface(&img, S::R32G32Float).unwrap(), back);
    }

    #[test]
    fn snorm_rule_clamps_minimum() {
        let f = file(S::R8Snorm, 3, 1, vec![0x80, 0x81, 0x7F]);
        let img = f.primary_image().unwrap();
        assert_eq!(img.format, P::GrayF32Le);
        let vals: Vec<f32> = img
            .as_bytes()
            .unwrap()
            .chunks_exact(4)
            .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
            .collect();
        assert_eq!(vals, vec![-1.0, -1.0, 1.0]);
        assert_eq!(
            image_to_surface(&img, S::R8Snorm).unwrap(),
            vec![0x81, 0x81, 0x7F]
        );
    }

    #[test]
    fn depth_only_layouts_are_unsupported() {
        let f = file(S::R32Uint, 1, 1, vec![0; 4]);
        assert!(matches!(f.primary_image(), Err(DdsError::Unsupported(_))));
        let rgba = DdsImage::from_rgba8(1, 1, vec![0; 4]).unwrap();
        assert!(matches!(
            image_to_surface(&rgba, S::R32Uint),
            Err(DdsError::Unsupported(_))
        ));
    }

    #[test]
    fn downsample_each_domain() {
        let g = DdsImage::tight(2, 2, P::Gray8, vec![0, 10, 20, 30]).unwrap();
        assert_eq!(downsample(&g).unwrap().as_bytes().unwrap(), &[15]);
        let odd = DdsImage::tight(3, 1, P::Gray8, vec![0, 100, 200]).unwrap();
        let d = downsample(&odd).unwrap();
        assert_eq!(d.width, 1);
        assert_eq!(d.as_bytes().unwrap(), &[50]);
        let g16 = DdsImage::tight(2, 1, P::Gray16Le, vec![0, 0, 0xFF, 0xFF]).unwrap();
        assert_eq!(downsample(&g16).unwrap().as_bytes().unwrap(), &[0x00, 0x80]);
        let mut fd = Vec::new();
        push_f32s(&mut fd, &[1.0, 3.0]);
        let f = DdsImage::tight(2, 1, P::GrayF32Le, fd).unwrap();
        assert_eq!(
            downsample(&f).unwrap().as_bytes().unwrap(),
            &2.0f32.to_le_bytes()
        );
        assert_eq!(full_mip_chain(1, 1), 1);
        assert_eq!(full_mip_chain(256, 16), 9);
        assert_eq!(full_mip_chain(5, 3), 3);
    }
}
