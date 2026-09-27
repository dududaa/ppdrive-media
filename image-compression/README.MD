# ppdrive-image-compression

Rust image compression library that uses **FFmpeg as the native execution
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
└── image/    safe public API: ImageCompressor, CompressionOptions,
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
cargo test          # 28 tests
cargo clippy --all-targets
cargo run --example compress -- input.png output.webp --quality 85
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
use image_compression::{CompressionOptions, ImageCompressor, ImageFormat};

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

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `format` | `ImageFormat` | `Jpeg` | `Jpeg`, `Png`, `WebP` or `Avif` |
| `quality` | `u8` | `80` | 0–100; values above 100 are clamped to 100 |
| `width` | `Option<u32>` | `None` | Target width in pixels |
| `height` | `Option<u32>` | `None` | Target height in pixels |

### Quality mapping (per format)

| Format | `quality` 0 → 100 |
|--------|-------------------|
| JPEG | QSCALE 31 → 2 (FFmpeg's valid range is 2–31) |
| PNG | zlib compression level 0 → 9 (always lossless) |
| WebP | lossy quality 0 → 99; exactly **100 switches to lossless** |
| AVIF | AV1 CRF 63 → 0 (row-multithreaded encoding) |

### Resizing

- `width`/`height` both `None` → keep input dimensions
- only one given → other derived to **preserve aspect ratio**
- both given → **exact** size (aspect ratio not preserved)
- `0` is rejected with `Error::InvalidInput`
- JPEG/AVIF round dimensions **up to even** (chroma subsampling),
  e.g. `63×47 → 64×48`

### Alpha

Preserved for **PNG** and **WebP** outputs when the input has an alpha
channel; JPEG and AVIF outputs are opaque.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input image in this FFmpeg |
| `EncoderNotFound` | output encoder missing (e.g. AVIF without libaom) |
| `InvalidInput` | empty input or zero width/height option |
| `UnsupportedFormat` | AVIF **input** (FFmpeg has no AVIF demuxer), unknown output extension |

## Example CLI

```
Usage: compress <input> <output> [options]

Options:
  --format <jpeg|png|webp|avif>  Output format (default: from output extension)
  --quality <0-100>              Compression quality (default: 80)
  --width <pixels>               Target width
  --height <pixels>              Target height
  -h, --help                     Show this help
```

```bash
cargo run --example compress -- photo.jpg thumb.avif --quality 60 --width 800
# photo.jpg: 245760 bytes -> thumb.avif: 18432 bytes (7.5%)
```

## Notes & limitations

- **AVIF input is not supported** — FFmpeg ships an AVIF muxer but no
  demuxer; such input is rejected with `UnsupportedFormat`.
- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI).
- Single images only — no animated GIF/WebP outputs.
- The `ffi` module is internal; only `ImageCompressor`,
  `CompressionOptions`, `ImageFormat` and `Error` are public.

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
cd image-compression
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

Artifacts follow `ppdrive-<package-name>-<os>-<arch>.<ext>` (`.zip`
on Windows, `.tar.gz` otherwise), for example
`ppdrive-image-compression-linux-x86_64.tar.gz`. Each archive
contains the `libimage_compression` ppdrive plugin cdylib, the
`compress` example CLI and this README. FFmpeg shared libraries are
**not** bundled (licensing) — the target machine needs its own
FFmpeg installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
