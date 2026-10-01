//! ppdrive plugin cdylib for `audio-conversion`.
//!
//! `plugin_dispatch` lives in this cdylib-only crate instead of in the
//! `audio-conversion` library itself, following the same structural
//! rule as `image-conversion`: an rlib whose code is linked into
//! another plugin cdylib must not define `plugin_dispatch`, or the
//! symbol collides at link time with the other crate's own entry point.

use ppff_audio_conversion::{AudioConverter, ConversionOptions};
use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;

/// ppdrive plugin entry point: converts an audio stream.
///
/// `args` must be a pointer to a `Box` of `(&[u8], &serde_json::Value)`
/// (input bytes + options JSON), as produced by the ppdrive plugin
/// loader. The JSON deserializes into [`ConversionOptions`]. Returns a
/// boxed `DispatchResponse`; the caller takes ownership of the returned
/// pointer.
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
    let options = serde_json::from_value::<ConversionOptions>(args.1.clone());

    if let Err(err) = &options {
        return unsafe_err(err);
    }

    let resp = match AudioConverter::new() {
        Ok(converter) => match converter.convert(args.0, options.unwrap_or_default()) {
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
