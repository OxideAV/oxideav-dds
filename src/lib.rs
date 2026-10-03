//! Pure-Rust DDS (DirectDraw Surface) reader / writer.
//!
//! DDS is Microsoft's container for Direct3D textures: a 4-byte ASCII
//! magic, a fixed-layout 124-byte `DDS_HEADER`, an optional 20-byte
//! `DDS_HEADER_DXT10` extension (when the legacy header signals
//! `four_cc == "DX10"`), and the raw pixel array (or block-compressed
//! block array) for every mip level, cubemap face, array slice and
//! volume depth slice the file carries.
//!
//! The crate follows the OxideAV image-crate contract
//! (`IMAGE_CRATE_API`) and has two layers:
//!
//! * **The contract layer** — [`probe`], [`info`], [`decode`],
//!   [`decode_with`], [`decode_rgb8`], [`decode_rgba8`], [`decode_all`],
//!   [`decode_from`], [`encode`], [`encode_rgb8`], [`encode_rgba8`],
//!   [`encode_to`], [`encode_all`] over [`DdsImage`] (one surface in a
//!   layout `oxideav_core::PixelFormat` names: `Gray8`, `Ya8`,
//!   `Gray16Le`, `Rgb24`, `Bgr24`, `Rgba`, `Bgra`, `Rgba64Le`,
//!   `GrayF32Le`, `RgbF32Le`, `RgbaF32Le`), [`ImageInfo`], [`Frame`],
//!   [`DecodeOptions`], [`EncodeOptions`], [`DdsError`]. Every stored
//!   layout with a colour meaning expands into one of those — the BC1..BC7
//!   and ASTC LDR block decoders run inside [`decode`]; see
//!   [`SurfaceFormat::contract_format`] for the table.
//! * **The depth layer** — [`parse_dds`] → [`DdsFile`] (the whole
//!   mip / face / slice / depth tree in its stored [`SurfaceFormat`],
//!   ~70 `D3DFMT` / `DXGI_FORMAT` layouts including the plain-integer,
//!   depth-stencil and YUV families the contract layer does not expand),
//!   [`write_dds_file`] and the `encode_dds_*` writers, the per-layout
//!   block codecs (`decode_bc1` … `decode_bc7`, `decode_bc6h`,
//!   `encode_bc1` … `encode_bc7`, `encode_bc6h*`, the ASTC LDR codec in
//!   [`astc`]) and the per-layout surface decoders in [`hdr`], [`yuv`]
//!   and [`depth`].
//!
//! ## Standalone vs registry-integrated
//!
//! The crate's default `registry` Cargo feature pulls in `oxideav-core`
//! and exposes `register` / `make_decoder` / `make_encoder`, the
//! `.dds` container demuxer + muxer and the `VideoFrame` bridge. Disable
//! the feature (`default-features = false`) for an `oxideav-core`-free
//! build with the full contract and depth layers.
//!
//! ## Clean-room provenance
//!
//! Every byte of the parser was written from Microsoft's public DDS
//! programming-guide pages on learn.microsoft.com (the "DDS file
//! layout for textures", "DDS pixel format", and "Programming guide
//! for DDS" articles plus the public DXGI format reference) and, for
//! ASTC, the Khronos Data Format Specification. Binaries (`magick`,
//! `texconv`) are used only as black-box validators when generating
//! test fixtures, not as a source of constants or layout.

pub mod api;
pub mod astc;
pub mod bc6h;
pub mod bc6h_enc;
pub mod bc7;
pub mod bc7_enc;
pub mod bcn;
pub mod bcn_enc;
#[cfg(feature = "registry")]
pub mod container;
pub(crate) mod convert;
pub mod decoder;
pub mod depth;
pub mod encoder;
pub mod error;
pub mod hdr;
pub mod image;
pub mod options;
pub mod surface;
pub mod types;
pub mod yuv;

#[cfg(feature = "registry")]
pub mod registry;

/// Codec id for DDS image frames.
pub const CODEC_ID_STR: &str = "dds";

// ---- The image-crate contract (IMAGE_CRATE_API) ----------------------------

pub use api::{
    decode, decode_all, decode_all_with, decode_from, decode_rgb8, decode_rgba8, decode_with,
    encode, encode_all, encode_rgb8, encode_rgba8, encode_to, info, info_with, probe,
};
pub use error::{DdsError, Error, Result};
pub use image::{
    ColorInfo, ColorRange, DdsImage, DdsPixelFormat, Frame, ImageInfo, Metadata, PixelFormat,
    Plane, RgbImage, RgbaImage,
};
pub use options::{DecodeOptions, EncodeOptions};

// ---- Depth API: the stored-surface model and the per-layout codecs ---------

pub use decoder::{parse_dds, parse_dds_with};
pub use encoder::write_dds_file;
#[allow(deprecated)]
pub use surface::DdsPlane;
pub use surface::{CubemapFace, DdsFile, DdsSurface, SurfaceFormat};
pub use types::{
    DdsHeader, DdsHeaderDxt10, DdsPixelFormatHeader, DxgiFormat, DDS_HEADER_DXT10_SIZE,
    DDS_HEADER_SIZE, DDS_MAGIC, DDS_PIXELFORMAT_SIZE,
};

pub use astc::{
    decode_astc_ldr, decode_astc_ldr_block, decode_astc_ldr_surface, encode_astc_ldr,
    encode_astc_ldr_block, encode_astc_ldr_surface, is_valid_footprint,
    ERROR_COLOR as ASTC_ERROR_COLOR, LDR_BLOCK_FOOTPRINTS,
};
pub use bc6h::decode_bc6h;
pub use bc6h_enc::{
    encode_bc6h, encode_bc6h_from_f32, encode_bc6h_sf16, encode_bc6h_sf16_from_f32,
};
pub use bc7::decode_bc7;
pub use bc7_enc::encode_bc7;
pub use bcn::{
    decode_bc1, decode_bc2, decode_bc3, decode_bc4_snorm, decode_bc4_snorm_i8, decode_bc4_unorm,
    decode_bc5_snorm, decode_bc5_snorm_i8, decode_bc5_unorm,
};
pub use bcn_enc::{
    encode_bc1, encode_bc2, encode_bc3, encode_bc4_snorm, encode_bc4_unorm, encode_bc5_snorm,
    encode_bc5_unorm,
};
pub use depth::{
    decode_depth_d16_surface, decode_depth_d24s8_surface, decode_depth_d32_surface,
    decode_depth_d32s8_surface, decode_depth_r24_unorm_x8_surface,
    decode_depth_r32_float_x8x24_surface, decode_depth_x24_g8_uint_surface,
    decode_depth_x32_g8x24_uint_surface, DepthStencil,
};
pub use encoder::{
    encode_dds_astc, encode_dds_block_compressed, encode_dds_block_compressed_from_rgba8,
    encode_dds_uncompressed, encode_dds_uncompressed_cubemap_array, encode_dds_uncompressed_dx10,
    encode_dds_volume, encode_dds_volume_block_compressed,
};
pub use hdr::{
    decode_a2r10g10b10_surface, decode_a4b4g4r4_unorm_surface, decode_a8r3g3b2_surface,
    decode_float_surface, decode_g8r8_g8b8_unorm_surface, decode_r10g10b10a2_uint_surface,
    decode_r10g10b10a2_unorm_surface, decode_r11g11b10_float_surface,
    decode_r8g8_b8g8_unorm_surface, decode_r9g9b9e5_sharedexp_surface, decode_rgba16_snorm_surface,
    decode_rgba16_unorm_surface, decode_sint16_surface, decode_sint32_surface,
    decode_sint8_surface, decode_snorm_surface, decode_uint16_surface, decode_uint32_surface,
    decode_uint8_surface, decode_unorm_surface,
};
pub use yuv::{
    decode_420_opaque_surface, decode_ayuv_surface, decode_nv11_surface, decode_nv12_surface,
    decode_p010_surface, decode_p016_surface, decode_uyvy_surface, decode_y210_surface,
    decode_y216_surface, decode_y410_surface, decode_y416_surface, decode_yuy2_surface, YuvFormat,
    YuvSampling,
};

// ---- Framework integration (`registry` feature) ----------------------------

#[cfg(feature = "registry")]
pub use registry::{
    from_color_signal, from_core_pixel_format, make_decoder, make_encoder, register,
    register_codecs, register_containers, to_color_signal, to_core_pixel_format, DdsDecoder,
    DdsEncoder,
};

#[cfg(feature = "registry")]
#[doc(hidden)]
pub use registry::__oxideav_entry;
