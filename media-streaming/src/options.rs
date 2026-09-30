use serde::Deserialize;

use crate::error::Error;

/// Output protocol for packaged streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StreamingProtocol {
    /// Apple HTTP Live Streaming: `master.m3u8` + variant playlists
    /// over MPEG-TS segments.
    #[default]
    Hls,
    /// MPEG-DASH: `manifest.mpd` over fragmented-MP4 segments.
    Dash,
}

/// One adaptive rendition (quality level) of the output.
///
/// Resolution follows the same rules as video conversion's
/// `ConversionOptions`: explicit `width`/`height` win, `scale`
/// applies only when both are `None`, and `None`/`None`/`None`
/// keeps the source dimensions (never upscaled on auto ladders;
/// manual specs may upscale).
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
pub struct RenditionSpec {
    /// Target width in pixels for this rung.
    pub width: Option<u32>,
    /// Target height in pixels for this rung.
    pub height: Option<u32>,
    /// Scale factor when both dimensions are `None`.
    pub scale: Option<f32>,
    /// Per-rung quality override (0–100, clamped); falls back to
    /// [`StreamingOptions::quality`].
    pub quality: Option<u8>,
    /// Rung bitrate in bits per second used for playlist bandwidth
    /// attributes; derived from the ladder table when `None`.
    pub bitrate: Option<u64>,
}

/// Options for HLS/DASH packaging.
///
/// # Example
///
/// ```text
/// StreamingOptions {
///     protocol: StreamingProtocol::Hls,
///     segment_duration: 4,
///     quality: 80,
///     renditions: None,   // auto ladder from the source resolution
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct StreamingOptions {
    /// Output protocol. Default: [`StreamingProtocol::Hls`].
    #[serde(default)]
    pub protocol: StreamingProtocol,
    /// Target segment duration in seconds (1–30). Default: `4`.
    #[serde(default = "default_segment_duration")]
    pub segment_duration: u32,
    /// Base quality (0–100, values above 100 are clamped) applied to
    /// every rendition that does not override it — and to the shared
    /// AAC audio rendition. Default: `80`.
    #[serde(default = "default_quality")]
    pub quality: u8,
    /// Explicit rendition list; `None` derives an auto ladder from
    /// the source resolution. An empty list is rejected. Default:
    /// `None`.
    pub renditions: Option<Vec<RenditionSpec>>,
}

fn default_segment_duration() -> u32 {
    4
}

fn default_quality() -> u8 {
    80
}

impl Default for StreamingOptions {
    /// HLS, 4-second segments, quality 80, auto ladder.
    fn default() -> Self {
        StreamingOptions {
            protocol: StreamingProtocol::Hls,
            segment_duration: 4,
            quality: 80,
            renditions: None,
        }
    }
}

impl StreamingOptions {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.segment_duration == 0 || self.segment_duration > 30 {
            return Err(Error::InvalidInput);
        }
        if let Some(renditions) = &self.renditions {
            if renditions.is_empty() {
                return Err(Error::InvalidInput);
            }
            for rendition in renditions {
                if rendition.width == Some(0) || rendition.height == Some(0) {
                    return Err(Error::InvalidInput);
                }
                if let Some(scale) = rendition.scale
                    && (!scale.is_finite() || scale <= 0.0)
                {
                    return Err(Error::InvalidInput);
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_hls_four_seconds_quality_80_auto() {
        let default = StreamingOptions::default();
        assert_eq!(default.protocol, StreamingProtocol::Hls);
        assert_eq!(default.segment_duration, 4);
        assert_eq!(default.quality, 80);
        assert_eq!(default.renditions, None);
    }

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: StreamingOptions = serde_json::from_str("{}").unwrap();
        assert_eq!(opts, StreamingOptions::default());

        let opts: StreamingOptions =
            serde_json::from_str(r#"{"protocol":"dash","segment_duration":6,"quality":70}"#)
                .unwrap();
        assert_eq!(opts.protocol, StreamingProtocol::Dash);
        assert_eq!(opts.segment_duration, 6);
        assert_eq!(opts.quality, 70);

        let opts: StreamingOptions =
            serde_json::from_str(r#"{"renditions":[{"width":640,"scale":0.5,"bitrate":800000}]}"#)
                .unwrap();
        let renditions = opts.renditions.unwrap();
        assert_eq!(renditions.len(), 1);
        assert_eq!(renditions[0].width, Some(640));
        assert_eq!(renditions[0].scale, Some(0.5));
        assert_eq!(renditions[0].bitrate, Some(800_000));
        assert_eq!(renditions[0].quality, None);
    }

    #[test]
    fn validate_rejects_bad_segment_duration_and_renditions() {
        for duration in [0u32, 31] {
            let opts = StreamingOptions {
                segment_duration: duration,
                ..StreamingOptions::default()
            };
            assert_eq!(opts.validate(), Err(Error::InvalidInput));
        }
        let opts = StreamingOptions {
            renditions: Some(Vec::new()),
            ..StreamingOptions::default()
        };
        assert_eq!(opts.validate(), Err(Error::InvalidInput));

        let opts = StreamingOptions {
            renditions: Some(vec![RenditionSpec {
                width: Some(0),
                ..RenditionSpec::default()
            }]),
            ..StreamingOptions::default()
        };
        assert_eq!(opts.validate(), Err(Error::InvalidInput));

        let opts = StreamingOptions {
            renditions: Some(vec![RenditionSpec {
                scale: Some(-1.0),
                ..RenditionSpec::default()
            }]),
            ..StreamingOptions::default()
        };
        assert_eq!(opts.validate(), Err(Error::InvalidInput));

        let opts = StreamingOptions {
            segment_duration: 30,
            renditions: Some(vec![RenditionSpec {
                scale: Some(2.0),
                ..RenditionSpec::default()
            }]),
            ..StreamingOptions::default()
        };
        assert!(opts.validate().is_ok());
    }
}
