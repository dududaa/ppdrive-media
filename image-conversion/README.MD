# ppff-image-conversion

Rust image conversion library that uses **FFmpeg as the native execution
engine**. Rust orchestrates; FFmpeg does all decoding, resizing, pixel
conversion and encoding through dynamically linked `libavcodec`,
`libavformat`, `libavutil` and `libswscale`.

- **Formats out:** JPEG, PNG, WebP, AVIF
- **Formats in:** anything FFmpeg can probe from content — JPEG, PNG,
  WebP, GIF, BMP, TIFF, …
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no codecs in
  Rust — one `bindgen`-generated FFI layer over the shared libraries

## Architecture

```
src/
├── ffi/      unsafe: bindings (build.rs + bindgen), AVIO/AVFrame/AVCodec
│             RAII wrappers, demux/encode loops
└── image/    safe public API: ImageConverter, ConversionOptions,
              decode → resize → encode pipeline
```

`build.rs` discovers FFmpeg via `pkg-config` (honours `PKG_CONFIG_PATH`)
and emits version cfgs so one crate builds across FFmpeg 6.x → 9.x.

## Requirements

- Rust 1.94+ (edition 2024; the MSRV is driven by the `ppdrive`
  dependency tree, not by this crate itself)
- FFmpeg **development** libraries: `libavcodec`, `libavformat`,
  `libavutil`, `libswscale`
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev
    libavutil-dev libswscale-dev pkg-config`
  - AVIF out requires an FFmpeg built with `--enable-libaom`,
    WebP out with `--enable-libwebp`
- Verified against FFmpeg **6.1.5** (system) and **9.0.2** (`/opt`);
  expected to work on 6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 49 tests
cargo clippy --all-targets
cargo run --example convert -- input.png output.webp --quality 85
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
use ppff_image_conversion::{ConversionOptions, ImageConverter, ImageFormat};

let converter = ImageConverter::new()?;
let output = converter.convert(
    &input_bytes,
    ConversionOptions {
        format: ImageFormat::WebP,
        quality: 80,
        width: Some(800),
        height: Some(600),
        ..Default::default()
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `format` | `ImageFormat` | `Jpeg` | `Jpeg`, `Png`, `WebP` or `Avif` |
| `quality` | `u8` | `80` | 0–100; values above 100 are clamped to 100 |
| `width` | `Option<u32>` | `None` | Target width in pixels |
| `height` | `Option<u32>` | `None` | Target height in pixels |
| `scale` | `Option<f32>` | `None` | Proportional resize factor (used only when `width`/`height` are both `None`) |
| `effort` | `Option<u8>` | `None` | Encoding effort 0–100; AVIF only (see below) |
| `max_bytes` | `Option<u64>` | `None` | Shrink quality until the output fits this size |

### Quality mapping (per format)

| Format | `quality` 0 → 100 |
|--------|-------------------|
| JPEG | QSCALE 31 → 2 (FFmpeg's valid range is 2–31) |
| PNG | zlib compression level 0 → 9 (always lossless) |
| WebP | lossy quality 0 → 99; exactly **100 switches to lossless** |
| AVIF | AV1 CRF 63 → 0 (row-multithreaded encoding) |

### Resizing

- `width`/`height` both `None` and no `scale` → keep input dimensions
- only `width` or only `height` given → other derived to **preserve aspect ratio**
- both given → **exact** size (aspect ratio not preserved)
- `scale: Some(s)` with neither dimension set → `round(source × s)`,
  minimum 1 px; explicit dimensions win over `scale`
- non-finite, zero or negative scale, and `0` dimensions, are rejected
  with `Error::InvalidInput`
- JPEG/AVIF round dimensions **up to even** (chroma subsampling),
  e.g. `63×47 → 64×48`

### Effort

`effort: 0–100` (clamped), higher is slower and usually smaller at
equal quality. Only **AVIF** honours it — it maps to AV1 `cpu-used`
(`8` at effort 0 → `0` at effort 100). JPEG, PNG and WebP expose no
effort knob through their FFmpeg encoders and ignore the value.
`None` leaves the encoder default (AV1 `cpu-used` 1).

### Size budget

With `max_bytes: Some(n)` the converter encodes at `quality` first and,
if too large, binary-searches lower qualities for the highest one that
fits (at most eight encodes total; the image is resized only once).
`Some(0)` is rejected with `InvalidInput`; when even quality 0 cannot
fit, `TargetSizeUnreachable` is returned.

### Alpha

Preserved for **PNG** and **WebP** outputs when the input has an alpha
channel; JPEG and AVIF outputs are opaque.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input image in this FFmpeg |
| `EncoderNotFound` | output encoder missing (e.g. AVIF without libaom) |
| `InvalidInput` | empty input, zero width/height, zero `max_bytes`, or invalid `scale` |
| `UnsupportedFormat` | AVIF **input** (FFmpeg has no AVIF demuxer), unknown output extension |
| `TargetSizeUnreachable` | `max_bytes` cannot be met even at quality 0 |

## Example CLI

```
Usage: convert <input> <output> [options]

Options:
  --format <jpeg|png|webp|avif>  Output format (default: from output extension)
  --quality <0-100>              Quality (default: 80)
  --width <pixels>               Target width
  --height <pixels>              Target height
  --scale <factor>               Proportional resize (e.g. 0.5; ignored if
                                 --width/--height is given)
  --effort <0-100>               Encoding effort, AVIF only (default: encoder default)
  --max-bytes <bytes>            Shrink quality until the output fits this size
  -h, --help                     Show this help
```

```bash
cargo run --example convert -- photo.jpg thumb.avif --quality 60 --width 800
# photo.jpg: 245760 bytes -> thumb.avif: 18432 bytes (7.5%)
cargo run --example convert -- photo.jpg icon.avif --quality 80 --scale 0.25 --max-bytes 20480
```

## Notes & limitations

- **AVIF input is not supported** — FFmpeg ships an AVIF muxer but no
  demuxer; such input is rejected with `UnsupportedFormat`.
- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI).
- Single images only — no animated GIF/WebP outputs.
- The `ffi` module is internal; only `ImageConverter`,
  `ConversionOptions`, `Frame`, `ImageFormat` and `Error` are public.

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
cd image-conversion
cargo fuzz run fuzz_decode            # smoke run: -- -max_total_time=60
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
`ppff-image-conversion-<os>-<arch>.<ext>` (ppdrive's plugin library
naming), for example
`ppff-image-conversion-linux-x86_64.so`. FFmpeg shared libraries are
**not** bundled (licensing) — the target machine needs its own FFmpeg
installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
