//! High-performance image transformation library with FFmpeg's
//! libavfilter as the native execution engine.
//!
//! Rust orchestrates and validates; FFmpeg performs all pixel work in
//! a single `buffer → filter chain → buffersink` graph pass. Decoding
//! and encoding are reused from the `image-compression` crate via its
//! public [`Frame`] bridge.
//!
//! ```no_run
//! use image_transformation::{ImageTransformer, TransformOperation, TransformOptions};
//! use image_compression::ImageFormat;
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let transformer = ImageTransformer::new()?;
//! let output = transformer.transform(
//!     &input_bytes,
//!     TransformOptions {
//!         operations: vec![TransformOperation::Grayscale],
//!         custom_filters: Some("vignette=PI/5".to_string()),
//!         format: Some(ImageFormat::Png),
//!         quality: None,
//!     },
//! )?;
//! # Ok::<(), image_compression::Error>(())
//! ```

mod ffi;
mod image;

pub use image::{ImageTransformer, TransformOperation, TransformOptions};
pub use image_compression::{Error, ImageFormat};
use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;

/// ppdrive plugin entry point: transforms an image with libavfilter.
///
/// `args` must be a pointer to a `Box` of `(&[u8], &serde_json::Value)`
/// (input bytes + options JSON), as produced by the ppdrive plugin
/// loader. The JSON deserializes into [`TransformOptions`]. Returns a
/// boxed [`DispatchResponse`]; the caller takes ownership of the
/// returned pointer.
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
    let options = serde_json::from_value::<TransformOptions>(args.1.clone());

    if let Err(err) = &options {
        return unsafe_err(err);
    }

    let resp = match ImageTransformer::new() {
        Ok(transformer) => match transformer.transform(args.0, options.unwrap_or_default()) {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: TransformOptions =
            serde_json::from_value(serde_json::json!({"custom_filters": "hflip"})).unwrap();
        assert!(opts.operations.is_empty());
        assert_eq!(opts.custom_filters.as_deref(), Some("hflip"));
        assert_eq!(opts.format, None);
        assert_eq!(opts.quality, None);
    }

    #[test]
    fn options_deserialize_full_document() {
        let opts: TransformOptions = serde_json::from_value(serde_json::json!({
            "operations": [
                {"crop": {"x": 0, "y": 0, "width": 80, "height": 60}},
                {"rotate": {"degrees": 90}},
                "grayscale",
                {"adjust": {"brightness": 0.1, "contrast": 1.2, "saturation": 0.8}},
                {"blur": {"sigma": 2.0}},
                {"sharpen": {"amount": 0.7}},
                {"scale": {"width": 40, "height": 30}},
                {"flip": {"horizontal": true, "vertical": false}},
                {"pad": {"left": 4, "top": 4, "right": 4, "bottom": 4, "color": "black"}}
            ],
            "format": "Jpeg",
            "quality": 90
        }))
        .unwrap();
        assert_eq!(opts.operations.len(), 9);
        assert_eq!(opts.format, Some(ImageFormat::Jpeg));
        assert_eq!(opts.quality, Some(90));
    }

    #[test]
    fn options_reject_invalid_json() {
        assert!(
            serde_json::from_value::<TransformOptions>(serde_json::json!({
                "operations": [{"rotate": {"degrees": "left"}}]
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<TransformOptions>(serde_json::json!("nope")).is_err());
    }
}
