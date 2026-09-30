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
/// # Example
///
/// ```text
/// ConversionOptions {
///     format: ImageFormat::WebP,
///     quality: 80,
///     width: Some(800),
///     height: None,   // keep aspect ratio at 800px wide
/// }
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
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
}

impl Default for ConversionOptions {
    /// JPEG at quality 80, original dimensions.
    fn default() -> Self {
        ConversionOptions {
            format: ImageFormat::Jpeg,
            quality: 80,
            width: None,
            height: None,
        }
    }
}
