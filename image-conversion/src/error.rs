use std::fmt;

/// Errors produced by the conversion pipeline.
///
/// Implements [`std::fmt::Display`] (human-readable message) and
/// [`std::error::Error`], so it works with `?` into any error sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An FFmpeg call failed; the payload is the formatted
    /// `av_strerror` message (e.g. `"Invalid data found when processing
    /// input"`). Also returned for unreadable input containers.
    FfmpegError(String),
    /// No decoder for the input image is available in the linked FFmpeg.
    DecoderNotFound,
    /// No encoder for the requested output format is available —
    /// typically AVIF without `--enable-libaom` or WebP without
    /// `--enable-libwebp` in the FFmpeg build.
    EncoderNotFound,
    /// Empty input, a zero width/height option, or a dimension the
    /// pipeline cannot honour.
    InvalidInput,
    /// The format is recognized but not supported by this build/path —
    /// notably AVIF *input* (FFmpeg has no AVIF demuxer) and unknown
    /// output extensions.
    UnsupportedFormat,
    /// `ConversionOptions::max_bytes` was set, but even the smallest
    /// output (quality 0) exceeds the requested budget.
    TargetSizeUnreachable,
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
            Error::TargetSizeUnreachable => {
                write!(f, "output cannot fit the requested maximum size")
            }
        }
    }
}

impl std::error::Error for Error {}
