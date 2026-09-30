use std::fmt;

/// Errors produced by the streaming pipeline.
///
/// Implements [`std::fmt::Display`] (human-readable message) and
/// [`std::error::Error`], so it works with `?` into any error sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// An FFmpeg call failed; the payload is the formatted
    /// `av_strerror` message (e.g. `"Invalid data found when processing
    /// input"`). Also returned for unreadable input containers.
    FfmpegError(String),
    /// Empty input, a zero width/height rendition, a bad scale, or a
    /// segment duration the pipeline cannot honour.
    InvalidInput,
    /// The input holds neither a video nor an audio stream, or the
    /// linked FFmpeg build lacks the requested muxer/encoder.
    UnsupportedFormat,
    /// No encoder for the requested stream codec is available in the
    /// linked FFmpeg build.
    EncoderNotFound,
    /// A filesystem operation failed (reading the input file or
    /// creating/writing the output directory).
    Io(String),
}

impl Error {
    pub(crate) fn from_code(code: i32) -> Error {
        Error::FfmpegError(crate::ffi::av_err_string(code))
    }

    pub(crate) fn io(err: std::io::Error) -> Error {
        Error::Io(err.to_string())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::FfmpegError(msg) => write!(f, "FFmpeg error: {msg}"),
            Error::InvalidInput => write!(f, "invalid or unreadable input media"),
            Error::UnsupportedFormat => {
                write!(f, "input contains no stream this pipeline can package")
            }
            Error::EncoderNotFound => {
                write!(f, "no encoder available for this output stream codec")
            }
            Error::Io(msg) => write!(f, "I/O error: {msg}"),
        }
    }
}

impl std::error::Error for Error {}
