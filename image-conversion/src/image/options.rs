use serde::Deserialize;

/// Output format to encode the converted image into.
///
/// Selecting a variant also selects the underlying FFmpeg encoder:
/// MJPEG for [`ImageFormat::Jpeg`], the native PNG encoder for
/// [`ImageFormat::Png`], libwebp for [`ImageFormat::WebP`] and
/// libaom-AV1 (via the AVIF muxer) for [`ImageFormat::Avif`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum ImageFormat {
    /// Lossy JPEG. Alpha channels are discarded; dimensions are rounded up
    /// to even values.
    Jpeg,
    /// Lossless PNG. Alpha is preserved when the input has it.
    Png,
    /// Lossy (or lossless at quality 100) WebP. Alpha is preserved when
    /// the input has it.
    WebP,
    /// Lossy AVIF (AV1). Alpha channels are discarded; dimensions are
    /// rounded up to even values.
    Avif,
}

impl ImageFormat {
    /// Maps a file extension to a format.
    ///
    /// Matching is case-insensitive and accepts `jpg`, `jpeg`, `png`,
    /// `webp` and `avif`. Returns `None` for anything else, including
    /// AVIF's less common `.avifs` spelling.
    pub fn from_extension(ext: &str) -> Option<ImageFormat> {
        match ext.to_ascii_lowercase().as_str() {
            "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
            "png" => Some(ImageFormat::Png),
            "webp" => Some(ImageFormat::WebP),
            "avif" => Some(ImageFormat::Avif),
            _ => None,
        }
    }
}

/// Parameters controlling a single [`crate::ImageConverter::convert`] call.
///
/// # Fields
///
/// - **`format`** — target container/codec. See [`ImageFormat`].
///   Defaults to [`ImageFormat::Jpeg`].
///
/// - **`quality`** — compression quality on a `0..=100` scale.
///   The value is clamped to `100` internally, so anything above `100`
///   behaves as `100`. Default is `80`. It is translated per format:
///
///   | Format | Mapping of `quality` |
///   |--------|----------------------|
///   | JPEG   | QSCALE: `31` at 0 → `2` at 100 (FFmpeg valid range is 2–31) |
///   | PNG    | zlib compression level: `0` at 0 → `9` at 100 (always lossless) |
///   | WebP   | lossy quality `0`–`99`; exactly `100` switches to lossless mode |
///   | AVIF   | AV1 CRF: `63` at 0 → `0` at 100 (plus multithreaded row encoding) |
///
/// - **`width`** / **`height`** — optional output dimensions in pixels.
///   Default is `None`/`None`, which keeps the input dimensions.
///   - Only `width` given → height computed to preserve aspect ratio.
///   - Only `height` given → width computed to preserve aspect ratio.
///   - Both given → exact resize (aspect ratio is *not* preserved).
///   - `0` for either value is rejected with [`crate::Error::InvalidInput`].
///
///   JPEG and AVIF additionally round dimensions up to even values
///   (chroma-subsampling requirement), e.g. `63×47` becomes `64×48`.
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
///   `quality`. Honoured by AVIF only, where it maps to AV1 `cpu-used`
///   (`8` at effort 0 → `0` at effort 100). JPEG, PNG and WebP expose
///   no effort knob through their FFmpeg encoders and ignore it.
///   Default: `None` (encoder default).
///
/// - **`max_bytes`** — maximum output size in bytes. The converter
///   encodes at `quality` first; if the result is too large it
///   binary-searches lower qualities (at most seven extra encodes)
///   for the highest quality that fits. `Some(0)` is rejected with
///   [`crate::Error::InvalidInput`]. When even quality 0 cannot fit
///   the budget, [`crate::Error::TargetSizeUnreachable`] is returned.
///   Default: `None` (no size target).
///
/// # Example
///
/// ```text
/// ConversionOptions {
///     format: ImageFormat::WebP,
///     quality: 80,
///     width: None,
///     height: None,
///     scale: Some(0.5),   // half-size output
///     effort: None,
///     max_bytes: None,
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct ConversionOptions {
    /// Output format. Default: [`ImageFormat::Jpeg`].
    pub format: ImageFormat,
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
    /// Encoding effort on a 0–100 scale (AVIF only). Default: `None`.
    pub effort: Option<u8>,
    /// Maximum output size in bytes; `None` imposes no budget.
    /// Default: `None`.
    pub max_bytes: Option<u64>,
}

impl Default for ConversionOptions {
    /// JPEG at quality 80, original dimensions, encoder defaults and
    /// no size budget.
    fn default() -> Self {
        ConversionOptions {
            format: ImageFormat::Jpeg,
            quality: 80,
            width: None,
            height: None,
            scale: None,
            effort: None,
            max_bytes: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_jpeg_quality_80_with_no_optional_knobs() {
        let default = ConversionOptions::default();
        assert_eq!(default.format, ImageFormat::Jpeg);
        assert_eq!(default.quality, 80);
        assert_eq!(default.width, None);
        assert_eq!(default.height, None);
        assert_eq!(default.scale, None);
        assert_eq!(default.effort, None);
        assert_eq!(default.max_bytes, None);
    }

    #[test]
    fn options_deserialize_from_partial_json() {
        let opts: ConversionOptions = serde_json::from_value(serde_json::json!({
            "format": "WebP",
            "quality": 60
        }))
        .unwrap();
        assert_eq!(opts.format, ImageFormat::WebP);
        assert_eq!(opts.quality, 60);
        assert_eq!(opts.width, None);
        assert_eq!(opts.height, None);
        assert_eq!(opts.scale, None);
        assert_eq!(opts.effort, None);
        assert_eq!(opts.max_bytes, None);
    }

    #[test]
    fn options_deserialize_new_fields() {
        let opts: ConversionOptions = serde_json::from_value(serde_json::json!({
            "format": "Avif",
            "quality": 90,
            "width": 800,
            "height": null,
            "scale": 0.5,
            "effort": 40,
            "max_bytes": 65536
        }))
        .unwrap();
        assert_eq!(opts.scale, Some(0.5));
        assert_eq!(opts.effort, Some(40));
        assert_eq!(opts.max_bytes, Some(65536));
    }

    #[test]
    fn options_reject_invalid_json() {
        assert!(
            serde_json::from_value::<ConversionOptions>(serde_json::json!({"quality": 80}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<ConversionOptions>(serde_json::json!({
                "format": "webp",
                "quality": "loud"
            }))
            .is_err()
        );
        assert!(serde_json::from_value::<ConversionOptions>(serde_json::json!("nope")).is_err());
    }
}
