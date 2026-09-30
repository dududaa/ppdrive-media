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
    /// No decoder for the input video is available in the linked FFmpeg.
    DecoderNotFound,
    /// No encoder for the requested output format is available —
    /// typically libx264 (MP4/H.264) or libvpx-vp9 (WebM/VP9) missing
    /// from the FFmpeg build.
    EncoderNotFound,
    /// Empty input, a zero width/height option, or a dimension the
    /// pipeline cannot honour.
    InvalidInput,
    /// The container decoded fine but holds no video stream.
    UnsupportedFormat,
    /// The request would exceed a safety limit; the payload names the
    /// limit and how to stay under it (e.g. the `Reverse` decoded-frame
    /// buffering estimate over 1 GiB).
    LimitExceeded(String),
    /// `ConversionOptions::max_bytes` cannot be satisfied even at
    /// quality 0.
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
            Error::DecoderNotFound => write!(f, "no decoder available for this input video"),
            Error::EncoderNotFound => {
                write!(f, "no encoder available for this output video format")
            }
            Error::InvalidInput => write!(f, "invalid or unreadable input video"),
            Error::UnsupportedFormat => write!(f, "input contains no video stream"),
            Error::LimitExceeded(msg) => write!(f, "{msg}"),
            Error::TargetSizeUnreachable => {
                write!(f, "output cannot fit the requested maximum size")
            }
        }
    }
}

impl std::error::Error for Error {}
