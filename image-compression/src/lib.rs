//! High-performance image compression library with FFmpeg as the native
//! execution engine.
//!
//! Rust orchestrates; FFmpeg does all decoding, resizing, pixel conversion
//! and encoding through dynamically linked `libavcodec`, `libavformat`,
//! `libavutil` and `libswscale`.
//!
//! ```no_run
//! use image_compression::{CompressionOptions, ImageCompressor, ImageFormat};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let compressor = ImageCompressor::new()?;
//! let output = compressor.compress(
//!     &input_bytes,
//!     CompressionOptions {
//!         format: ImageFormat::WebP,
//!         quality: 80,
//!         width: Some(800),
//!         height: Some(600),
//!     },
//! )?;
//! # Ok::<(), image_compression::Error>(())
//! ```

mod error;
mod ffi;
mod image;

pub use error::Error;
pub use image::{CompressionOptions, Frame, ImageCompressor, ImageFormat};
