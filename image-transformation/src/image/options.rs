use ppff_image_conversion::ImageFormat;
use serde::Deserialize;

/// A single typed transformation applied inside the filter graph.
///
/// Operations run in the order they appear in
/// [`TransformOptions::operations`]. Serde uses lowercase
/// snake_case tags, e.g. `{"crop": {"x": 0, "y": 0, "width": 800,
/// "height": 600}}`.
///
/// Invalid arguments are rejected with [`ppff_image_conversion::Error::InvalidInput`]
/// before any FFmpeg call: crop rectangles must lie inside the source
/// image, rotate only accepts 90/180/270, scale dimensions must be
/// non-zero and all floating-point parameters must be finite.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransformOperation {
    /// Extract a rectangle (`crop=w:h:x:y`). Bounds are validated
    /// against the source dimensions.
    Crop {
        x: u32,
        y: u32,
        width: u32,
        height: u32,
    },
    /// Rotate by `degrees` (90, 180 or 270) using lossless
    /// `transpose` passes.
    Rotate { degrees: u16 },
    /// Mirror the image; at least one of `horizontal`/`vertical` must
    /// be `true` (both = 180° rotation via flips).
    Flip { horizontal: bool, vertical: bool },
    /// Add a border of the given width (pixels) painted in `color`
    /// (any FFmpeg color name or `#RRGGBB`). All-zero padding is a no-op.
    Pad {
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
        color: String,
    },
    /// Desaturate to gray (`hue=s=0`).
    Grayscale,
    /// Color adjustment: `brightness` is clamped to `-1..=1` (added
    /// as `brightness × 255` per RGB channel), `contrast` to
    /// `-1000..=1000` (linear scale around mid-grey) and
    /// `saturation` to `0..=3`. Non-finite values are rejected.
    ///
    /// Implemented as `format=rgb24|rgba` → `lutrgb` → `hue=s=…`
    /// because FFmpeg's `eq` filter is GPL-only and missing from
    /// LGPL builds. The stage temporarily forces RGB (preserving
    /// alpha as `rgba` when the input has one).
    Adjust {
        brightness: f32,
        contrast: f32,
        saturation: f32,
    },
    /// Gaussian blur (`gblur`); `sigma` must be finite and `> 0`.
    Blur { sigma: f32 },
    /// Unsharp masking (`unsharp=5:5:amount:5:5:amount`); `amount`
    /// must be finite and is clamped to `-2..=5`.
    Sharpen { amount: f32 },
    /// Exact resize (`scale=w:h`); dimensions must be non-zero.
    Scale { width: u32, height: u32 },
}

/// Parameters for a single [`crate::ImageTransformer::transform`] call.
///
/// # Fields
///
/// - **`operations`** — typed operations applied in order. Default: `[]`.
///
/// - **`custom_filters`** — raw FFmpeg filter string appended *after*
///   the typed operations (escape hatch, e.g. `"vignette=PI/5"`).
///   Embedded NUL bytes are rejected; because the graph is a single
///   linked path (`buffer → chain → buffersink`), standalone source
///   filters such as `movie=` stay unlinked and make graph
///   configuration fail — they cannot hijack the chain.
///
/// - **`format`** — output format. Default `None` = keep the input
///   format; falls back to [`ImageFormat::Png`] when the input format
///   cannot be determined.
///
/// - **`quality`** — encoder quality `0..=100` (clamped to 100
///   internally). Default `None` = `80`. Semantics per format are
///   documented on [`ppff_image_conversion::ConversionOptions`].
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TransformOptions {
    /// Typed operations, applied in order. `#[serde(default)]`.
    #[serde(default)]
    pub operations: Vec<TransformOperation>,
    /// Raw FFmpeg filter string appended last. Default `None`.
    pub custom_filters: Option<String>,
    /// Output format; `None` keeps the input format. Default `None`.
    pub format: Option<ImageFormat>,
    /// Encoder quality 0–100; `None` means 80. Default `None`.
    pub quality: Option<u8>,
}
