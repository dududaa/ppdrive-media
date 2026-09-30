use crate::error::Error;
use crate::video::options::ConversionOptions;

/// Rounds both dimensions up to the nearest even value (no-op when a
/// dimension is already even). Zero is passed through untouched so the
/// caller's emptiness checks stay the single source of truth.
pub(crate) fn round_to_even(width: u32, height: u32, force_even: bool) -> (u32, u32) {
    if !force_even {
        return (width, height);
    }
    (width + width % 2, height + height % 2)
}

/// Applies the optional proportional `scale` factor to the source
/// dimensions: `round(source × scale)`, minimum 1 px per axis.
/// Non-finite, zero or negative factors, or results beyond `u32`, are
/// rejected with [`Error::InvalidInput`].
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

/// Resolves the output dimensions against the input dimensions:
/// both options absent → `scale` (or the input size), one present →
/// aspect-preserving scale, both present → exact size. An explicit
/// `width`/`height` wins over `scale`.
pub(crate) fn target_dimensions(
    input_width: u32,
    input_height: u32,
    options: &ConversionOptions,
) -> Result<(u32, u32), Error> {
    if input_width == 0 || input_height == 0 {
        return Err(Error::InvalidInput);
    }
    match (options.width, options.height) {
        (None, None) => scaled_dimensions(input_width, input_height, options.scale),
        (Some(w), None) => {
            if w == 0 {
                return Err(Error::InvalidInput);
            }
            let h = ((u64::from(input_height) * u64::from(w)) / u64::from(input_width)) as u32;
            Ok((w, h.max(1)))
        }
        (None, Some(h)) => {
            if h == 0 {
                return Err(Error::InvalidInput);
            }
            let w = ((u64::from(input_width) * u64::from(h)) / u64::from(input_height)) as u32;
            Ok((w.max(1), h))
        }
        (Some(w), Some(h)) => {
            if w == 0 || h == 0 {
                return Err(Error::InvalidInput);
            }
            Ok((w, h))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_to_even_bumps_up() {
        assert_eq!(round_to_even(63, 47, true), (64, 48));
        assert_eq!(round_to_even(64, 48, true), (64, 48));
        assert_eq!(round_to_even(63, 47, false), (63, 47));
    }

    #[test]
    fn width_only_preserves_aspect_ratio() {
        let options = ConversionOptions {
            width: Some(160),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (160, 90));
    }

    #[test]
    fn both_present_is_exact() {
        let options = ConversionOptions {
            width: Some(100),
            height: Some(100),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (100, 100));
    }

    #[test]
    fn zero_options_rejected() {
        let options = ConversionOptions {
            width: Some(0),
            ..ConversionOptions::default()
        };
        assert_eq!(
            target_dimensions(320, 180, &options).unwrap_err(),
            Error::InvalidInput
        );
    }

    #[test]
    fn scale_applies_when_dimensions_absent() {
        let options = ConversionOptions {
            scale: Some(0.5),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (160, 90));
    }

    #[test]
    fn scale_rounds_and_keeps_at_least_one_pixel() {
        let options = ConversionOptions {
            scale: Some(0.04),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (13, 7));

        let options = ConversionOptions {
            scale: Some(0.001),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (1, 1));
    }

    #[test]
    fn explicit_dimensions_win_over_scale() {
        let options = ConversionOptions {
            width: Some(160),
            scale: Some(0.25),
            ..ConversionOptions::default()
        };
        assert_eq!(target_dimensions(320, 180, &options).unwrap(), (160, 90));
    }

    #[test]
    fn invalid_scale_is_rejected() {
        for scale in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let options = ConversionOptions {
                scale: Some(scale),
                ..ConversionOptions::default()
            };
            assert_eq!(
                target_dimensions(320, 180, &options).unwrap_err(),
                Error::InvalidInput,
                "scale {scale}"
            );
        }
    }
}
