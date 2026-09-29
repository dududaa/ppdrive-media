use super::filters::{self, ChainPlan};
use super::options::TransformOptions;
use crate::ffi::FilterGraph;
use video_compression::{
    Error, StreamEvent, VideoEncoder, VideoFormat, VideoFrame, VideoStream, VideoStreamInfo,
};

/// Estimated decoded-frame buffering of a `Reverse` op must stay under
/// this many bytes (1 GiB): `width × height × fps × duration × 2`.
const MAX_REVERSE_ESTIMATED_BYTES: f64 = 1024.0 * 1024.0 * 1024.0;

/// FFmpeg-backed video transformer.
///
/// Stateless: each [`transform`](VideoTransformer::transform) call
/// demuxes, runs one filter graph over the decoded frames and
/// re-encodes — no intermediate Rust pixel buffers.
///
/// # Example
///
/// ```no_run
/// use video_transformation::{TransformOperation, TransformOptions, VideoTransformer};
///
/// # let input_bytes: Vec<u8> = Vec::new();
/// let transformer = VideoTransformer::new()?;
/// let output = transformer.transform(
///     &input_bytes,
///     TransformOptions {
///         operations: vec![
///             TransformOperation::Crop { x: 0, y: 0, width: 1280, height: 720 },
///             TransformOperation::Grayscale,
///         ],
///         custom_filters: None,
///         format: None,
///         quality: Some(80),
///     },
/// )?;
/// # Ok::<(), video_compression::Error>(())
/// ```
pub struct VideoTransformer;

impl VideoTransformer {
    /// Verifies that libavfilter exposes the `buffer`/`buffersink`
    /// filters and that both video encoders are present (via
    /// [`video_compression::VideoCompressor::new`]).
    pub fn new() -> Result<VideoTransformer, Error> {
        unsafe {
            if crate::ffi::avfilter_get_by_name(c"buffer".as_ptr()).is_null()
                || crate::ffi::avfilter_get_by_name(c"buffersink".as_ptr()).is_null()
            {
                return Err(Error::FfmpegError(
                    "libavfilter buffer/buffersink filters unavailable".to_string(),
                ));
            }
        }
        video_compression::VideoCompressor::new()?;
        Ok(VideoTransformer)
    }

    /// Transforms one encoded video: demux → filter graph → encode.
    ///
    /// `options.format` picks the output container; when omitted the
    /// input container is kept (MP4/MOV → [`VideoFormat::Mp4`],
    /// WebM/Matroska → [`VideoFormat::WebM`], anything else falls
    /// back to MP4). The input's audio track is stream-copied when the
    /// target container accepts its codec **and** no `Speed`/`Reverse`
    /// operation is present; otherwise it is dropped.
    ///
    /// Returns [`Error::InvalidInput`] for empty input or invalid
    /// operation arguments, [`Error::UnsupportedFormat`] when the
    /// input holds no video stream and [`Error::LimitExceeded`] when a
    /// `Reverse` op would buffer more than 1 GiB of decoded frames (or
    /// the input duration is unknown).
    pub fn transform(&self, input: &[u8], options: TransformOptions) -> Result<Vec<u8>, Error> {
        let mut stream = VideoStream::open(input)?;
        let info = stream.info().clone();
        let format = options.format.unwrap_or_else(|| default_format(&info));
        let quality = options.quality.unwrap_or(80);

        let plan = filters::build_chain(
            &info,
            &options.operations,
            options.custom_filters.as_deref(),
        )?;
        check_reverse(&info, &plan)?;

        let src_tb = (info.time_base_num, info.time_base_den);
        let mut audio = stream.audio_source_for(format);
        if plan.drops_audio {
            audio = None;
        }
        let audio_tb = audio.as_ref().map(|audio| audio.time_base());

        let graph = FilterGraph::build(info.width, info.height, info.pix_fmt, src_tb, &plan.chain)?;
        let (sink_width, sink_height) = (graph.sink_width(), graph.sink_height());
        if sink_width == 0 || sink_height == 0 {
            return Err(Error::InvalidInput);
        }
        let encoder_info = VideoStreamInfo {
            time_base_num: graph.sink_time_base().0,
            time_base_den: graph.sink_time_base().1,
            ..info.clone()
        };
        let mut encoder = VideoEncoder::muxed(
            format,
            quality,
            sink_width + sink_width % 2,
            sink_height + sink_height % 2,
            &encoder_info,
            audio,
        )?;

        let window = plan.window;
        let video_shift = if plan.pts_modified {
            None
        } else {
            window.and_then(|window| ticks(window.start, src_tb))
        };
        let audio_shift = audio_tb.and_then(|tb| window.and_then(|w| ticks(w.start, tb)));
        while let Some(event) = stream.next_event()? {
            match event {
                StreamEvent::Video(mut frame) => {
                    if !window_accepts(window, frame_secs(frame.pts(), src_tb)) {
                        continue;
                    }
                    if let Some(shift) = video_shift {
                        let pts = frame.pts();
                        if pts != i64::MIN {
                            frame.set_pts((pts - shift).max(0));
                        }
                    }
                    unsafe { graph.push(frame.as_raw_frame().cast())? };
                    drain(&graph, &mut encoder)?;
                }
                StreamEvent::Audio(mut packet) => {
                    if let Some(tb) = audio_tb
                        && window_accepts(window, frame_secs(packet.pts(), tb))
                    {
                        if let Some(shift) = audio_shift {
                            let pts = packet.pts();
                            if pts != i64::MIN {
                                packet.set_pts((pts - shift).max(0));
                            }
                            let dts = packet.dts();
                            if dts != i64::MIN {
                                packet.set_dts((dts - shift).max(0));
                            }
                        }
                        encoder.write_audio_packet(&packet)?;
                    }
                }
            }
        }
        graph.close()?;
        drain(&graph, &mut encoder)?;
        encoder.finish()
    }
}

fn window_accepts(window: Option<filters::Window>, secs: Option<f64>) -> bool {
    window.is_none_or(|window| window.accepts(secs))
}

fn frame_secs(pts: i64, time_base: (i32, i32)) -> Option<f64> {
    if pts == i64::MIN || time_base.0 <= 0 || time_base.1 <= 0 {
        None
    } else {
        Some(pts as f64 * f64::from(time_base.0) / f64::from(time_base.1))
    }
}

/// Converts seconds to ticks in `time_base` (`None` for a non-positive
/// start or a broken time base) — the offset a trimmed clip is
/// re-based by so its output timeline starts at zero.
fn ticks(secs: f64, time_base: (i32, i32)) -> Option<i64> {
    if !secs.is_finite() || secs <= 0.0 || time_base.0 <= 0 || time_base.1 <= 0 {
        None
    } else {
        Some((secs * f64::from(time_base.1) / f64::from(time_base.0)).round() as i64)
    }
}

fn drain(graph: &FilterGraph, encoder: &mut VideoEncoder) -> Result<(), Error> {
    while let Some(sink_frame) = graph.pull()? {
        let mut frame = unsafe { VideoFrame::from_raw(sink_frame.into_raw().cast())? };
        encoder.encode_frame(&mut frame)?;
    }
    Ok(())
}

/// Picks the output container when the caller did not specify one:
/// WebM/Matroska first (their probe name contains both tokens), then
/// MP4/MOV, then MP4 as the fallback.
fn default_format(info: &VideoStreamInfo) -> VideoFormat {
    let name = info.format_name.as_deref().unwrap_or_default();
    if name.contains("webm") || name.contains("matroska") {
        VideoFormat::WebM
    } else {
        VideoFormat::Mp4
    }
}

/// Rejects `Reverse` before the graph or encoder is built when the
/// decoded-frame buffering would exceed [`MAX_REVERSE_ESTIMATED_BYTES`]
/// (`width × height × fps × effective_duration × 2`) or the effective
/// duration cannot be determined.
fn check_reverse(info: &VideoStreamInfo, plan: &ChainPlan) -> Result<(), Error> {
    if !plan.has_reverse {
        return Ok(());
    }
    let duration = info.duration.filter(|d| d.is_finite() && *d > 0.0);
    let Some(duration) = duration else {
        return Err(Error::LimitExceeded(
            "Reverse requires a known input duration; this input does not report one. \
             Remux it to MP4/WebM first or drop the Reverse operation"
                .to_string(),
        ));
    };

    let effective = match plan.window {
        None => duration,
        Some(window) => match window.end {
            Some(end) => (end - window.start).max(0.0),
            None => (duration - window.start).max(0.0),
        },
    };

    let estimated =
        f64::from(info.width) * f64::from(info.height) * info.frame_rate() * effective * 2.0;
    if estimated > MAX_REVERSE_ESTIMATED_BYTES {
        return Err(Error::LimitExceeded(format!(
            "Reverse would buffer about {:.0} MiB of decoded frames (limit {} MiB); \
             shorten the clip, lower its resolution or frame rate, or drop the Reverse operation",
            estimated / (1024.0 * 1024.0),
            MAX_REVERSE_ESTIMATED_BYTES / (1024.0 * 1024.0)
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TransformOperation;

    fn info(duration: Option<f64>) -> VideoStreamInfo {
        VideoStreamInfo {
            width: 3840,
            height: 2160,
            pix_fmt: 0,
            frame_rate_num: 60,
            frame_rate_den: 1,
            time_base_num: 1,
            time_base_den: 90000,
            duration,
            format_name: Some("mov,mp4,m4a,3gp,3g2,mj2".to_string()),
            has_audio: false,
        }
    }

    fn plan(operations: Vec<TransformOperation>) -> ChainPlan {
        filters::build_chain(&info(Some(2.0)), &operations, None).unwrap()
    }

    #[test]
    fn reverse_rejected_without_duration() {
        let plan = filters::build_chain(&info(None), &[TransformOperation::Reverse], None).unwrap();
        let err = check_reverse(&info(None), &plan).unwrap_err();
        match err {
            Error::LimitExceeded(msg) => assert!(msg.contains("duration"), "{msg}"),
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reverse_rejected_when_estimated_buffer_exceeds_limit() {
        let err =
            check_reverse(&info(Some(2.0)), &plan(vec![TransformOperation::Reverse])).unwrap_err();
        match err {
            Error::LimitExceeded(msg) => {
                assert!(msg.contains("MiB"), "{msg}");
                assert!(msg.contains("Reverse"), "{msg}");
            }
            other => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn reverse_allowed_for_small_clips() {
        let mut small = info(Some(2.0));
        small.width = 320;
        small.height = 180;
        small.frame_rate_num = 25;
        let plan = filters::build_chain(&small, &[TransformOperation::Reverse], None).unwrap();
        check_reverse(&small, &plan).unwrap();
    }

    #[test]
    fn reverse_allowed_when_absent_even_for_large_clips() {
        check_reverse(&info(Some(2.0)), &plan(vec![TransformOperation::Grayscale])).unwrap();
    }

    #[test]
    fn trim_window_shrinks_the_estimate() {
        let big = plan(vec![
            TransformOperation::Trim {
                start: 1.9,
                duration: Some(0.1),
            },
            TransformOperation::Reverse,
        ]);
        check_reverse(&info(Some(2.0)), &big).unwrap();
    }

    #[test]
    fn default_format_follows_the_source_container() {
        let mut probe = info(None);
        probe.format_name = Some("matroska,webm".to_string());
        assert_eq!(default_format(&probe), VideoFormat::WebM);
        probe.format_name = Some("mov,mp4,m4a,3gp,3g2,mj2".to_string());
        assert_eq!(default_format(&probe), VideoFormat::Mp4);
        probe.format_name = Some("mpegts".to_string());
        assert_eq!(default_format(&probe), VideoFormat::Mp4);
        probe.format_name = None;
        assert_eq!(default_format(&probe), VideoFormat::Mp4);
    }

    #[test]
    fn frame_secs_maps_timestamps() {
        assert_eq!(frame_secs(i64::MIN, (1, 25)), None);
        assert_eq!(frame_secs(12800, (1, 12800)), Some(1.0));
        assert_eq!(frame_secs(0, (0, 1)), None);
    }

    #[test]
    fn ticks_rebase_offsets() {
        assert_eq!(ticks(0.5, (1, 12800)), Some(6400));
        assert_eq!(ticks(0.0, (1, 12800)), None);
        assert_eq!(ticks(-1.0, (1, 12800)), None);
        assert_eq!(ticks(0.5, (0, 1)), None);
        assert_eq!(ticks(f64::NAN, (1, 12800)), None);
    }
}
