//! High-performance video conversion library with FFmpeg as the native
//! execution engine.
//!
//! Rust orchestrates; FFmpeg does all demuxing, decoding, scaling,
//! pixel conversion, encoding and muxing through dynamically linked
//! `libavcodec`, `libavformat`, `libavutil` and `libswscale`.
//!
//! ```no_run
//! use video_conversion::{ConversionOptions, VideoConverter, VideoFormat};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let converter = VideoConverter::new()?;
//! let output = converter.convert(
//!     &input_bytes,
//!     ConversionOptions {
//!         format: VideoFormat::Mp4,
//!         quality: 80,
//!         width: Some(1280),
//!         height: None,
//!     },
//! )?;
//! # Ok::<(), video_conversion::Error>(())
//! ```

mod error;
mod ffi;
mod video;

pub use error::Error;
pub use ffi::encode::{AudioSource, VideoEncoderParams, VideoPacket};
pub use video::{
    AudioPacket, ConversionOptions, StreamEvent, VideoConverter, VideoEncoder, VideoFormat,
    VideoFrame, VideoStream, VideoStreamInfo, crf_for_quality,
};
