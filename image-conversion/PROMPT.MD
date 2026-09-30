# 🦀 Rust Image Compressor (FFmpeg Backend) — AI Build Prompt

## 🎯 Objective

Build a **high-performance Rust image compression library** that uses **FFmpeg as the native execution engine**.

---

## 📌 Core Requirements

- Use **FFmpeg libraries**:
  - `libavcodec`
  - `libavformat`
  - `libavutil`
  - `libswscale`

- FFmpeg is a **guaranteed dependency**:
  - Must be **dynamically linked at build time**
  - ❌ Do NOT use runtime loading (`dlopen`, `libloading`)

- Use:
  - `bindgen` in `build.rs` to generate FFI bindings
  - `pkg-config` to discover FFmpeg installation

- Do NOT:
  - Use high-level wrappers (`ffmpeg-next`)
  - Reimplement compression in Rust

---

## 🏗 Architecture

Design a **two-layer system**:

### 1. FFI Layer (Unsafe)

**Location:**
```
src/ffi/
```

**Responsibilities:**
- Contains `bindgen`-generated bindings
- Direct interaction with FFmpeg C API
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
  - decoding
  - resizing
  - pixel conversion
  - encoding
- Hide all unsafe code internally

---

## 🧩 Public API Design

```rust
pub struct ImageCompressor;

pub enum ImageFormat {
    Jpeg,
    Png,
    WebP,
    Avif,
}

pub struct CompressionOptions {
    pub format: ImageFormat,
    pub quality: u8,
    pub width: Option<u32>,
    pub height: Option<u32>,
}

impl ImageCompressor {
    pub fn new() -> Result<Self, Error>;

    pub fn compress(
        &self,
        input: &[u8],
        options: CompressionOptions,
    ) -> Result<Vec<u8>, Error>;
}
```

---

## 🔄 Internal Pipeline (FFmpeg-driven)

```
input bytes
    ↓
FFmpeg decode (avcodec)
    ↓
AVFrame
    ↓
(optional) resize / convert (libswscale)
    ↓
AVFrame
    ↓
FFmpeg encode (avcodec)
    ↓
output bytes
```

### Constraints

- Avoid unnecessary copying
- Use `AVFrame` and `AVPacket` directly
- Rust orchestrates — FFmpeg does the heavy work

---

## ⚙️ Build System

Implement `build.rs` that:

- Uses `pkg-config` to locate:
  - `libavcodec`
  - `libavformat`
  - `libavutil`
  - `libswscale`

- Uses `bindgen` to generate bindings
- Outputs bindings to:

```
$OUT_DIR/bindings.rs
```

---

## 🌍 Cross-Platform Support

Support:

- **Linux** (apt/yum FFmpeg dev packages)
- **macOS** (Homebrew)
- **Windows** (MSYS2 or prebuilt FFmpeg)

### Rules

- ❌ Do NOT hardcode paths
- ✅ Use `pkg-config` and environment variables

---

## 🛡 Safety Requirements

- All `unsafe` code must be isolated
- Wrap:
  - `AVFrame` lifecycle
  - `AVPacket` lifecycle
  - codec context allocation/free

### Prevent:

- Memory leaks
- Use-after-free
- Null pointer dereferencing

---

## ❗ Error Handling

Define:

```rust
pub enum Error {
    FfmpegError(String),
    DecoderNotFound,
    EncoderNotFound,
    InvalidInput,
    UnsupportedFormat,
}
```

Include helper to convert FFmpeg errors:

- `av_strerror`

---

## ⚡ Performance Goals

- Zero/minimal-copy pipeline
- No intermediate Rust image buffers unless required
- Reuse codec contexts when possible
- Keep hot path inside FFmpeg

---

## 📦 Deliverables

- Complete Cargo project
- `build.rs` with:
  - `bindgen`
  - `pkg-config`
- FFI module using `include!(...)`
- Safe compression API
- Working example:

```rust
let compressor = ImageCompressor::new()?;

let output = compressor.compress(
    &input_bytes,
    CompressionOptions {
        format: ImageFormat::WebP,
        quality: 80,
        width: Some(800),
        height: Some(600),
    },
)?;
```

---

## ⭐ Bonus (Optional)

- Support multiple formats:
  - JPEG
  - WebP
  - AVIF

- Add:
  - benchmark example
  - feature flags for codecs
  - extensible design for video/audio

---

## 🚫 Hard Constraints

Do NOT use:

- `libloading`
- `ffmpeg-next`
- Rust-based image compression implementations

---

## 🧠 Guiding Principle

> Rust manages safety and orchestration.  
> FFmpeg performs all media processing natively.

---

## 🚀 Intended Outcome

A **production-grade Rust media core** that can be reused for:

- Backend services
- POS systems (receipts, uploads)
- Offline-first tools
- Media pipelines
