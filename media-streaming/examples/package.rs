use std::path::PathBuf;
use std::process::ExitCode;

use ppff_media_streaming::{MediaStreamer, RenditionSpec, StreamingOptions, StreamingProtocol};

const USAGE: &str = "\
Usage: package <input> <output_dir> [options]

Options:
  --protocol <hls|dash>    Output protocol (default: hls)
  --segment-duration <1-30>
                           Segment duration in seconds (default: 4)
  --quality <0-100>        Quality for every rendition (default: 80)
  --rendition <spec>       Explicit rendition, repeatable. Spec is
                           `scale`, `WxH`, `width`, or `height`,
                           optionally followed by `,bitrate` (bps).
                           Without --rendition an auto ladder from the
                           source resolution is used.
  -h, --help               Show this help
";

struct Args {
    input: String,
    output: String,
    options: StreamingOptions,
}

fn parse_rendition(spec: &str) -> Result<RenditionSpec, String> {
    let (dims, bitrate) = match spec.split_once(',') {
        Some((dims, bitrate)) => {
            let bitrate: u64 = bitrate
                .parse()
                .map_err(|_| format!("invalid bitrate in rendition: {spec}"))?;
            (dims, Some(bitrate))
        }
        None => (spec, None),
    };
    let rendered = if let Some((width, height)) = dims.split_once('x') {
        RenditionSpec {
            width: Some(
                width
                    .parse()
                    .map_err(|_| format!("invalid rendition: {spec}"))?,
            ),
            height: Some(
                height
                    .parse()
                    .map_err(|_| format!("invalid rendition: {spec}"))?,
            ),
            ..RenditionSpec::default()
        }
    } else if let Ok(scale) = dims.parse::<f32>() {
        RenditionSpec {
            scale: Some(scale),
            ..RenditionSpec::default()
        }
    } else {
        return Err(format!("invalid rendition: {spec}"));
    };
    Ok(RenditionSpec {
        bitrate,
        ..rendered
    })
}

fn parse_args() -> Result<Args, String> {
    let mut input: Option<String> = None;
    let mut output: Option<String> = None;
    let mut options = StreamingOptions::default();
    let mut renditions: Vec<RenditionSpec> = Vec::new();

    let mut argv = std::env::args().skip(1);
    while let Some(arg) = argv.next() {
        let mut take_value = |name: &str| -> Result<String, String> {
            argv.next()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "-h" | "--help" => return Err(USAGE.to_string()),
            "--protocol" => {
                let value = take_value("--protocol")?.to_ascii_lowercase();
                options.protocol = match value.as_str() {
                    "hls" => StreamingProtocol::Hls,
                    "dash" => StreamingProtocol::Dash,
                    _ => return Err(format!("unknown protocol: {value}")),
                };
            }
            "--segment-duration" => {
                options.segment_duration = take_value("--segment-duration")?
                    .parse()
                    .map_err(|_| "invalid --segment-duration".to_string())?;
            }
            "--quality" => {
                options.quality = take_value("--quality")?
                    .parse()
                    .map_err(|_| "invalid --quality".to_string())?;
            }
            "--rendition" => {
                renditions.push(parse_rendition(&take_value("--rendition")?)?);
            }
            _ if arg.starts_with('-') => return Err(format!("unknown option: {arg}")),
            _ => {
                if input.is_none() {
                    input = Some(arg);
                } else if output.is_none() {
                    output = Some(arg);
                } else {
                    return Err(format!("unexpected argument: {arg}"));
                }
            }
        }
    }
    if !renditions.is_empty() {
        options.renditions = Some(renditions);
    }
    Ok(Args {
        input: input.ok_or_else(|| USAGE.to_string())?,
        output: output.ok_or_else(|| USAGE.to_string())?,
        options,
    })
}

fn run(args: Args) -> Result<(), String> {
    let streamer = MediaStreamer::new().map_err(|e| format!("init failed: {e}"))?;
    let output = PathBuf::from(&args.output);
    let playlist = streamer
        .stream_file(std::path::Path::new(&args.input), &output, &args.options)
        .map_err(|e| format!("packaging failed: {e}"))?;

    println!("{}", playlist.playlist.display());
    for file in &playlist.files {
        println!("  {}", file.display());
    }
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
