//! `oxideav-core` integration layer for `oxideav-dds`.
//!
//! Gated behind the default-on `registry` feature so image-library
//! consumers can depend on `oxideav-dds` with `default-features = false`
//! and skip the `oxideav-core` dependency entirely.
//!
//! The module exposes:
//! * [`register`] — the unified `RuntimeContext` entry point the
//!   umbrella `oxideav` crate calls during framework initialisation.
//!   Internally calls [`register_codecs`] and [`register_containers`].
//! * [`register_codecs`] — registers the DDS codec (decoder + encoder)
//!   into a [`CodecRegistry`].
//! * [`register_containers`] — registers the `.dds` file extension
//!   into the [`oxideav_core::ContainerRegistry`] so CLI tools can
//!   resolve `.dds` outputs by extension. The actual demuxer / muxer
//!   for the `.dds` still-image container lands in round 2; the
//!   extension entry alone is enough for `cli-convert` to pick the
//!   right codec name from the central registry.
//! * The `From<DdsError> for oxideav_core::Error` conversion that lets
//!   the trait-side `Decoder` / `Encoder` impls bubble bitstream
//!   errors up through the framework error type.
//! * The [`DdsDecoder`] / [`DdsEncoder`] structs that implement the
//!   framework `Decoder` / `Encoder` traits. Both wrap the
//!   framework-free [`crate::parse_dds`] / [`crate::encode_dds_uncompressed`]
//!   entry points.

use std::collections::VecDeque;

use oxideav_core::frame::VideoPlane;
use oxideav_core::{
    CodecCapabilities, CodecId, CodecInfo, CodecParameters, CodecRegistry, ContainerRegistry,
    Decoder, Encoder, Error, Frame, MediaType, Packet, PixelFormat, Result, RuntimeContext,
    TimeBase, VideoFrame,
};

use crate::decoder::{make_decoder, parse_dds};
use crate::encoder::{encode_dds_uncompressed, make_encoder};
use crate::error::DdsError;
use crate::image::{DdsImage, DdsPixelFormat, DdsPlane, DdsSurface};
use crate::CODEC_ID_STR;

// ---- Error / pixel-format / frame conversions --------------------------

impl From<DdsError> for Error {
    fn from(e: DdsError) -> Self {
        match e {
            DdsError::InvalidData(s) => Error::InvalidData(s),
            DdsError::Unsupported(s) => Error::Unsupported(s),
        }
    }
}

/// Map a [`DdsPixelFormat`] onto the closest `oxideav_core::PixelFormat`.
///
/// Only the formats that have a direct `oxideav-core` counterpart (RGBA
/// 8-bit, grayscale 8-bit) are mapped — block-compressed and packed
/// 16-bit / 24-bit BGR layouts have no canonical `PixelFormat` today
/// and surface as `None`. Callers using the standalone
/// [`crate::parse_dds`] always see the full enum; only the registry
/// path drops detail.
fn pix_to_core(p: DdsPixelFormat) -> Option<PixelFormat> {
    Some(match p {
        // RGBA on disk maps cleanly; BGRA / BGRX would need a swap to
        // become Rgba. Round 1 surfaces only the lossless mapping.
        DdsPixelFormat::A8B8G8R8 => PixelFormat::Rgba,
        DdsPixelFormat::L8 => PixelFormat::Gray8,
        _ => return None,
    })
}

fn pix_from_core(p: PixelFormat) -> Option<DdsPixelFormat> {
    Some(match p {
        PixelFormat::Rgba => DdsPixelFormat::A8B8G8R8,
        PixelFormat::Gray8 => DdsPixelFormat::L8,
        _ => return None,
    })
}

impl From<DdsImage> for VideoFrame {
    fn from(img: DdsImage) -> Self {
        VideoFrame {
            pts: img.pts,
            planes: img
                .planes
                .into_iter()
                .map(|p| VideoPlane {
                    stride: p.stride,
                    data: p.data,
                })
                .collect(),
        }
    }
}

// ---- CodecRegistry entry points ---------------------------------------

/// Register the DDS codec into the supplied [`CodecRegistry`].
pub fn register_codecs(reg: &mut CodecRegistry) {
    let caps = CodecCapabilities::video("dds_sw")
        .with_intra_only(true)
        .with_lossless(true)
        .with_max_size(65535, 65535)
        .with_pixel_formats(vec![PixelFormat::Rgba, PixelFormat::Gray8]);
    reg.register(
        CodecInfo::new(CodecId::new(CODEC_ID_STR))
            .capabilities(caps)
            .decoder(make_decoder)
            .encoder(make_encoder),
    );
}

// ---- ContainerRegistry entry points -----------------------------------

/// Register the `.dds` file extension into the supplied
/// [`ContainerRegistry`] so CLI tools (and any caller using
/// [`ContainerRegistry::container_for_extension`]) can resolve a
/// `.dds` output path to the `dds` codec name.
///
/// No demuxer or muxer is registered here — round 1 surfaces only the
/// codec, and consumers that want a one-frame-per-file `.dds` stream
/// drive [`crate::parse_dds`] / [`crate::encode_dds_uncompressed`]
/// directly. Round 2 will add the still-image container demuxer/muxer.
pub fn register_containers(reg: &mut ContainerRegistry) {
    reg.register_extension("dds", "dds");
}

/// Unified entry point: install every codec and container provided by
/// `oxideav-dds` into a [`RuntimeContext`].
///
/// Also auto-registered into [`oxideav_core::REGISTRARS`] via the
/// [`oxideav_core::register!`] macro below so consumers calling
/// [`oxideav_core::RuntimeContext::with_all_features`] pick DDS up
/// without any explicit umbrella plumbing.
pub fn register(ctx: &mut RuntimeContext) {
    register_codecs(&mut ctx.codecs);
    register_containers(&mut ctx.containers);
}

oxideav_core::register!("dds", register);

// ---- Decoder trait impl ------------------------------------------------

pub(crate) struct DdsDecoder {
    codec_id: CodecId,
    pending: Option<Packet>,
    eof: bool,
}

impl DdsDecoder {
    pub fn new(codec_id: CodecId) -> Self {
        Self {
            codec_id,
            pending: None,
            eof: false,
        }
    }
}

impl Decoder for DdsDecoder {
    fn codec_id(&self) -> &CodecId {
        &self.codec_id
    }

    fn send_packet(&mut self, packet: &Packet) -> Result<()> {
        if self.pending.is_some() {
            return Err(Error::other(
                "DDS decoder: receive_frame must be called before sending another packet",
            ));
        }
        self.pending = Some(packet.clone());
        Ok(())
    }

    fn receive_frame(&mut self) -> Result<Frame> {
        let Some(pkt) = self.pending.take() else {
            return if self.eof {
                Err(Error::Eof)
            } else {
                Err(Error::NeedMore)
            };
        };
        let mut img = parse_dds(&pkt.data)?;
        img.pts = pkt.pts;
        Ok(Frame::Video(img.into()))
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

// ---- Encoder trait impl ------------------------------------------------

pub(crate) struct DdsEncoder {
    output_params: CodecParameters,
    width: u32,
    height: u32,
    pix: DdsPixelFormat,
    time_base: TimeBase,
    pending: VecDeque<Packet>,
    eof: bool,
}

impl DdsEncoder {
    pub fn from_params(params: &CodecParameters) -> Result<Self> {
        let width = params
            .width
            .ok_or_else(|| Error::invalid("DDS encoder: missing width"))?;
        let height = params
            .height
            .ok_or_else(|| Error::invalid("DDS encoder: missing height"))?;
        let pix_core = params.pixel_format.unwrap_or(PixelFormat::Rgba);
        let pix = pix_from_core(pix_core).ok_or_else(|| {
            Error::unsupported(format!(
                "DDS encoder: pixel format {pix_core:?} not supported"
            ))
        })?;
        let mut output_params = params.clone();
        output_params.media_type = MediaType::Video;
        output_params.codec_id = CodecId::new(CODEC_ID_STR);
        output_params.width = Some(width);
        output_params.height = Some(height);
        output_params.pixel_format = pix_to_core(pix);
        Ok(Self {
            output_params,
            width,
            height,
            pix,
            time_base: TimeBase::new(1, 1),
            pending: VecDeque::new(),
            eof: false,
        })
    }
}

impl Encoder for DdsEncoder {
    fn codec_id(&self) -> &CodecId {
        &self.output_params.codec_id
    }

    fn output_params(&self) -> &CodecParameters {
        &self.output_params
    }

    fn send_frame(&mut self, frame: &Frame) -> Result<()> {
        let vf = match frame {
            Frame::Video(v) => v,
            _ => return Err(Error::invalid("DDS encoder: video frames only")),
        };
        if vf.planes.is_empty() {
            return Err(Error::invalid("DDS encoder: empty frame plane"));
        }
        let plane = &vf.planes[0];
        let dds_plane = DdsPlane {
            stride: plane.stride,
            data: plane.data.clone(),
        };
        let img = DdsImage {
            width: self.width,
            height: self.height,
            pixel_format: self.pix,
            planes: vec![dds_plane.clone()],
            surfaces: vec![DdsSurface {
                width: self.width,
                height: self.height,
                mip_level: 0,
                array_slice: 0,
                face: None,
                plane: dds_plane,
            }],
            pts: vf.pts,
            mip_map_count: 1,
            has_dxt10_header: false,
            dxgi_format: None,
            is_cubemap: false,
            array_size: 1,
        };
        let bytes = encode_dds_uncompressed(&img)?;
        let mut pkt = Packet::new(0, self.time_base, bytes);
        pkt.pts = vf.pts;
        pkt.dts = vf.pts;
        pkt.flags.keyframe = true;
        self.pending.push_back(pkt);
        Ok(())
    }

    fn receive_packet(&mut self) -> Result<Packet> {
        self.pending.pop_front().ok_or(Error::NeedMore)
    }

    fn flush(&mut self) -> Result<()> {
        self.eof = true;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_containers_resolves_dds_extension_case_insensitive() {
        let mut reg = ContainerRegistry::new();
        register_containers(&mut reg);

        // Canonical lowercase lookup resolves to the codec name.
        assert_eq!(reg.container_for_extension("dds"), Some("dds"));

        // Lookups are case-insensitive — uppercase + mixed case both
        // resolve via the lowercase-keyed extension table.
        assert_eq!(reg.container_for_extension("DDS"), Some("dds"));
        assert_eq!(reg.container_for_extension("Dds"), Some("dds"));
        assert_eq!(reg.container_for_extension("dDs"), Some("dds"));

        // Unrelated extensions still miss.
        assert_eq!(reg.container_for_extension("png"), None);
    }

    #[test]
    fn register_via_runtime_context_installs_factories() {
        let mut ctx = RuntimeContext::new();
        register(&mut ctx);
        assert!(
            ctx.codecs.decoder_ids().next().is_some(),
            "register(ctx) should install codec decoder factories"
        );
        assert_eq!(
            ctx.containers.container_for_extension("dds"),
            Some("dds"),
            "register(ctx) should install .dds extension hint"
        );
    }
}
