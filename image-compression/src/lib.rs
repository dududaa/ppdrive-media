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
pub use image::{CompressionOptions, ImageCompressor, ImageFormat};
use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;

#[unsafe(no_mangle)]
pub unsafe extern "C" fn plugin_dispatch(args: *mut c_void) -> *mut DispatchResponse {
    let args = unsafe { Box::from_raw(args as *mut (&[u8], &Value)) };
    let options = serde_json::from_value::<CompressionOptions>(args.1.clone());

    if let Err(err) = &options {
        return unsafe_err(err);
    }

    let resp = match ImageCompressor::new() {
        Ok(compressor) => match compressor.compress(args.0, options.unwrap_or_default()) {
            Ok(data) => {
                let data = Box::into_raw(Box::new(data)) as *mut c_void;
                DispatchResponse::Ok(data)
            }
            Err(err) => DispatchResponse::Error(err.to_string()),
        },
        Err(err) => DispatchResponse::Error(err.to_string()),
    };

    Box::into_raw(Box::new(resp))
}

fn unsafe_err(err: impl Display) -> *mut DispatchResponse {
    let resp = DispatchResponse::Error(err.to_string());
    Box::into_raw(Box::new(resp))
}
