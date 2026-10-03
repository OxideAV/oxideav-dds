//! The image-crate contract root vocabulary (`IMAGE_CRATE_API.md`):
//! `probe` / `info` / `decode*` / `encode*`, framework-free.
//!
//! Every function here is a thin layer over the depth API: the parser
//! ([`crate::parse_dds_with`] → [`DdsFile`]), the stored ↔ contract
//! conversion (the crate-private `convert` module) and the general writer
//! ([`crate::write_dds_file`]).

use std::io::{Read, Write};

use crate::convert;
use crate::decoder;
use crate::encoder::write_dds_file;
use crate::error::{DdsError, Result};
use crate::image::{DdsImage, Frame, ImageInfo, PixelFormat, RgbImage, RgbaImage};
use crate::options::{DecodeOptions, EncodeOptions};
use crate::surface::{CubemapFace, DdsFile, DdsSurface, Plane, SurfaceFormat};
use crate::types::{DDS_HEADER_SIZE, DDS_MAGIC};

/// `true` when `bytes` starts with the `DDS ` magic followed by a
/// `DDS_HEADER` whose `dwSize` is 124. Allocation-free, never panics,
/// `false` on short input.
pub fn probe(bytes: &[u8]) -> bool {
    bytes.len() >= 8
        && u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) == DDS_MAGIC
        && u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) == DDS_HEADER_SIZE as u32
}

/// Header-only description: top-level dimensions, the layout
/// [`decode`] would return, the surface count [`decode_all`] would
/// return, colour signalling and the texture shape. No surface byte is
/// read; the surface geometry is walked to validate the file length.
///
/// Like [`decode`], returns [`DdsError::Unsupported`] for a stored
/// layout that has no contract layout (see
/// [`SurfaceFormat::contract_format`]) — [`crate::parse_dds`] still
/// describes such files.
pub fn info(bytes: &[u8]) -> Result<ImageInfo> {
    info_with(bytes, &DecodeOptions::default().unlimited())
}

/// [`info`] with explicit limits / strictness.
pub fn info_with(bytes: &[u8], opts: &DecodeOptions) -> Result<ImageInfo> {
    let layout = decoder::parse_layout(bytes, opts)?;
    let format = contract_format_or_err(layout.pix)?;
    let mut info = ImageInfo::new(layout.width, layout.height, format, layout.pix);
    info.frames = u32::try_from(layout.surface_count).unwrap_or(u32::MAX);
    info.has_alpha = layout.pix.has_alpha();
    info.dxgi_format = layout.dxgi;
    info.dx10_header = layout.has_dxt10;
    info.mip_levels = layout.mip_count;
    info.cubemap = layout.is_cubemap;
    info.array_size = layout.array_size;
    info.depth = layout.base_depth;
    let srgb = match layout.pix {
        SurfaceFormat::Astc { srgb, .. } => srgb,
        SurfaceFormat::Bc7UnormSrgb => true,
        _ => layout.dxgi.is_some_and(|d| d.is_srgb()),
    };
    info.color = if srgb {
        crate::image::ColorInfo::srgb()
    } else {
        crate::image::ColorInfo::dds_default()
    };
    Ok(info)
}

fn contract_format_or_err(pix: SurfaceFormat) -> Result<PixelFormat> {
    pix.contract_format().ok_or_else(|| {
        DdsError::unsupported(format!(
            "{} surfaces have no contract layout; use parse_dds and the matching decode_*_surface helper",
            pix.name()
        ))
    })
}

/// Decode the top-level surface (mip 0, first cubemap face, first array
/// slice, first depth slice) into its contract layout with
/// [`DecodeOptions::default`].
pub fn decode(bytes: &[u8]) -> Result<DdsImage> {
    decode_with(bytes, &DecodeOptions::default())
}

/// Decode with explicit limits / strictness.
pub fn decode_with(bytes: &[u8], opts: &DecodeOptions) -> Result<DdsImage> {
    let file = decoder::parse_dds_with(bytes, opts)?;
    file.primary_image()
}

/// Decode straight to tightly packed 8-bit RGB (alpha dropped).
pub fn decode_rgb8(bytes: &[u8]) -> Result<RgbImage> {
    let img = decode(bytes)?;
    Ok(RgbImage::new(img.width, img.height, img.to_rgb8()))
}

/// Decode straight to tightly packed 8-bit RGBA (opaque where the
/// file has no alpha).
pub fn decode_rgba8(bytes: &[u8]) -> Result<RgbaImage> {
    let img = decode(bytes)?;
    Ok(RgbaImage::new(img.width, img.height, img.to_rgba8()))
}

/// Every surface of the file as a [`Frame`] — mip levels, cubemap faces,
/// array slices and volume depth slices in Microsoft's on-disk order —
/// each expanded to the contract layout; `delay` is always `None`. A
/// plain 2D texture yields one frame.
pub fn decode_all(bytes: &[u8]) -> Result<Vec<Frame>> {
    decode_all_with(bytes, &DecodeOptions::default())
}

/// [`decode_all`] with explicit limits / strictness. `max_bytes` is
/// checked against the stored bytes plus the expansion of the top-level
/// surface, as for [`decode_with`]; the expansion of every surface is
/// at most twice that for a full mip chain.
pub fn decode_all_with(bytes: &[u8], opts: &DecodeOptions) -> Result<Vec<Frame>> {
    let file = decoder::parse_dds_with(bytes, opts)?;
    file.surfaces
        .iter()
        .map(|s| {
            Ok(Frame::new(file.to_image(s)?)
                .with_mip_level(s.mip_level)
                .with_array_slice(s.array_slice)
                .with_face(s.face)
                .with_depth_slice(s.depth_slice))
        })
        .collect()
}

/// Read `r` to its end and [`decode`] the bytes.
pub fn decode_from<R: Read>(mut r: R) -> Result<DdsImage> {
    let mut bytes = Vec::new();
    r.read_to_end(&mut bytes)?;
    decode(&bytes)
}

/// Encode one image as a complete DDS file.
///
/// The stored layout is [`EncodeOptions::surface_format`] when set, else
/// the natural layout of the image's [`PixelFormat`]:
///
/// | `PixelFormat` | Stored layout | Header |
/// |---|---|---|
/// | `Gray8` | `L8` | legacy |
/// | `Ya8` | `A8L8` | legacy |
/// | `Gray16Le` | `L16` | legacy |
/// | `Rgb24` (swizzled), `Bgr24` | `R8G8B8` | legacy |
/// | `Rgba` | `A8B8G8R8` (`R8G8B8A8_UNORM`) | legacy |
/// | `Bgra` | `A8R8G8B8` (`B8G8R8A8_UNORM`) | legacy |
/// | `Rgba64Le` | `R16G16B16A16_UNORM` | DX10 |
/// | `GrayF32Le` | `R32_FLOAT` | DX10 |
/// | `RgbF32Le` (alpha 1.0 added), `RgbaF32Le` | `R32G32B32A32_FLOAT` | DX10 |
///
/// `Rgb24` → `R8G8B8` is the only natural-layout conversion (a byte
/// swizzle, exact); every other natural layout is a byte copy and
/// round-trips through [`decode`] exactly. With an explicit
/// `surface_format` the image is converted as documented on
/// [`EncodeOptions`] and [`SurfaceFormat::contract_format`], or the call
/// fails with [`DdsError::Unsupported`]. `mip_levels` generates the
/// chain in the contract layout before conversion. The image's `color`
/// has no DDS encoding except through an `_SRGB` `surface_format`, and
/// its `metadata` is not stored.
pub fn encode(image: &DdsImage, opts: &EncodeOptions) -> Result<Vec<u8>> {
    let file = build_file(std::slice::from_ref(&Frame::new(image.clone())), opts)?;
    write_dds_file(&file, opts.dx10_header)
}

/// Encode tightly packed 8-bit RGB as `R8G8B8` (legacy header).
pub fn encode_rgb8(width: u32, height: u32, rgb: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    encode(&DdsImage::from_rgb8(width, height, rgb.to_vec())?, opts)
}

/// Encode tightly packed 8-bit RGBA as `R8G8B8A8_UNORM` (`A8B8G8R8`,
/// legacy header).
pub fn encode_rgba8(width: u32, height: u32, rgba: &[u8], opts: &EncodeOptions) -> Result<Vec<u8>> {
    encode(&DdsImage::from_rgba8(width, height, rgba.to_vec())?, opts)
}

/// [`encode`] into a writer.
pub fn encode_to<W: Write>(image: &DdsImage, opts: &EncodeOptions, mut w: W) -> Result<()> {
    let bytes = encode(image, opts)?;
    w.write_all(&bytes)?;
    Ok(())
}

/// Encode several surfaces as one DDS file — the mirror of
/// [`decode_all`]. The texture shape is read off the frames' extras:
///
/// * `face: Some(..)` on any frame makes the file a cubemap; all six
///   faces must then be present for every (slice, mip);
/// * `array_slice` values `0..n` make a DX10 texture array of `n`;
/// * `depth_slice` values `0..d` at mip 0 make a volume texture of
///   depth `d` (never combined with faces or array slices);
/// * `mip_level` values `0..m` supply an explicit chain; when every
///   frame is mip 0 and [`EncodeOptions::mip_levels`] is not `1`, the
///   chain is generated per face / slice as [`encode`] does.
///
/// Frames may come in any order; they are sorted into Microsoft's
/// on-disk order. Every frame must share the top-level `format`, and a
/// frame's dimensions must match its position in the chain
/// (`width >> mip`, `height >> mip`, floored to 1). Missing or
/// duplicate positions are [`DdsError::InvalidData`].
pub fn encode_all(frames: &[Frame], opts: &EncodeOptions) -> Result<Vec<u8>> {
    let file = build_file(frames, opts)?;
    write_dds_file(&file, opts.dx10_header)
}

/// Assemble the [`DdsFile`] for `frames` under `opts`.
fn build_file(frames: &[Frame], opts: &EncodeOptions) -> Result<DdsFile> {
    let Some(first) = frames.iter().find(|f| {
        f.mip_level == 0 && f.array_slice == 0 && f.depth_slice == 0 && f.face.is_none_or_px()
    }) else {
        return Err(DdsError::invalid(
            "encode_all: no top-level frame (mip 0, slice 0, first face, depth slice 0)",
        ));
    };
    let top = &first.image;
    let width = top.width;
    let height = top.height;
    if width == 0 || height == 0 {
        return Err(DdsError::invalid("DDS: zero-sized image"));
    }
    let format = top.format;
    let surface_format = opts
        .surface_format
        .unwrap_or_else(|| convert::natural_surface_format(format));

    // Shape.
    let is_cubemap = frames.iter().any(|f| f.face.is_some());
    let array_size = frames.iter().map(|f| f.array_slice).max().unwrap_or(0) + 1;
    let depth = frames
        .iter()
        .filter(|f| f.mip_level == 0)
        .map(|f| f.depth_slice)
        .max()
        .unwrap_or(0)
        + 1;
    let explicit_mips = frames.iter().map(|f| f.mip_level).max().unwrap_or(0) + 1;
    if depth > 1 && (is_cubemap || array_size > 1) {
        return Err(DdsError::invalid(
            "encode_all: a volume texture (depth_slice > 0) cannot carry faces or array slices",
        ));
    }
    let full = convert::full_mip_chain(width.max(depth), height);
    let mip_count = if explicit_mips > 1 {
        if opts.mip_levels != 1 && opts.mip_levels != explicit_mips {
            return Err(DdsError::invalid(format!(
                "encode_all: frames supply {explicit_mips} mip levels but EncodeOptions::mip_levels asks for {}",
                opts.mip_levels
            )));
        }
        explicit_mips
    } else if opts.mip_levels == 0 {
        full
    } else {
        opts.mip_levels
    };
    if mip_count > full {
        return Err(DdsError::invalid(format!(
            "DDS: {mip_count} mip levels exceed the {full} a {width}x{height} surface can hold"
        )));
    }

    // Index the frames by position and check them.
    for f in frames {
        if f.image.format != format {
            return Err(DdsError::invalid(format!(
                "encode_all: every frame must share the top-level layout {}, found {}",
                format.name(),
                f.image.format.name()
            )));
        }
        if is_cubemap && f.face.is_none() {
            return Err(DdsError::invalid(
                "encode_all: cubemap frames must all name a face",
            ));
        }
        let (mw, mh) = (
            (width >> f.mip_level).max(1),
            (height >> f.mip_level).max(1),
        );
        if f.image.width != mw || f.image.height != mh {
            return Err(DdsError::invalid(format!(
                "encode_all: frame at mip {} is {}x{}, expected {mw}x{mh}",
                f.mip_level, f.image.width, f.image.height
            )));
        }
        if f.mip_level >= mip_count {
            return Err(DdsError::invalid(format!(
                "encode_all: frame at mip {} is beyond the {mip_count}-level chain",
                f.mip_level
            )));
        }
        let mip_depth = (depth >> f.mip_level).max(1);
        if f.depth_slice >= mip_depth {
            return Err(DdsError::invalid(format!(
                "encode_all: depth slice {} at mip {} is beyond that level's depth {mip_depth}",
                f.depth_slice, f.mip_level
            )));
        }
    }
    let find = |slice: u32, face: Option<CubemapFace>, mip: u32, z: u32| -> Option<&Frame> {
        frames.iter().find(|f| {
            f.array_slice == slice && f.face == face && f.mip_level == mip && f.depth_slice == z
        })
    };
    let count = |slice: u32, face: Option<CubemapFace>, mip: u32, z: u32| -> usize {
        frames
            .iter()
            .filter(|f| {
                f.array_slice == slice && f.face == face && f.mip_level == mip && f.depth_slice == z
            })
            .count()
    };

    // Walk the on-disk order, generating missing mips when the frames
    // supply only the top level.
    let faces: Vec<Option<CubemapFace>> = if is_cubemap {
        CubemapFace::ALL.iter().map(|f| Some(*f)).collect()
    } else {
        vec![None]
    };
    let generate = explicit_mips == 1 && mip_count > 1;
    let mut surfaces: Vec<DdsSurface> = Vec::new();
    let mut push =
        |img: &DdsImage, slice: u32, face: Option<CubemapFace>, mip: u32, z: u32| -> Result<()> {
            let data = convert::image_to_surface(img, surface_format)?;
            let stride = if img.width == 0 {
                0
            } else {
                surface_stride(surface_format, img.width, data.len(), img.height)
            };
            surfaces.push(
                DdsSurface::new(img.width, img.height, Plane::new(stride, data))
                    .with_mip_level(mip)
                    .with_array_slice(slice)
                    .with_face(face)
                    .with_depth_slice(z),
            );
            Ok(())
        };

    if depth > 1 {
        // Volume: mip-major, then depth slice. Generated mips of a volume
        // halve the depth too: slice z of level m is the average of
        // slices 2z and 2z+1 of level m-1, each already box-filtered.
        let mut level: Vec<DdsImage> = (0..depth)
            .map(|z| {
                find(0, None, 0, z).map(|f| f.image.clone()).ok_or_else(|| {
                    DdsError::invalid(format!("encode_all: missing depth slice {z}"))
                })
            })
            .collect::<Result<_>>()?;
        for z in 0..depth {
            if count(0, None, 0, z) > 1 {
                return Err(DdsError::invalid(format!(
                    "encode_all: duplicate depth slice {z}"
                )));
            }
        }
        for mip in 0..mip_count {
            let mip_depth = (depth >> mip).max(1);
            if mip > 0 {
                if generate {
                    let mut next = Vec::with_capacity(mip_depth as usize);
                    for z in 0..mip_depth as usize {
                        let a = convert::downsample(&level[(2 * z).min(level.len() - 1)])?;
                        let b = convert::downsample(&level[(2 * z + 1).min(level.len() - 1)])?;
                        next.push(average_images(&a, &b)?);
                    }
                    level = next;
                } else {
                    level = (0..mip_depth)
                        .map(|z| {
                            if count(0, None, mip, z) > 1 {
                                return Err(DdsError::invalid(format!(
                                    "encode_all: duplicate surface at mip {mip} depth slice {z}"
                                )));
                            }
                            find(0, None, mip, z)
                                .map(|f| f.image.clone())
                                .ok_or_else(|| {
                                    DdsError::invalid(format!(
                                        "encode_all: missing surface at mip {mip} depth slice {z}"
                                    ))
                                })
                        })
                        .collect::<Result<_>>()?;
                }
            }
            for (z, img) in level.iter().enumerate() {
                push(img, 0, None, mip, z as u32)?;
            }
        }
    } else {
        for slice in 0..array_size {
            for face in &faces {
                let mut current: Option<DdsImage> = None;
                for mip in 0..mip_count {
                    let img = if generate && mip > 0 {
                        convert::downsample(current.as_ref().expect("mip 0 set"))?
                    } else {
                        if count(slice, *face, mip, 0) > 1 {
                            return Err(DdsError::invalid(format!(
                                "encode_all: duplicate surface at slice {slice} face {} mip {mip}",
                                face.map(|f| f.short_name()).unwrap_or("-")
                            )));
                        }
                        find(slice, *face, mip, 0)
                            .map(|f| f.image.clone())
                            .ok_or_else(|| {
                                DdsError::invalid(format!(
                                    "encode_all: missing surface at slice {slice} face {} mip {mip}",
                                    face.map(|f| f.short_name()).unwrap_or("-")
                                ))
                            })?
                    };
                    push(&img, slice, *face, mip, 0)?;
                    current = Some(img);
                }
            }
        }
    }

    let mut file = DdsFile::new(width, height, surface_format, surfaces)?
        .with_mip_map_count(mip_count)
        .with_cubemap(is_cubemap)
        .with_array_size(array_size)
        .with_depth(depth)
        .with_dxt10_header(opts.dx10_header);
    // An explicit sRGB request through the surface format is the only
    // colour DDS can carry; the DXGI code follows from the layout.
    if let SurfaceFormat::Astc { .. } | SurfaceFormat::Bc7UnormSrgb = surface_format {
        file.has_dxt10_header = true;
    }
    Ok(file)
}

/// Row stride of a freshly converted surface: `bytes / rows` for the
/// uncompressed layouts, one row of blocks for the block layouts.
fn surface_stride(pix: SurfaceFormat, width: u32, bytes: usize, height: u32) -> usize {
    if let Some((bw, _)) = pix.astc_footprint() {
        return (width.max(1).div_ceil(bw) * 16) as usize;
    }
    if let Some(bb) = pix.block_bytes() {
        return (width.max(1).div_ceil(4) * bb) as usize;
    }
    bytes / height.max(1) as usize
}

/// Average two same-shape contract images sample by sample (the depth
/// step of a volume mip).
fn average_images(a: &DdsImage, b: &DdsImage) -> Result<DdsImage> {
    if a.width != b.width || a.height != b.height || a.format != b.format {
        return Err(DdsError::invalid("volume mip: slice shape mismatch"));
    }
    let bpp = a.format.bytes_per_pixel();
    let (sample, chans) = match a.format {
        PixelFormat::Gray16Le | PixelFormat::Rgba64Le => (2usize, bpp / 2),
        f if f.is_float() => (4, bpp / 4),
        _ => (1, bpp),
    };
    const ZERO: [u8; 4] = [0; 4];
    let n = a.width as usize * a.height as usize * chans;
    let pa = a.as_bytes().unwrap_or(&[]);
    let pb = b.as_bytes().unwrap_or(&[]);
    let mut out = Vec::with_capacity(n * sample);
    for i in 0..n {
        let o = i * sample;
        let sa = pa.get(o..o + sample).unwrap_or(&ZERO[..sample]);
        let sb = pb.get(o..o + sample).unwrap_or(&ZERO[..sample]);
        match sample {
            1 => out.push((sa[0] as u32 + sb[0] as u32).div_ceil(2) as u8),
            2 => {
                let va = u16::from_le_bytes([sa[0], sa[1]]) as u32;
                let vb = u16::from_le_bytes([sb[0], sb[1]]) as u32;
                out.extend_from_slice(&((va + vb).div_ceil(2) as u16).to_le_bytes());
            }
            _ => {
                let va = f32::from_le_bytes([sa[0], sa[1], sa[2], sa[3]]);
                let vb = f32::from_le_bytes([sb[0], sb[1], sb[2], sb[3]]);
                out.extend_from_slice(&((va + vb) / 2.0).to_le_bytes());
            }
        }
    }
    Ok(DdsImage::tight(a.width, a.height, a.format, out)?.with_color(a.color))
}

/// `None` or the first cubemap face: what the top-level frame may carry.
trait FaceIsTop {
    fn is_none_or_px(&self) -> bool;
}
impl FaceIsTop for Option<CubemapFace> {
    fn is_none_or_px(&self) -> bool {
        matches!(self, None | Some(CubemapFace::PositiveX))
    }
}
