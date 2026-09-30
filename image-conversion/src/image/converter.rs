use crate::error::Error;
use crate::ffi;
use crate::image::options::ConversionOptions;
use crate::image::{decode, encode, resize};

/// Entry point for decoding, resizing and encoding images through FFmpeg.
///
/// The struct itself is a zero-sized handle; creating it verifies that all
/// four encoders (JPEG, PNG, WebP, AVIF) are available in the linked
/// FFmpeg libraries and lowers FFmpeg's log verbosity to errors only.
///
/// Instances are `Send + Sync` in spirit (no shared state) and may be
/// reused for any number of [`convert`](ImageConverter::convert) calls.
pub struct ImageConverter;

impl ImageConverter {
    /// Creates a converter after checking that JPEG, PNG, WebP and AVIF
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
        Ok(ImageConverter)
    }

    /// Converts an encoded image into the requested format.
    ///
    /// `input` may be any still image format FFmpeg can decode
    /// (JPEG, PNG, WebP, GIF, BMP, TIFF, …), probed from content — file
    /// extensions are irrelevant. AVIF *input* is rejected with
    /// [`Error::UnsupportedFormat`] because FFmpeg ships no AVIF demuxer.
    ///
    /// Pipeline: decode → resize/convert per `options` → encode. When no
    /// resize or pixel-format change is needed the decoded frame is passed
    /// through untouched. With [`ConversionOptions::max_bytes`] set, the
    /// encode step is retried at lower qualities until the output fits
    /// (the frame is resized only once, before the search).
    ///
    /// Returns [`Error::InvalidInput`] if `options.width` or
    /// `options.height` is `Some(0)`, `options.max_bytes` is `Some(0)`,
    /// `options.scale` is non-finite or non-positive, or the input is
    /// empty or undecodable. Returns [`Error::TargetSizeUnreachable`]
    /// when a size budget cannot be met even at quality 0.
    pub fn convert(&self, input: &[u8], options: ConversionOptions) -> Result<Vec<u8>, Error> {
        validate(&options)?;

        let decoded = decode::decode(input)?;
        let (width, height) =
            resize::target_dimensions(decoded.width(), decoded.height(), &options)?;
        let spec = encode::spec_for(options.format);
        let converted = encode::prepare(decoded.raw(), &spec, width, height)?;
        let prepared = converted.as_ref().unwrap_or_else(|| decoded.raw());
        match options.max_bytes {
            None => encode::encode(prepared, &options, &spec),
            Some(max_bytes) => encode_within(prepared, &options, &spec, max_bytes),
        }
    }
}

fn validate(options: &ConversionOptions) -> Result<(), Error> {
    if options.width == Some(0) || options.height == Some(0) {
        return Err(Error::InvalidInput);
    }
    if options.max_bytes == Some(0) {
        return Err(Error::InvalidInput);
    }
    if let Some(scale) = options.scale
        && (!scale.is_finite() || scale <= 0.0)
    {
        return Err(Error::InvalidInput);
    }
    Ok(())
}

/// Binary search over `quality` for the highest quality whose encoded
/// output fits `max_bytes`. The frame must already be prepared; only
/// the encode step repeats (at most eight times for the 0–100 range).
fn encode_within(
    frame: &crate::ffi::wrappers::Frame,
    options: &ConversionOptions,
    spec: &encode::EncodeSpec,
    max_bytes: u64,
) -> Result<Vec<u8>, Error> {
    let top = options.quality.min(100);
    let mut attempt = options.clone();
    attempt.quality = top;
    let first = encode::encode(frame, &attempt, spec)?;
    if first.len() as u64 <= max_bytes {
        return Ok(first);
    }
    if top == 0 {
        return Err(Error::TargetSizeUnreachable);
    }

    let mut low = 0u8;
    let mut high = top - 1;
    let mut best: Option<Vec<u8>> = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        attempt.quality = mid;
        let out = encode::encode(frame, &attempt, spec)?;
        if out.len() as u64 <= max_bytes {
            best = Some(out);
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    best.ok_or(Error::TargetSizeUnreachable)
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
        let converter = ImageConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::WebP,
                    quality: 80,
                    ..Default::default()
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
        let converter = ImageConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Png,
                    quality: 80,
                    ..Default::default()
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert!(frame.has_alpha());
    }

    #[test]
    fn scale_halves_output_dimensions() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Png,
                    scale: Some(0.5),
                    ..Default::default()
                },
            )
            .unwrap();
        let frame = decode::decode(&output).unwrap();
        assert_eq!((frame.width(), frame.height()), (80, 60));
    }

    #[test]
    fn invalid_scale_is_rejected() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        for scale in [0.0, -0.5, f32::NAN, f32::INFINITY, 1e20] {
            let err = converter
                .convert(
                    &input,
                    ConversionOptions {
                        scale: Some(scale),
                        ..Default::default()
                    },
                )
                .unwrap_err();
            assert_eq!(err, Error::InvalidInput, "scale {scale}");
        }
    }

    #[test]
    fn zero_max_bytes_is_invalid() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        let err = converter
            .convert(
                &input,
                ConversionOptions {
                    max_bytes: Some(0),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn max_bytes_budget_is_honored() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        let full = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Jpeg,
                    quality: 95,
                    ..Default::default()
                },
            )
            .unwrap();
        let budget = full.len() * 2 / 3;
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Jpeg,
                    quality: 95,
                    max_bytes: Some(budget as u64),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(output.len() <= budget, "{} > {}", output.len(), budget);
        assert_eq!(output[0], 0xFF);
        assert_eq!(output[1], 0xD8);
    }

    #[test]
    fn impossible_budget_reports_target_size_unreachable() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        let err = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::Jpeg,
                    quality: 95,
                    max_bytes: Some(1),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::TargetSizeUnreachable);
    }

    #[test]
    fn budget_already_met_returns_immediately() {
        let input = fixture("input.png");
        let converter = ImageConverter::new().unwrap();
        let full = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::WebP,
                    quality: 50,
                    ..Default::default()
                },
            )
            .unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: ImageFormat::WebP,
                    quality: 50,
                    max_bytes: Some(full.len() as u64),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(output.len(), full.len());
    }
}
