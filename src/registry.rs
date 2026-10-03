//! `oxideav-core` integration layer for `oxideav-dds`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-dds` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] / [`register_codecs`] / [`register_containers`] — the
//!   `RuntimeContext` / `CodecRegistry` / `ContainerRegistry` entry
//!   points `oxideav_meta::register_all` and the umbrella call.
//! * [`make_decoder`] / [`make_encoder`] — the trait-side factories; the
//!   `Decoder` / `Encoder` impls are thin adapters over
//!   [`crate::decode_with`] / [`crate::encode`] (one implementation).
//! * `From<DdsImage> for VideoFrame` and [`DdsImage::from_video_frame`]
//!   — the frame bridge: one packed plane in the native layout plus the
//!   colour-signal side-channel when the file declares sRGB.
//! * `From<DdsError> for oxideav_core::Error` and the
//!   `CodecOptionsStruct` schema for [`EncodeOptions`].
//!
//! The framework boundary is the **native contract layout**: the
//! registry `Decoder` emits the top-level surface in the layout
//! [`crate::decode`] returns (`Bgra` for a `B8G8R8A8` file, `Rgba` for a
//! decoded BC7, `RgbaF32Le` for a float surface, …), never a
//! pre-converted `Rgba`; `oxideav-pixfmt` / `oxideav-image` convert
//! further. The container demuxer declares that layout on the stream
//! from the header alone.

use oxideav_core::{
    parse_options, CodecCapabilities, CodecId, CodecInfo, CodecOptionsStruct, CodecParameters,
    CodecRegistry, ColorPrimaries, ColorSignal, ContainerRegistry, Decoder, Encoder, Frame,
    MatrixCoefficients, OptionField, OptionKind, OptionValue, Packet, PixelFormat, RuntimeContext,
    TimeBase, TransferCharacteristics, VideoFrame, VideoPlane,
};

use crate::container;
use crate::error::DdsError;
use crate::image::{ColorInfo, ColorRange, DdsImage, DdsPixelFormat, Plane};
use crate::options::{DecodeOptions, EncodeOptions};
use crate::surface::SurfaceFormat;
use crate::CODEC_ID_STR;

/// Convert a [`DdsError`] into the framework-shared `oxideav_core::Error`
/// so trait impls in this crate can use `?` on errors returned by the
/// framework-free decode/encode functions.
impl From<DdsError> for oxideav_core::Error {
    fn from(e: DdsError) -> Self {
        match e {
            DdsError::InvalidData(s) => oxideav_core::Error::InvalidData(s),
            DdsError::Unsupported(s) => oxideav_core::Error::Unsupported(s),
            DdsError::LimitExceeded(s) => oxideav_core::Error::ResourceExhausted(s),
            DdsError::Io(e) => oxideav_core::Error::Io(e),
        }
    }
}

// ---- Pixel-format + colour-signal mapping --------------------------------

/// The framework pixel format a [`DdsPixelFormat`] maps to by name
/// (every contract layout has one).
pub fn to_core_pixel_format(pf: DdsPixelFormat) -> PixelFormat {
    match pf {
        DdsPixelFormat::Gray8 => PixelFormat::Gray8,
        DdsPixelFormat::Ya8 => PixelFormat::Ya8,
        DdsPixelFormat::Gray16Le => PixelFormat::Gray16Le,
        DdsPixelFormat::Rgb24 => PixelFormat::Rgb24,
        DdsPixelFormat::Bgr24 => PixelFormat::Bgr24,
        DdsPixelFormat::Rgba => PixelFormat::Rgba,
        DdsPixelFormat::Bgra => PixelFormat::Bgra,
        DdsPixelFormat::Rgba64Le => PixelFormat::Rgba64Le,
        DdsPixelFormat::GrayF32Le => PixelFormat::GrayF32Le,
        DdsPixelFormat::RgbF32Le => PixelFormat::RgbF32Le,
        DdsPixelFormat::RgbaF32Le => PixelFormat::RgbaF32Le,
    }
}

/// Map a framework pixel format to [`DdsPixelFormat`]; `Err` for every
/// layout the contract image cannot hold.
pub fn from_core_pixel_format(pf: PixelFormat) -> Result<DdsPixelFormat, DdsError> {
    Ok(match pf {
        PixelFormat::Gray8 => DdsPixelFormat::Gray8,
        PixelFormat::Ya8 => DdsPixelFormat::Ya8,
        PixelFormat::Gray16Le => DdsPixelFormat::Gray16Le,
        PixelFormat::Rgb24 => DdsPixelFormat::Rgb24,
        PixelFormat::Bgr24 => DdsPixelFormat::Bgr24,
        PixelFormat::Rgba => DdsPixelFormat::Rgba,
        PixelFormat::Bgra => DdsPixelFormat::Bgra,
        PixelFormat::Rgba64Le => DdsPixelFormat::Rgba64Le,
        PixelFormat::GrayF32Le => DdsPixelFormat::GrayF32Le,
        PixelFormat::RgbF32Le => DdsPixelFormat::RgbF32Le,
        PixelFormat::RgbaF32Le => DdsPixelFormat::RgbaF32Le,
        other => {
            return Err(DdsError::unsupported(format!(
                "DDS: pixel format {other:?} not supported"
            )))
        }
    })
}

impl From<DdsPixelFormat> for PixelFormat {
    fn from(pf: DdsPixelFormat) -> Self {
        to_core_pixel_format(pf)
    }
}

impl TryFrom<PixelFormat> for DdsPixelFormat {
    type Error = DdsError;
    fn try_from(pf: PixelFormat) -> Result<Self, DdsError> {
        from_core_pixel_format(pf)
    }
}

/// [`ColorInfo`] as the framework's [`ColorSignal`] (code points map
/// 1:1; `Unspecified` range stays unspecified).
pub fn to_color_signal(c: &ColorInfo) -> ColorSignal {
    let range = match c.range {
        ColorRange::Unspecified => oxideav_core::ColorRange::Unspecified,
        ColorRange::Limited => oxideav_core::ColorRange::Limited,
        ColorRange::Full => oxideav_core::ColorRange::Full,
    };
    ColorSignal::new(
        range,
        ColorPrimaries(c.primaries),
        TransferCharacteristics(c.transfer),
        MatrixCoefficients(c.matrix),
    )
}

/// The inverse of [`to_color_signal`].
pub fn from_color_signal(s: &ColorSignal) -> ColorInfo {
    let range = match s.range {
        oxideav_core::ColorRange::Limited => ColorRange::Limited,
        oxideav_core::ColorRange::Full => ColorRange::Full,
        _ => ColorRange::Unspecified,
    };
    ColorInfo::new(range, s.primaries.0, s.transfer.0, s.matrix.0)
}

// ---- Frame bridge ----------------------------------------------------------

fn image_into_video_frame(mut image: DdsImage, pts: Option<i64>) -> VideoFrame {
    let stride = image.stride();
    let data = if image.planes.is_empty() {
        Vec::new()
    } else {
        std::mem::take(&mut image.planes[0].data)
    };
    let mut frame = VideoFrame {
        pts,
        planes: vec![VideoPlane { stride, data }],
    };
    // DDS defines colour only through the `_SRGB` DXGI codes; the
    // full-range default is this crate's convention and is not stamped.
    if image.color.is_srgb() {
        frame.set_color_signal(to_color_signal(&image.color));
    }
    frame
}

impl From<DdsImage> for VideoFrame {
    /// The pixel plane (`pts` `None`) plus the colour-signal
    /// side-channel when the image is sRGB (an `_SRGB` DXGI code).
    fn from(image: DdsImage) -> Self {
        image_into_video_frame(image, None)
    }
}

impl From<&DdsImage> for VideoFrame {
    fn from(image: &DdsImage) -> Self {
        image_into_video_frame(image.clone(), None)
    }
}

impl DdsImage {
    /// Rebuild an image from a framework frame and the stream
    /// parameters that describe it (`width`, `height` and
    /// `pixel_format` are required; the layout must be one the contract
    /// image can hold). The frame's colour-signal side-channel, when
    /// attached, becomes `color`.
    pub fn from_video_frame(
        frame: &VideoFrame,
        params: &CodecParameters,
    ) -> Result<Self, DdsError> {
        let width = params
            .width
            .ok_or_else(|| DdsError::invalid("DDS: width missing in CodecParameters"))?;
        let height = params
            .height
            .ok_or_else(|| DdsError::invalid("DDS: height missing in CodecParameters"))?;
        let pix =
            from_core_pixel_format(params.pixel_format.ok_or_else(|| {
                DdsError::invalid("DDS: pixel_format missing in CodecParameters")
            })?)?;
        let plane = frame
            .image_planes()
            .first()
            .ok_or_else(|| DdsError::invalid("DDS: frame has no planes"))?;
        let mut img = DdsImage::new(
            width,
            height,
            pix,
            vec![Plane::new(plane.stride, plane.data.clone())],
        )?;
        if let Some(sig) = frame.color_signal() {
            img.color = from_color_signal(&sig);
        }
        Ok(img)
    }
}

impl TryFrom<(&VideoFrame, &CodecParameters)> for DdsImage {
    type Error = DdsError;
    fn try_from((frame, params): (&VideoFrame, &CodecParameters)) -> Result<Self, DdsError> {
        DdsImage::from_video_frame(frame, params)
    }
}

// ---- CodecOptionsStruct (registry-only schema for EncodeOptions) ----------

/// The framework's options schema for the DDS encoder: `surface_format`
/// (a [`SurfaceFormat::from_name`] spelling, empty = natural layout),
/// `mip_levels` (`1` none, `0` full chain) and `dx10_header`.
impl CodecOptionsStruct for EncodeOptions {
    const SCHEMA: &'static [OptionField] = &[
        OptionField {
            name: "surface_format",
            kind: OptionKind::String,
            default: OptionValue::String(String::new()),
            help: "Stored layout by name (A8R8G8B8, BC7_UNORM, R32G32B32A32_FLOAT, \
                   ASTC_4x4, …); empty picks the natural layout of the input.",
        },
        OptionField {
            name: "mip_levels",
            kind: OptionKind::U32,
            default: OptionValue::U32(1),
            help: "Mip levels to write: 1 = none, 0 = full chain to 1x1, n = n levels.",
        },
        OptionField {
            name: "dx10_header",
            kind: OptionKind::Bool,
            default: OptionValue::Bool(false),
            help: "Force the DDS_HEADER_DXT10 extension (written automatically when needed).",
        },
    ];
    fn apply(&mut self, key: &str, v: &OptionValue) -> oxideav_core::Result<()> {
        match key {
            "surface_format" => {
                let name = v.as_str()?;
                self.surface_format = if name.is_empty() {
                    None
                } else {
                    Some(SurfaceFormat::from_name(name).ok_or_else(|| {
                        oxideav_core::Error::invalid(format!(
                            "DDS: unknown surface_format {name:?}"
                        ))
                    })?)
                };
            }
            "mip_levels" => self.mip_levels = v.as_u32()?,
            "dx10_header" => self.dx10_header = v.as_bool()?,
            _ => unreachable!("guarded by SCHEMA"),
        }
        Ok(())
    }
}

// ---- Decoder trait impl + factory ------------------------------------------

/// Factory registered with the codec registry. Consumes one packet per
/// whole DDS file and produces one frame: the top-level surface in its
/// native contract layout ([`crate::decode`]). DDS is a single-image
/// format, so `flush()` just drains the one pending frame.
pub fn make_decoder(_params: &CodecParameters) -> oxideav_core::Result<Box<dyn Decoder>> {
    Ok(Box::new(DdsDecoder {
        codec_id: CodecId::new(CODEC_ID_STR),
        pending: None,
        eof: false,
    }))
}

/// DDS `Decoder` trait impl (see [`make_decoder`]).
pub struct DdsDecoder {
    codec_id: CodecId,
    pending: Option<VideoFrame>,
    eof: bool,
}

impl Decoder for DdsDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn send_packet(&mut self, packet: &Packet) -> oxideav_core::Result<()> {
        if self.pending.is_some() {
            return Err(oxideav_core::Error::other(
                "DDS decoder: receive_frame must be called before sending another packet",
            ));
        }
        let image = crate::api::decode_with(&packet.data, &DecodeOptions::default())?;
        self.pending = Some(image_into_video_frame(image, packet.pts));
        Ok(())
    }
    fn receive_frame(&mut self) -> oxideav_core::Result<Frame> {
        match self.pending.take() {
            Some(f) => Ok(Frame::Video(f)),
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---- Encoder trait impl + factory ------------------------------------------

/// Factory registered with the codec registry: one `VideoFrame` (any
/// contract layout per `CodecParameters::pixel_format`) becomes one DDS
/// file packet. `CodecParameters::options` is parsed as
/// [`EncodeOptions`] (`surface_format`, `mip_levels`, `dx10_header`).
pub fn make_encoder(params: &CodecParameters) -> oxideav_core::Result<Box<dyn Encoder>> {
    let opts = parse_options::<EncodeOptions>(&params.options)?;
    let width = params
        .width
        .ok_or_else(|| oxideav_core::Error::invalid("DDS encoder: missing width"))?;
    let height = params
        .height
        .ok_or_else(|| oxideav_core::Error::invalid("DDS encoder: missing height"))?;
    let pix_core = params.pixel_format.unwrap_or(PixelFormat::Rgba);
    from_core_pixel_format(pix_core)?;
    let mut out_params = params.clone();
    out_params.media_type = oxideav_core::MediaType::Video;
    out_params.codec_id = CodecId::new(CODEC_ID_STR);
    out_params.width = Some(width);
    out_params.height = Some(height);
    out_params.pixel_format = Some(pix_core);
    Ok(Box::new(DdsEncoder {
        codec_id: CodecId::new(CODEC_ID_STR),
        out_params,
        opts,
        pending: None,
        eof: false,
    }))
}

/// DDS `Encoder` trait impl (see [`make_encoder`]).
pub struct DdsEncoder {
    codec_id: CodecId,
    out_params: CodecParameters,
    opts: EncodeOptions,
    pending: Option<Packet>,
    eof: bool,
}

impl Encoder for DdsEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }
    fn output_params(&self) -> &CodecParameters {
        &self.out_params
    }
    fn send_frame(&mut self, frame: &Frame) -> oxideav_core::Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => {
                return Err(oxideav_core::Error::invalid(
                    "DDS encoder: video frames only",
                ))
            }
        };
        let image = DdsImage::from_video_frame(vf, &self.out_params)?;
        let bytes = crate::api::encode(&image, &self.opts)?;
        let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
        pkt.pts = vf.pts;
        pkt.dts = vf.pts;
        pkt.flags.keyframe = true;
        self.pending = Some(pkt);
        Ok(())
    }
    fn receive_packet(&mut self) -> oxideav_core::Result<Packet> {
        match self.pending.take() {
            Some(p) => Ok(p),
            None => {
                if self.eof {
                    Err(oxideav_core::Error::Eof)
                } else {
                    Err(oxideav_core::Error::NeedMore)
                }
            }
        }
    }
    fn flush(&mut self) -> oxideav_core::Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---- Registration ----------------------------------------------------------

/// Register the DDS codec into the supplied [`CodecRegistry`].
pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("dds_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![
            PixelFormat::Rgba,
            PixelFormat::Bgra,
            PixelFormat::Rgb24,
            PixelFormat::Bgr24,
            PixelFormat::Gray8,
            PixelFormat::Ya8,
            PixelFormat::Gray16Le,
            PixelFormat::Rgba64Le,
            PixelFormat::GrayF32Le,
            PixelFormat::RgbF32Le,
            PixelFormat::RgbaF32Le,
        ]);
    reg.register(
        CodecInfo::new(CodecId::new(CODEC_ID_STR))
            .capabilities(caps)
            .decoder(make_decoder)
            .encoder(make_encoder)
            .encoder_options::<EncodeOptions>(),
    );
}

/// Register the `.dds` still-image container demuxer + muxer + probe
/// + extension into the supplied [`ContainerRegistry`].
///
/// The demuxer slurps the entire DDS file and emits exactly one packet
/// on stream 0 (single-frame-per-file convention shared with every
/// other still-image container in the workspace); it declares the
/// contract layout [`crate::info`] reports on the stream. The muxer
/// writes a single packet's bytes verbatim to its output stream.
pub fn register_containers(reg: &mut ContainerRegistry) {
    container::register(reg);
}

/// Unified entry point: install every codec and container provided by
/// `oxideav-dds` into a [`RuntimeContext`].
///
/// Also wired into `oxideav_meta::register_all` via the
/// [`oxideav_core::register!`] macro below.
pub fn register(ctx: &mut RuntimeContext) {
    register_codecs(&mut ctx.codecs);
    register_containers(&mut ctx.containers);
}

oxideav_core::register!("dds", register);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_containers_resolves_dds_extension_case_insensitive() {
        let mut reg = ContainerRegistry::new();
        register_containers(&mut reg);
        assert_eq!(reg.container_for_extension("dds"), Some("dds"));
        assert_eq!(reg.container_for_extension("DDS"), Some("dds"));
        assert_eq!(reg.container_for_extension("Dds"), Some("dds"));
        assert_eq!(reg.container_for_extension("png"), None);
    }

    #[test]
    fn register_via_runtime_context_installs_factories() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        let id = CodecId::new(CODEC_ID_STR);
        assert!(ctx.codecs.has_decoder(&id));
        assert!(ctx.codecs.has_encoder(&id));
        assert_eq!(ctx.containers.container_for_extension("dds"), Some("dds"));
    }

    #[test]
    fn pixel_format_names_mirror_core() {
        for (d, c) in [
            (DdsPixelFormat::Gray8, PixelFormat::Gray8),
            (DdsPixelFormat::Ya8, PixelFormat::Ya8),
            (DdsPixelFormat::Gray16Le, PixelFormat::Gray16Le),
            (DdsPixelFormat::Rgb24, PixelFormat::Rgb24),
            (DdsPixelFormat::Bgr24, PixelFormat::Bgr24),
            (DdsPixelFormat::Rgba, PixelFormat::Rgba),
            (DdsPixelFormat::Bgra, PixelFormat::Bgra),
            (DdsPixelFormat::Rgba64Le, PixelFormat::Rgba64Le),
            (DdsPixelFormat::GrayF32Le, PixelFormat::GrayF32Le),
            (DdsPixelFormat::RgbF32Le, PixelFormat::RgbF32Le),
            (DdsPixelFormat::RgbaF32Le, PixelFormat::RgbaF32Le),
        ] {
            assert_eq!(format!("{d:?}"), format!("{c:?}"));
            assert_eq!(to_core_pixel_format(d), c);
            assert_eq!(from_core_pixel_format(c).unwrap(), d);
        }
        assert!(from_core_pixel_format(PixelFormat::Yuv420P).is_err());
    }

    #[test]
    fn frame_bridge_round_trips_and_stamps_srgb_only() {
        let img = DdsImage::from_rgba8(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]).unwrap();
        let frame = VideoFrame::from(&img);
        assert!(
            frame.color_signal().is_none(),
            "default colour is not stamped"
        );
        let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        params.width = Some(2);
        params.height = Some(1);
        params.pixel_format = Some(PixelFormat::Rgba);
        let back = DdsImage::try_from((&frame, &params)).unwrap();
        assert_eq!(back, img);

        let srgb = img.clone().with_color(ColorInfo::srgb());
        let frame = VideoFrame::from(srgb.clone());
        let sig = frame.color_signal().expect("sRGB is stamped");
        assert_eq!(sig.transfer.0, ColorInfo::TRANSFER_SRGB);
        let back = DdsImage::from_video_frame(&frame, &params).unwrap();
        assert_eq!(back, srgb);
    }

    #[test]
    fn registry_decoder_emits_native_layout() {
        // A B8G8R8A8 file decodes to a Bgra frame, not a converted Rgba.
        let img = DdsImage::tight(1, 1, DdsPixelFormat::Bgra, vec![1, 2, 3, 4]).unwrap();
        let bytes = crate::api::encode(&img, &EncodeOptions::default()).unwrap();
        let info = crate::api::info(&bytes).unwrap();
        assert_eq!(info.format, DdsPixelFormat::Bgra);
        let params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        let mut dec = make_decoder(&params).unwrap();
        let pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
        dec.send_packet(&pkt).unwrap();
        match dec.receive_frame().unwrap() {
            Frame::Video(v) => assert_eq!(v.planes[0].data, vec![1, 2, 3, 4]),
            _ => panic!("video frame expected"),
        }
    }

    #[test]
    fn encoder_options_parse_surface_format() {
        let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        params.width = Some(4);
        params.height = Some(4);
        params.pixel_format = Some(PixelFormat::Rgba);
        params.options.insert("surface_format", "BC7_UNORM");
        params.options.insert("mip_levels", "0");
        let mut enc = make_encoder(&params).unwrap();
        let img = DdsImage::from_rgba8(4, 4, vec![200; 64]).unwrap();
        enc.send_frame(&Frame::Video(VideoFrame::from(img)))
            .unwrap();
        let pkt = enc.receive_packet().unwrap();
        let info = crate::api::info(&pkt.data).unwrap();
        assert_eq!(info.surface_format, SurfaceFormat::Bc7Unorm);
        assert_eq!(info.mip_levels, 3);
        assert_eq!(info.frames, 3);
    }
}
