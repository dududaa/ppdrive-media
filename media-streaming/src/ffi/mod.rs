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

use std::ffi::CString;
use std::os::raw::{c_char, c_int};

use crate::error::Error;

pub fn av_err_string(code: c_int) -> String {
    let mut buf = [0 as c_char; 256];
    let ret = unsafe { av_strerror(code, buf.as_mut_ptr(), buf.len()) };
    if ret < 0 {
        return format!("unknown error code {code}");
    }
    unsafe { std::ffi::CStr::from_ptr(buf.as_ptr()) }
        .to_string_lossy()
        .into_owned()
}

pub fn to_cstr(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::InvalidInput)
}

/// Maps video-conversion errors onto this crate's error type.
pub(crate) fn from_video(err: video_conversion::Error) -> Error {
    use video_conversion::Error as V;
    match err {
        V::FfmpegError(msg) => Error::FfmpegError(msg),
        V::EncoderNotFound => Error::EncoderNotFound,
        V::InvalidInput => Error::InvalidInput,
        V::UnsupportedFormat | V::DecoderNotFound => Error::UnsupportedFormat,
        V::LimitExceeded(msg) => Error::FfmpegError(msg),
        V::TargetSizeUnreachable => Error::InvalidInput,
    }
}

/// Maps audio-conversion errors onto this crate's error type.
pub(crate) fn from_audio(err: audio_conversion::Error) -> Error {
    use audio_conversion::Error as A;
    match err {
        A::FfmpegError(msg) => Error::FfmpegError(msg),
        A::EncoderNotFound => Error::EncoderNotFound,
        A::InvalidInput => Error::InvalidInput,
        A::UnsupportedFormat | A::DecoderNotFound => Error::UnsupportedFormat,
    }
}

/// Resolves a codec name (`avcodec_get_name` style, e.g. `"h264"`)
/// to an [`AVCodecID`] via the encoder table first, then the codec
/// descriptor table.
pub(crate) fn codec_id_by_name(name: &str) -> Result<AVCodecID, Error> {
    let c = to_cstr(name)?;
    unsafe {
        let encoder = avcodec_find_encoder_by_name(c.as_ptr());
        if !encoder.is_null() {
            return Ok((*encoder).id);
        }
        let descriptor = avcodec_descriptor_get_by_name(c.as_ptr());
        if !descriptor.is_null() {
            return Ok((*descriptor).id);
        }
    }
    Err(Error::EncoderNotFound)
}

/// Clones `data` into fresh codec-parameter extradata storage
/// (zero-padded for the `AV_INPUT_BUFFER_PADDING_SIZE` requirement).
pub(crate) unsafe fn set_extradata(par: *mut AVCodecParameters, data: &[u8]) -> Result<(), Error> {
    unsafe {
        if data.is_empty() {
            return Ok(());
        }
        let buf = av_mallocz(data.len() + 64) as *mut u8;
        if buf.is_null() {
            return Err(Error::FfmpegError("av_mallocz returned null".to_string()));
        }
        std::ptr::copy_nonoverlapping(data.as_ptr(), buf, data.len());
        (*par).extradata = buf;
        (*par).extradata_size = data.len() as i32;
        Ok(())
    }
}
