# ppdrive-video-conversion

Rust video conversion library that uses **FFmpeg as the native execution
engine**. Rust orchestrates; FFmpeg does all demuxing, decoding, scaling,
encoding and muxing through dynamically linked `libavcodec`,
`libavformat`, `libavutil` and `libswscale`.

- **Formats out:** MP4 (H.264 via libx264), WebM (VP9 via libvpx-vp9)
- **Formats in:** anything FFmpeg can probe with a video stream — MP4,
  WebM, MKV, MOV, AVI, …
- **Audio:** stream-copied when the target container accepts the codec
  (AAC/MP3 into MP4, Vorbis/Opus into WebM), dropped otherwise — audio
  is never transcoded
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no codecs in
  Rust — one `bindgen`-generated FFI layer over the shared libraries

## Architecture

```
src/
├── ffi/      unsafe: bindings (build.rs + bindgen), AVIO/AVFrame/AVCodec
│             RAII wrappers, event-driven demux loop, encode loop
└── video/    safe public API: VideoConverter, ConversionOptions,
              demux → decode → encode pipeline
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
- MP4 out requires an FFmpeg built with `--enable-libx264`, WebM out
  with `--enable-libvpx` (both present in the distro/CI builds; a
  missing encoder surfaces as `Error::EncoderNotFound` from
  `VideoConverter::new`)
- Verified against FFmpeg **6.1** (system) and expected to work on
  6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 22 tests
cargo clippy --all-targets
cargo run --example transcode -- input.mp4 output.webm --quality 85
```

### Switching FFmpeg versions

```bash
# default: system FFmpeg (6.1)
cargo test

# FFmpeg installed elsewhere
PKG_CONFIG_PATH=/opt/ffmpeg-9/lib/pkgconfig \
LD_LIBRARY_PATH=/opt/ffmpeg-9/lib \
cargo test
```

`PKG_CONFIG_PATH` selects headers/libraries at build time;
`LD_LIBRARY_PATH` selects them at runtime for tests and examples.

## Library usage

```rust
use video_conversion::{ConversionOptions, VideoConverter, VideoFormat};

let converter = VideoConverter::new()?;
let output = converter.convert(
    &input_bytes,
    ConversionOptions {
        format: VideoFormat::Mp4,
        quality: 80,
        width: Some(1280),
        height: None,
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `format` | `VideoFormat` | `Mp4` | `Mp4` (H.264) or `WebM` (VP9) |
| `quality` | `u8` | `80` | 0–100; values above 100 are clamped to 100 |
| `width` | `Option<u32>` | `None` | Target width in pixels |
| `height` | `Option<u32>` | `None` | Target height in pixels |

### Quality mapping (per format)

| Format | `quality` 0 → 100 |
|--------|-------------------|
| MP4 | libx264 CRF 51 → 18 (plus `preset=veryfast`) |
| WebM | libvpx-vp9 CRF 63 → 24 (plus `deadline=good`, `cpu-used=4`) |

`crf_for_quality(format, quality)` exposes the mapping publicly.

### Resizing

- `width`/`height` both `None` → keep input dimensions
- only one given → other derived to **preserve aspect ratio**
- both given → **exact** size (aspect ratio not preserved)
- `0` is rejected with `Error::InvalidInput`
- dimensions round **up to even** (H.264/VP9 chroma subsampling),
  e.g. `63×47 → 64×48`

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input video in this FFmpeg |
| `EncoderNotFound` | libx264 or libvpx-vp9 missing from FFmpeg |
| `InvalidInput` | empty input, zero width/height, or a container that would encode no frames |
| `UnsupportedFormat` | input holds no video stream |
| `LimitExceeded(msg)` | a safety limit would be exceeded (used by transformation guards, e.g. `Reverse` frame buffering) |

## Streaming (planned)

`convert_to` is the phase-1 packetized sink: it emits
`VideoEncoderParams` once and every `VideoPacket`
(`pts`/`dts`/`duration`/`time_base`/`is_keyframe`, timestamps in the
source stream time base) for external muxers. A pull-based frame
iterator and progressive/HTTP output are planned for a later phase.

## Example CLI

```
Usage: transcode <input> <output> [options]

Options:
  --format <mp4|webm>       Output format (default: from output extension)
  --quality <0-100>         Quality (default: 80)
  --width <pixels>          Target width
  --height <pixels>         Target height
  -h, --help                Show this help
```

```bash
cargo run --example transcode -- clip.mov clip.webm --quality 70 --width 1280
# clip.mov: 5242880 bytes -> clip.webm: 1310720 bytes (25.0%)
```

## Notes & limitations

- Frame timestamps of the source are preserved in the source time base;
  frames without a timestamp get `index × frame_ticks`.
- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI).
- The `ffi` module is internal; the public surface is
  `VideoConverter`, `ConversionOptions`, `VideoFormat`,
  `VideoStream`/`VideoFrame`/`VideoEncoder` (streaming extension
  points), `crf_for_quality` and `Error`.

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
cd video-conversion
cargo fuzz run fuzz_transcode        # smoke run: -- -max_total_time=60
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
`ppdrive-video-conversion-<os>-<arch>.<ext>` (ppdrive's plugin library
naming), for example
`ppdrive-video-conversion-linux-x86_64.so`. FFmpeg shared libraries are
**not** bundled (licensing) — the target machine needs its own FFmpeg
installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
