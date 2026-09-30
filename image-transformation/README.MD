# ppdrive-image-transformation

Rust image transformation library that uses **FFmpeg's libavfilter as
the native execution engine**. Rust validates and orchestrates; FFmpeg
performs all pixel work in a single `buffer → filter chain → buffersink`
graph pass. Decoding and encoding are reused from the
`ppdrive-image-conversion` crate via its public `Frame` bridge — no
codec/AVIO code is duplicated here.

- **Operations:** crop, rotate (90/180/270), flip, pad, grayscale,
  brightness/contrast/saturation, blur, sharpen, scale
- **Escape hatch:** raw FFmpeg filter strings (`custom_filters`)
- **Formats in/out:** JPEG, PNG, WebP in (anything FFmpeg probes);
  output keeps the input format by default
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no pixel
  loops in Rust — one `bindgen`-generated FFI layer over
  `libavfilter` + `libavutil`

## Architecture

```
src/
├── ffi/      unsafe: bindgen bindings (build.rs), AVFilterGraph /
│             sink AVFrame RAII, graph build/push/pull
└── image/    safe public API: ImageTransformer, TransformOptions,
              operation → filter-string builder, validation,
              decode → filter → encode pipeline
```

`build.rs` discovers libavfilter/libavutil via `pkg-config` (honours
`PKG_CONFIG_PATH`) with the same proven bindgen settings as
`ppdrive-image-conversion`.

### Pipeline

```
input bytes → image_conversion::Frame::decode
           → buffer src → <filter chain> → buffersink   (libavfilter)
           → image_conversion::Frame::encode            (even-dim fix inside)
           → output bytes
```

One graph per call (stateless transformer), zero intermediate Rust
pixel buffers.

## Requirements

- Rust 1.94+ (edition 2024; the MSRV is driven by the `ppdrive`
  dependency tree, not by this crate itself)
- FFmpeg **development** libraries: `libavfilter`, `libavutil`
  - Debian/Ubuntu: `sudo apt install libavfilter-dev libavutil-dev pkg-config`
  - WebP/AVIF *outputs* additionally require `--enable-libwebp` /
    `--enable-libaom` in the FFmpeg build (handled by the compression
    bridge)
- Verified against FFmpeg **6.1.5** (system) and **9.0.2** (`/opt`);
  expected to work on 6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 47 tests
cargo clippy --all-targets
cargo run --example transform -- input.png out.jpg --crop 800:600:0:0 --rotate 90
```

### Switching FFmpeg versions

```bash
# default: system FFmpeg (6.1)
cargo test

# FFmpeg 9.0.2 installed to /opt/ffmpeg-9
PKG_CONFIG_PATH=/opt/ffmpeg-9/lib/pkgconfig \
LD_LIBRARY_PATH=/opt/ffmpeg-9/lib \
cargo test
```

`PKG_CONFIG_PATH` selects headers/libraries at build time;
`LD_LIBRARY_PATH` selects them at runtime for tests and examples.

## Library usage

```rust
use image_transformation::{ImageTransformer, TransformOperation, TransformOptions};
use image_conversion::ImageFormat;

let transformer = ImageTransformer::new()?;
let output = transformer.transform(
    &input_bytes,
    TransformOptions {
        operations: vec![
            TransformOperation::Crop { x: 0, y: 0, width: 800, height: 600 },
            TransformOperation::Rotate { degrees: 90 },
            TransformOperation::Grayscale,
        ],
        custom_filters: Some("vignette=PI/5".to_string()),
        format: Some(ImageFormat::WebP),
        quality: Some(80),
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `operations` | `Vec<TransformOperation>` | `[]` | Typed ops, applied **in order** |
| `custom_filters` | `Option<String>` | `None` | Raw FFmpeg filter chain, appended last |
| `format` | `Option<ImageFormat>` | `None` | `None` = keep input format (PNG fallback if unknown) |
| `quality` | `Option<u8>` | `None` (= 80) | 0–100; clamped to 100, mapped per format (see conversion docs) |

### Operations → filters

| Operation | Filter | Validation |
|-----------|--------|------------|
| `Crop { x, y, width, height }` | `crop=w:h:x:y` | rectangle must lie inside the source, non-zero |
| `Rotate { degrees }` | `transpose=1` / twice / `transpose=2` | only 90, 180, 270 |
| `Flip { horizontal, vertical }` | `hflip`, `vflip` | at least one flag |
| `Pad { left, top, right, bottom, color }` | `pad=iw+l+r:ih+t+b:…:color=…` | non-empty color; all-zero is a no-op |
| `Grayscale` | `hue=s=0` | — |
| `Adjust { brightness, contrast, saturation }` | `format=rgb24\|rgba` → `lutrgb=…` → `hue=s=…` | finite; clamped to `-1..1`, `-1000..1000`, `0..3` |
| `Blur { sigma }` | `gblur=sigma=…` | finite and `> 0` |
| `Sharpen { amount }` | `unsharp=5:5:a:5:5:a` | finite; clamped to `-2..5` |
| `Scale { width, height }` | `scale=w:h` | non-zero |

Invalid arguments return `Error::InvalidInput` **before** any FFmpeg
call. Operations run in vector order, so a `Crop` after a `Rotate`
sees the rotated dimensions.

### Custom filter safety

- NUL bytes are rejected in Rust.
- The graph topology is a single linked path (`buffer → chain →
  buffersink`); standalone source filters (`movie=…`) stay unlinked
  and make `avfilter_graph_config` fail — they cannot hijack the chain
  or inject media.
- Unknown/malformed filter text surfaces as `Error::FfmpegError`.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input image in this FFmpeg |
| `EncoderNotFound` | output encoder missing (e.g. AVIF without libaom) |
| `InvalidInput` | empty/garbage input, out-of-bounds op, bad parameter, NUL in filter string |
| `UnsupportedFormat` | AVIF **input** (FFmpeg has no AVIF demuxer) |

## Example CLI

```
Usage: transform <input> <output> [options]

Options:
  --crop W:H:X:Y             Crop to a rectangle (validated against input size)
  --rotate <90|180|270>      Rotate by quarter turns (lossless transpose)
  --flip <h|v|both>          Mirror horizontally / vertically / both
  --pad L,T,R,B[:COLOR]      Add a border (e.g. 10,10,10,10:#ff0000 or ...,black)
  --grayscale                Desaturate to gray
  --brightness <f>           brightness (-1..=1), default 0
  --contrast <f>             contrast, default 1
  --saturation <f>           saturation (0..3), default 1
  --blur <sigma>             Gaussian blur (sigma > 0)
  --sharpen <amount>         Unsharp masking (-2..=5, clamped)
  --scale WxH                Exact resize (e.g. 640x480)
  --filter <chain>           Raw FFmpeg filter string, applied last
  --format <jpeg|png|webp|avif>
                             Output format (default: keep input format)
  --quality <0-100>          Encoder quality (default: 80)
  -h, --help                 Show this help

Operations are applied in the order they appear on the command line.
```

```bash
cargo run --example transform -- photo.png thumb.jpg \
  --crop 1600:1200:0:0 --scale 800x600 --grayscale --quality 80
```

## ppdrive plugin

`plugin_dispatch` is exported with the same contract as
`ppdrive-image-conversion`: the loader passes a boxed
`(&[u8], &serde_json::Value)`; the JSON deserializes into
`TransformOptions` (snake_case op tags, e.g.
`{"operations":[{"crop":{"x":0,"y":0,"width":800,"height":600}},"grayscale"]}`).

## Notes & limitations

- **AVIF input is not supported** (no FFmpeg AVIF demuxer); AVIF is
  encode-only via `format: Some(ImageFormat::Avif)`.
- Still images only — one frame goes through the graph per call.
- Odd output dimensions are rounded up to even for JPEG/AVIF by the
  compression bridge (`63×47 → 64×48`).
- The `ffi` module is internal; public API is `ImageTransformer`,
  `TransformOptions`, `TransformOperation`, `Error` and
  `ImageFormat` (re-exported from the bridge).

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull
request:

- **Format & Clippy** — `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`
- **Test** — the full suite on Ubuntu 24.04 (system FFmpeg 6.1) and
  macOS (Homebrew FFmpeg), covering both ends of the supported FFmpeg
  range
- **MSRV** — `cargo check` with Rust 1.94.0
- **Fuzz** — a 60-second `cargo fuzz` smoke run; non-blocking, because
  a crash found during fuzzing may be in upstream FFmpeg

## Fuzzing

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd image-transformation
cargo fuzz run fuzz_transform         # smoke run: -- -max_total_time=60
```

## Releases

`.github/workflows/release.yml` publishes artifacts on every `v*` tag:

| Target | Runner | FFmpeg used |
|--------|--------|-------------|
| `x86_64-unknown-linux-gnu` | Ubuntu 24.04 | system FFmpeg 6.1 dev libs |
| `aarch64-apple-darwin` | macOS arm64 | Homebrew FFmpeg |
| `x86_64-apple-darwin` | macOS Intel (`macos-15-intel`) | Homebrew FFmpeg |
| `x86_64-pc-windows-gnu` | Windows + MSYS2 | MSYS2 MinGW FFmpeg |

Each release asset is the plugin cdylib itself (no archive), renamed to
`ppdrive-image-transformation-<os>-<arch>.<ext>` (ppdrive's plugin library
naming), for example
`ppdrive-image-transformation-linux-x86_64.so`. FFmpeg shared libraries are
**not** bundled (licensing) — the target machine needs its own FFmpeg
installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
