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

/// Resolves the output dimensions against the input dimensions:
/// both options absent → input size, one present → aspect-preserving
/// scale, both present → exact size.
pub(crate) fn target_dimensions(
    input_width: u32,
    input_height: u32,
    options: &ConversionOptions,
) -> Result<(u32, u32), Error> {
    if input_width == 0 || input_height == 0 {
        return Err(Error::InvalidInput);
    }
    match (options.width, options.height) {
        (None, None) => Ok((input_width, input_height)),
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
}
