# ppff-media-streaming

Rust adaptive-streaming library that packages media into **HLS** and
**MPEG-DASH** VOD streams with **FFmpeg as the native execution
engine**. Rust orchestrates; FFmpeg does all decoding, encoding and
segmentation through dynamically linked `libavcodec`, `libavformat`
and `libavutil`.

- **Protocols out:** HLS (`master.m3u8` + variant playlists over
  MPEG-TS segments) and DASH (`manifest.mpd` over fragmented-MP4
  segments)
- **Renditions:** an auto ladder derived from the source resolution,
  or an explicit `--rendition` list (scale / dimensions / bitrate)
  with shared AAC audio
- **Inputs:** anything FFmpeg can probe — video inputs become an
  adaptive ladder, audio-only inputs a single audio rendition
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no codecs in
  Rust — one `bindgen`-generated FFI layer over the shared libraries,
  reusing `video-conversion` and `audio-conversion` for encoding

## Architecture

```
src/
├── ffi/        unsafe: bindings (build.rs + bindgen), codec_id and
│               extradata helpers, error mapping
├── session.rs  one open hls/dash AVFormatContext: stream declaration,
│               header options, packet writing, trailer
├── ladder.rs   auto/manual rendition ladder resolution
├── options.rs  StreamingOptions / StreamingProtocol / RenditionSpec
└── lib.rs      MediaStreamer: decode once → encode N renditions →
                timestamp-merged audio → playlist output
```

`build.rs` discovers FFmpeg via `pkg-config` (honours
`PKG_CONFIG_PATH`) and generates bindings with `bindgen`, so one
crate builds across FFmpeg 6.x → 9.x.

## Requirements

- Rust 1.94+ (edition 2024)
- FFmpeg **development** libraries: `libavcodec`, `libavformat`,
  `libavutil`
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev
    libavutil-dev pkg-config`
- The linked FFmpeg build must provide the `hls` and `dash` muxers
  (checked in `MediaStreamer::new`) — present in distro and CI builds
- Ladder codecs are H.264 (libx264) and AAC, so an FFmpeg with
  `--enable-libx264` is required (present in distro/CI builds)
- Verified against FFmpeg **6.1** (system) and expected to work on
  6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 16 tests
cargo clippy --all-targets
cargo run --example package -- input.mp4 out_dir --segment-duration 4
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
use ppff_media_streaming::{MediaStreamer, StreamingOptions, StreamingProtocol};

let streamer = MediaStreamer::new()?;
let output = streamer.stream_file(
    std::path::Path::new("input.mp4"),
    std::path::Path::new("./out"),
    &StreamingOptions {
        protocol: StreamingProtocol::Hls,
        segment_duration: 4,
        ..StreamingOptions::default()
    },
)?;
println!("{}", output.playlist.display()); // out/master.m3u8
for file in &output.files {
    println!("{}", file.display());
}
```

`stream` takes bytes instead of a path; both create the output
directory when missing and return the master playlist plus every file
written (`StreamingOutput`).

## Options

| Option | Default | Meaning |
|--------|---------|---------|
| `protocol` | `Hls` | `Hls` (`master.m3u8`, TS segments) or `Dash` (`manifest.mpd`, fMP4 segments) |
| `segment_duration` | `4` | Target seconds per segment (1–30) |
| `quality` | `80` | Base encoder quality 0–100 for every rendition and the audio track |
| `renditions` | auto | Explicit ladder; `None` derives one from the source resolution |

### Ladder

The auto ladder never upscales: it keeps the source height when it is
below 1080p and adds every standard rung (720p, 480p, 360p, 240p) at
or below it, highest first. Each rung gets a nominal bitrate from a
height table (1080p: 5 Mbit/s … 240p: 400 kbit/s), which feeds the
playlist's bandwidth attribute.

A manual `RenditionSpec` resolves like video-conversion's sizing
rules: explicit `width`/`height` win, `scale` applies when both are
`None`, and `None`/`None`/`None` keeps the source dimensions. Manual
specs may upscale (e.g. `scale: 2.0`). `bitrate` overrides the
derived value; `quality` overrides `StreamingOptions::quality` per
rung.

### Keyframe alignment

Video is encoded with a GOP of `fps × segment_duration` so keyframes
land on segment boundaries; the HLS muxer additionally signals
`#EXT-X-INDEPENDENT-SEGMENTS`.

## Errors

`ppff_media_streaming::Error` mirrors the sibling crates:

- `FfmpegError(String)` — an FFmpeg call failed (`av_strerror` text)
- `InvalidInput` — empty input, bad options, unusable paths
- `UnsupportedFormat` — the input has no stream we can package, or
  the linked FFmpeg lacks an `hls`/`dash` muxer
- `EncoderNotFound` — libx264/AAC encoder missing from the build
- `Io(String)` — output directory could not be created or read

## Example CLI

```bash
cargo run --example package -- input.mp4 out_dir \
  --protocol hls \
  --segment-duration 4 \
  --quality 85 \
  --rendition 640x360,800000 \
  --rendition 1280x720,2800000
```

Prints the master playlist followed by every file written.

## Notes & limitations

- **VOD only** (`hls_playlist_type=vod`, DASH `window_size=0`); no
  live/low-latency mode.
- Ladder codecs are H.264 + AAC. Inputs with other codecs are
  transcoded.
- HLS copies the audio rendition once per variant (the FFmpeg HLS
  muxer rejects a shared elementary stream across variants); DASH
  shares one audio stream in its own adaptation set.
- Audio is encoded once into memory and merged into the output by
  timestamp, so very long audio inputs hold their packets in RAM.
- `StreamingOutput.files` is a directory scan of the output
  directory filtered to media extensions (`m3u8`, `mpd`, `ts`, `m4s`,
  `mp4`, `aac`), sorted by name.
- The `ffi` and `session` modules are internal; the public surface is
  `MediaStreamer`, `StreamingOutput`, `StreamingOptions`,
  `StreamingProtocol`, `RenditionSpec` and `Error`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull
request, covering this crate through the workspace:

- **Format & Clippy** — `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`
- **Test** — the full suite on Ubuntu 24.04 (system FFmpeg 6.1) and
  macOS (Homebrew FFmpeg)
- **MSRV** — `cargo check` with Rust 1.94.0

## Plugin

`media-streaming/plugin` (`ppff-media-streaming-plugin`) is the
ppdrive cdylib entry point. It mirrors the other plugins' C ABI —
`plugin_dispatch(args: *mut c_void) -> *mut DispatchResponse` — but
is **file-based** instead of byte-based, because one call produces
many files:

- **Args**: a boxed `(&Path, &Path, &serde_json::Value)` — input
  file path, output directory path, options JSON (deserializes into
  `StreamingOptions`).
- **Response**: `Ok` payload is a boxed `Vec<u8>` of UTF-8 JSON —
  `{"playlist": "...", "files": ["...", ...]}` — describing the
  package written to the output directory; `Error` carries the
  message string.

## Releases

`.github/workflows/release.yml` publishes artifacts on every `v*` tag:

| Target | Runner | FFmpeg used |
|--------|--------|-------------|
| `x86_64-unknown-linux-gnu` | Ubuntu 24.04 | system FFmpeg 6.1 dev libs |
| `aarch64-apple-darwin` | macOS arm64 | Homebrew FFmpeg |
| `x86_64-apple-darwin` | macOS Intel (`macos-15-intel`) | Homebrew FFmpeg |
| `x86_64-pc-windows-gnu` | Windows + MSYS2 | MSYS2 MinGW FFmpeg |

Each release asset is the plugin cdylib itself (no archive), renamed to
`ppff-media-streaming-<os>-<arch>.<ext>` (ppdrive's plugin library
naming). FFmpeg shared libraries are **not** bundled (licensing) —
the target machine needs its own FFmpeg installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
