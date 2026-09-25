use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    FfmpegError(String),
    DecoderNotFound,
    EncoderNotFound,
    InvalidInput,
    UnsupportedFormat,
}

impl Error {
    pub(crate) fn from_code(code: i32) -> Error {
        Error::FfmpegError(crate::ffi::av_err_string(code))
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::FfmpegError(msg) => write!(f, "FFmpeg error: {msg}"),
            Error::DecoderNotFound => write!(f, "no decoder available for this input image"),
            Error::EncoderNotFound => write!(f, "no encoder available for this output format"),
            Error::InvalidInput => write!(f, "invalid or unreadable input image"),
            Error::UnsupportedFormat => write!(f, "unsupported image format"),
        }
    }
}

impl std::error::Error for Error {}
