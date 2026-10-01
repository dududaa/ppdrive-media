# ppff-audio-conversion

Rust audio transcoding library that uses **FFmpeg as the native execution
engine**. Rust orchestrates; FFmpeg does all decoding, resampling and
encoding through dynamically linked `libavcodec`, `libavformat`,
`libavutil` and `libswresample`.

- **Formats out:** WAV, MP3, FLAC, AAC (ADTS), Ogg Vorbis, Opus
- **Formats in:** anything FFmpeg can probe with an audio stream — WAV,
  MP3, FLAC, AAC, Ogg, Opus, M4A, AIFF, …
- **No reimplementation:** no `ffmpeg-next`, no `dlopen`, no codecs in
  Rust — one `bindgen`-generated FFI layer over the shared libraries

## Architecture

```
src/
├── ffi/      unsafe: bindings (build.rs + bindgen), AVIO/AVFrame/AVCodec
│             RAII wrappers, demux/resample/encode loops
└── audio/    safe public API: AudioConverter, ConversionOptions,
              decode → resample → encode pipeline
```

`build.rs` discovers FFmpeg via `pkg-config` (honours `PKG_CONFIG_PATH`)
and emits version cfgs so one crate builds across FFmpeg 6.x → 9.x.

## Requirements

- Rust 1.94+ (edition 2024; the MSRV is driven by the `ppdrive`
  dependency tree, not by this crate itself)
- FFmpeg **development** libraries: `libavcodec`, `libavformat`,
  `libavutil`, `libswresample`
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev
    libavutil-dev libswresample-dev pkg-config`
- MP3 out requires an FFmpeg built with `--enable-libmp3lame`,
  Ogg out with `--enable-libvorbis`, Opus out with `--enable-libopus`
  (all present in the distro/CI builds; a missing encoder surfaces as
  `Error::EncoderNotFound` for that request only)
- Verified against FFmpeg **6.1** (system) and expected to work on
  6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 21 tests
cargo clippy --all-targets
cargo run --example convert -- input.wav output.mp3 --quality 75
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
use ppff_audio_conversion::{AudioConverter, AudioFormat, ConversionOptions};

let converter = AudioConverter::new()?;
let output = converter.convert(
    &input_bytes,
    ConversionOptions {
        format: AudioFormat::Mp3,
        quality: 75,
        sample_rate: None,
        channels: Some(1),
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `format` | `AudioFormat` | `Mp3` | `Wav`, `Mp3`, `Flac`, `Aac`, `Ogg` or `Opus` |
| `quality` | `u8` | `80` | 0–100; values above 100 are clamped to 100 |
| `sample_rate` | `Option<u32>` | `None` | Target rate in Hz; `None` keeps the source rate |
| `channels` | `Option<u8>` | `None` | `Some(1)` or `Some(2)`; `None` keeps mono/stereo and downmixes >2 to stereo |

### Quality mapping (per format)

| Format | `quality` 0 → 100 |
|--------|-------------------|
| WAV | ignored (always PCM s16) |
| FLAC | ignored (always lossless 16-bit) |
| MP3 | 32 → 320 kbps |
| AAC | 64 → 256 kbps |
| Ogg | 32 → 320 kbps |
| Opus | 16 → 256 kbps |

`bitrate_for_quality(format, quality) -> Option<u32>` is public so
callers (and the future streaming crate) can compute the same targets.

### Sample rate & channel rules

- **Opus** always encodes at 48000 Hz regardless of request.
- **MP3** and **AAC** snap to their standard rate tables (nearest
  supported rate); **WAV/FLAC/Ogg** honour any rate.
- `sample_rate: Some(0)` or `channels: Some(0)` or `channels: Some(_)`
  above 2 → `Error::InvalidInput`.
- Multichannel input (>2) is downmixed to stereo when the target is a
  stereo-only codec.

### Pts / duration contract

- Decoded frames carry pts as **cumulative sample offsets** at the
  source rate (time base `1/source_rate`); the first frame starts at 0.
- Encoded packets carry **sequential pts** in the encoder time base
  `1/output_rate`, assigned by the encoder path — downstream pts from
  the container are never trusted or forwarded.
- For lossy codecs the decoded-back duration can differ from the input
  by codec priming/padding (tens of milliseconds); WAV and FLAC are
  sample-exact.

## Streaming (planned)

HLS/DASH packaging is **not implemented yet**; the extension points it
needs are already in place so a future `media-streaming` crate can build
on this one without API breakage:

- `DecodedAudio::encode_to(format, quality, global_headers, sink)` —
  streams `EncoderParams` (once) and `EncodedPacket`s to a caller sink
  instead of assembling one muxed byte vector. The structured
  packet-package → ustar transport maps directly onto this.
- `EncoderParams` exposes `extradata`, sample format, rate, channels,
  bitrate and time base — everything a muxer needs to open rendition
  streams.
- `bitrate_for_quality` lets the streaming crate pick rendition
  bitrates with the exact same quality ladder.
- The pts/duration contract above is what segment boundaries will be
  computed from.

The design target: FFmpeg `hls`/`dash` muxers with multiple rendition
streams (`var_stream_map` / `adaptation_sets`) written through AVIO into
a temp directory, returned as a structured package of files.

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message) |
| `DecoderNotFound` | no decoder for the input stream in this FFmpeg |
| `EncoderNotFound` | output encoder missing (e.g. libmp3lame build without lame) |
| `InvalidInput` | empty input, `sample_rate: 0` or `channels` outside 1–2 |
| `UnsupportedFormat` | input has no audio stream, unknown output extension |

## Example CLI

```
Usage: convert <input> <output> [options]

Options:
  --format <wav|mp3|flac|aac|ogg|opus>
                             Output format (default: from output extension)
  --quality <0-100>          Encoding quality (default: 80)
  --sample-rate <hz>         Target sample rate (default: keep source)
  --channels <1|2>           Target channel count (default: keep/downmix)
  -h, --help                 Show this help
```

```bash
cargo run --example convert -- tone.wav tone.mp3 --quality 60
# tone.wav: 176478 bytes -> tone.mp3: 25748 bytes
```

## Notes & limitations

- Input format is sniffed from content; extensions on the input are
  irrelevant. Output format comes from `options.format` (or the file
  extension in the CLI).
- Encoders with a fixed frame size (MP3 1152, AAC 1024, FLAC) are fed
  exact-size frames with the tail padded on the final frame.
- The `ffi` module is internal; `AudioConverter`, `ConversionOptions`,
  `AudioFormat`, `DecodedAudio`, `bitrate_for_quality`, `EncoderParams`,
  `EncodedPacket`, `AudioStreamParams` and `Error` are public.
- Encoder availability is checked per request, not at `AudioConverter::new()`.

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull
request:

- **Format & Clippy** — `cargo fmt --all --check` and
  `cargo clippy --workspace --all-targets -- -D warnings`
- **Test** — the full suite on Ubuntu 24.04 (system FFmpeg 6.1) and
  macOS (Homebrew FFmpeg), covering both ends of the supported FFmpeg
  range; the release workflow additionally builds Windows (MSYS2)
- **MSRV** — `cargo check` with Rust 1.94.0
- **Fuzz** — a 60-second `cargo fuzz` smoke run; non-blocking, because
  a crash found during fuzzing may be in upstream FFmpeg

## Fuzzing

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd audio-conversion
cargo fuzz run fuzz_convert         # smoke run: -- -max_total_time=60
```

The fuzzer feeds arbitrary bytes with a selector byte controlling
format/quality/options and expects no panics, aborts or leaks.

## Releases

`.github/workflows/release.yml` publishes artifacts on every `v*` tag:

| Target | Runner | FFmpeg used |
|--------|--------|-------------|
| `x86_64-unknown-linux-gnu` | Ubuntu 24.04 | system FFmpeg 6.1 dev libs |
| `aarch64-apple-darwin` | macOS arm64 | Homebrew FFmpeg |
| `x86_64-apple-darwin` | macOS Intel (`macos-15-intel`) | Homebrew FFmpeg |
| `x86_64-pc-windows-gnu` | Windows + MSYS2 | MSYS2 MinGW FFmpeg |

Each release asset is the plugin cdylib itself (no archive), renamed to
`ppff-audio-conversion-<os>-<arch>.<ext>` (ppdrive's plugin library
naming), for example `ppff-audio-conversion-linux-x86_64.so`. FFmpeg
shared libraries are **not** bundled (licensing) — the target machine
needs its own FFmpeg installation at runtime.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
