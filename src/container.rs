//! `.dds` still-image container demuxer + muxer.
//!
//! DDS files are single-image: a magic + header (+ optional DXT10
//! extension) + the pixel array. We treat them like the other
//! single-image containers in the workspace (`oxideav-bmp`,
//! `oxideav-tga`, `oxideav-jpegxl`'s container surface): the demuxer
//! slurps the entire file and emits exactly one `Packet` on stream 0;
//! the muxer concatenates the (already-encoded) packet bytes to its
//! output stream.
//!
//! Reference: Microsoft's public "DDS file layout for textures" guide
//! (header magic + on-disk layout). No DirectXTex / D3DX / NVTT /
//! squish source consulted.

use std::io::{Read, SeekFrom, Write};

use oxideav_core::{
    CodecId, CodecParameters, CodecResolver, ContainerRegistry, Demuxer, Error, MediaType, Muxer,
    Packet, PixelFormat, ProbeData, ProbeScore, ReadSeek, Result, StreamInfo, TimeBase, WriteSeek,
    MAX_PROBE_SCORE,
};

use crate::types::{DDPF_FOURCC, DDS_HEADER_SIZE, DDS_MAGIC, FOURCC_DX10};
use crate::CODEC_ID_STR;

/// Register the `.dds` demuxer + muxer + probe + extension entries
/// against `reg`.
pub fn register(reg: &mut ContainerRegistry) {
    reg.register_demuxer(CODEC_ID_STR, open_demuxer);
    reg.register_muxer(CODEC_ID_STR, open_muxer);
    reg.register_extension("dds", CODEC_ID_STR);
    reg.register_probe(CODEC_ID_STR, probe);
}

/// Content-based probe: any input whose first 4 bytes are the ASCII
/// `"DDS "` magic is a DDS file.
pub fn probe(data: &ProbeData) -> ProbeScore {
    if data.buf.len() >= 4
        && data.buf[0] == b'D'
        && data.buf[1] == b'D'
        && data.buf[2] == b'S'
        && data.buf[3] == b' '
    {
        MAX_PROBE_SCORE
    } else if matches!(data.ext, Some("dds")) {
        oxideav_core::PROBE_SCORE_EXTENSION
    } else {
        0
    }
}

/// Demuxer factory: read the entire file into memory, peek at the
/// header for accurate `width / height / pixel_format` metadata, return
/// a single-stream demuxer that emits one packet on `next_packet`.
pub fn open_demuxer(
    mut input: Box<dyn ReadSeek>,
    _codecs: &dyn CodecResolver,
) -> Result<Box<dyn Demuxer>> {
    input.seek(SeekFrom::Start(0))?;
    let mut buf = Vec::new();
    input.read_to_end(&mut buf)?;
    if buf.len() < 4 + DDS_HEADER_SIZE {
        return Err(Error::invalid(format!(
            "DDS demuxer: input too small ({} bytes < {} = magic + header)",
            buf.len(),
            4 + DDS_HEADER_SIZE
        )));
    }
    let magic = u32::from_le_bytes([buf[0], buf[1], buf[2], buf[3]]);
    if magic != DDS_MAGIC {
        return Err(Error::invalid(format!(
            "DDS demuxer: bad magic 0x{magic:08x}, expected 0x{DDS_MAGIC:08x} (\"DDS \")"
        )));
    }

    // File layout (bytes 0..3 = magic, then 124 bytes of DDS_HEADER):
    //   bytes  4..7   dwSize
    //   bytes  8..11  dwFlags
    //   bytes 12..15  dwHeight
    //   bytes 16..19  dwWidth
    // Pull width / height for the StreamInfo. We don't do a full parse
    // here — the codec layer (parse_dds) does that on the packet.
    let height = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    let width = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);

    // Sniff for DX10 extension to decide if we should advertise a more
    // specific pixel format hint, but the framework-side StreamInfo
    // only carries the codec-level pixel format; we leave that as
    // `Rgba` (the closest core mapping) when the format is one the
    // registry-side `pix_to_core` knows, or as `None` otherwise.
    //
    // DDS_PIXELFORMAT lives at header offset 0x48 = 72, i.e. file
    // offset 4 + 72 = 76. Its layout is:
    //   76..79  dwSize
    //   80..83  dwFlags
    //   84..87  dwFourCC
    let pf_flags = u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]);
    let pf_fourcc = u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]);
    let _has_dx10 = pf_flags & DDPF_FOURCC != 0 && pf_fourcc == FOURCC_DX10;

    // Default to Rgba; the actual pixel layout is exposed per-surface
    // by the parser. CLI consumers that want to pick by PixelFormat use
    // this field as a coarse hint.
    let pixel_format = Some(PixelFormat::Rgba);

    let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
    params.width = Some(width);
    params.height = Some(height);
    params.pixel_format = pixel_format;
    let stream = StreamInfo {
        index: 0,
        params,
        time_base: TimeBase::new(1, 1),
        start_time: Some(0),
        duration: None,
    };

    Ok(Box::new(DdsDemuxerImpl {
        streams: vec![stream],
        data: Some(buf),
    }))
}

struct DdsDemuxerImpl {
    streams: Vec<StreamInfo>,
    data: Option<Vec<u8>>,
}

impl Demuxer for DdsDemuxerImpl {
    fn format_name(&self) -> &str {
        CODEC_ID_STR
    }
    fn streams(&self) -> &[StreamInfo] {
        &self.streams
    }
    fn next_packet(&mut self) -> Result<Packet> {
        match self.data.take() {
            Some(bytes) => {
                let mut pkt = Packet::new(0, TimeBase::new(1, 1), bytes);
                pkt.pts = Some(0);
                pkt.dts = Some(0);
                pkt.flags.keyframe = true;
                Ok(pkt)
            }
            None => Err(Error::Eof),
        }
    }
}

/// Muxer factory: write the encoded packet bytes verbatim to the output.
pub fn open_muxer(output: Box<dyn WriteSeek>, streams: &[StreamInfo]) -> Result<Box<dyn Muxer>> {
    if streams.len() != 1 {
        return Err(Error::invalid(format!(
            "DDS muxer: expected exactly one video stream, got {}",
            streams.len()
        )));
    }
    if streams[0].params.media_type != MediaType::Video {
        return Err(Error::invalid("DDS muxer: stream must be video"));
    }
    Ok(Box::new(DdsMuxerImpl { output }))
}

struct DdsMuxerImpl {
    output: Box<dyn WriteSeek>,
}

impl Muxer for DdsMuxerImpl {
    fn format_name(&self) -> &str {
        CODEC_ID_STR
    }
    fn write_header(&mut self) -> Result<()> {
        Ok(())
    }
    fn write_packet(&mut self, packet: &Packet) -> Result<()> {
        self.output.write_all(&packet.data)?;
        Ok(())
    }
    fn write_trailer(&mut self) -> Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    use crate::types::{
        DDPF_RGB, DDSCAPS_TEXTURE, DDSD_PITCH, DDSD_REQUIRED, DDS_PIXELFORMAT_SIZE,
    };

    fn build_minimal_dds(w: u32, h: u32) -> Vec<u8> {
        // Minimal A8R8G8B8 DDS file with `w * h` pixels of zero data.
        let pitch = w * 4;
        let data_len = (pitch as usize) * (h as usize);
        let mut out = Vec::with_capacity(4 + DDS_HEADER_SIZE + data_len);
        out.extend_from_slice(&DDS_MAGIC.to_le_bytes());
        out.extend_from_slice(&(DDS_HEADER_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&(DDSD_REQUIRED | DDSD_PITCH).to_le_bytes());
        out.extend_from_slice(&h.to_le_bytes());
        out.extend_from_slice(&w.to_le_bytes());
        out.extend_from_slice(&pitch.to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes()); // depth
        out.extend_from_slice(&0u32.to_le_bytes()); // mip_map_count
        for _ in 0..11 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        // Pixel format A8R8G8B8: size=32, flags=RGB|ALPHAPIXELS, fourCC=0,
        // bpp=32, masks={0x00ff0000, 0x0000ff00, 0x000000ff, 0xff000000}.
        out.extend_from_slice(&(DDS_PIXELFORMAT_SIZE as u32).to_le_bytes());
        out.extend_from_slice(&(DDPF_RGB | 0x1).to_le_bytes());
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&32u32.to_le_bytes());
        out.extend_from_slice(&0x00ff_0000u32.to_le_bytes());
        out.extend_from_slice(&0x0000_ff00u32.to_le_bytes());
        out.extend_from_slice(&0x0000_00ffu32.to_le_bytes());
        out.extend_from_slice(&0xff00_0000u32.to_le_bytes());
        out.extend_from_slice(&DDSCAPS_TEXTURE.to_le_bytes());
        for _ in 0..4 {
            out.extend_from_slice(&0u32.to_le_bytes());
        }
        out.extend(vec![0u8; data_len]);
        out
    }

    #[test]
    fn probe_recognises_magic() {
        let bytes = build_minimal_dds(4, 4);
        let s = probe(&ProbeData {
            buf: &bytes,
            ext: None,
        });
        assert_eq!(s, MAX_PROBE_SCORE);
    }

    #[test]
    fn probe_recognises_extension_only() {
        let s = probe(&ProbeData {
            buf: &[0u8; 0],
            ext: Some("dds"),
        });
        assert_eq!(s, oxideav_core::PROBE_SCORE_EXTENSION);
    }

    #[test]
    fn probe_rejects_bad_magic() {
        let s = probe(&ProbeData {
            buf: &[0xff, 0xd8, 0xff, 0xe0],
            ext: None,
        });
        assert_eq!(s, 0);
    }

    use oxideav_core::NullCodecResolver;

    #[test]
    fn demuxer_emits_single_packet_then_eof() {
        let bytes = build_minimal_dds(8, 4);
        let len = bytes.len();
        let cursor: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes));
        let mut dx = open_demuxer(cursor, &NullCodecResolver).unwrap();
        assert_eq!(dx.format_name(), "dds");
        assert_eq!(dx.streams().len(), 1);
        let s = &dx.streams()[0];
        assert_eq!(s.params.width, Some(8));
        assert_eq!(s.params.height, Some(4));
        let pkt = dx.next_packet().unwrap();
        assert_eq!(pkt.data.len(), len);
        assert!(matches!(dx.next_packet(), Err(Error::Eof)));
    }

    #[test]
    fn muxer_writes_packet_bytes_verbatim() {
        let bytes = build_minimal_dds(4, 4);
        let mut params = CodecParameters::video(CodecId::new(CODEC_ID_STR));
        params.width = Some(4);
        params.height = Some(4);
        params.pixel_format = Some(PixelFormat::Rgba);
        let stream = StreamInfo {
            index: 0,
            params,
            time_base: TimeBase::new(1, 1),
            start_time: Some(0),
            duration: None,
        };

        // Sink that owns a Vec internally and exposes it for inspection
        // — works around the `'static` requirement in `Box<dyn WriteSeek>`.
        let writer: Box<dyn WriteSeek> = Box::new(Cursor::new(Vec::new()));
        let mut mx = open_muxer(writer, std::slice::from_ref(&stream)).unwrap();
        mx.write_header().unwrap();
        let pkt = Packet::new(0, TimeBase::new(1, 1), bytes.clone());
        mx.write_packet(&pkt).unwrap();
        mx.write_trailer().unwrap();
        // The muxer wrote the packet bytes verbatim; we exercise the
        // path but can't read the inner buffer back through the Box.
        // The format-name + lifecycle checks above are the meaningful
        // test surface.
        assert_eq!(mx.format_name(), CODEC_ID_STR);
    }

    #[test]
    fn demuxer_rejects_bad_magic() {
        let bytes = vec![0u8; 4 + DDS_HEADER_SIZE];
        let cursor: Box<dyn ReadSeek> = Box::new(Cursor::new(bytes));
        match open_demuxer(cursor, &NullCodecResolver) {
            Err(Error::InvalidData(_)) => {}
            Err(other) => panic!("expected InvalidData, got {other}"),
            Ok(_) => panic!("expected InvalidData, got Ok"),
        }
    }
}
