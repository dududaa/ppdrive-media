# ppdrive-video-conversion

Rust video conversion library that uses **FFmpeg as the native execution
engine**. Rust orchestrates; FFmpeg does all demuxing, decoding, scaling,
encoding and muxing through dynamically linked `libavcodec`,
`libavformat`, `libavutil` and `libswscale`.

- **Formats out:** MP4, WebM, MOV, MKV and AVI (H.264 via libx264;
  VP9 via libvpx-vp9 for WebM), plus AV1 (libaom-av1) in MP4/WebM/MKV
  and H.265/HEVC (libx265) in MP4/MOV
- **Formats in:** anything FFmpeg can probe with a video stream — MP4,
  WebM, MKV, MOV, AVI, …
- **Audio:** stream-copied when the target container accepts the codec
  (AAC/MP3 into MP4/MOV/AVI, Vorbis/Opus into WebM, either family into
  Matroska), dropped otherwise — audio is never transcoded
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
- MP4/MOV/MKV/AVI out require an FFmpeg built with `--enable-libx264`,
  WebM out with `--enable-libvpx`, AV1 out with `--enable-libaom-av1`
  and HEVC out with `--enable-libx265` (all present in the distro/CI
  builds). libx264/libvpx are checked in `VideoConverter::new`; the
  other encoders are looked up when a conversion requests their format
  and surface as `Error::EncoderNotFound` then
- Verified against FFmpeg **6.1** (system) and expected to work on
  6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 27 tests
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
        ..ConversionOptions::default()
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `format` | `VideoFormat` | `Mp4` | `Mp4`/`Mov`/`Mkv`/`Avi` (H.264), `WebM` (VP9), `Mp4Av1`/`WebMAv1`/`MkvAv1` (AV1), `Mp4Hevc`/`MovHevc` (H.265) |
| `quality` | `u8` | `80` | 0–100; values above 100 are clamped to 100 |
| `width` | `Option<u32>` | `None` | Target width in pixels |
| `height` | `Option<u32>` | `None` | Target height in pixels |
| `scale` | `Option<f32>` | `None` | Scale factor applied when both `width`/`height` are `None`; must be finite and > 0 |
| `effort` | `Option<u8>` | `None` | Encoder effort 0–100; `None` keeps the encoder's defaults (`preset=veryfast` / `cpu-used=4`) |
| `max_bytes` | `Option<u64>` | `None` | Search quality (≤ 8 full re-encodes) for an output at most this many bytes |
| `fps` | `Option<u32>` | `None` | Force a constant output frame rate (1–1000); frames are dropped/duplicated to fit |
| `drop_audio` | `bool` | `false` | Drop the input's audio track instead of stream-copying it |
| `keyframe_interval` | `Option<u32>` | `None` | Maximum keyframe distance in frames (`g` GOP knob); lets segmenters cut on boundaries |

### Quality mapping (per format)

| Format | `quality` 0 → 100 |
|--------|-------------------|
| MP4, MOV, MKV, AVI | libx264 CRF 51 → 18 (plus `preset=veryfast`) |
| WebM | libvpx-vp9 CRF 63 → 24 (plus `deadline=good`, `cpu-used=4`) |
| AV1 (MP4/WebM/MKV) | libaom-av1 CRF 63 → 24 (plus `row-mt=1`, `cpu-used=4`) |
| HEVC (MP4/MOV) | libx265 CRF 51 → 18 (plus `preset=veryfast`) |

`crf_for_quality(format, quality)` exposes the mapping publicly.

### Resizing

- `width`/`height` both `None` → keep input dimensions (or apply
  `scale` when set)
- only one given → other derived to **preserve aspect ratio**
- both given → **exact** size (aspect ratio not preserved); `scale` is
  ignored
- `0` is rejected with `Error::InvalidInput`
- dimensions round **up to even** (H.264/VP9 chroma subsampling),
  e.g. `63×47 → 64×48`
- `scale` rounds each dimension to the nearest pixel and keeps at
  least `1×1`

### Effort

`None` keeps the encoder's defaults. `Some(e)` maps 0 (fastest) → 100
(slowest): x264/x265 step through the nine libx264 presets
(`ultrafast` … `veryslow`), VP9/AV1 set `cpu-used = 8 − e·8/100`
(slower encoders produce smaller files at the same CRF).

### Frame rate

`fps: Some(n)` (1–1000, anything else is `Error::InvalidInput`)
resamples output onto a constant timeline: frames are dropped when
they land on an occupied slot and gaps up to five seconds are filled
by duplicating the previous frame. Audio is untouched. `None` keeps
source timestamps in the source time base.

### Size budget

`max_bytes: Some(n)` binary-searches `quality` so the muxed output is
at most `n` bytes, starting from `quality` and walking down (at most
eight full re-encodes of the input). Failing even at quality 0 returns
`Error::TargetSizeUnreachable`. `convert_to` rejects a size budget —
its packet sink is a side effect and cannot be re-run.

### Audio

The input's audio is stream-copied when the target container accepts
the codec, dropped otherwise (and always with `drop_audio: true`) —
never transcoded.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input video in this FFmpeg |
| `EncoderNotFound` | the requested format's encoder (libx264, libvpx-vp9, libaom-av1 or libx265) missing from FFmpeg |
| `InvalidInput` | empty input, zero width/height, `max_bytes: Some(0)`, `fps` outside 1–1000, `keyframe_interval: Some(0)`, a bad `scale`, a size budget on `convert_to`, or a container that would encode no frames |
| `UnsupportedFormat` | input holds no video stream |
| `TargetSizeUnreachable` | `max_bytes` cannot be met even at quality 0 |
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
  --format <fmt>            Output format: mp4, webm, mov, mkv, avi,
                            mp4av1, webmav1, mkvav1, mp4hevc, movhevc
                            (default: from output extension)
  --quality <0-100>         Quality (default: 80)
  --width <pixels>          Target width
  --height <pixels>         Target height
  --scale <factor>          Scale factor applied when width/height are omitted
  --effort <0-100>          Encoder effort (default: encoder defaults)
  --max-bytes <bytes>       Search quality for an output at most this size
  --fps <1-1000>            Force a constant output frame rate
  --no-audio                 Drop the input's audio track
  -h, --help                Show this help
```

```bash
cargo run --example transcode -- clip.mov clip.webm --quality 70 --width 1280
# clip.mov: 5242880 bytes -> clip.webm: 1310720 bytes (25.0%)
cargo run --example transcode -- in.mp4 out.mp4 --scale 0.5 --max-bytes 200000 --no-audio
```

## Notes & limitations

- Frame timestamps of the source are preserved in the source time
  base when `fps` is `None`; frames without a timestamp get
  `index × frame_ticks`. With `fps` set, output is resampled onto a
  constant-rate timeline as described above.
- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI; codec-qualified variants like AV1/HEVC cannot
  be inferred and must be passed explicitly).
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
