use crate::audio::decoded::DecodedAudio;
use crate::audio::options::ConversionOptions;
use crate::error::Error;
use crate::ffi;

/// Entry point for decoding and encoding audio through FFmpeg.
///
/// The struct itself is a zero-sized handle; creating it lowers
/// FFmpeg's log verbosity to errors only. Encoder availability is
/// checked per [`convert`](AudioConverter::convert) call so a missing
/// encoder only fails conversions that target that specific format.
///
/// Instances may be reused for any number of
/// [`convert`](AudioConverter::convert) calls.
pub struct AudioConverter;

impl AudioConverter {
    /// Creates a converter and lowers FFmpeg's global log level to
    /// `AV_LOG_ERROR` on first call (never raised above it afterwards).
    pub fn new() -> Result<Self, Error> {
        unsafe {
            if ffi::av_log_get_level() > ffi::AV_LOG_ERROR as i32 {
                ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
            }
        }
        Ok(AudioConverter)
    }

    /// Converts an encoded audio stream into the requested format.
    ///
    /// `input` may be any audio format FFmpeg can decode (WAV, MP3,
    /// FLAC, AAC, Ogg, Opus/M4A, AIFF, …), probed from content — file
    /// extensions are irrelevant.
    ///
    /// Pipeline: decode → resample/downmix per `options` → encode.
    ///
    /// Returns [`Error::UnsupportedFormat`] for inputs without an audio
    /// stream, [`Error::InvalidInput`] for empty or undecodable input
    /// and [`Error::EncoderNotFound`] when the FFmpeg build lacks the
    /// target encoder.
    pub fn convert(&self, input: &[u8], options: ConversionOptions) -> Result<Vec<u8>, Error> {
        let decoded = DecodedAudio::decode(input)?;
        decoded.encode_inner(
            options.format,
            options.quality,
            options.sample_rate,
            options.channels,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::options::AudioFormat;

    #[test]
    fn empty_input_is_invalid() {
        let converter = AudioConverter::new().unwrap();
        let err = converter
            .convert(&[], ConversionOptions::default())
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn still_image_is_unsupported_format() {
        let png = [
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, b'I', b'H',
            b'D', b'R',
        ];
        let converter = AudioConverter::new().unwrap();
        let err = converter
            .convert(&png, ConversionOptions::default())
            .unwrap_err();
        assert_eq!(err, Error::UnsupportedFormat);
    }

    #[test]
    fn garbage_input_is_invalid() {
        let garbage = [0x00u8, 0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88];
        let converter = AudioConverter::new().unwrap();
        let err = converter
            .convert(
                &garbage,
                ConversionOptions {
                    format: AudioFormat::Wav,
                    quality: 80,
                    sample_rate: None,
                    channels: None,
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }
}
