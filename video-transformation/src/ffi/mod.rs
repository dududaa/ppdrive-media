pub mod graph;

pub use graph::FilterGraph;

#[allow(
    dead_code,
    non_camel_case_types,
    non_snake_case,
    non_upper_case_globals,
    improper_ctypes,
    clippy::all
)]
mod bindings {
    include!(concat!(env!("OUT_DIR"), "/bindings.rs"));
}
pub use bindings::*;

use std::ffi::CStr;
use std::os::raw::{c_char, c_int};
use video_compression::Error;

/// `AVERROR_EOF` — tag `"EOF "` inverted (see FFmpeg's `FFERRTAG`).
const AVERROR_EOF_CODE: c_int = -0x20464f45;
/// `AVERROR(EAGAIN)` — `EAGAIN` is 11 on every supported target.
const AVERROR_EAGAIN_CODE: c_int = -11;

pub fn av_err_string(code: c_int) -> String {
    let mut buf = [0 as c_char; 256];
    let ret = unsafe { av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    if ret < 0 {
        return format!("unknown error code {code}");
    }
    unsafe { CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub fn err_from_code(code: c_int) -> Error {
    Error::FfmpegError(av_err_string(code))
}
