use super::options::EffectOperation;
use audio_conversion::Error;

const MAX_ATTEMPO_STAGES: usize = 16;

/// Builds the libavfilter chain for one call: typed operations in
/// order (tracking how each one shifts the running duration), then
/// the optional custom filter string.
///
/// `duration` is the decoded stream length in seconds — the starting
/// timeline `atrim`/`afade` validate and position against.
///
/// An empty result becomes `anull` (identity) so the graph still has
/// a linkable chain.
pub(crate) fn build_chain(
    duration: f64,
    operations: &[EffectOperation],
    custom_filters: Option<&str>,
) -> Result<String, Error> {
    let mut parts: Vec<String> = Vec::new();
    let mut timeline = duration;

    for op in operations {
        if let Some(filter) = op.to_filter(&mut timeline)? {
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
        parts.push("anull".to_string());
    }

    Ok(parts.join(","))
}

/// Splits a speed factor into `atempo` stages each within the
/// filter's per-stage `0.5..=2.0` range (`f32` display is the
/// shortest round-trip representation, so the stages stay exact).
fn atempo_stages(factor: f32) -> Result<Vec<f32>, Error> {
    if !factor.is_finite() || factor <= 0.0 {
        return Err(Error::InvalidInput);
    }
    let mut stages = Vec::new();
    let mut f = factor;
    while f < 0.5 {
        if stages.len() >= MAX_ATTEMPO_STAGES {
            return Err(Error::InvalidInput);
        }
        stages.push(0.5);
        f /= 0.5;
    }
    while f > 2.0 {
        if stages.len() >= MAX_ATTEMPO_STAGES {
            return Err(Error::InvalidInput);
        }
        stages.push(2.0);
        f /= 2.0;
    }
    if (f - 1.0).abs() > f32::EPSILON {
        stages.push(f);
    }
    Ok(stages)
}

fn finite(value: f32) -> Result<f32, Error> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(Error::InvalidInput)
    }
}

impl EffectOperation {
    /// Maps one operation to its filter-string fragment, validating
    /// all arguments against the running timeline first. Trim and
    /// Speed update `*timeline` to the output duration they imply.
    fn to_filter(&self, timeline: &mut f64) -> Result<Option<String>, Error> {
        Ok(match self {
            EffectOperation::Volume { gain_db } => {
                let gain_db = finite(*gain_db)?;
                Some(format!("volume={gain_db}dB"))
            }
            EffectOperation::Fade {
                fade_in_secs,
                fade_out_secs,
            } => {
                let fade_in = finite(*fade_in_secs)?;
                let fade_out = finite(*fade_out_secs)?;
                if fade_in < 0.0 || fade_out < 0.0 {
                    return Err(Error::InvalidInput);
                }
                if fade_in == 0.0 && fade_out == 0.0 {
                    None
                } else {
                    let mut fades = Vec::new();
                    if fade_in > 0.0 {
                        fades.push(format!("afade=t=in:st=0:d={fade_in}"));
                    }
                    if fade_out > 0.0 {
                        let start = (*timeline - f64::from(fade_out)).max(0.0);
                        fades.push(format!("afade=t=out:st={start}:d={fade_out}"));
                    }
                    Some(fades.join(","))
                }
            }
            EffectOperation::Speed { factor } => {
                let factor = finite(*factor)?;
                let stages = atempo_stages(factor)?;
                *timeline /= f64::from(factor);
                if stages.is_empty() {
                    None
                } else {
                    let chain: Vec<String> = stages.iter().map(|s| format!("atempo={s}")).collect();
                    Some(chain.join(","))
                }
            }
            EffectOperation::Bass {
                gain_db,
                frequency,
                width,
            } => {
                let gain = finite(*gain_db)?.clamp(-900.0, 900.0);
                let freq = finite(*frequency)?.clamp(0.0, 999_999.0);
                let width = finite(*width)?.clamp(0.0, 99_999.0);
                Some(format!("bass=g={gain}:f={freq}:w={width}"))
            }
            EffectOperation::Treble {
                gain_db,
                frequency,
                width,
            } => {
                let gain = finite(*gain_db)?.clamp(-900.0, 900.0);
                let freq = finite(*frequency)?.clamp(0.0, 999_999.0);
                let width = finite(*width)?.clamp(0.0, 99_999.0);
                Some(format!("treble=g={gain}:f={freq}:w={width}"))
            }
            EffectOperation::Echo { delay_ms, decay } => {
                if *delay_ms > 60_000 {
                    return Err(Error::InvalidInput);
                }
                let decay = finite(*decay)?;
                if decay <= 0.0 || decay >= 1.0 {
                    return Err(Error::InvalidInput);
                }
                Some(format!("aecho=0.6:0.3:{delay_ms}:{decay}"))
            }
            EffectOperation::Trim {
                start_secs,
                end_secs,
            } => {
                let start = finite(*start_secs)?;
                let end = finite(*end_secs)?;
                if start < 0.0 || end <= start || f64::from(start) >= *timeline {
                    return Err(Error::InvalidInput);
                }
                let end = end.min(*timeline as f32);
                *timeline = f64::from(end - start);
                Some(format!(
                    "atrim=start={start}:end={end},asetpts=PTS-STARTPTS"
                ))
            }
            EffectOperation::Reverse => Some("areverse".to_string()),
            EffectOperation::Normalize { target_lufs } => {
                let target = finite(*target_lufs)?.clamp(-70.0, -5.0);
                Some(format!("loudnorm=I={target}:TP=-2:LRA=7"))
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DURATION: f64 = 5.0;

    fn chain(ops: &[EffectOperation]) -> Result<String, Error> {
        build_chain(DURATION, ops, None)
    }

    #[test]
    fn empty_chain_is_identity() {
        assert_eq!(chain(&[]).unwrap(), "anull");
        assert_eq!(build_chain(DURATION, &[], Some("  ")).unwrap(), "anull");
    }

    #[test]
    fn volume_maps_and_validates() {
        let op = EffectOperation::Volume { gain_db: -6.0 };
        assert_eq!(chain(&[op]).unwrap(), "volume=-6dB");
        let op = EffectOperation::Volume { gain_db: f32::NAN };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        let op = EffectOperation::Volume {
            gain_db: f32::INFINITY,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn fade_positions_out_at_current_timeline() {
        let op = EffectOperation::Fade {
            fade_in_secs: 1.0,
            fade_out_secs: 0.5,
        };
        assert_eq!(
            chain(&[op]).unwrap(),
            "afade=t=in:st=0:d=1,afade=t=out:st=4.5:d=0.5"
        );
        let op = EffectOperation::Fade {
            fade_in_secs: 0.0,
            fade_out_secs: 0.0,
        };
        assert_eq!(chain(&[op]).unwrap(), "anull");
        let op = EffectOperation::Fade {
            fade_in_secs: -1.0,
            fade_out_secs: 0.0,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn fade_tracks_trim_and_speed_timeline() {
        let ops = [
            EffectOperation::Trim {
                start_secs: 1.0,
                end_secs: 4.0,
            },
            EffectOperation::Fade {
                fade_in_secs: 0.0,
                fade_out_secs: 0.5,
            },
        ];
        assert_eq!(
            chain(&[ops[0].clone(), ops[1].clone()]).unwrap(),
            "atrim=start=1:end=4,asetpts=PTS-STARTPTS,afade=t=out:st=2.5:d=0.5"
        );

        let ops = [
            EffectOperation::Speed { factor: 2.0 },
            EffectOperation::Fade {
                fade_in_secs: 0.0,
                fade_out_secs: 1.0,
            },
        ];
        assert_eq!(chain(&ops).unwrap(), "atempo=2,afade=t=out:st=1.5:d=1");
    }

    #[test]
    fn speed_decomposes_and_validates() {
        assert_eq!(
            chain(&[EffectOperation::Speed { factor: 2.0 }]).unwrap(),
            "atempo=2"
        );
        assert_eq!(
            chain(&[EffectOperation::Speed { factor: 4.0 }]).unwrap(),
            "atempo=2,atempo=2"
        );
        assert_eq!(
            chain(&[EffectOperation::Speed { factor: 0.25 }]).unwrap(),
            "atempo=0.5,atempo=0.5"
        );
        assert_eq!(
            chain(&[EffectOperation::Speed { factor: 1.0 }]).unwrap(),
            "anull"
        );
        for factor in [0.0, -2.0, f32::NAN, f32::INFINITY] {
            assert_eq!(
                chain(&[EffectOperation::Speed { factor }]).unwrap_err(),
                Error::InvalidInput
            );
        }
        assert_eq!(
            chain(&[EffectOperation::Speed { factor: 1.0e-9 }]).unwrap_err(),
            Error::InvalidInput
        );
    }

    #[test]
    fn bass_and_treble_clamp_and_validate() {
        let op = EffectOperation::Bass {
            gain_db: 3000.0,
            frequency: -1.0,
            width: f32::NAN,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        let op = EffectOperation::Bass {
            gain_db: 3.0,
            frequency: 100.0,
            width: 0.5,
        };
        assert_eq!(chain(&[op]).unwrap(), "bass=g=3:f=100:w=0.5");
        let op = EffectOperation::Treble {
            gain_db: -9999.0,
            frequency: 3000.0,
            width: 0.5,
        };
        assert_eq!(chain(&[op]).unwrap(), "treble=g=-900:f=3000:w=0.5");
    }

    #[test]
    fn echo_validates_range() {
        let op = EffectOperation::Echo {
            delay_ms: 250,
            decay: 0.5,
        };
        assert_eq!(chain(&[op]).unwrap(), "aecho=0.6:0.3:250:0.5");
        let op = EffectOperation::Echo {
            delay_ms: 60_001,
            decay: 0.5,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        for decay in [0.0, 1.0, -0.5, f32::NAN] {
            let op = EffectOperation::Echo {
                delay_ms: 100,
                decay,
            };
            assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
        }
    }

    #[test]
    fn trim_maps_validates_and_shortens_timeline() {
        let op = EffectOperation::Trim {
            start_secs: 1.0,
            end_secs: 3.5,
        };
        assert_eq!(
            chain(&[op]).unwrap(),
            "atrim=start=1:end=3.5,asetpts=PTS-STARTPTS"
        );
        for (start, end) in [
            (-0.1, 1.0),
            (1.0, 1.0),
            (1.0, 0.5),
            (5.0, 6.0),
            (4.5, f32::NAN),
        ] {
            let op = EffectOperation::Trim {
                start_secs: start,
                end_secs: end,
            };
            assert_eq!(
                chain(&[op]).unwrap_err(),
                Error::InvalidInput,
                "start={start} end={end}"
            );
        }
        let op = EffectOperation::Trim {
            start_secs: 1.0,
            end_secs: 99.0,
        };
        assert_eq!(
            chain(&[op]).unwrap(),
            "atrim=start=1:end=5,asetpts=PTS-STARTPTS"
        );
    }

    #[test]
    fn reverse_and_normalize_map() {
        assert_eq!(chain(&[EffectOperation::Reverse]).unwrap(), "areverse");
        let op = EffectOperation::Normalize { target_lufs: -16.0 };
        assert_eq!(chain(&[op]).unwrap(), "loudnorm=I=-16:TP=-2:LRA=7");
        let op = EffectOperation::Normalize {
            target_lufs: -100.0,
        };
        assert_eq!(chain(&[op]).unwrap(), "loudnorm=I=-70:TP=-2:LRA=7");
        let op = EffectOperation::Normalize {
            target_lufs: f32::NAN,
        };
        assert_eq!(chain(&[op]).unwrap_err(), Error::InvalidInput);
    }

    #[test]
    fn custom_appended_last_and_nul_rejected() {
        let op = EffectOperation::Volume { gain_db: -3.0 };
        assert_eq!(
            build_chain(DURATION, &[op], Some("volume=0.5")).unwrap(),
            "volume=-3dB,volume=0.5"
        );
        let err = build_chain(DURATION, &[], Some("a\0b")).unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn operations_run_in_order() {
        let ops = [
            EffectOperation::Volume { gain_db: -6.0 },
            EffectOperation::Reverse,
        ];
        assert_eq!(chain(&ops).unwrap(), "volume=-6dB,areverse");
    }
}
