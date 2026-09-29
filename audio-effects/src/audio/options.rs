use audio_conversion::AudioFormat;
use serde::Deserialize;

/// A single typed effect applied inside the filter graph.
///
/// Operations run in the order they appear in
/// [`EffectOptions::operations`]. Serde uses lowercase snake_case tags,
/// e.g. `{"volume": {"gain_db": -6.0}}` or `"reverse"`.
///
/// Invalid arguments are rejected with [`audio_conversion::Error::InvalidInput`] before
/// any FFmpeg call: all floating-point parameters must be finite,
/// `speed.factor` must be `> 0` (and decomposable into at most 16
/// `atempo` stages), `trim` must select a non-empty window inside the
/// stream and `echo.decay` must lie in `(0, 1)`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectOperation {
    /// Gain change in decibels (`volume=<gain>dB`).
    Volume { gain_db: f32 },
    /// Fade in at the start and/or fade out at the end of the (possibly
    /// trimmed/sped-up) stream, in seconds (`afade`).
    Fade {
        fade_in_secs: f32,
        fade_out_secs: f32,
    },
    /// Change playback speed (`atempo` stages; 2.0 = twice as fast,
    /// 0.5 = half speed). Alters the output duration.
    Speed { factor: f32 },
    /// Low-shelf EQ (`bass=g=…:f=…:w=…`); gain is clamped to
    /// `-900..=900`, frequency to `0..=999999` and width to `0..=99999`.
    Bass {
        gain_db: f32,
        frequency: f32,
        width: f32,
    },
    /// High-shelf EQ (`treble=…`); same clamps as [`Bass`](Self::Bass).
    Treble {
        gain_db: f32,
        frequency: f32,
        width: f32,
    },
    /// Single tap echo (`aecho=0.6:0.3:<delay>:<decay>`); delay in
    /// milliseconds `0..=60000`, decay strictly between 0 and 1.
    Echo { delay_ms: u32, decay: f32 },
    /// Keep only `[start_secs, end_secs)` of the stream
    /// (`atrim` + `asetpts`); alters the output duration.
    Trim { start_secs: f32, end_secs: f32 },
    /// Play the stream backwards (`areverse`).
    Reverse,
    /// EBU R128 loudness normalisation (`loudnorm`); `target_lufs` is
    /// clamped to the filter's valid range `-70..=-5`.
    Normalize { target_lufs: f32 },
}

/// Parameters for a single [`crate::AudioEffects::apply`] call.
///
/// # Fields
///
/// - **`operations`** — typed effects applied in order. Default: `[]`.
///
/// - **`custom_filters`** — raw FFmpeg filter string appended *after*
///   the typed operations (escape hatch, e.g. `"volume=0.5"`).
///   Embedded NUL bytes are rejected; because the graph is a single
///   linked path (`abuffer → chain → abuffersink`), standalone source
///   filters such as `movie=` stay unlinked and make graph
///   configuration fail — they cannot hijack the chain.
///
/// - **`format`** — output format. Default `None` = [`AudioFormat::Wav`]
///   (lossless PCM, no second generation loss from the effect pass).
///
/// - **`quality`** — encoder quality `0..=100` (clamped to 100
///   internally). Default `None` = `80`. Semantics per format are
///   documented on [`audio_conversion::ConversionOptions`].
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct EffectOptions {
    /// Typed effects, applied in order. `#[serde(default)]`.
    #[serde(default)]
    pub operations: Vec<EffectOperation>,
    /// Raw FFmpeg filter string appended last. Default `None`.
    pub custom_filters: Option<String>,
    /// Output format; `None` means WAV. Default `None`.
    pub format: Option<AudioFormat>,
    /// Encoder quality 0–100; `None` means 80. Default `None`.
    pub quality: Option<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_wav_identity() {
        let opts = EffectOptions::default();
        assert!(opts.operations.is_empty());
        assert_eq!(opts.custom_filters, None);
        assert_eq!(opts.format, None);
        assert_eq!(opts.quality, None);
    }

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: EffectOptions =
            serde_json::from_value(serde_json::json!({"custom_filters": "volume=0.5"})).unwrap();
        assert!(opts.operations.is_empty());
        assert_eq!(opts.custom_filters.as_deref(), Some("volume=0.5"));
        assert_eq!(opts.format, None);
        assert_eq!(opts.quality, None);
    }

    #[test]
    fn operations_deserialize_snake_case() {
        let opts: EffectOptions = serde_json::from_value(serde_json::json!({
            "operations": [
                {"volume": {"gain_db": -6.0}},
                {"fade": {"fade_in_secs": 0.5, "fade_out_secs": 1.5}},
                {"speed": {"factor": 2.0}},
                {"bass": {"gain_db": 3.0, "frequency": 100.0, "width": 0.5}},
                {"treble": {"gain_db": -3.0, "frequency": 3000.0, "width": 0.5}},
                {"echo": {"delay_ms": 250, "decay": 0.4}},
                {"trim": {"start_secs": 0.25, "end_secs": 4.0}},
                "reverse",
                {"normalize": {"target_lufs": -16.0}}
            ],
            "format": "Mp3",
            "quality": 90
        }))
        .unwrap();
        assert_eq!(opts.operations.len(), 9);
        assert_eq!(opts.format, Some(AudioFormat::Mp3));
        assert_eq!(opts.quality, Some(90));
    }

    #[test]
    fn options_reject_invalid_json() {
        assert!(
            serde_json::from_value::<EffectOptions>(serde_json::json!({
                "operations": [{"speed": {"factor": "fast"}}]
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<EffectOptions>(serde_json::json!({"quality": "loud"}))
                .is_err()
        );
        assert!(serde_json::from_value::<EffectOptions>(serde_json::json!("nope")).is_err());
    }
}
