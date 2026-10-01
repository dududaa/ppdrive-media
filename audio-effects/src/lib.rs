//! High-performance audio effects library with FFmpeg's libavfilter
//! as the native execution engine.
//!
//! Rust orchestrates and validates; FFmpeg performs all sample work in
//! a single `abuffer → filter chain → abuffersink` graph pass.
//! Decoding and encoding are reused from the `audio-conversion` crate
//! via its [`ppff_audio_conversion::DecodedAudio`] bridge.
//!
//! ```no_run
//! use audio_effects::{AudioEffects, EffectOperation, EffectOptions};
//!
//! # let input_bytes: Vec<u8> = Vec::new();
//! let effects = AudioEffects::new()?;
//! let output = effects.apply(
//!     &input_bytes,
//!     EffectOptions {
//!         operations: vec![EffectOperation::Normalize { target_lufs: -16.0 }],
//!         custom_filters: None,
//!         format: None,
//!         quality: None,
//!     },
//! )?;
//! # Ok::<(), audio_effects::Error>(())
//! ```

mod audio;
mod ffi;

pub use audio::{AudioEffects, EffectOperation, EffectOptions};
pub use ppff_audio_conversion::{AudioFormat, Error};

use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;

/// ppdrive plugin entry point: applies audio effects with libavfilter.
///
/// `args` must be a pointer to a `Box` of `(&[u8], &serde_json::Value)`
/// (input bytes + options JSON), as produced by the ppdrive plugin
/// loader. The JSON deserializes into [`EffectOptions`]. Returns a
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
    let options = serde_json::from_value::<EffectOptions>(args.1.clone());

    if let Err(err) = &options {
        return unsafe_err(err);
    }

    let resp = match AudioEffects::new() {
        Ok(effects) => match effects.apply(args.0, options.unwrap_or_default()) {
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
