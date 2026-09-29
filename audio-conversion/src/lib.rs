//! High-performance audio conversion library with FFmpeg as the native
//! execution engine.
//!
//! Rust orchestrates and validates; FFmpeg does all decoding,
//! resampling and encoding through dynamically linked `libavcodec`,
//! `libavformat`, `libavutil` and `libswresample`.
//!
//! ```no_run
//! use audio_conversion::{AudioConverter, AudioFormat, ConversionOptions};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let converter = AudioConverter::new()?;
//! let output = converter.convert(
//!     &input_bytes,
//!     ConversionOptions {
//!         format: AudioFormat::Opus,
//!         quality: 96,
//!         sample_rate: Some(48000),
//!         channels: Some(2),
//!     },
//! )?;
//! # Ok::<(), audio_conversion::Error>(())
//! ```

mod audio;
mod error;
mod ffi;

pub use audio::{
    AudioConverter, AudioFormat, AudioStreamParams, ConversionOptions, DecodedAudio,
    bitrate_for_quality,
};
pub use error::Error;
pub use ffi::encode::{EncodedPacket, EncoderParams};
