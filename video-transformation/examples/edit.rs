use std::process::ExitCode;
use video_transformation::{TransformOperation, TransformOptions, VideoFormat, VideoTransformer};

const USAGE: &str = "\
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
";

struct Args {
    input: String,
    output: String,
    options: TransformOptions,
}

fn number<T: std::str::FromStr>(value: &str, name: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{name} must be a number: {value}"))
}

fn u32s(value: &str, name: &str) -> Result<Vec<u32>, String> {
    value
        .split(',')
        .map(|part| number(part.trim(), name))
        .collect()
}

fn set_adjust(
    operations: &mut Vec<TransformOperation>,
    brightness: f32,
    contrast: f32,
    saturation: f32,
    replace: fn(&mut TransformOperation) -> Option<&mut f32>,
    value: f32,
) {
    if let Some(slot) = operations.last_mut().and_then(replace) {
        *slot = value;
    } else {
        operations.push(TransformOperation::Adjust {
            brightness,
            contrast,
            saturation,
        });
    }
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut format: Option<VideoFormat> = None;
    let mut quality: Option<u8> = None;
    let mut operations: Vec<TransformOperation> = Vec::new();
    let mut custom_filters: Option<String> = None;

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut take_value = |name: &str| -> Result<String, String> {
            argv.next()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--crop" => {
                let value = u32s(&take_value("--crop")?, "--crop")?;
                if value.len() != 4 {
                    return Err("--crop needs x,y,w,h".to_string());
                }
                operations.push(TransformOperation::Crop {
                    x: value[0],
                    y: value[1],
                    width: value[2],
                    height: value[3],
                });
            }
            "--scale" => {
                let value = take_value("--scale")?;
                let (width, height) = value
                    .split_once('x')
                    .ok_or_else(|| "--scale needs widthxheight".to_string())?;
                operations.push(TransformOperation::Scale {
                    width: number(width.trim(), "width")?,
                    height: number(height.trim(), "height")?,
                });
            }
            "--rotate" => {
                let degrees = number::<u16>(&take_value("--rotate")?, "rotate")?;
                operations.push(TransformOperation::Rotate { degrees });
            }
            "--flip" => {
                let (horizontal, vertical) =
                    match take_value("--flip")?.to_ascii_lowercase().as_str() {
                        "h" | "horizontal" => (true, false),
                        "v" | "vertical" => (false, true),
                        "both" | "hv" => (true, true),
                        other => return Err(format!("unknown flip direction: {other}")),
                    };
                operations.push(TransformOperation::Flip {
                    horizontal,
                    vertical,
                });
            }
            "--pad" => {
                let value = take_value("--pad")?;
                let (sizes, color) = match value.split_once(':') {
                    Some((sizes, color)) => (sizes, Some(color.trim().to_string())),
                    None => (value.as_str(), None),
                };
                let sides = u32s(sizes, "pad")?;
                if sides.len() != 4 {
                    return Err("--pad needs left,top,right,bottom[:color]".to_string());
                }
                operations.push(TransformOperation::Pad {
                    left: sides[0],
                    top: sides[1],
                    right: sides[2],
                    bottom: sides[3],
                    color: color.unwrap_or_else(|| "black".to_string()),
                });
            }
            "--grayscale" => operations.push(TransformOperation::Grayscale),
            "--brightness" => {
                let value: f32 = number(&take_value("--brightness")?, "brightness")?;
                set_adjust(
                    &mut operations,
                    value,
                    1.0,
                    1.0,
                    |op| {
                        if let TransformOperation::Adjust { brightness, .. } = op {
                            Some(brightness)
                        } else {
                            None
                        }
                    },
                    value,
                );
            }
            "--contrast" => {
                let value: f32 = number(&take_value("--contrast")?, "contrast")?;
                set_adjust(
                    &mut operations,
                    0.0,
                    value,
                    1.0,
                    |op| {
                        if let TransformOperation::Adjust { contrast, .. } = op {
                            Some(contrast)
                        } else {
                            None
                        }
                    },
                    value,
                );
            }
            "--saturation" => {
                let value: f32 = number(&take_value("--saturation")?, "saturation")?;
                set_adjust(
                    &mut operations,
                    0.0,
                    1.0,
                    value,
                    |op| {
                        if let TransformOperation::Adjust { saturation, .. } = op {
                            Some(saturation)
                        } else {
                            None
                        }
                    },
                    value,
                );
            }
            "--blur" => operations.push(TransformOperation::Blur {
                sigma: number(&take_value("--blur")?, "blur")?,
            }),
            "--sharpen" => operations.push(TransformOperation::Sharpen {
                amount: number(&take_value("--sharpen")?, "sharpen")?,
            }),
            "--trim" => {
                let value = take_value("--trim")?;
                let (start, duration) = match value.split_once(',') {
                    Some((start, "")) => (start, None),
                    Some((start, duration)) => {
                        (start, Some(number::<f64>(duration, "trim duration")?))
                    }
                    None => (value.as_str(), None),
                };
                operations.push(TransformOperation::Trim {
                    start: number(start.trim(), "trim start")?,
                    duration,
                });
            }
            "--speed" => operations.push(TransformOperation::Speed {
                factor: number(&take_value("--speed")?, "speed")?,
            }),
            "--reverse" => operations.push(TransformOperation::Reverse),
            "--filter" => custom_filters = Some(take_value("--filter")?),
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

    let format = format.or_else(|| {
        let ext = std::path::Path::new(&output)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or_default();
        VideoFormat::from_extension(ext)
    });

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

    let transformer = VideoTransformer::new().map_err(|e| format!("init failed: {e}"))?;
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
