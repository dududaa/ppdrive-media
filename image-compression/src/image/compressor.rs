use crate::error::Error;
use crate::ffi;
use crate::image::options::CompressionOptions;
use crate::image::{decode, encode, resize};

/// Entry point for decoding, resizing and encoding images through FFmpeg.
///
/// The struct itself is a zero-sized handle; creating it verifies that all
/// four encoders (JPEG, PNG, WebP, AVIF) are available in the linked
/// FFmpeg libraries and lowers FFmpeg's log verbosity to errors only.
///
/// Instances are `Send + Sync` in spirit (no shared state) and may be
/// reused for any number of [`compress`](ImageCompressor::compress) calls.
pub struct ImageCompressor;

impl ImageCompressor {
    /// Creates a compressor after checking that JPEG, PNG, WebP and AVIF
    /// encoders are present in the linked FFmpeg build.
    ///
    /// Returns [`crate::Error::EncoderNotFound`] if any of them is missing,
    /// which can happen when FFmpeg was configured without
    /// `--enable-libaom` (AVIF) or `--enable-libwebp` (WebP).
    ///
    /// FFmpeg's global log level is set to `AV_LOG_ERROR` on first call
    /// and never raised above it.
    pub fn new() -> Result<Self, Error> {
        unsafe {
            if ffi::av_log_get_level() > ffi::AV_LOG_ERROR as i32 {
                ffi::av_log_set_level(ffi::AV_LOG_ERROR as i32);
            }
        }
        for id in [
            ffi::AV_CODEC_ID_MJPEG,
            ffi::AV_CODEC_ID_PNG,
            ffi::AV_CODEC_ID_WEBP,
            ffi::AV_CODEC_ID_AV1,
        ] {
            ffi::wrappers::find_encoder_by_id(id)?;
        }
        Ok(ImageCompressor)
    }

    /// Compresses an encoded image into the requested format.
    ///
    /// `input` may be any still image format FFmpeg can decode
    /// (JPEG, PNG, WebP, GIF, BMP, TIFF, …), probed from content — file
    /// extensions are irrelevant. AVIF *input* is rejected with
    /// [`Error::UnsupportedFormat`] because FFmpeg ships no AVIF demuxer.
    ///
    /// Pipeline: decode → resize/convert per `options` → encode. When no
    /// resize or pixel-format change is needed the decoded frame is passed
    /// through untouched.
    ///
    /// Returns [`Error::InvalidInput`] if `options.width` or
    /// `options.height` is `Some(0)`, or if the input is empty or
    /// undecodable.
    pub fn compress(&self, input: &[u8], options: CompressionOptions) -> Result<Vec<u8>, Error> {
        if options.width == Some(0) || options.height == Some(0) {
            return Err(Error::InvalidInput);
        }

        let frame = decode::decode(input)?;
        let spec = encode::spec_for(options.format);

        let (width, height) = resize::target_dimensions(frame.width(), frame.height(), &options)?;
        let (width, height) = resize::round_to_even(width, height, spec.force_even);

        let dst_fmt = spec.pix_fmt(frame.has_alpha());
        let frame = if frame.width() == width
            && frame.height() == height
            && frame.format() == dst_fmt as i32
        {
            frame
        } else {
            resize::convert(frame, width, height, dst_fmt)?
        };

        encode::encode(&frame, &options, &spec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image::options::ImageFormat;

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn webp_roundtrip_preserves_alpha() {
        let input = fixture("input_alpha.png");
        let compressor = ImageCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    format: ImageFormat::WebP,
                    quality: 80,
                    width: None,
                    height: None,
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert!(frame.has_alpha());
        assert_eq!(frame.width(), 64);
        assert_eq!(frame.height(), 64);
    }

    #[test]
    fn png_roundtrip_preserves_alpha() {
        let input = fixture("input_alpha.png");
        let compressor = ImageCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    format: ImageFormat::Png,
                    quality: 80,
                    width: None,
                    height: None,
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert!(frame.has_alpha());
    }
}
