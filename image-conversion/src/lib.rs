//! High-performance image conversion library with FFmpeg as the native
//! execution engine.
//!
//! Rust orchestrates; FFmpeg does all decoding, resizing, pixel conversion
//! and encoding through dynamically linked `libavcodec`, `libavformat`,
//! `libavutil` and `libswscale`.
//!
//! ```no_run
//! use image_conversion::{ConversionOptions, ImageConverter, ImageFormat};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let converter = ImageConverter::new()?;
//! let output = converter.convert(
//!     &input_bytes,
//!     ConversionOptions {
//!         format: ImageFormat::WebP,
//!         quality: 80,
//!         width: Some(800),
//!         height: Some(600),
//!         ..Default::default()
//!     },
//! )?;
//! # Ok::<(), image_conversion::Error>(())
//! ```

mod error;
mod ffi;
mod image;

pub use error::Error;
pub use image::{ConversionOptions, Frame, ImageConverter, ImageFormat};
