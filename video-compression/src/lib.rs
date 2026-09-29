//! High-performance video compression library with FFmpeg as the native
//! execution engine.
//!
//! Rust orchestrates; FFmpeg does all demuxing, decoding, scaling,
//! pixel conversion, encoding and muxing through dynamically linked
//! `libavcodec`, `libavformat`, `libavutil` and `libswscale`.
//!
//! ```no_run
//! use video_compression::{CompressionOptions, VideoCompressor, VideoFormat};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let compressor = VideoCompressor::new()?;
//! let output = compressor.compress(
//!     &input_bytes,
//!     CompressionOptions {
//!         format: VideoFormat::Mp4,
//!         quality: 80,
//!         width: Some(1280),
//!         height: None,
//!     },
//! )?;
//! # Ok::<(), video_compression::Error>(())
//! ```

mod error;
mod ffi;
mod video;

pub use error::Error;
pub use ffi::encode::{AudioSource, VideoEncoderParams, VideoPacket};
pub use video::{
    AudioPacket, CompressionOptions, StreamEvent, VideoCompressor, VideoEncoder, VideoFormat,
    VideoFrame, VideoStream, VideoStreamInfo, crf_for_quality,
};
