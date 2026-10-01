use ppff_audio_conversion::AudioFormat;
use audio_effects::{AudioEffects, EffectOperation, EffectOptions};
use std::process::ExitCode;

const USAGE: &str = "\
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

Operations are applied in the order they appear on the command line.
";

struct Args {
    input: String,
    output: String,
    options: EffectOptions,
}

fn take_value(argv: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    argv.next()
        .ok_or_else(|| format!("missing value for {name}"))
}

fn parse_pair_f32(value: &str, name: &str) -> Result<(f32, f32), String> {
    let mut parts = value.split(',');
    let parse = |part: Option<&str>| -> Result<f32, String> {
        part.unwrap_or_default()
            .trim()
            .parse::<f32>()
            .map_err(|_| format!("{name} requires two numbers separated by ','"))
    };
    let first = parse(parts.next())?;
    let second = parse(parts.next())?;
    if parts.next().is_some() {
        return Err(format!("{name} requires exactly two numbers"));
    }
    Ok((first, second))
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut format: Option<AudioFormat> = None;
    let mut quality: Option<u8> = None;
    let mut operations: Vec<EffectOperation> = Vec::new();
    let mut custom_filters: Option<String> = None;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--volume" => {
                let value: f32 = take_value(&mut argv, "--volume")?
                    .parse()
                    .map_err(|_| "--volume requires a number".to_string())?;
                operations.push(EffectOperation::Volume { gain_db: value });
            }
            "--speed" => {
                let value: f32 = take_value(&mut argv, "--speed")?
                    .parse()
                    .map_err(|_| "--speed requires a number".to_string())?;
                operations.push(EffectOperation::Speed { factor: value });
            }
            "--fade" => {
                let (fade_in_secs, fade_out_secs) =
                    parse_pair_f32(&take_value(&mut argv, "--fade")?, "--fade")?;
                operations.push(EffectOperation::Fade {
                    fade_in_secs,
                    fade_out_secs,
                });
            }
            "--trim" => {
                let (start_secs, end_secs) =
                    parse_pair_f32(&take_value(&mut argv, "--trim")?, "--trim")?;
                operations.push(EffectOperation::Trim {
                    start_secs,
                    end_secs,
                });
            }
            "--normalize" => {
                let target_lufs: f32 = take_value(&mut argv, "--normalize")?
                    .parse()
                    .map_err(|_| "--normalize requires a number".to_string())?;
                operations.push(EffectOperation::Normalize { target_lufs });
            }
            "--echo" => {
                let value = take_value(&mut argv, "--echo")?;
                let (delay, decay) = parse_pair_f32(&value, "--echo")?;
                if delay < 0.0 || delay.fract() != 0.0 || delay > u32::MAX as f32 {
                    return Err("--echo delay must be a whole number of milliseconds".to_string());
                }
                operations.push(EffectOperation::Echo {
                    delay_ms: delay as u32,
                    decay,
                });
            }
            "--bass" => {
                let gain_db: f32 = take_value(&mut argv, "--bass")?
                    .parse()
                    .map_err(|_| "--bass requires a number".to_string())?;
                operations.push(EffectOperation::Bass {
                    gain_db,
                    frequency: 100.0,
                    width: 0.5,
                });
            }
            "--treble" => {
                let gain_db: f32 = take_value(&mut argv, "--treble")?
                    .parse()
                    .map_err(|_| "--treble requires a number".to_string())?;
                operations.push(EffectOperation::Treble {
                    gain_db,
                    frequency: 3000.0,
                    width: 0.5,
                });
            }
            "--reverse" => operations.push(EffectOperation::Reverse),
            "--filter" => custom_filters = Some(take_value(&mut argv, "--filter")?),
            "--format" => {
                let value = take_value(&mut argv, "--format")?;
                format = Some(match value.to_ascii_lowercase().as_str() {
                    "wav" => AudioFormat::Wav,
                    "mp3" => AudioFormat::Mp3,
                    "flac" => AudioFormat::Flac,
                    "aac" => AudioFormat::Aac,
                    "ogg" => AudioFormat::Ogg,
                    "opus" => AudioFormat::Opus,
                    _ => return Err(format!("unknown format: {value}")),
                });
            }
            "--quality" => {
                let value: u8 = take_value(&mut argv, "--quality")?
                    .parse()
                    .map_err(|_| "quality must be an integer 0-100".to_string())?;
                quality = Some(value.min(100));
            }
            _ if arg.starts_with('-') => {
                return Err(format!("unknown option: {arg}"));
            }
            _ if input.is_none() => input = Some(arg),
            _ if output.is_none() => output = Some(arg),
            _ => return Err(format!("unexpected argument: {arg}")),
        }
    }

    let input = input.ok_or_else(|| format!("missing <input> <output>\n{USAGE}"))?;
    let output = output.ok_or_else(|| format!("missing <input> <output>\n{USAGE}"))?;

    Ok(Args {
        input,
        output,
        options: EffectOptions {
            operations,
            custom_filters,
            format,
            quality,
        },
    })
}

fn run(args: Args) -> Result<(), String> {
    let input_bytes = std::fs::read(&args.input).map_err(|e| format!("{}: {e}", args.input))?;
    let input_len = input_bytes.len();

    let effects = AudioEffects::new().map_err(|e| format!("init failed: {e}"))?;
    let output_bytes = effects
        .apply(&input_bytes, args.options)
        .map_err(|e| format!("effect failed: {e}"))?;

    std::fs::write(&args.output, &output_bytes).map_err(|e| format!("{}: {e}", args.output))?;

    println!(
        "{}: {} bytes -> {}: {} bytes",
        args.input,
        input_len,
        args.output,
        output_bytes.len()
    );
    Ok(())
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(args) => args,
        Err(msg) => {
            eprint!("{msg}");
            if !msg.ends_with('\n') {
                eprintln!();
            }
            return if msg == USAGE {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            };
        }
    };
    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}
