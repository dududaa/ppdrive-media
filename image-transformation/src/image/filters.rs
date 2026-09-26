use super::options::TransformOperation;
use image_compression::Error;

/// Builds the libavfilter chain for one call: typed operations in
/// order, then the optional custom filter string.
///
/// `has_alpha` selects `format=rgba` vs `format=rgb24` for the
/// [`TransformOperation::Adjust`] stage (lutrgb needs RGB input and
/// forcing it must not strip an existing alpha channel).
///
/// An empty result becomes `null` (identity) so the graph still has a
/// linkable chain.
pub(crate) fn build_chain(
    src_width: u32,
    src_height: u32,
    has_alpha: bool,
    operations: &[TransformOperation],
    custom_filters: Option<&str>,
) -> Result<String, Error> {
    let mut parts: Vec<String> = Vec::new();

    for op in operations {
        if let Some(filter) = op.to_filter(src_width, src_height, has_alpha)? {
            parts.push(filter);
        }
    }

    if let Some(custom) = custom_filters {
        if custom.contains('\0') {
            return Err(Error::InvalidInput);
        }
        let custom = custom.trim();
        if !custom.is_empty() {
            parts.push(custom.to_string());
        }
    }

    if parts.is_empty() {
        parts.push("null".to_string());
    }

    Ok(parts.join(","))
}

impl TransformOperation {
    /// Maps one operation to its filter-string fragment, validating
    /// all arguments against the source dimensions first.
    fn to_filter(
        &self,
        src_width: u32,
        src_height: u32,
        has_alpha: bool,
    ) -> Result<Option<String>, Error> {
        Ok(match self {
            TransformOperation::Crop {
                x,
                y,
                width,
                height,
            } => {
                if *width == 0 || *height == 0 {
                    return Err(Error::InvalidInput);
                }
                let x_end = u64::from(*x) + u64::from(*width);
                let y_end = u64::from(*y) + u64::from(*height);
                if x_end > u64::from(src_width) || y_end > u64::from(src_height) {
                    return Err(Error::InvalidInput);
                }
                Some(format!("crop={width}:{height}:{x}:{y}"))
            }
            TransformOperation::Rotate { degrees } => match degrees {
                90 => Some("transpose=1".to_string()),
                180 => Some("transpose=1,transpose=1".to_string()),
                270 => Some("transpose=2".to_string()),
                _ => return Err(Error::InvalidInput),
            },
            TransformOperation::Flip {
                horizontal,
                vertical,
            } => {
                if !*horizontal && !*vertical {
                    return Err(Error::InvalidInput);
                }
                let mut parts: Vec<&str> = Vec::new();
                if *horizontal {
                    parts.push("hflip");
                }
                if *vertical {
                    parts.push("vflip");
                }
                Some(parts.join(","))
            }
            TransformOperation::Pad {
                left,
                top,
                right,
                bottom,
                color,
            } => {
                if *left == 0 && *top == 0 && *right == 0 && *bottom == 0 {
                    None
                } else if color.is_empty() || color.contains('\0') {
                    return Err(Error::InvalidInput);
                } else {
                    Some(format!(
                        "pad=iw+{left}+{right}:ih+{top}+{bottom}:{left}:{top}:color={color}"
                    ))
                }
            }
            TransformOperation::Grayscale => Some("hue=s=0".to_string()),
            TransformOperation::Adjust {
                brightness,
                contrast,
                saturation,
            } => {
                for value in [brightness, contrast, saturation] {
                    if !value.is_finite() {
                        return Err(Error::InvalidInput);
                    }
                }
                let brightness = brightness.clamp(-1.0, 1.0);
                let contrast = contrast.clamp(-1000.0, 1000.0);
                let saturation = saturation.clamp(0.0, 3.0);
                // (val - 127.5) * contrast + 127.5 + brightness * 255,
                // clipped to 0..=255 — same linear map as FFmpeg's
                // `eq` brightness/contrast, but expressed per RGB
                // channel so it also works in non-GPL builds where
                // the GPL-only `eq` filter is absent.
                let offset = brightness * 255.0;
                let term = if offset > 0.0 {
                    format!("+{offset}")
                } else if offset < 0.0 {
                    format!("-{}", -offset)
                } else {
                    String::new()
                };
                let expr = format!("clip((val-127.5)*{contrast}+127.5{term},0,255)");
                let pixel_fmt = if has_alpha { "rgba" } else { "rgb24" };
                Some(format!(
                    "format={pixel_fmt},lutrgb=r='{expr}':g='{expr}':b='{expr}',hue=s={saturation}"
                ))
            }
            TransformOperation::Blur { sigma } => {
                if !sigma.is_finite() || *sigma <= 0.0 {
                    return Err(Error::InvalidInput);
                }
                Some(format!("gblur=sigma={sigma}"))
            }
            TransformOperation::Sharpen { amount } => {
                if !amount.is_finite() {
                    return Err(Error::InvalidInput);
                }
                let amount = amount.clamp(-2.0, 5.0);
                Some(format!("unsharp=5:5:{amount}:5:5:{amount}"))
            }
            TransformOperation::Scale { width, height } => {
                if *width == 0 || *height == 0 {
                    return Err(Error::InvalidInput);
                }
                Some(format!("scale={width}:{height}"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const W: u32 = 160;
    const H: u32 = 120;

    fn chain(ops: &[TransformOperation]) -> Result<String, Error> {
        build_chain(W, H, false, ops, None)
    }

    fn chain_alpha(ops: &[TransformOperation]) -> Result<String, Error> {
        build_chain(W, H, true, ops, None)
    }

    #[test]
    fn empty_chain_is_identity() {
        assert_eq!(chain(&[]).unwrap(), "null");
        assert_eq!(build_chain(W, H, false, &[], Some("  ")).unwrap(), "null");
    }

    #[test]
    fn crop_maps_and_validates() {
        let op = TransformOperation::Crop {
            x: 10,
            y: 20,
            width: 80,
            height: 60,
        };
        assert_eq!(chain(&[op]).unwrap(), "crop=80:60:10:20");
    }

    #[test]
    fn crop_out_of_bounds_rejected() {
        let op = TransformOperation::Crop {
            x: 100,
            y: 0,
            width: 80,
            height: 10,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        let op = TransformOperation::Crop {
            x: 0,
            y: 0,
            width: 0,
            height: 10,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn rotate_maps() {
        for (degrees, expected) in [
            (90, "transpose=1"),
            (180, "transpose=1,transpose=1"),
            (270, "transpose=2"),
        ] {
            let op = TransformOperation::Rotate { degrees };
            assert_eq!(chain(&[op]).unwrap(), expected);
        }
        let op = TransformOperation::Rotate { degrees: 45 };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn flip_maps() {
        let op = TransformOperation::Flip {
            horizontal: true,
            vertical: false,
        };
        assert_eq!(chain(&[op]).unwrap(), "hflip");
        let op = TransformOperation::Flip {
            horizontal: true,
            vertical: true,
        };
        assert_eq!(chain(&[op]).unwrap(), "hflip,vflip");
        let op = TransformOperation::Flip {
            horizontal: false,
            vertical: false,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn pad_maps_and_skips_when_empty() {
        let op = TransformOperation::Pad {
            left: 10,
            top: 20,
            right: 30,
            bottom: 40,
            color: "#ff0000".to_string(),
        };
        assert_eq!(
            chain(&[op]).unwrap(),
            "pad=iw+10+30:ih+20+40:10:20:color=#ff0000"
        );
        let op = TransformOperation::Pad {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
            color: "black".to_string(),
        };
        assert_eq!(chain(&[op]).unwrap(), "null");
        let op = TransformOperation::Pad {
            left: 1,
            top: 0,
            right: 0,
            bottom: 0,
            color: String::new(),
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn adjust_clamps_and_rejects_nan() {
        let op = TransformOperation::Adjust {
            brightness: 5.0,
            contrast: 0.5,
            saturation: 0.5,
        };
        let expr = "clip((val-127.5)*0.5+127.5+255,0,255)";
        assert_eq!(
            chain(&[op]).unwrap(),
            format!("format=rgb24,lutrgb=r='{expr}':g='{expr}':b='{expr}',hue=s=0.5")
        );
        let op = TransformOperation::Adjust {
            brightness: -0.2,
            contrast: 1.2,
            saturation: 1.0,
        };
        let expr = "clip((val-127.5)*1.2+127.5-51,0,255)";
        assert_eq!(
            chain(&[op]).unwrap(),
            format!("format=rgb24,lutrgb=r='{expr}':g='{expr}':b='{expr}',hue=s=1")
        );
        let op = TransformOperation::Adjust {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        };
        let expr = "clip((val-127.5)*1+127.5,0,255)";
        assert_eq!(
            chain_alpha(&[op]).unwrap(),
            format!("format=rgba,lutrgb=r='{expr}':g='{expr}':b='{expr}',hue=s=1")
        );
        let op = TransformOperation::Adjust {
            brightness: f32::NAN,
            contrast: 1.0,
            saturation: 1.0,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn blur_and_sharpen_validate() {
        let op = TransformOperation::Blur { sigma: 2.5 };
        assert_eq!(chain(&[op]).unwrap(), "gblur=sigma=2.5");
        for sigma in [0.0, -1.0, f32::NAN, f32::INFINITY] {
            let op = TransformOperation::Blur { sigma };
            assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        }
        let op = TransformOperation::Sharpen { amount: 0.8 };
        assert_eq!(chain(&[op]).unwrap(), "unsharp=5:5:0.8:5:5:0.8");
        let op = TransformOperation::Sharpen { amount: 99.0 };
        assert_eq!(chain(&[op]).unwrap(), "unsharp=5:5:5:5:5:5");
        let op = TransformOperation::Sharpen { amount: f32::NAN };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn scale_validates() {
        let op = TransformOperation::Scale {
            width: 640,
            height: 480,
        };
        assert_eq!(chain(&[op]).unwrap(), "scale=640:480");
        let op = TransformOperation::Scale {
            width: 0,
            height: 480,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn custom_appended_last_and_nul_rejected() {
        let op = TransformOperation::Grayscale;
        assert_eq!(
            build_chain(W, H, false, &[op], Some("vignette")).unwrap(),
            "hue=s=0,vignette"
        );
        let err = build_chain(W, H, false, &[], Some("a\0b")).unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn operations_run_in_order() {
        let ops = [
            TransformOperation::Grayscale,
            TransformOperation::Scale {
                width: 80,
                height: 60,
            },
        ];
        assert_eq!(chain(&ops).unwrap(), "hue=s=0,scale=80:60");
    }
}
