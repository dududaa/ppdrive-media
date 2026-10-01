use ppff_video_conversion::VideoFormat;
use serde::Deserialize;

/// A single typed transformation applied inside the filter graph (or,
/// for the timeline operations [`TransformOperation::Trim`],
/// [`TransformOperation::Speed`] and [`TransformOperation::Reverse`],
/// by the surrounding pipeline).
///
/// Operations run in the order they appear in
/// [`TransformOptions::operations`]. Serde uses lowercase snake_case
/// tags, e.g. `{"crop": {"x": 0, "y": 0, "width": 1280, "height":
/// 720}}` or the bare string `"reverse"`.
///
/// Invalid arguments are rejected with
/// [`ppff_video_conversion::Error::InvalidInput`] before the filter graph
/// is built: crop rectangles must lie inside the source frame, rotate
/// only accepts 90/180/270, scale dimensions must be non-zero, speed
/// must be a finite positive factor and all floating-point parameters
/// must be finite.
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
    /// Exact resize (`scale=w:h`); dimensions must be non-zero.
    Scale { width: u32, height: u32 },
    /// Rotate by `degrees` (90, 180 or 270) using lossless
    /// `transpose` passes.
    Rotate { degrees: u16 },
    /// Mirror the frame; at least one of `horizontal`/`vertical` must
    /// be `true` (both = 180° rotation via flips).
    Flip { horizontal: bool, vertical: bool },
    /// Desaturate to gray (`hue=s=0`).
    Grayscale,
    /// Color adjustment: `brightness` is clamped to `-1..=1`,
    /// `contrast` to `-1000..=1000` and `saturation` to `0..=3`.
    /// Non-finite values are rejected.
    ///
    /// Uses FFmpeg's `eq` filter when the linked build provides it
    /// (GPL builds); otherwise falls back to `format=rgb24` →
    /// `lutrgb` → `hue=s=…` with the same linear map.
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
    /// Add a border of the given width (pixels) painted in `color`
    /// (any FFmpeg color name or `#RRGGBB`). All-zero padding is a
    /// no-op.
    Pad {
        left: u32,
        top: u32,
        right: u32,
        bottom: u32,
        color: String,
    },
    /// Keep only `[start, start + duration)` **seconds of the source
    /// timeline** (`duration` omitted = through the end). Selected
    /// before any filter runs and applied to the audio track too.
    Trim { start: f64, duration: Option<f64> },
    /// Change playback speed by `factor` (`2.0` = twice as fast,
    /// `0.5` = half speed) via `setpts=PTS/factor`. Frame count is
    /// unchanged; the audio track is dropped (timestamps would no
    /// longer match).
    Speed { factor: f64 },
    /// Play the video backwards (`reverse` filter — FFmpeg buffers
    /// every decoded frame and flushes them in reverse). The audio
    /// track is dropped.
    ///
    /// Because the whole clip is held in memory as decoded frames,
    /// requests whose estimated frame buffering
    /// (`width × height × fps × duration × 2` bytes) exceeds **1 GiB**
    /// — or whose duration cannot be determined — are rejected with
    /// [`ppff_video_conversion::Error::LimitExceeded`].
    #[serde(rename = "reverse")]
    Reverse,
}

/// Parameters for a single [`crate::VideoTransformer::transform`] call.
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
///   container when it is MP4/MOV or WebM/Matroska, otherwise fall
///   back to [`VideoFormat::Mp4`].
///
/// - **`quality`** — encoder quality `0..=100` (clamped to 100
///   internally). Default `None` = `80`. Semantics per format are
///   documented on [`ppff_video_conversion::ConversionOptions`].
#[derive(Debug, Clone, PartialEq, Deserialize, Default)]
pub struct TransformOptions {
    /// Typed operations, applied in order. `#[serde(default)]`.
    #[serde(default)]
    pub operations: Vec<TransformOperation>,
    /// Raw FFmpeg filter string appended last. Default `None`.
    pub custom_filters: Option<String>,
    /// Output format; `None` keeps the input format. Default `None`.
    pub format: Option<VideoFormat>,
    /// Encoder quality 0–100; `None` means 80. Default `None`.
    pub quality: Option<u8>,
}
