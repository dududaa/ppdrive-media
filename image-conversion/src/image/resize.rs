use crate::error::Error;
use crate::ffi::wrappers::{Frame, scale_frame};
use crate::image::options::ConversionOptions;

pub(crate) fn target_dimensions(
    src_width: u32,
    src_height: u32,
    options: &ConversionOptions,
) -> Result<(u32, u32), Error> {
    if src_width == 0 || src_height == 0 {
        return Err(Error::InvalidInput);
    }
    match (options.width, options.height) {
        (None, None) => scaled_dimensions(src_width, src_height, options.scale),
        (Some(width), None) => {
            if width == 0 {
                return Err(Error::InvalidInput);
            }
            let height = (u64::from(src_height) * u64::from(width) / u64::from(src_width)).max(1);
            Ok((width, height as u32))
        }
        (None, Some(height)) => {
            if height == 0 {
                return Err(Error::InvalidInput);
            }
            let width = (u64::from(src_width) * u64::from(height) / u64::from(src_height)).max(1);
            Ok((width as u32, height))
        }
        (Some(width), Some(height)) => {
            if width == 0 || height == 0 {
                return Err(Error::InvalidInput);
            }
            Ok((width, height))
        }
    }
}

fn scaled_dimensions(
    src_width: u32,
    src_height: u32,
    scale: Option<f32>,
) -> Result<(u32, u32), Error> {
    let Some(scale) = scale else {
        return Ok((src_width, src_height));
    };
    if !scale.is_finite() || scale <= 0.0 {
        return Err(Error::InvalidInput);
    }
    let width = (f64::from(src_width) * f64::from(scale)).round();
    let height = (f64::from(src_height) * f64::from(scale)).round();
    if !width.is_finite() || !height.is_finite() {
        return Err(Error::InvalidInput);
    }
    if width > f64::from(u32::MAX) || height > f64::from(u32::MAX) {
        return Err(Error::InvalidInput);
    }
    Ok((width.max(1.0) as u32, height.max(1.0) as u32))
}

pub(crate) fn round_to_even(width: u32, height: u32, force_even: bool) -> (u32, u32) {
    if !force_even {
        return (width, height);
    }
    (width + width % 2, height + height % 2)
}

pub(crate) fn convert(
    frame: &Frame,
    width: u32,
    height: u32,
    pix_fmt: crate::ffi::AVPixelFormat,
) -> Result<Frame, Error> {
    scale_frame(frame, width, height, pix_fmt)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn opts(width: Option<u32>, height: Option<u32>) -> ConversionOptions {
        ConversionOptions {
            width,
            height,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_original_size_without_options() {
        assert_eq!(
            target_dimensions(160, 120, &opts(None, None)).unwrap(),
            (160, 120)
        );
    }

    #[test]
    fn width_only_preserves_aspect() {
        assert_eq!(
            target_dimensions(160, 120, &opts(Some(64), None)).unwrap(),
            (64, 48)
        );
    }

    #[test]
    fn height_only_preserves_aspect() {
        assert_eq!(
            target_dimensions(160, 120, &opts(None, Some(60))).unwrap(),
            (80, 60)
        );
    }

    #[test]
    fn both_dimensions_are_exact() {
        assert_eq!(
            target_dimensions(160, 120, &opts(Some(100), Some(50))).unwrap(),
            (100, 50)
        );
    }

    #[test]
    fn zero_dimensions_rejected() {
        assert!(target_dimensions(160, 120, &opts(Some(0), None)).is_err());
        assert!(target_dimensions(160, 120, &opts(None, Some(0))).is_err());
        assert!(target_dimensions(0, 120, &opts(None, None)).is_err());
    }

    fn opts_scale(scale: Option<f32>) -> ConversionOptions {
        ConversionOptions {
            scale,
            ..Default::default()
        }
    }

    #[test]
    fn scale_halves_dimensions() {
        assert_eq!(
            target_dimensions(160, 120, &opts_scale(Some(0.5))).unwrap(),
            (80, 60)
        );
    }

    #[test]
    fn scale_rounds_and_clamps_to_one_pixel() {
        assert_eq!(
            target_dimensions(161, 121, &opts_scale(Some(0.5))).unwrap(),
            (81, 61)
        );
        assert_eq!(
            target_dimensions(160, 120, &opts_scale(Some(0.001))).unwrap(),
            (1, 1)
        );
    }

    #[test]
    fn scale_can_upscale() {
        assert_eq!(
            target_dimensions(160, 120, &opts_scale(Some(2.0))).unwrap(),
            (320, 240)
        );
    }

    #[test]
    fn scale_ignored_when_explicit_dimensions_set() {
        assert_eq!(
            target_dimensions(
                160,
                120,
                &ConversionOptions {
                    width: Some(64),
                    scale: Some(0.5),
                    ..Default::default()
                }
            )
            .unwrap(),
            (64, 48)
        );
    }

    #[test]
    fn invalid_scale_rejected() {
        assert!(target_dimensions(160, 120, &opts_scale(Some(0.0))).is_err());
        assert!(target_dimensions(160, 120, &opts_scale(Some(-1.0))).is_err());
        assert!(target_dimensions(160, 120, &opts_scale(Some(f32::NAN))).is_err());
        assert!(target_dimensions(160, 120, &opts_scale(Some(f32::INFINITY))).is_err());
        assert!(target_dimensions(160, 120, &opts_scale(Some(1e20))).is_err());
    }

    #[test]
    fn rounding_to_even() {
        assert_eq!(round_to_even(63, 47, true), (64, 48));
        assert_eq!(round_to_even(64, 48, true), (64, 48));
        assert_eq!(round_to_even(63, 47, false), (63, 47));
    }
}
