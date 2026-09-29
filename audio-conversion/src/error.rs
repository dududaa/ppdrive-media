use std::fmt;

/// Errors produced by the audio conversion pipeline.
///
/// Implements [`std::fmt::Display`] (human-readable message) and
/// [`std::error::Error`], so it works with `?` into any error sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An FFmpeg call failed; the payload is the formatted
    /// `av_strerror` message (e.g. `"Invalid data found when processing
    /// input"`). Also returned for unreadable input containers.
    FfmpegError(String),
    /// No decoder for the input audio is available in the linked FFmpeg.
    DecoderNotFound,
    /// No encoder for the requested output format is available —
    /// typically `libmp3lame`, `libvorbis` or `libopus` missing from the
    /// FFmpeg build.
    EncoderNotFound,
    /// Empty input, a zero sample rate/channel count, or an option the
    /// pipeline cannot honour (e.g. `channels: 0`).
    InvalidInput,
    /// The container decoded fine but holds no audio stream (e.g. a
    /// video-only or still-image file passed to [`crate::AudioConverter`]).
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
            Error::DecoderNotFound => write!(f, "no decoder available for this input audio"),
            Error::EncoderNotFound => {
                write!(f, "no encoder available for this output audio format")
            }
            Error::InvalidInput => write!(f, "invalid or unreadable input audio"),
            Error::UnsupportedFormat => write!(f, "input contains no audio stream"),
        }
    }
}

impl std::error::Error for Error {}
