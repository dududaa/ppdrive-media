# ppdrive-video-transformation

Rust video transformation library that uses **FFmpeg's libavfilter as
the native execution engine**. Rust validates and orchestrates; FFmpeg
performs all pixel work in a single `buffer → filter chain → buffersink`
graph pass over the decoded frames. Demuxing, decoding, encoding and
muxing are reused from the `ppff-video-conversion` crate via its
public `VideoStream`/`VideoEncoder` bridge — no codec/AVIO code is
duplicated here.

- **Operations:** crop, scale, rotate (90/180/270), flip, pad,
  grayscale, brightness/contrast/saturation, blur, sharpen, trim,
  speed, reverse
- **Escape hatch:** raw FFmpeg filter strings (`custom_filters`)
- **Formats in/out:** anything FFmpeg demuxes in; output MP4, WebM,
  MOV, MKV or AVI (H.264/VP9), plus AV1 (MP4/WebM/MKV) and H.265/HEVC
  (MP4/MOV), defaulting to the source container (MP4/MOV → Mp4,
  WebM/Matroska → WebM)
- **Audio:** stream-copied when the target container accepts its
  codec; dropped for `Speed`/`Reverse` (no audio transcoding)
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no pixel
  loops in Rust — one `bindgen`-generated FFI layer over
  `libavfilter` + `libavutil`

## Architecture

```
src/
├── ffi/      unsafe: bindgen bindings (build.rs), AVFilterGraph /
│             sink AVFrame RAII, graph build/push/pull/close
└── video/    safe public API: VideoTransformer, TransformOptions,
              operation → filter-string builder, validation, the
              Reverse safety guard and the transform pipeline
```

`build.rs` discovers libavfilter/libavutil via `pkg-config` (honours
`PKG_CONFIG_PATH`) with the same proven bindgen settings as
`ppdrive-image-transformation`.

### Pipeline

```
input bytes → ppff_video_conversion::VideoStream (demux + decode + probe)
           → window filter (Trim, source timeline, video + audio)
           → buffer src → <filter chain> → buffersink   (libavfilter)
           → ppff_video_conversion::VideoEncoder::muxed      (even-dim fix inside)
           → output bytes
```

One graph per call (stateless transformer), zero intermediate Rust
pixel buffers. The graph's negotiated sink size/time base is queried
and fed to the encoder, so filter-driven dimension changes (crop,
scale, rotate, pad) are muxed correctly.

## Requirements

- Rust 1.94+ (edition 2024; the MSRV is driven by the `ppdrive`
  dependency tree, not by this crate itself)
- FFmpeg **development** libraries: `libavfilter`, `libavutil` plus
  everything the `ppff-video-conversion` bridge needs
  (`libavformat`, `libavcodec`, `libswscale`, `libswresample`)
  - Debian/Ubuntu: `sudo apt install libavfilter-dev libavutil-dev
    libavformat-dev libavcodec-dev libswscale-dev libswresample-dev
    pkg-config`
  - MP4/MOV/MKV/AVI output needs FFmpeg built with `libx264`, WebM
    output with `libvpx-vp9`, AV1 output with `libaom-av1` and HEVC
    output with `libx265` (all present in the standard builds)
- Verified against FFmpeg **6.1.5** (system) and **9.0.2** (`/opt`);
  expected to work on 6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 33 tests
cargo clippy --all-targets
cargo run --example edit -- input.mp4 out.webm --crop 0,0,160,90 --grayscale
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
use video_transformation::{TransformOperation, TransformOptions, VideoTransformer};
use ppff_video_conversion::VideoFormat;

let transformer = VideoTransformer::new()?;
let output = transformer.transform(
    &input_bytes,
    TransformOptions {
        operations: vec![
            TransformOperation::Trim { start: 1.0, duration: Some(4.0) },
            TransformOperation::Scale { width: 1280, height: 720 },
            TransformOperation::Grayscale,
        ],
        custom_filters: Some("vignette=PI/5".to_string()),
        format: Some(VideoFormat::WebM),
        quality: Some(75),
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `operations` | `Vec<TransformOperation>` | `[]` | Typed ops, applied **in order** |
| `custom_filters` | `Option<String>` | `None` | Raw FFmpeg filter chain, appended last |
| `format` | `Option<VideoFormat>` | `None` | `None` = keep source container (MP4/MOV → Mp4, WebM/Matroska → WebM, fallback Mp4); any `VideoFormat` variant can be set explicitly |
| `quality` | `Option<u8>` | `None` (= 80) | 0–100; mapped to CRF per format (see `ppff-video-conversion`) |

### Operations → filters

| Operation | Filter / mechanism | Validation |
|-----------|--------------------|------------|
| `Crop { x, y, width, height }` | `crop=w:h:x:y` | rectangle must lie inside the source, non-zero |
| `Scale { width, height }` | `scale=w:h` | non-zero |
| `Rotate { degrees }` | `transpose=1` / twice / `transpose=2` | only 90, 180, 270 |
| `Flip { horizontal, vertical }` | `hflip`, `vflip` | at least one flag |
| `Pad { left, top, right, bottom, color }` | `pad=iw+l+r:ih+t+b:…:color=…` | non-empty color; all-zero is a no-op |
| `Grayscale` | `hue=s=0` | — |
| `Adjust { brightness, contrast, saturation }` | `eq=brightness=…:contrast=…:saturation=…` (falls back to `format=rgb24,lutrgb=…,hue=s=…` when `eq` is absent) | finite; clamped to `-1..1`, `-1000..1000`, `0..3` |
| `Blur { sigma }` | `gblur=sigma=…` | finite and `> 0` |
| `Sharpen { amount }` | `unsharp=5:5:a:5:5:a` | finite; clamped to `-2..5` |
| `Trim { start, duration }` | window filter in Rust (video **and** audio), re-based to start at zero | finite, `start ≥ 0`, `duration ≥ 0`; multiple trims intersect and must overlap |
| `Speed { factor }` | `setpts=PTS/factor` | finite and `> 0`; audio is dropped |
| `Reverse` | `reverse` | audio is dropped; see the buffering guard below |

Invalid arguments return `Error::InvalidInput` **before** the filter
graph or encoder is built. Operations run in vector order, so a `Crop`
after a `Rotate` sees the rotated dimensions. In-graph filters
(`setpts`, `reverse`) rewrite timestamps, so a trimmed clip is only
re-based to zero when no such filter is present.

### Custom filter safety

- NUL bytes are rejected in Rust.
- The graph topology is a single linked path (`buffer → chain →
  buffersink`); standalone source filters (`movie=…`) stay unlinked
  and make `avfilter_graph_config` fail — they cannot hijack the chain
  or inject media.
- Unknown/malformed filter text surfaces as `Error::FfmpegError`.

### Reverse buffering guard

`reverse` must buffer the whole clip as decoded frames. Before the
graph is built the estimated buffer
(`width × height × fps × effective_duration × 2`) is checked against
**1 GiB**; the effective duration is the trim window when a `Trim`
is present. Over the limit (or when the input reports no duration)
returns `Error::LimitExceeded` with the estimate in the message.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input video in this FFmpeg |
| `EncoderNotFound` | libx264 or libvpx-vp9 missing from FFmpeg |
| `InvalidInput` | empty/garbage input, out-of-bounds op, bad parameter, NUL in filter string, disjoint trims |
| `UnsupportedFormat` | input holds no video stream |
| `LimitExceeded(msg)` | a safety limit would be exceeded (e.g. `Reverse` frame buffering) |

## Example CLI

```
Usage: edit <input> <output> [options]

Options (applied in the order given):
  --crop <x,y,w,h>           Crop rectangle
  --scale <width>x<height>   Scale to an exact size
  --rotate <90|180|270>      Rotate clockwise
  --flip <h|v|both>          Mirror horizontally and/or vertically
  --pad <l,t,r,b[:color]>    Pad borders with a color (default black)
  --grayscale                Convert to grayscale
  --brightness <-1..1>       Adjust brightness (default 0.0)
  --contrast <n>             Adjust contrast (default 1.0)
  --saturation <0..3>        Adjust saturation (default 1.0)
  --blur <sigma>             Gaussian blur
  --sharpen <-2..5>          Sharpen amount
  --trim <start>[,<duration>]  Keep only that window of the source
  --speed <factor>           Playback speed (audio is dropped)
  --reverse                  Reverse playback (audio is dropped)
  --filter <chain>           Raw libavfilter chain, applied last
  --format <fmt>            Output format: mp4, webm, mov, mkv, avi,
                            mp4av1, webmav1, mkvav1, mp4hevc, movhevc
                            (default: from output extension)
  --quality <0-100>          Output quality (default: 80)
  -h, --help                 Show this help
```

```bash
cargo run --example edit -- clip.mov clip.webm \
  --trim 0.5,10 --scale 1280x720 --grayscale --quality 70
# clip.mov: 5242880 bytes -> clip.webm: 1153433 bytes
```

## ppdrive plugin

`plugin_dispatch` is exported with the same contract as the other
ppdrive media plugins: the loader passes a boxed
`(&[u8], &serde_json::Value)`; the JSON deserializes into
`TransformOptions` (snake_case op tags, e.g.
`{"operations":[{"crop":{"x":0,"y":0,"width":640,"height":360}},"grayscale"]}`).
The cdylib lives in `video-transformation` itself (`crate-type =
["cdylib", "lib"]`), unlike `ppff-video-conversion`, whose plugin
is the separate `plugin/` package.

## Notes & limitations

- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI).
- Only MP4 (H.264) and WebM (VP9) outputs; quality maps to CRF the
  same way as `ppff-video-conversion`.
- `Speed`/`Reverse` drop the audio track; `Trim` keeps and trims it.
  Audio is never transcoded — kept tracks are stream-copied and
  dropped when the target container cannot carry their codec.
- Odd filter-negotiated dimensions are rounded up to even for the
  encoder (H.264/VP9 chroma subsampling).
- The `ffi` module is internal; the public surface is
  `VideoTransformer`, `TransformOptions`, `TransformOperation`,
  `Error` and `VideoFormat` (re-exported from the bridge).

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
cd video-transformation
cargo fuzz run fuzz_edit             # smoke run: -- -max_total_time=60
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
`ppdrive-video-transformation-<os>-<arch>.<ext>` (ppdrive's plugin library
naming), for example
`ppdrive-video-transformation-linux-x86_64.so`. FFmpeg shared libraries are
**not** bundled (licensing) — the target machine needs its own FFmpeg
installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
