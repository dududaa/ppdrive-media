use crate::error::Error;
use crate::ffi;
use crate::ffi::encode::{EncodeConfig, Encoder};
use crate::ffi::wrappers::{Frame, find_encoder_by_id, find_encoder_by_name};
use crate::image::options::{ConversionOptions, ImageFormat};
use crate::image::resize;

pub(crate) struct EncodeSpec {
    pub muxer: &'static str,
    pub filename: &'static str,
    pub codec_id: ffi::AVCodecID,
    pub codec_name: Option<&'static str>,
    pub force_even: bool,
    pub allow_alpha: bool,
}

pub(crate) fn spec_for(format: ImageFormat) -> EncodeSpec {
    match format {
        ImageFormat::Jpeg => EncodeSpec {
            muxer: "image2pipe",
            filename: "out.jpg",
            codec_id: ffi::AV_CODEC_ID_MJPEG,
            codec_name: None,
            force_even: true,
            allow_alpha: false,
        },
        ImageFormat::Png => EncodeSpec {
            muxer: "image2pipe",
            filename: "out.png",
            codec_id: ffi::AV_CODEC_ID_PNG,
            codec_name: None,
            force_even: false,
            allow_alpha: true,
        },
        ImageFormat::WebP => EncodeSpec {
            muxer: "webp",
            filename: "out.webp",
            codec_id: ffi::AV_CODEC_ID_WEBP,
            codec_name: None,
            force_even: false,
            allow_alpha: true,
        },
        ImageFormat::Avif => EncodeSpec {
            muxer: "avif",
            filename: "out.avif",
            codec_id: ffi::AV_CODEC_ID_AV1,
            codec_name: Some("libaom-av1"),
            force_even: true,
            allow_alpha: false,
        },
    }
}

impl EncodeSpec {
    pub fn pix_fmt(&self, has_alpha: bool) -> ffi::AVPixelFormat {
        match self.codec_id {
            ffi::AV_CODEC_ID_MJPEG => ffi::AV_PIX_FMT_YUVJ420P,
            ffi::AV_CODEC_ID_PNG => {
                if self.allow_alpha && has_alpha {
                    ffi::AV_PIX_FMT_RGBA
                } else {
                    ffi::AV_PIX_FMT_RGB24
                }
            }
            ffi::AV_CODEC_ID_WEBP => ffi::AV_PIX_FMT_BGRA,
            _ => ffi::AV_PIX_FMT_YUV420P,
        }
    }
}

fn quality_settings(
    format: ImageFormat,
    quality: u8,
    effort: Option<u8>,
) -> (Option<i32>, Vec<(String, String)>) {
    let quality = i32::from(quality.min(100));
    match format {
        ImageFormat::Jpeg => (Some(31 - quality * 29 / 100), Vec::new()),
        ImageFormat::Png => {
            let level = quality * 9 / 100;
            (
                None,
                vec![("compression_level".to_string(), level.to_string())],
            )
        }
        ImageFormat::WebP => {
            let mut options = vec![("quality".to_string(), quality.to_string())];
            if quality == 100 {
                options.push(("lossless".to_string(), "1".to_string()));
            }
            (None, options)
        }
        ImageFormat::Avif => {
            let crf = 63 - quality * 63 / 100;
            let mut options = vec![
                ("crf".to_string(), crf.to_string()),
                ("row-mt".to_string(), "1".to_string()),
            ];
            if let Some(effort) = effort {
                let effort = i32::from(effort.min(100));
                options.push(("cpu-used".to_string(), (8 - effort * 8 / 100).to_string()));
            }
            (None, options)
        }
    }
}

pub(crate) fn encode(
    frame: &Frame,
    options: &ConversionOptions,
    spec: &EncodeSpec,
) -> Result<Vec<u8>, Error> {
    let codec = match spec.codec_name {
        Some(name) => find_encoder_by_name(name).or_else(|_| find_encoder_by_id(spec.codec_id))?,
        None => find_encoder_by_id(spec.codec_id)?,
    };

    let (qscale, dict_options) = quality_settings(options.format, options.quality, options.effort);

    let config = EncodeConfig {
        codec,
        muxer: spec.muxer.to_string(),
        filename: spec.filename.to_string(),
        width: frame.width(),
        height: frame.height(),
        pix_fmt: spec.pix_fmt(frame.has_alpha()),
        options: dict_options,
        qscale,
    };

    let mut encoder = Encoder::new(config)?;
    encoder.encode_frame(frame)?;
    encoder.finish()
}

/// Rounds dimensions for `spec` and converts the frame's pixel format
/// only when needed. Returns `None` when the frame already has the
/// target dimensions and pixel format (pass-through).
pub(crate) fn prepare(
    frame: &Frame,
    spec: &EncodeSpec,
    width: u32,
    height: u32,
) -> Result<Option<Frame>, Error> {
    let (width, height) = resize::round_to_even(width, height, spec.force_even);
    let dst_fmt = spec.pix_fmt(frame.has_alpha());
    if frame.width() == width && frame.height() == height && frame.format() == dst_fmt {
        Ok(None)
    } else {
        Ok(Some(resize::convert(frame, width, height, dst_fmt)?))
    }
}

/// Prepares a decoded frame for `options.format` (even-dimension
/// rounding for chroma-subsampled targets + pixel-format conversion)
/// and encodes it.
pub(crate) fn encode_prepared(
    frame: &crate::ffi::wrappers::Frame,
    options: &ConversionOptions,
    width: u32,
    height: u32,
) -> Result<Vec<u8>, Error> {
    let spec = spec_for(options.format);
    let converted = prepare(frame, &spec, width, height)?;
    encode(converted.as_ref().unwrap_or(frame), options, &spec)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jpeg_quality_maps_to_qscale_range() {
        assert_eq!(quality_settings(ImageFormat::Jpeg, 100, None).0, Some(2));
        assert_eq!(quality_settings(ImageFormat::Jpeg, 0, None).0, Some(31));
        assert_eq!(quality_settings(ImageFormat::Jpeg, 255, None).0, Some(2));
    }

    #[test]
    fn avif_quality_maps_to_crf_range() {
        let high = quality_settings(ImageFormat::Avif, 100, None);
        let low = quality_settings(ImageFormat::Avif, 0, None);
        assert!(high.1.iter().any(|(k, v)| k == "crf" && v == "0"));
        assert!(low.1.iter().any(|(k, v)| k == "crf" && v == "63"));
    }

    #[test]
    fn png_quality_maps_to_compression_level() {
        let level = quality_settings(ImageFormat::Png, 100, None).1[0].clone();
        assert_eq!(level, ("compression_level".to_string(), "9".to_string()));
    }

    #[test]
    fn webp_lossless_at_max_quality() {
        assert!(
            quality_settings(ImageFormat::WebP, 100, None)
                .1
                .iter()
                .any(|(k, v)| k == "lossless" && v == "1")
        );
        assert!(
            !quality_settings(ImageFormat::WebP, 99, None)
                .1
                .iter()
                .any(|(k, _)| k == "lossless")
        );
    }

    #[test]
    fn avif_effort_maps_to_cpu_used_range() {
        let fastest = quality_settings(ImageFormat::Avif, 80, Some(0));
        let slowest = quality_settings(ImageFormat::Avif, 80, Some(100));
        let clamped = quality_settings(ImageFormat::Avif, 80, Some(255));
        let unset = quality_settings(ImageFormat::Avif, 80, None);
        assert!(fastest.1.iter().any(|(k, v)| k == "cpu-used" && v == "8"));
        assert!(slowest.1.iter().any(|(k, v)| k == "cpu-used" && v == "0"));
        assert!(clamped.1.iter().any(|(k, v)| k == "cpu-used" && v == "0"));
        assert!(!unset.1.iter().any(|(k, _)| k == "cpu-used"));
    }

    #[test]
    fn effort_ignored_for_formats_without_a_knob() {
        for format in [ImageFormat::Jpeg, ImageFormat::Png, ImageFormat::WebP] {
            assert_eq!(
                quality_settings(format, 80, Some(100)),
                quality_settings(format, 80, None)
            );
        }
    }

    #[test]
    fn spec_metadata_is_consistent() {
        let jpeg = spec_for(ImageFormat::Jpeg);
        assert_eq!(jpeg.muxer, "image2pipe");
        assert!(jpeg.force_even);
        assert!(!jpeg.allow_alpha);

        let webp = spec_for(ImageFormat::WebP);
        assert_eq!(webp.muxer, "webp");
        assert!(!webp.force_even);
        assert!(webp.allow_alpha);
    }
}
