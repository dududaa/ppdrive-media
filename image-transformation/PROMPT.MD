# 🦀 Rust Image Transformer (FFmpeg Backend) — AI Build Prompt

## 🎯 Objective

Build a **high-performance Rust image transformation library** that uses
**FFmpeg as the native execution engine** (libavfilter).

---

## 📌 Core Requirements

- Use **FFmpeg libraries**:
  - `libavfilter` (filter graphs: crop/rotate/flip/pad/adjust/blur/…)
  - `libavutil` (frames, errors, pixel formats)
  - Decode/encode **reused from the `image-compression` crate** via its
    public `Frame` bridge — do NOT duplicate codec/AVIO code

- FFmpeg is a **guaranteed dependency**:
  - Must be **dynamically linked at build time**
  - ❌ Do NOT use runtime loading (`dlopen`, `libloading`) inside this crate

- Use:
  - `bindgen` in `build.rs` to generate FFI bindings
  - `pkg-config` to discover FFmpeg installation (`libavfilter`, `libavutil`)

- Do NOT:
  - Use high-level wrappers (`ffmpeg-next`)
  - Reimplement any pixel processing in Rust — FFmpeg filters do all work

---

## 🏗 Architecture

Design a **two-layer system**:

### 1. FFI Layer (Unsafe)

**Location:**
```
src/ffi/
```

**Responsibilities:**
- Contains `bindgen`-generated bindings (libavfilter + libavutil)
- Direct interaction with FFmpeg C filter API
- RAII wrappers for `AVFilterGraph`, sink `AVFrame`
- No business logic
- Fully `unsafe`

---

### 2. Safe API Layer

**Location:**
```
src/image/
```

**Responsibilities:**
- Provide safe, ergonomic Rust APIs
- Handle:
  - operation → filter-string building
  - input validation and clamping
  - filter graph execution
- Hide all unsafe code internally

---

## 🧩 Public API Design

```rust
pub struct ImageTransformer;

pub enum TransformOperation {
    Crop { x: u32, y: u32, width: u32, height: u32 },
    Rotate { degrees: u16 },        // 90 / 180 / 270 (lossless transpose)
    Flip { horizontal: bool, vertical: bool },
    Pad { left: u32, top: u32, right: u32, bottom: u32, color: String },
    Grayscale,
    Adjust { brightness: f32, contrast: f32, saturation: f32 },
    Blur { sigma: f32 },
    Sharpen { amount: f32 },
    Scale { width: u32, height: u32 },
}

pub struct TransformOptions {
    pub operations: Vec<TransformOperation>,  // applied in order
    pub custom_filters: Option<String>,       // raw FFmpeg filter string, appended last
    pub format: Option<ImageFormat>,          // None = keep input format
    pub quality: Option<u8>,                  // None = 80
}

impl ImageTransformer {
    pub fn new() -> Result<Self, Error>;

    pub fn transform(
        &self,
        input: &[u8],
        options: TransformOptions,
    ) -> Result<Vec<u8>, Error>;
}
```

Reuse `image_compression::{Error, ImageFormat, Frame}`.

---

## 🔄 Internal Pipeline (FFmpeg-driven)

```
input bytes
    ↓
image_compression::Frame::decode  (bridge, avcodec/avformat)
    ↓
AVFrame
    ↓
buffer src → filter graph (libavfilter) → buffersink
    (typed ops → filter strings, or custom passthrough)
    ↓
AVFrame (transformed)
    ↓
image_compression::Frame::encode  (bridge; even-dim fix inside)
    ↓
output bytes
```

### Constraints

- Use `av_buffersrc_write_frame` (does not consume the source frame —
  the bridge keeps ownership)
- Sink frames are owned by us and must be unreffed (RAII)
- Rust orchestrates — FFmpeg does the heavy work

### Op → filter string mapping

| Operation | Filter |
|-----------|--------|
| Crop | `crop=w:h:x:y` |
| Rotate 90/180/270 | `transpose=1` / `transpose=1,transpose=1` / `transpose=2` |
| Flip | `hflip`, `vflip` |
| Pad | `pad=iw+l+r:ih+t+b:l:t:color=#RRGGBB` |
| Grayscale | `hue=s=0` |
| Adjust | `format=rgb24\|rgba`, `lutrgb=…` (clamped), `hue=s=…` — port of `eq` semantics, since `eq` is GPL-only |
| Blur | `gblur=sigma=…` |
| Sharpen | `unsharp=5:5:…` |
| Scale | `scale=w:h` |

---

## 🛡 Custom Filter String Safety

- `custom_filters` is appended after typed operations
- Reject embedded NUL bytes
- Graph topology (`buffer → chain → buffersink`) guarantees a single
  linked path: standalone source filters (`movie=…`) cannot hijack the
  chain (they stay unlinked → `avfilter_graph_config` fails)
- Validate typed-op arguments in Rust (bounds vs source dims, enum
  ranges, clamped filter parameters) → `Error::InvalidInput`

---

## ⚙️ Build System

Implement `build.rs` that:

- Uses `pkg-config` to locate `libavfilter` and `libavutil`
- Uses `bindgen` to generate bindings with the same proven settings as
  `image-compression`:
  - `.generate_comments(false)` (header docs become failing doctests)
  - `.prepend_enum_name(false)`
  - `.layout_tests(false)`
- Outputs bindings to:

```
$OUT_DIR/bindings.rs
```

- No hardcoded paths; honour `PKG_CONFIG_PATH`

---

## ❗ Error Handling

Reuse:

```rust
pub enum Error {
    FfmpegError(String),
    DecoderNotFound,
    EncoderNotFound,
    InvalidInput,
    UnsupportedFormat,
}
```

Map FFmpeg return codes with `av_strerror`.

---

## ⚡ Performance Goals

- Zero/minimal-copy pipeline
- Single filter-graph pass for all operations
- No intermediate Rust pixel buffers ever
- Graph built per call (stateless transformer)

---

## 📦 Deliverables

- Complete Cargo project (`crate-type = ["cdylib", "lib"]`)
- `PROMPT.MD` (this file), `README.MD`
- `build.rs` with `bindgen` + `pkg-config`
- FFI module using `include!(...)`
- Safe transformation API
- `plugin_dispatch` C entry point (same contract as `image-compression`:
  args = `(&[u8], &Value)`, returns `Box<DispatchResponse>`,
  serde-deserializable `TransformOptions`)
- Working example CLI:

```rust
let transformer = ImageTransformer::new()?;

let output = transformer.transform(
    &input_bytes,
    TransformOptions {
        operations: vec![
            TransformOperation::Crop { x: 0, y: 0, width: 800, height: 600 },
            TransformOperation::Rotate { degrees: 90 },
            TransformOperation::Grayscale,
        ],
        custom_filters: None,
        format: Some(ImageFormat::WebP),
        quality: Some(80),
    },
)?;
```

- Tests covering every operation, format keep/override, validation
  errors, alpha roundtrip, odd-dimension even-fix

---

## 🚫 Hard Constraints

Do NOT use:

- `libloading`
- `ffmpeg-next`
- Rust-based pixel-processing crates (`image`, `rayon` transforms, …)

---

## 🧠 Guiding Principle

> Rust manages safety and orchestration.
> FFmpeg performs all media processing natively.
