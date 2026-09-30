//! ppdrive plugin cdylib for `media-streaming`.
//!
//! `plugin_dispatch` lives in this cdylib-only crate instead of in the
//! `media-streaming` library itself, following the same structural
//! rule as the other plugins: a `#[no_mangle]` symbol defined in an
//! rlib that is linked into another cdylib would collide with that
//! crate's own entry point at link time.
//!
//! Unlike the byte-oriented plugins (`(&[u8], &Value)` in, bytes out),
//! this entry point is **file-based**: the input is a path to an
//! existing media file and the output is a directory the package is
//! written into, because one call produces many files (playlists plus
//! segments).

use media_streaming::{MediaStreamer, StreamingOptions};
use ppdrive::plugin::loader::DispatchResponse;
use serde_json::Value;
use std::ffi::c_void;
use std::fmt::Display;
use std::path::Path;

/// ppdrive plugin entry point: packages a media file into an HLS or
/// DASH stream on disk.
///
/// `args` must be a pointer to a `Box` of
/// `(&Path, &Path, &serde_json::Value)` (input file path, output
/// directory path, options JSON), as produced by the ppdrive plugin
/// loader. The JSON deserializes into [`StreamingOptions`]. The
/// output directory is created when missing. Returns a boxed
/// [`DispatchResponse`]; the caller takes ownership of the returned
/// pointer. On success the `Ok` payload points at a
/// `Box<Vec<u8>>` of UTF-8 JSON — `{"playlist": "...", "files":
/// ["...", ...]}` — describing the package written to the output
/// directory.
///
/// # Safety
///
/// - `args` must be a valid, uniquely owned
///   `*mut (&Path, &Path, &Value)` as created by the loader — it is
///   reclaimed with `Box::from_raw` here.
/// - Both paths and the JSON reference must remain valid for the
///   duration of the call.
/// - The returned pointer must be reclaimed exactly once by the
///   caller; the `Ok` payload must be reclaimed as a
///   `Box<Vec<u8>>`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn plugin_dispatch(args: *mut c_void) -> *mut DispatchResponse {
    let args = unsafe { Box::from_raw(args as *mut (&Path, &Path, &Value)) };
    let options = serde_json::from_value::<StreamingOptions>(args.2.clone());

    if let Err(err) = &options {
        return unsafe_err(err);
    }

    let resp = match MediaStreamer::new() {
        Ok(streamer) => match streamer.stream_file(args.0, args.1, &options.unwrap_or_default()) {
            Ok(output) => match serde_json::to_vec(&output) {
                Ok(bytes) => {
                    let data = Box::into_raw(Box::new(bytes)) as *mut c_void;
                    DispatchResponse::Ok(data)
                }
                Err(err) => DispatchResponse::Error(err.to_string()),
            },
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
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> TempDir {
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "ppdrive-media-streaming-plugin-{tag}-{}-{}",
                std::process::id(),
                COUNTER.fetch_add(1, Ordering::Relaxed)
            ));
            let _ = std::fs::remove_dir_all(&path);
            TempDir(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures")
            .join(name)
    }

    /// Calls `plugin_dispatch` exactly like the host loader: boxes the
    /// args, reclaims the response, and decodes the `Ok` payload.
    unsafe fn dispatch(input: &Path, output: &Path, options: &Value) -> Result<Vec<u8>, String> {
        let args = Box::into_raw(Box::new((input, output, options)));
        let resp = unsafe { plugin_dispatch(args as *mut c_void) };
        assert!(!resp.is_null());
        match *unsafe { Box::from_raw(resp) } {
            DispatchResponse::Ok(ptr) => {
                let bytes = *unsafe { Box::from_raw(ptr as *mut Vec<u8>) };
                Ok(bytes)
            }
            DispatchResponse::Error(msg) => Err(msg),
        }
    }

    #[test]
    fn dispatch_packages_file_and_returns_playlist_json() {
        let dir = TempDir::new("roundtrip");
        let out = dir.path().join("out");
        let options = serde_json::json!({"protocol": "hls", "segment_duration": 1});

        let bytes = unsafe { dispatch(&fixture("input.mp4"), &out, &options) }.unwrap();
        let json: Value = serde_json::from_slice(&bytes).unwrap();
        let playlist = json["playlist"].as_str().unwrap();
        assert!(playlist.ends_with("master.m3u8"), "{playlist}");
        assert!(
            Path::new(playlist).is_file(),
            "playlist missing: {playlist}"
        );
        let files = json["files"].as_array().unwrap();
        assert!(files.len() >= 3, "{json}");
        assert!(
            files
                .iter()
                .any(|f| { f.as_str().is_some_and(|f| f.ends_with(".ts")) }),
            "{json}"
        );
    }

    #[test]
    fn dispatch_reports_invalid_options() {
        let dir = TempDir::new("bad-options");
        let options = serde_json::json!({"segment_duration": 0});
        let err = unsafe { dispatch(&fixture("input.mp4"), dir.path(), &options) }.unwrap_err();
        assert!(!err.is_empty());
        assert!(!dir.path().join("master.m3u8").exists());
    }

    #[test]
    fn dispatch_reports_missing_input_file() {
        let dir = TempDir::new("missing-input");
        let missing = dir.path().join("nope.mp4");
        let options = serde_json::json!({});
        let err = unsafe { dispatch(&missing, dir.path(), &options) }.unwrap_err();
        assert!(!err.is_empty());
    }
}
