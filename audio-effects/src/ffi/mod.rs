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

use ppff_audio_conversion::Error;
use std::ffi::CStr;
use std::os::raw::{c_char, c_int};

pub const AVERROR_EOF_CODE: c_int = -0x20464f45;
pub const AV_NOPTS_VALUE: i64 = i64::MIN;

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

pub fn is_eagain(code: c_int) -> bool {
    if code >= 0 {
        return false;
    }
    std::io::Error::from_raw_os_error(-code).kind() == std::io::ErrorKind::WouldBlock
}
