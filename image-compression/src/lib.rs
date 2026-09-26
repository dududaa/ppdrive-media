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
use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;

/// ppdrive plugin entry point: decodes and compresses an image.
///
/// `args` must be a pointer to a `Box` of `(&[u8], &serde_json::Value)`
/// (input bytes + options JSON), as produced by the ppdrive plugin
/// loader. Returns a boxed `DispatchResponse`; the caller takes
/// ownership of the returned pointer.
///
/// # Safety
///
/// - `args` must be a valid, uniquely owned
///   `*mut (&[u8], &serde_json::Value)` as created by the loader — it
///   is reclaimed with `Box::from_raw` here.
/// - The input slice and JSON reference must remain valid for the
///   duration of the call.
/// - The returned pointer must be reclaimed exactly once by the caller.
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
