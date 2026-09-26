use image_compression::ImageFormat;
use image_transformation::{ImageTransformer, TransformOperation, TransformOptions};
use std::process::ExitCode;

const USAGE: &str = "\
Usage: transform <input> <output> [options]

Options:
  --crop W:H:X:Y             Crop to a rectangle (validated against input size)
  --rotate <90|180|270>      Rotate by quarter turns (lossless transpose)
  --flip <h|v|both>          Mirror horizontally / vertically / both
  --pad L,T,R,B[:COLOR]      Add a border (e.g. 10,10,10,10:#ff0000 or ...,black)
  --grayscale                Desaturate to gray
  --brightness <f>           brightness (-1..=1), default 0
  --contrast <f>             contrast, default 1
  --saturation <f>           saturation, default 0..3, default 1
  --blur <sigma>             Gaussian blur (sigma > 0)
  --sharpen <amount>         Unsharp masking (-2..=5, clamped)
  --scale WxH                Exact resize (e.g. 640x480)
  --filter <chain>           Raw FFmpeg filter string, applied last
  --format <jpeg|png|webp|avif>
                             Output format (default: keep input format)
  --quality <0-100>          Encoder quality (default: 80)
  -h, --help                 Show this help

Operations are applied in the order they appear on the command line.
";

struct Args {
    input: String,
    output: String,
    options: TransformOptions,
}

fn take_value(argv: &mut impl Iterator<Item = String>, name: &str) -> Result<String, String> {
    argv.next()
        .ok_or_else(|| format!("missing value for {name}"))
}

fn parse_pair(value: &str, sep: char, name: &str) -> Result<Vec<u32>, String> {
    value
        .split(sep)
        .map(|part| {
            part.trim()
                .parse::<u32>()
                .map_err(|_| format!("{name} requires integers separated by '{sep}'"))
        })
        .collect()
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut format: Option<ImageFormat> = None;
    let mut quality: Option<u8> = None;
    let mut operations: Vec<TransformOperation> = Vec::new();
    let mut custom_filters: Option<String> = None;
    let mut brightness: Option<f32> = None;
    let mut contrast: Option<f32> = None;
    let mut saturation: Option<f32> = None;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--crop" => {
                let value = take_value(&mut argv, "--crop")?;
                let parts = parse_pair(&value, ':', "--crop")?;
                if parts.len() != 4 {
                    return Err("--crop requires W:H:X:Y".to_string());
                }
                operations.push(TransformOperation::Crop {
                    width: parts[0],
                    height: parts[1],
                    x: parts[2],
                    y: parts[3],
                });
            }
            "--rotate" => {
                let value = take_value(&mut argv, "--rotate")?;
                let degrees = value
                    .parse::<u16>()
                    .map_err(|_| "--rotate requires 90, 180 or 270".to_string())?;
                operations.push(TransformOperation::Rotate { degrees });
            }
            "--flip" => {
                let value = take_value(&mut argv, "--flip")?;
                let (horizontal, vertical) = match value.as_str() {
                    "h" => (true, false),
                    "v" => (false, true),
                    "both" => (true, true),
                    _ => return Err("--flip requires h, v or both".to_string()),
                };
                operations.push(TransformOperation::Flip {
                    horizontal,
                    vertical,
                });
            }
            "--pad" => {
                let value = take_value(&mut argv, "--pad")?;
                let (dims, color) = match value.split_once(':') {
                    Some((dims, color)) => (dims, color.to_string()),
                    None => (value.as_str(), "black".to_string()),
                };
                let parts = parse_pair(dims, ',', "--pad")?;
                if parts.len() != 4 {
                    return Err("--pad requires L,T,R,B[:COLOR]".to_string());
                }
                operations.push(TransformOperation::Pad {
                    left: parts[0],
                    top: parts[1],
                    right: parts[2],
                    bottom: parts[3],
                    color,
                });
            }
            "--grayscale" => operations.push(TransformOperation::Grayscale),
            "--brightness" => {
                let value = take_value(&mut argv, "--brightness")?;
                brightness = Some(
                    value
                        .parse()
                        .map_err(|_| "--brightness requires a number".to_string())?,
                );
            }
            "--contrast" => {
                let value = take_value(&mut argv, "--contrast")?;
                contrast = Some(
                    value
                        .parse()
                        .map_err(|_| "--contrast requires a number".to_string())?,
                );
            }
            "--saturation" => {
                let value = take_value(&mut argv, "--saturation")?;
                saturation = Some(
                    value
                        .parse()
                        .map_err(|_| "--saturation requires a number".to_string())?,
                );
            }
            "--blur" => {
                let value = take_value(&mut argv, "--blur")?;
                let sigma = value
                    .parse()
                    .map_err(|_| "--blur requires a number".to_string())?;
                operations.push(TransformOperation::Blur { sigma });
            }
            "--sharpen" => {
                let value = take_value(&mut argv, "--sharpen")?;
                let amount = value
                    .parse()
                    .map_err(|_| "--sharpen requires a number".to_string())?;
                operations.push(TransformOperation::Sharpen { amount });
            }
            "--scale" => {
                let value = take_value(&mut argv, "--scale")?;
                let parts = parse_pair(&value, 'x', "--scale")?;
                if parts.len() != 2 {
                    return Err("--scale requires WxH".to_string());
                }
                operations.push(TransformOperation::Scale {
                    width: parts[0],
                    height: parts[1],
                });
            }
            "--filter" => custom_filters = Some(take_value(&mut argv, "--filter")?),
            "--format" => {
                let value = take_value(&mut argv, "--format")?;
                format = Some(match value.to_ascii_lowercase().as_str() {
                    "jpeg" | "jpg" => ImageFormat::Jpeg,
                    "png" => ImageFormat::Png,
                    "webp" => ImageFormat::WebP,
                    "avif" => ImageFormat::Avif,
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

    if brightness.is_some() || contrast.is_some() || saturation.is_some() {
        operations.push(TransformOperation::Adjust {
            brightness: brightness.unwrap_or(0.0),
            contrast: contrast.unwrap_or(1.0),
            saturation: saturation.unwrap_or(1.0),
        });
    }

    let input = input.ok_or_else(|| format!("missing <input> <output>\n{USAGE}"))?;
    let output = output.ok_or_else(|| format!("missing <input> <output>\n{USAGE}"))?;

    Ok(Args {
        input,
        output,
        options: TransformOptions {
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

    let transformer = ImageTransformer::new().map_err(|e| format!("init failed: {e}"))?;
    let output_bytes = transformer
        .transform(&input_bytes, args.options)
        .map_err(|e| format!("transformation failed: {e}"))?;

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
