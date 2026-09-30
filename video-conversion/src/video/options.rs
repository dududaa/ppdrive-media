use crate::error::Error;
use crate::ffi;
use serde::Deserialize;

/// Output container/codec pair to encode the converted video into.
///
/// Selecting a variant also selects the underlying FFmpeg encoder and
/// muxer:
///
/// | Variant | Encoder | Muxer |
/// |---------|---------|-------|
/// | [`VideoFormat::Mp4`] | libx264 (H.264) | MP4 |
/// | [`VideoFormat::WebM`] | libvpx-vp9 (VP9) | WebM |
/// | [`VideoFormat::Mov`] | libx264 (H.264) | QuickTime MOV |
/// | [`VideoFormat::Mkv`] | libx264 (H.264) | Matroska |
/// | [`VideoFormat::Avi`] | libx264 (H.264) | AVI |
/// | [`VideoFormat::Mp4Av1`] | libaom-av1 (AV1) | MP4 |
/// | [`VideoFormat::WebMAv1`] | libaom-av1 (AV1) | WebM |
/// | [`VideoFormat::MkvAv1`] | libaom-av1 (AV1) | Matroska |
/// | [`VideoFormat::Mp4Hevc`] | libx265 (H.265/HEVC) | MP4 |
/// | [`VideoFormat::MovHevc`] | libx265 (H.265/HEVC) | QuickTime MOV |
///
/// Encoders are resolved lazily when a conversion starts: a missing
/// encoder surfaces as [`crate::Error::EncoderNotFound`] for that
/// format only, never at [`crate::VideoConverter::new`] time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum VideoFormat {
    /// Lossy H.264 in an MP4 container. Accepts an optional stream-copied
    /// audio track (AAC, MP3); any other audio codec is dropped.
    Mp4,
    /// Lossy VP9 in a WebM container. Accepts an optional stream-copied
    /// audio track (Vorbis, Opus); any other audio codec is dropped.
    WebM,
    /// Lossy H.264 in a QuickTime MOV container. Accepts an optional
    /// stream-copied audio track (AAC, MP3); any other audio codec is
    /// dropped.
    Mov,
    /// Lossy H.264 in a Matroska container. Accepts an optional
    /// stream-copied audio track (AAC, MP3, Vorbis, Opus); any other
    /// audio codec is dropped.
    Mkv,
    /// Lossy H.264 in an AVI container. Accepts an optional
    /// stream-copied audio track (AAC, MP3); any other audio codec is
    /// dropped.
    Avi,
    /// Lossy AV1 in an MP4 container. Accepts an optional stream-copied
    /// audio track (AAC, MP3); any other audio codec is dropped.
    Mp4Av1,
    /// Lossy AV1 in a WebM container. Accepts an optional stream-copied
    /// audio track (Vorbis, Opus); any other audio codec is dropped.
    WebMAv1,
    /// Lossy AV1 in a Matroska container. Accepts an optional
    /// stream-copied audio track (AAC, MP3, Vorbis, Opus); any other
    /// audio codec is dropped.
    MkvAv1,
    /// Lossy H.265/HEVC in an MP4 container. Accepts an optional
    /// stream-copied audio track (AAC, MP3); any other audio codec is
    /// dropped.
    Mp4Hevc,
    /// Lossy H.265/HEVC in a QuickTime MOV container. Accepts an
    /// optional stream-copied audio track (AAC, MP3); any other audio
    /// codec is dropped.
    MovHevc,
}

impl VideoFormat {
    /// Maps a file extension to a format.
    ///
    /// Matching is case-insensitive and accepts `mp4`, `m4v` →
    /// [`VideoFormat::Mp4`], `webm` → [`VideoFormat::WebM`], `mov` →
    /// [`VideoFormat::Mov`], `mkv` → [`VideoFormat::Mkv`] and `avi` →
    /// [`VideoFormat::Avi`]. Returns `None` for anything else.
    ///
    /// Codec-qualified variants ([`VideoFormat::Mp4Av1`],
    /// [`VideoFormat::Mp4Hevc`], …) cannot be inferred from an
    /// extension; pass them explicitly.
    pub fn from_extension(ext: &str) -> Option<VideoFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" => Some(VideoFormat::Mp4),
            "webm" => Some(VideoFormat::WebM),
            "mov" => Some(VideoFormat::Mov),
            "mkv" => Some(VideoFormat::Mkv),
            "avi" => Some(VideoFormat::Avi),
            _ => None,
        }
    }

    /// Whether a stream copy of this audio codec is allowed into the
    /// container. Incompatible audio is dropped, never transcoded.
    pub(crate) fn accepts_audio_codec(&self, codec_id: u32) -> bool {
        let h264_family = matches!(codec_id, ffi::AV_CODEC_ID_AAC | ffi::AV_CODEC_ID_MP3);
        let webm_family = matches!(codec_id, ffi::AV_CODEC_ID_VORBIS | ffi::AV_CODEC_ID_OPUS);
        match self {
            VideoFormat::Mp4
            | VideoFormat::Mov
            | VideoFormat::Avi
            | VideoFormat::Mp4Av1
            | VideoFormat::Mp4Hevc
            | VideoFormat::MovHevc => h264_family,
            VideoFormat::WebM | VideoFormat::WebMAv1 => webm_family,
            VideoFormat::Mkv | VideoFormat::MkvAv1 => h264_family || webm_family,
        }
    }
}

/// Parameters controlling a single [`crate::VideoConverter::convert`]
/// call.
///
/// # Fields
///
/// - **`format`** — target container/codec. See [`VideoFormat`].
///   Defaults to [`VideoFormat::Mp4`].
///
/// - **`quality`** — compression quality on a `0..=100` scale, clamped
///   to `100` internally. Default is `80`. It is translated to an
///   encoder rate-control value via [`crate::crf_for_quality`]:
///
///   | Format | Mapping of `quality` |
///   |--------|----------------------|
///   | MP4, MOV, MKV, AVI | x264 CRF: `51` at 0 → `18` at 100 (plus `preset=veryfast`) |
///   | WebM   | VP9 CRF: `63` at 0 → `24` at 100 (plus `deadline=good`, `cpu-used=4`) |
///   | AV1 (MP4/WebM/MKV) | AV1 CRF: `63` at 0 → `24` at 100 (plus `row-mt=1`, `cpu-used=4`) |
///   | HEVC (MP4/MOV) | x265 CRF: `51` at 0 → `18` at 100 (plus `preset=veryfast`) |
///
/// - **`width`** / **`height`** — optional output dimensions in pixels.
///   Default is `None`/`None`, which keeps the input dimensions.
///   - Only `width` given → height computed to preserve aspect ratio.
///   - Only `height` given → width computed to preserve aspect ratio.
///   - Both given → exact resize (aspect ratio is *not* preserved).
///   - `0` for either value is rejected with [`crate::Error::InvalidInput`].
///
///   Dimensions are rounded up to even values (chroma-subsampling
///   requirement for H.264/VP9), e.g. `63×47` becomes `64×48`.
///
/// - **`scale`** — proportional resize factor applied to the source
///   dimensions (`round(source × scale)`, minimum 1 px on each axis).
///   Used only when *both* `width` and `height` are `None`; when either
///   explicit dimension is set, those win and `scale` is ignored.
///   Non-finite, zero or negative factors, or factors that would
///   produce dimensions beyond `u32`, are rejected with
///   [`crate::Error::InvalidInput`]. Default: `None`.
///
/// - **`effort`** — encoding effort on a `0..=100` scale (clamped to
///   `100` internally); higher is slower and usually smaller at equal
///   `quality`. `None` keeps the encoder defaults shown in the quality
///   table above (`preset=veryfast` for x264/x265, `cpu-used=4` for
///   VP9/AV1):
///
///   | Codec family | `effort` 0 → 100 |
///   |--------------|------------------|
///   | x264 / x265  | preset `ultrafast` → `veryslow` |
///   | VP9          | `cpu-used` `8` → `0` |
///   | AV1          | `cpu-used` `8` → `0` |
///
///   Default: `None`.
///
/// - **`max_bytes`** — maximum output size in bytes. The converter
///   encodes at `quality` first; if the result is too large it
///   binary-searches lower qualities (at most seven extra full
///   re-encodes of the input) for the highest quality that fits.
///   `Some(0)` is rejected with [`crate::Error::InvalidInput`]. When
///   even quality 0 cannot fit the budget,
///   [`crate::Error::TargetSizeUnreachable`] is returned. Honoured by
///   [`crate::VideoConverter::convert`] only;
///   [`crate::VideoConverter::convert_to`] rejects it with
///   [`crate::Error::InvalidInput`] because its sink cannot be re-run
///   for a search. Default: `None` (no size target).
///
/// - **`fps`** — output frame rate. `None` keeps the source timing;
///   `Some(n)` resamples the video to a constant `n` fps by dropping
///   frames that would collide and duplicating the previous frame
///   across gaps (audio is untouched). `Some(0)` and values above
///   `1000` are rejected with [`crate::Error::InvalidInput`].
///   Default: `None`.
///
/// - **`drop_audio`** — when `true`, the source audio track is never
///   stream-copied, even if the target container would accept it.
///   `false` (the default) keeps the automatic behaviour: copy when
///   compatible, drop otherwise. Audio is never transcoded.
///
/// - **`keyframe_interval`** — maximum keyframe distance in frames.
///   `None` keeps the encoder's default GOP; `Some(n)` caps it at `n`
///   (x264/x265 `keyint`, libvpx/libaom `g`), which lets packaging
///   pipelines cut HLS/DASH segments exactly on segment boundaries.
///   `Some(0)` is rejected with [`crate::Error::InvalidInput`].
///   Default: `None`.
///
/// # Example
///
/// ```text
/// ConversionOptions {
///     format: VideoFormat::Mp4,
///     quality: 80,
///     width: None,
///     height: None,
///     scale: Some(0.5),   // half-size output
///     effort: None,
///     max_bytes: None,
///     fps: None,
///     drop_audio: false,
///     keyframe_interval: None,
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConversionOptions {
    /// Output format. Default: [`VideoFormat::Mp4`].
    pub format: VideoFormat,
    /// Quality on a 0–100 scale (values above 100 are clamped).
    /// Default: `80`.
    pub quality: u8,
    /// Target width in pixels; `None` keeps the source width (or derives
    /// it from `height`). Default: `None`.
    pub width: Option<u32>,
    /// Target height in pixels; `None` keeps the source height (or derives
    /// it from `width`). Default: `None`.
    pub height: Option<u32>,
    /// Proportional resize factor; used only when `width` and `height`
    /// are both `None`. Default: `None`.
    pub scale: Option<f32>,
    /// Encoding effort on a 0–100 scale. Default: `None`.
    pub effort: Option<u8>,
    /// Maximum output size in bytes; `None` imposes no budget.
    /// Default: `None`.
    pub max_bytes: Option<u64>,
    /// Constant output frame rate; `None` keeps the source timing.
    /// Default: `None`.
    pub fps: Option<u32>,
    /// Drop the source audio track instead of stream-copying it when
    /// the container accepts it. Default: `false`.
    #[serde(default)]
    pub drop_audio: bool,
    /// Maximum keyframe distance in frames; `None` keeps the encoder
    /// default GOP. Default: `None`.
    pub keyframe_interval: Option<u32>,
}

impl Default for ConversionOptions {
    /// MP4 at quality 80, original dimensions, source timing, encoder
    /// defaults, no size budget, automatic audio handling.
    fn default() -> Self {
        ConversionOptions {
            format: VideoFormat::Mp4,
            quality: 80,
            width: None,
            height: None,
            scale: None,
            effort: None,
            max_bytes: None,
            fps: None,
            drop_audio: false,
            keyframe_interval: None,
        }
    }
}

impl ConversionOptions {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.width == Some(0) || self.height == Some(0) {
            return Err(Error::InvalidInput);
        }
        if self.max_bytes == Some(0) {
            return Err(Error::InvalidInput);
        }
        if let Some(fps) = self.fps
            && (fps == 0 || fps > 1000)
        {
            return Err(Error::InvalidInput);
        }
        if self.keyframe_interval == Some(0) {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audio_rules_follow_container_family() {
        let aac = ffi::AV_CODEC_ID_AAC;
        let mp3 = ffi::AV_CODEC_ID_MP3;
        let vorbis = ffi::AV_CODEC_ID_VORBIS;
        let opus = ffi::AV_CODEC_ID_OPUS;

        for format in [
            VideoFormat::Mp4,
            VideoFormat::Mov,
            VideoFormat::Avi,
            VideoFormat::Mp4Av1,
            VideoFormat::Mp4Hevc,
            VideoFormat::MovHevc,
        ] {
            assert!(format.accepts_audio_codec(aac), "{format:?}");
            assert!(format.accepts_audio_codec(mp3), "{format:?}");
            assert!(!format.accepts_audio_codec(vorbis), "{format:?}");
            assert!(!format.accepts_audio_codec(opus), "{format:?}");
        }

        for format in [VideoFormat::WebM, VideoFormat::WebMAv1] {
            assert!(format.accepts_audio_codec(vorbis), "{format:?}");
            assert!(format.accepts_audio_codec(opus), "{format:?}");
            assert!(!format.accepts_audio_codec(aac), "{format:?}");
            assert!(!format.accepts_audio_codec(mp3), "{format:?}");
        }

        for format in [VideoFormat::Mkv, VideoFormat::MkvAv1] {
            assert!(format.accepts_audio_codec(aac), "{format:?}");
            assert!(format.accepts_audio_codec(mp3), "{format:?}");
            assert!(format.accepts_audio_codec(vorbis), "{format:?}");
            assert!(format.accepts_audio_codec(opus), "{format:?}");
        }
    }

    #[test]
    fn extension_inference_covers_plain_containers() {
        assert_eq!(VideoFormat::from_extension("mp4"), Some(VideoFormat::Mp4));
        assert_eq!(VideoFormat::from_extension("webm"), Some(VideoFormat::WebM));
        assert_eq!(VideoFormat::from_extension("mov"), Some(VideoFormat::Mov));
        assert_eq!(VideoFormat::from_extension("MKV"), Some(VideoFormat::Mkv));
        assert_eq!(VideoFormat::from_extension("AvI"), Some(VideoFormat::Avi));
        assert_eq!(VideoFormat::from_extension("gif"), None);
    }

    #[test]
    fn default_has_no_optional_knobs() {
        let default = ConversionOptions::default();
        assert_eq!(default.format, VideoFormat::Mp4);
        assert_eq!(default.quality, 80);
        assert_eq!(default.width, None);
        assert_eq!(default.height, None);
        assert_eq!(default.scale, None);
        assert_eq!(default.effort, None);
        assert_eq!(default.max_bytes, None);
        assert_eq!(default.fps, None);
        assert_eq!(default.keyframe_interval, None);
        assert!(!default.drop_audio);
    }

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: ConversionOptions = serde_json::from_value(serde_json::json!({
            "format": "WebM",
            "quality": 60
        }))
        .unwrap();
        assert_eq!(opts.format, VideoFormat::WebM);
        assert_eq!(opts.quality, 60);
        assert_eq!(opts.width, None);
        assert_eq!(opts.height, None);
        assert_eq!(opts.scale, None);
        assert_eq!(opts.effort, None);
        assert_eq!(opts.max_bytes, None);
        assert_eq!(opts.fps, None);
        assert!(!opts.drop_audio);
    }

    #[test]
    fn options_deserialize_new_fields() {
        let opts: ConversionOptions = serde_json::from_value(serde_json::json!({
            "format": "Mp4",
            "quality": 90,
            "width": 640,
            "height": null,
            "scale": 0.5,
            "effort": 40,
            "max_bytes": 65536,
            "fps": 30,
            "drop_audio": true
        }))
        .unwrap();
        assert_eq!(opts.scale, Some(0.5));
        assert_eq!(opts.effort, Some(40));
        assert_eq!(opts.max_bytes, Some(65536));
        assert_eq!(opts.fps, Some(30));
        assert!(opts.drop_audio);
    }

    #[test]
    fn validate_rejects_zero_keyframe_interval() {
        let opts = ConversionOptions {
            keyframe_interval: Some(0),
            ..ConversionOptions::default()
        };
        assert_eq!(opts.validate(), Err(Error::InvalidInput));
        let opts = ConversionOptions {
            keyframe_interval: Some(10),
            ..ConversionOptions::default()
        };
        assert!(opts.validate().is_ok());
    }

    #[test]
    fn validate_rejects_zero_max_bytes_and_bad_fps() {
        let opts = ConversionOptions {
            max_bytes: Some(0),
            ..ConversionOptions::default()
        };
        assert_eq!(opts.validate(), Err(Error::InvalidInput));

        for fps in [0u32, 1001] {
            let opts = ConversionOptions {
                fps: Some(fps),
                ..ConversionOptions::default()
            };
            assert_eq!(opts.validate(), Err(Error::InvalidInput));
        }

        let ok = ConversionOptions {
            fps: Some(1000),
            ..ConversionOptions::default()
        };
        assert_eq!(ok.validate(), Ok(()));
    }
}
