use std::process::ExitCode;
use video_conversion::{ConversionOptions, VideoConverter, VideoFormat};

const USAGE: &str = "\
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
";

struct Args {
    input: String,
    output: String,
    options: ConversionOptions,
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut format: Option<VideoFormat> = None;
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
                    "mp4" => VideoFormat::Mp4,
                    "webm" => VideoFormat::WebM,
                    "mov" => VideoFormat::Mov,
                    "mkv" => VideoFormat::Mkv,
                    "avi" => VideoFormat::Avi,
                    "mp4av1" => VideoFormat::Mp4Av1,
                    "webmav1" => VideoFormat::WebMAv1,
                    "mkvav1" => VideoFormat::MkvAv1,
                    "mp4hevc" => VideoFormat::Mp4Hevc,
                    "movhevc" => VideoFormat::MovHevc,
                    _ => return Err(format!("unknown format: {value}")),
                });
            }
            "--quality" => {
                let value: u8 = take_value("--quality")?
                    .parse()
                    .map_err(|_| "quality must be an integer 0-100".to_string())?;
                options.quality = value.min(100);
            }
            "--width" => {
                let value: u32 = take_value("--width")?
                    .parse()
                    .map_err(|_| "width must be an integer".to_string())?;
                options.width = Some(value);
            }
            "--height" => {
                let value: u32 = take_value("--height")?
                    .parse()
                    .map_err(|_| "height must be an integer".to_string())?;
                options.height = Some(value);
            }
            "--scale" => {
                let value: f32 = take_value("--scale")?
                    .parse()
                    .map_err(|_| "scale must be a number".to_string())?;
                options.scale = Some(value);
            }
            "--effort" => {
                let value: u8 = take_value("--effort")?
                    .parse()
                    .map_err(|_| "effort must be an integer 0-100".to_string())?;
                options.effort = Some(value.min(100));
            }
            "--max-bytes" => {
                let value: u64 = take_value("--max-bytes")?
                    .parse()
                    .map_err(|_| "max-bytes must be an integer".to_string())?;
                options.max_bytes = Some(value);
            }
            "--fps" => {
                let value: u32 = take_value("--fps")?
                    .parse()
                    .map_err(|_| "fps must be an integer".to_string())?;
                options.fps = Some(value);
            }
            "--no-audio" => options.drop_audio = true,
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
            VideoFormat::from_extension(ext)
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

    let converter = VideoConverter::new().map_err(|e| format!("init failed: {e}"))?;
    let output_bytes = converter
        .convert(&input_bytes, args.options)
        .map_err(|e| format!("conversion failed: {e}"))?;

    std::fs::write(&args.output, &output_bytes).map_err(|e| format!("{}: {e}", args.output))?;

    let ratio = output_bytes.len() as f64 / input_len.max(1) as f64;
    println!(
        "{}: {} bytes -> {}: {} bytes ({:.1}%)",
        args.input,
        input_len,
        args.output,
        output_bytes.len(),
        ratio * 100.0
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
