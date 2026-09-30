use crate::error::Error;
use crate::ffi;
use serde::Deserialize;

/// Output container/codec pair to encode the converted video into.
///
/// Selecting a variant also selects the underlying FFmpeg encoder and
/// muxer: libx264 (H.264) in MP4 for [`VideoFormat::Mp4`], libvpx-vp9
/// (VP9) in WebM for [`VideoFormat::WebM`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum VideoFormat {
    /// Lossy H.264 in an MP4 container. Accepts an optional stream-copied
    /// audio track (AAC, MP3); any other audio codec is dropped.
    Mp4,
    /// Lossy VP9 in a WebM container. Accepts an optional stream-copied
    /// audio track (Vorbis, Opus); any other audio codec is dropped.
    WebM,
}

impl VideoFormat {
    /// Maps a file extension to a format.
    ///
    /// Matching is case-insensitive and accepts `mp4`, `m4v`, `webm`
    /// and `mkv`. Returns `None` for anything else.
    pub fn from_extension(ext: &str) -> Option<VideoFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "mp4" | "m4v" => Some(VideoFormat::Mp4),
            "webm" | "mkv" => Some(VideoFormat::WebM),
            _ => None,
        }
    }

    /// Whether a stream copy of this audio codec is allowed into the
    /// container. Incompatible audio is dropped, never transcoded.
    pub(crate) fn accepts_audio_codec(&self, codec_id: u32) -> bool {
        match self {
            VideoFormat::Mp4 => matches!(codec_id, ffi::AV_CODEC_ID_AAC | ffi::AV_CODEC_ID_MP3),
            VideoFormat::WebM => {
                matches!(codec_id, ffi::AV_CODEC_ID_VORBIS | ffi::AV_CODEC_ID_OPUS)
            }
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
///   | MP4    | x264 CRF: `51` at 0 → `18` at 100 (plus `preset=veryfast`) |
///   | WebM   | VP9 CRF: `63` at 0 → `24` at 100 (plus `deadline=good`, `cpu-used=4`) |
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
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
}

impl Default for ConversionOptions {
    /// MP4 at quality 80, original dimensions.
    fn default() -> Self {
        ConversionOptions {
            format: VideoFormat::Mp4,
            quality: 80,
            width: None,
            height: None,
        }
    }
}

impl ConversionOptions {
    pub(crate) fn validate(&self) -> Result<(), Error> {
        if self.width == Some(0) || self.height == Some(0) {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}
