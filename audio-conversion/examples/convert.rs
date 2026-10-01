use ppff_audio_conversion::{AudioConverter, AudioFormat, ConversionOptions};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: convert <input> <output> [options]

Options:
  --format <wav|mp3|flac|aac|ogg|opus>
                             Output format (default: from output extension)
  --quality <0-100>          Encoding quality (default: 80)
  --sample-rate <hz>         Target sample rate (default: keep source)
  --channels <1|2>           Target channel count (default: keep/downmix)
  -h, --help                 Show this help
";

struct Args {
    input: String,
    output: String,
    options: ConversionOptions,
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut format: Option<AudioFormat> = None;
    let mut options = ConversionOptions::default();

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut take_value = |name: &str| -> Result<String, String> {
            argv.next()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--format" => {
                let value = take_value("--format")?;
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
                let value: u8 = take_value("--quality")?
                    .parse()
                    .map_err(|_| "quality must be an integer 0-100".to_string())?;
                options.quality = value.min(100);
            }
            "--sample-rate" => {
                let value: u32 = take_value("--sample-rate")?
                    .parse()
                    .map_err(|_| "sample-rate must be an integer in Hz".to_string())?;
                options.sample_rate = Some(value);
            }
            "--channels" => {
                let value: u8 = take_value("--channels")?
                    .parse()
                    .map_err(|_| "channels must be 1 or 2".to_string())?;
                options.channels = Some(value);
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

    options.format = match format {
        Some(format) => format,
        None => {
            let ext = std::path::Path::new(&output)
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or_default();
            AudioFormat::from_extension(ext)
                .ok_or_else(|| "cannot infer format; pass --format".to_string())?
        }
    };

    Ok(Args {
        input,
        output,
        options,
    })
}

fn run(args: Args) -> Result<(), String> {
    let input_bytes = std::fs::read(&args.input).map_err(|e| format!("{}: {e}", args.input))?;
    let input_len = input_bytes.len();

    let converter = AudioConverter::new().map_err(|e| format!("init failed: {e}"))?;
    let output_bytes = converter
        .convert(&input_bytes, args.options)
        .map_err(|e| format!("conversion failed: {e}"))?;

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
