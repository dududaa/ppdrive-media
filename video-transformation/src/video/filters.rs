use crate::video::options::TransformOperation;
use video_compression::{Error, VideoStreamInfo};

/// Selected source-time window from the [`TransformOperation::Trim`]
/// operations, in seconds (`end == None` runs through the end of the
/// source).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Window {
    pub start: f64,
    pub end: Option<f64>,
}

impl Window {
    /// Whether a source timestamp (seconds) falls inside the window.
    /// Frames/packets without a usable timestamp are kept.
    pub fn accepts(&self, secs: Option<f64>) -> bool {
        match secs {
            None => true,
            Some(t) => t >= self.start && self.end.is_none_or(|end| t < end),
        }
    }
}

/// What one [`build_chain`] call decided: the filter string plus the
/// timeline facts the surrounding pipeline needs.
#[derive(Debug)]
pub(crate) struct ChainPlan {
    pub chain: String,
    /// Trim window on the source timeline (`None` = no trimming).
    pub window: Option<Window>,
    /// Whether a [`TransformOperation::Reverse`] op is present.
    pub has_reverse: bool,
    /// Whether the audio track must be dropped (Speed or Reverse).
    pub drops_audio: bool,
    /// Whether an in-graph filter rewrites frame timestamps (`setpts`
    /// or `reverse`) — a trimmed clip's pts re-base must be skipped
    /// then, because the sink timeline is no longer the source one.
    pub pts_modified: bool,
}

/// Builds the libavfilter chain for one call: typed operations in
/// order, then the optional custom filter string. Trim/Speed/Reverse
/// additionally update the returned [`ChainPlan`].
///
/// An empty result becomes `null` (identity) so the graph still has a
/// linkable chain.
pub(crate) fn build_chain(
    info: &VideoStreamInfo,
    operations: &[TransformOperation],
    custom_filters: Option<&str>,
) -> Result<ChainPlan, Error> {
    let mut parts: Vec<String> = Vec::new();
    let mut window: Option<Window> = None;
    let mut has_reverse = false;
    let mut drops_audio = false;
    let mut pts_modified = false;
    let eq_available = eq_available();

    for op in operations {
        match op {
            TransformOperation::Trim { start, duration } => {
                if !start.is_finite() || *start < 0.0 {
                    return Err(Error::InvalidInput);
                }
                if let Some(duration) = duration
                    && (!duration.is_finite() || *duration < 0.0)
                {
                    return Err(Error::InvalidInput);
                }
                let end = duration.map(|duration| start + duration);
                window = Some(match window {
                    None => Window { start: *start, end },
                    Some(existing) => Window {
                        start: existing.start.max(*start),
                        end: match (existing.end, end) {
                            (Some(a), Some(b)) => Some(a.min(b)),
                            (Some(a), None) => Some(a),
                            (None, b) => b,
                        },
                    },
                });
                let current = window.expect("just assigned");
                if current.end.is_some_and(|end| end <= current.start) {
                    return Err(Error::InvalidInput);
                }
            }
            TransformOperation::Speed { factor } => {
                if !factor.is_finite() || *factor <= 0.0 {
                    return Err(Error::InvalidInput);
                }
                drops_audio = true;
                if *factor != 1.0 {
                    pts_modified = true;
                    parts.push(format!("setpts=PTS/{factor}"));
                }
            }
            TransformOperation::Reverse => {
                has_reverse = true;
                drops_audio = true;
                pts_modified = true;
                parts.push("reverse".to_string());
            }
            other => {
                if let Some(filter) = other.to_filter(info, eq_available)? {
                    parts.push(filter);
                }
            }
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

    Ok(ChainPlan {
        chain: parts.join(","),
        window,
        has_reverse,
        drops_audio,
        pts_modified,
    })
}

fn eq_available() -> bool {
    unsafe { !crate::ffi::avfilter_get_by_name(c"eq".as_ptr()).is_null() }
}

impl TransformOperation {
    /// Maps one operation to its filter-string fragment, validating
    /// all arguments against the source dimensions first.
    fn to_filter(
        &self,
        info: &VideoStreamInfo,
        eq_available: bool,
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
                if x_end > u64::from(info.width) || y_end > u64::from(info.height) {
                    return Err(Error::InvalidInput);
                }
                Some(format!("crop={width}:{height}:{x}:{y}"))
            }
            TransformOperation::Scale { width, height } => {
                if *width == 0 || *height == 0 {
                    return Err(Error::InvalidInput);
                }
                Some(format!("scale={width}:{height}"))
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
                if eq_available {
                    Some(format!(
                        "eq=brightness={brightness}:contrast={contrast}:saturation={saturation}"
                    ))
                } else {
                    // (val - 127.5) * contrast + 127.5 + brightness * 255,
                    // clipped to 0..=255 — the same linear map `eq`
                    // applies, expressed per RGB channel for builds
                    // without the GPL-only `eq` filter.
                    let offset = brightness * 255.0;
                    let term = if offset > 0.0 {
                        format!("+{offset}")
                    } else if offset < 0.0 {
                        format!("-{}", -offset)
                    } else {
                        String::new()
                    };
                    let expr = format!("clip((val-127.5)*{contrast}+127.5{term},0,255)");
                    Some(format!(
                        "format=rgb24,lutrgb=r='{expr}':g='{expr}':b='{expr}',hue=s={saturation}"
                    ))
                }
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
            TransformOperation::Trim { .. }
            | TransformOperation::Speed { .. }
            | TransformOperation::Reverse => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info() -> VideoStreamInfo {
        VideoStreamInfo {
            width: 320,
            height: 180,
            frame_rate_num: 25,
            frame_rate_den: 1,
            time_base_num: 1,
            time_base_den: 12800,
            duration: Some(2.0),
            format_name: Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
            has_audio: false,
            pix_fmt: 0,
        }
    }

    fn chain(ops: &[TransformOperation]) -> Result<ChainPlan, Error> {
        build_chain(&info(), ops, None)
    }

    #[test]
    fn empty_chain_is_identity() {
        let plan = chain(&[]).unwrap();
        assert_eq!(plan.chain, "null");
        assert!(plan.window.is_none());
        assert!(!plan.drops_audio);
    }

    #[test]
    fn crop_maps_and_validates() {
        let op = TransformOperation::Crop {
            x: 10,
            y: 20,
            width: 80,
            height: 60,
        };
        assert_eq!(chain(&[op]).unwrap().chain, "crop=80:60:10:20");
        let op = TransformOperation::Crop {
            x: 300,
            y: 0,
            width: 80,
            height: 10,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn speed_builds_setpts_and_drops_audio() {
        let plan = chain(&[TransformOperation::Speed { factor: 2.0 }]).unwrap();
        assert_eq!(plan.chain, "setpts=PTS/2");
        assert!(plan.drops_audio);
        let plan = chain(&[TransformOperation::Speed { factor: 0.5 }]).unwrap();
        assert_eq!(plan.chain, "setpts=PTS/0.5");
        for factor in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert_eq!(
                chain(&[TransformOperation::Speed { factor }]).unwrap_err(),
                Error::InvalidInput
            );
        }
    }

    #[test]
    fn reverse_builds_filter_and_drops_audio() {
        let plan = chain(&[TransformOperation::Reverse]).unwrap();
        assert_eq!(plan.chain, "reverse");
        assert!(plan.has_reverse);
        assert!(plan.drops_audio);
    }

    #[test]
    fn trim_accumulates_window() {
        let plan = chain(&[TransformOperation::Trim {
            start: 0.5,
            duration: Some(1.0),
        }])
        .unwrap();
        assert_eq!(plan.chain, "null");
        assert_eq!(
            plan.window,
            Some(Window {
                start: 0.5,
                end: Some(1.5)
            })
        );
        assert!(!plan.drops_audio);

        let plan = chain(&[
            TransformOperation::Trim {
                start: 0.5,
                duration: Some(1.0),
            },
            TransformOperation::Trim {
                start: 1.0,
                duration: Some(3.0),
            },
        ])
        .unwrap();
        assert_eq!(
            plan.window,
            Some(Window {
                start: 1.0,
                end: Some(1.5)
            })
        );

        let err = chain(&[
            TransformOperation::Trim {
                start: 0.5,
                duration: Some(1.0),
            },
            TransformOperation::Trim {
                start: 2.0,
                duration: Some(3.0),
            },
        ])
        .unwrap_err();
        assert_eq!(err, Error::InvalidInput);

        let err = chain(&[TransformOperation::Trim {
            start: f64::NAN,
            duration: None,
        }])
        .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
        let err = chain(&[TransformOperation::Trim {
            start: -1.0,
            duration: None,
        }])
        .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn window_accepts_range_and_nopts() {
        let window = Window {
            start: 0.5,
            end: Some(1.5),
        };
        assert!(!window.accepts(Some(0.4)));
        assert!(window.accepts(Some(0.5)));
        assert!(window.accepts(Some(1.49)));
        assert!(!window.accepts(Some(1.5)));
        assert!(window.accepts(None));
        let open = Window {
            start: 1.0,
            end: None,
        };
        assert!(open.accepts(Some(99.0)));
    }

    #[test]
    fn custom_appended_last_and_nul_rejected() {
        let plan =
            build_chain(&info(), &[TransformOperation::Grayscale], Some("vignette")).unwrap();
        assert_eq!(plan.chain, "hue=s=0,vignette");
        let err = build_chain(&info(), &[], Some("a\0b")).unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn operations_run_in_order() {
        let ops = [
            TransformOperation::Grayscale,
            TransformOperation::Scale {
                width: 160,
                height: 90,
            },
            TransformOperation::Reverse,
        ];
        assert_eq!(
            chain(&[ops[0].clone(), ops[1].clone()]).unwrap().chain,
            "hue=s=0,scale=160:90"
        );
    }
}
