use serde::Deserialize;

/// Output format to encode the converted audio into.
///
/// Selecting a variant also selects the underlying FFmpeg encoder:
/// PCM s16le for [`AudioFormat::Wav`], the native FLAC encoder for
/// [`AudioFormat::Flac`], libmp3lame for [`AudioFormat::Mp3`], the
/// native AAC encoder for [`AudioFormat::Aac`], libvorbis for
/// [`AudioFormat::Ogg`] and libopus for [`AudioFormat::Opus`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum AudioFormat {
    /// Lossless WAV (PCM signed 16-bit little-endian).
    Wav,
    /// Lossy MP3 via libmp3lame.
    Mp3,
    /// Lossless FLAC (source is re-quantized to 16-bit samples).
    Flac,
    /// Lossy AAC in an ADTS (`.aac`) stream.
    Aac,
    /// Lossy Ogg Vorbis via libvorbis.
    Ogg,
    /// Lossy Opus in Ogg via libopus (always encoded at 48 kHz).
    Opus,
}

impl AudioFormat {
    /// Maps a file extension to a format.
    ///
    /// Matching is case-insensitive and accepts `wav`, `mp3`, `flac`,
    /// `aac`, `ogg`, `oga` and `opus`. Returns `None` for anything else.
    pub fn from_extension(ext: &str) -> Option<AudioFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "wav" => Some(AudioFormat::Wav),
            "mp3" => Some(AudioFormat::Mp3),
            "flac" => Some(AudioFormat::Flac),
            "aac" => Some(AudioFormat::Aac),
            "ogg" | "oga" => Some(AudioFormat::Ogg),
            "opus" => Some(AudioFormat::Opus),
            _ => None,
        }
    }
}

/// Maps a [`ConversionOptions::quality`] value (0–100, clamped) to the
/// encoder target bitrate in bits per second for `format`.
///
/// Lossless formats ([`AudioFormat::Wav`], [`AudioFormat::Flac`]) have
/// no bitrate knob and return `None`.
///
/// | Format | Range at quality 0 → 100 |
/// |--------|--------------------------|
/// | MP3    | 32 kbps → 320 kbps |
/// | AAC    | 64 kbps → 256 kbps |
/// | Ogg    | 32 kbps → 320 kbps |
/// | Opus   | 16 kbps → 256 kbps |
pub fn bitrate_for_quality(format: AudioFormat, quality: u8) -> Option<u32> {
    let q = u32::from(quality.min(100));
    match format {
        AudioFormat::Mp3 => Some((32 + 288 * q / 100) * 1000),
        AudioFormat::Aac => Some((64 + 192 * q / 100) * 1000),
        AudioFormat::Ogg => Some((32 + 288 * q / 100) * 1000),
        AudioFormat::Opus => Some((16 + 240 * q / 100) * 1000),
        AudioFormat::Wav | AudioFormat::Flac => None,
    }
}

/// Parameters controlling a single [`crate::AudioConverter::convert`] call.
///
/// # Fields
///
/// - **`format`** — target container/codec. See [`AudioFormat`].
///   Required in the plugin JSON (no implicit default).
///
/// - **`quality`** — encoding quality on a `0..=100` scale (clamped to
///   `100` internally). Default (programmatic) is `80`. It is translated
///   per format by [`bitrate_for_quality`]; lossless formats ignore it.
///
/// - **`sample_rate`** — target sample rate in Hz. Default `None` keeps
///   the source rate. Rates the target codec cannot encode are adjusted
///   to the nearest supported rate (Opus always uses 48000; MP3/AAC use
///   their standard rate tables). `Some(0)` is rejected with
///   [`crate::Error::InvalidInput`].
///
/// - **`channels`** — target channel count, `1` or `2`. Default `None`
///   keeps mono/stereo sources as-is and downmixes anything above
///   stereo to stereo. `Some(0)` or values above 2 are rejected with
///   [`crate::Error::InvalidInput`].
///
/// # Example
///
/// ```text
/// ConversionOptions {
///     format: AudioFormat::Opus,
///     quality: 96,
///     sample_rate: Some(48000),
///     channels: Some(2),
/// }
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct ConversionOptions {
    /// Output format.
    pub format: AudioFormat,
    /// Quality on a 0–100 scale (values above 100 are clamped).
    pub quality: u8,
    /// Target sample rate in Hz; `None` keeps the source rate.
    pub sample_rate: Option<u32>,
    /// Target channel count (1 or 2); `None` keeps/downmixes per format.
    pub channels: Option<u8>,
}

impl Default for ConversionOptions {
    /// MP3 at quality 80, source rate and channel layout.
    fn default() -> Self {
        ConversionOptions {
            format: AudioFormat::Mp3,
            quality: 80,
            sample_rate: None,
            channels: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bitrate_table_endpoints() {
        assert_eq!(bitrate_for_quality(AudioFormat::Mp3, 0), Some(32_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Mp3, 100), Some(320_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Mp3, 255), Some(320_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Aac, 0), Some(64_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Aac, 100), Some(256_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Ogg, 100), Some(320_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Opus, 0), Some(16_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Opus, 100), Some(256_000));
        assert_eq!(bitrate_for_quality(AudioFormat::Wav, 80), None);
        assert_eq!(bitrate_for_quality(AudioFormat::Flac, 80), None);
    }

    #[test]
    fn extensions_map_to_formats() {
        assert_eq!(AudioFormat::from_extension("wav"), Some(AudioFormat::Wav));
        assert_eq!(AudioFormat::from_extension("MP3"), Some(AudioFormat::Mp3));
        assert_eq!(AudioFormat::from_extension("flac"), Some(AudioFormat::Flac));
        assert_eq!(AudioFormat::from_extension("aac"), Some(AudioFormat::Aac));
        assert_eq!(AudioFormat::from_extension("ogg"), Some(AudioFormat::Ogg));
        assert_eq!(AudioFormat::from_extension("oga"), Some(AudioFormat::Ogg));
        assert_eq!(AudioFormat::from_extension("opus"), Some(AudioFormat::Opus));
        assert_eq!(AudioFormat::from_extension("m4a"), None);
        assert_eq!(AudioFormat::from_extension(""), None);
    }

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: ConversionOptions = serde_json::from_value(serde_json::json!({
            "format": "Opus",
            "quality": 96
        }))
        .unwrap();
        assert_eq!(opts.format, AudioFormat::Opus);
        assert_eq!(opts.quality, 96);
        assert_eq!(opts.sample_rate, None);
        assert_eq!(opts.channels, None);
    }

    #[test]
    fn options_reject_invalid_json() {
        assert!(
            serde_json::from_value::<ConversionOptions>(serde_json::json!({"quality": 80}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<ConversionOptions>(serde_json::json!({
                "format": "wav",
                "quality": "loud"
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<ConversionOptions>(serde_json::json!("nope")).is_err());
    }

    #[test]
    fn default_is_mp3_quality_80() {
        let default = ConversionOptions::default();
        assert_eq!(default.format, AudioFormat::Mp3);
        assert_eq!(default.quality, 80);
        assert_eq!(default.sample_rate, None);
        assert_eq!(default.channels, None);
    }
}
