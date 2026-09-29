# ppdrive-audio-effects

Rust audio effects library that uses **FFmpeg's libavfilter as the
native execution engine**. Rust orchestrates and validates; FFmpeg
performs all sample work in a single
`abuffer → filter chain → abuffersink` graph pass, linked against
dynamically shared `libavfilter`/`libavutil` (decoding and encoding are
reused from `ppdrive-audio-conversion`).

- **Input:** anything FFmpeg can probe with an audio stream — WAV, MP3,
  FLAC, AAC, Ogg, Opus, M4A, AIFF, …
- **Output:** WAV by default (lossless PCM after processing), or MP3,
  FLAC, AAC (ADTS), Ogg, Opus on request
- **Effects:** Volume, Fade, Speed, Bass, Treble, Echo, Trim, Reverse,
  Normalize — plus a raw `custom_filters` escape hatch
- **No reimplementation:** one `bindgen`-generated FFI layer; filter
  strings are validated and assembled in Rust before FFmpeg sees them

## Architecture

```
src/
├── ffi/      unsafe: bindings (build.rs + bindgen), abuffer →
│             chain → abuffersink graph RAII
└── audio/    safe public API: AudioEffects, EffectOptions,
              EffectOperation, decode → filter → encode pipeline
```

`build.rs` discovers FFmpeg via `pkg-config` (honours `PKG_CONFIG_PATH`).

## Requirements

- Rust 1.94+ (edition 2024; the MSRV is driven by the `ppdrive`
  dependency tree)
- FFmpeg **development** libraries: `libavcodec`, `libavformat`,
  `libavutil`, `libswresample`, `libavfilter`
  - Debian/Ubuntu: `sudo apt install libavcodec-dev libavformat-dev
    libavutil-dev libswresample-dev libavfilter-dev pkg-config`
- Output formats other than WAV need the matching encoders
  (`libmp3lame`, `libvorbis`, `libopus` — see `ppdrive-audio-conversion`)
- Verified against FFmpeg **6.1** (system); expected to work on 6.x–9.x

## Build & test

```bash
cargo build
cargo test          # 28 tests
cargo clippy --all-targets
cargo run --example apply -- input.wav output.mp3 --volume -6 --normalize -16
```

## Library usage

```rust
use audio_effects::{AudioEffects, EffectOperation, EffectOptions};

let effects = AudioEffects::new()?;
let output = effects.apply(
    &input_bytes,
    EffectOptions {
        operations: vec![
            EffectOperation::Trim { start_secs: 0.5, end_secs: 30.0 },
            EffectOperation::Volume { gain_db: -3.0 },
            EffectOperation::Normalize { target_lufs: -16.0 },
        ],
        ..Default::default()
    },
)?;
```

## Options

| Field | Type | Default | Meaning |
|-------|------|---------|---------|
| `operations` | `Vec<EffectOperation>` | `[]` | Typed effects, applied in order |
| `custom_filters` | `Option<String>` | `None` | Raw FFmpeg filter string appended last |
| `format` | `Option<AudioFormat>` | `None` = WAV | `Wav`, `Mp3`, `Flac`, `Aac`, `Ogg`, `Opus` |
| `quality` | `Option<u8>` | `None` = 80 | 0–100; lossless formats ignore it |

### Effects

| Operation | Filter | Validation |
|-----------|--------|------------|
| `Volume { gain_db }` | `volume=<g>dB` | finite |
| `Fade { fade_in_secs, fade_out_secs }` | `afade` in at 0, out at `duration - out` | finite, ≥ 0 |
| `Speed { factor }` | `atempo` stages (each 0.5–2.0, max 16) | finite, > 0 |
| `Bass/Treble { gain_db, frequency, width }` | `bass`/`treble` | finite; clamped to FFmpeg ranges |
| `Echo { delay_ms, decay }` | `aecho=0.6:0.3:<ms>:<decay>` | delay ≤ 60000, `0 < decay < 1` |
| `Trim { start_secs, end_secs }` | `atrim` + `asetpts` | window non-empty and inside the stream |
| `Reverse` | `areverse` | — |
| `Normalize { target_lufs }` | `loudnorm=I=…:TP=-2:LRA=7` | finite; clamped to `-70..=-5` |

Timeline-aware validation: `Trim` and `Speed` shorten the running
duration, so a later `Fade` positions its fade-out at the *effective*
end of the stream. An empty chain becomes `anull` (identity).

## Errors

| Variant | When |
|---------|------|
| `FfmpegError(msg)` | FFmpeg call failed (`av_strerror` message), incl. bad custom filter strings |
| `DecoderNotFound` | no decoder for the input stream in this FFmpeg |
| `EncoderNotFound` | output encoder missing (non-WAV target only) |
| `InvalidInput` | empty input, invalid effect arguments, NUL in `custom_filters` |
| `UnsupportedFormat` | input has no audio stream |

Validation happens **before** any FFmpeg call, so bad arguments fail
fast with `InvalidInput` instead of a graph-config error.

## Example CLI

```
Usage: apply <input> <output> [options]

Options:
  --volume <db>             Gain change in dB (e.g. -6)
  --speed <factor>          Speed factor (0.5 = half speed, 2 = double)
  --fade <in>,<out>         Fade in/out in seconds (either may be 0)
  --trim <start>,<end>      Keep only [start, end) seconds
  --normalize <lufs>        Loudness normalize (e.g. -16, clamped -70..-5)
  --echo <ms>,<decay>       Echo: delay ms and decay (0 < decay < 1)
  --bass <db>               Low-shelf EQ (100 Hz)
  --treble <db>             High-shelf EQ (3000 Hz)
  --reverse                 Play backwards
  --filter <chain>          Raw FFmpeg filter string, applied last
  --format <wav|mp3|flac|aac|ogg|opus>
                             Output format (default: wav)
  --quality <0-100>         Encoding quality (default: 80)
  -h, --help                Show this help
```

```bash
cargo run --example apply -- tone.wav intro.mp3 \
    --trim 0.5,30 --fade 0.5,2 --normalize -16 --quality 90
```

## Streaming (planned)

Segment packaging (HLS/DASH) is **not implemented here**; this crate is
the processing stage a future `media-streaming` crate would run before
packaging. The extension points live in `ppdrive-audio-conversion`:

- `DecodedAudio::encode_to(...)` streams `EncoderParams` + packets to a
  caller sink for custom muxing;
- `bitrate_for_quality` shares the rendition bitrate ladder;
- the pts/duration contract (sequential sample-offset pts at the output
  rate) is what segment boundaries will be computed from.

An effects pass feeding streaming should target `format: Some(Wav)` (or
decode-only use via `DecodedAudio`) so the streaming crate owns encoding.

## Notes & limitations

- Input format is sniffed from content; the input file extension is
  irrelevant.
- `aecho` flushes its delayed tail past the input end, so echo output
  can be up to `delay_ms` longer than the input.
- Single graph pass: operations run in list order; there is no
  cross-operation optimization (same as the image pipeline).
- The `ffi` module is internal; `AudioEffects`, `EffectOptions`,
  `EffectOperation`, `AudioFormat` and `Error` are public.

## Continuous integration

`.github/workflows/ci.yml` runs on every push to `main` and every pull
request (same jobs as the other crates — format, clippy, tests, MSRV,
60-second fuzz smoke runs).

## Fuzzing

```bash
rustup toolchain install nightly
cargo install cargo-fuzz
cd audio-effects
cargo fuzz run fuzz_apply             # smoke run: -- -max_total_time=60
```

The fuzzer feeds arbitrary bytes with a selector byte choosing an
operation set and expects no panics, aborts or leaks.

## Releases

Published with the other crates by `.github/workflows/release.yml` on
every `v*` tag: one raw cdylib asset per target renamed to
`ppdrive-audio-effects-<os>-<arch>.<ext>`. FFmpeg shared libraries are
**not** bundled — the target machine needs its own FFmpeg installation.

## License

MIT OR Apache-2.0 (as declared in `Cargo.toml`).
