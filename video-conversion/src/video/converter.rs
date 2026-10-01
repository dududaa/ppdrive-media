use crate::error::Error;
use crate::ffi::encode::{VideoEncoderParams, VideoPacket};
use crate::video::encoder::VideoEncoder;
use crate::video::options::ConversionOptions;
use crate::video::resize;
use crate::video::stream::{StreamEvent, VideoFrame, VideoStream, VideoStreamInfo};

/// Entry point for probing and converting videos through FFmpeg.
///
/// The struct itself is a zero-sized handle; creating it verifies that
/// both baseline encoders (libx264, libvpx-vp9) are available in the
/// linked FFmpeg libraries and lowers FFmpeg's log verbosity to errors
/// only. Encoders for other [`crate::VideoFormat`] variants
/// (libaom-av1, libx265) are looked up lazily when a conversion using
/// that format starts.
///
/// Instances may be reused for any number of
/// [`convert`](VideoConverter::convert) calls.
pub struct VideoConverter;

impl VideoConverter {
    /// Creates a converter after checking that the libx264 and
    /// libvpx-vp9 encoders are present in the linked FFmpeg build.
    ///
    /// Returns [`crate::Error::EncoderNotFound`] if either is missing,
    /// which can happen when FFmpeg was configured without
    /// `--enable-libx264` or `--enable-libvpx`. Encoders needed by
    /// other formats (libaom-av1, libx265) are not checked here; a
    /// conversion requesting a format whose encoder is missing fails
    /// with [`crate::Error::EncoderNotFound`] at
    /// [`convert`](VideoConverter::convert) time instead.
    ///
    /// FFmpeg's global log level is set to `AV_LOG_ERROR` on first call
    /// and never raised above it.
    pub fn new() -> Result<Self, Error> {
        unsafe {
            if crate::ffi::av_log_get_level() > crate::ffi::AV_LOG_ERROR as i32 {
                crate::ffi::av_log_set_level(crate::ffi::AV_LOG_ERROR as i32);
            }
        }
        crate::ffi::wrappers::find_encoder_by_name("libx264")?;
        crate::ffi::wrappers::find_encoder_by_name("libvpx-vp9")?;
        Ok(VideoConverter)
    }

    /// Converts a video into the format described by `options`.
    ///
    /// `input` may be any container/codec combination FFmpeg can decode
    /// (MP4, WebM, MKV, MOV, …), probed from content — file extensions
    /// are irrelevant. An input's audio track is stream-copied when the
    /// target container accepts its codec (AAC/MP3 into MP4, MOV and
    /// AVI, Vorbis/Opus into WebM, either family into Matroska) and
    /// dropped otherwise (or always, with
    /// [`ConversionOptions::drop_audio`]) — audio is never transcoded.
    ///
    /// Pipeline: demux → decode → optional resize/scale → encode + mux.
    /// Frame timestamps of the source are preserved in the source time
    /// base, or resampled to a constant rate when
    /// [`ConversionOptions::fps`] is set. [`ConversionOptions::scale`],
    /// [`ConversionOptions::effort`] and
    /// [`ConversionOptions::max_bytes`] apply as documented on those
    /// fields; a size budget costs up to eight full re-encodes.
    ///
    /// Returns [`Error::InvalidInput`] for empty input or an option
    /// the pipeline cannot honour (`Some(0)` width/height/max_bytes,
    /// a bad `scale`, `fps` outside `1..=1000`),
    /// [`Error::UnsupportedFormat`] when the input holds no video
    /// stream, [`Error::EncoderNotFound`] when `options` requests a
    /// format whose encoder is missing from the linked FFmpeg build,
    /// and [`Error::TargetSizeUnreachable`] when `options.max_bytes`
    /// cannot be met even at quality 0.
    pub fn convert(&self, input: &[u8], options: ConversionOptions) -> Result<Vec<u8>, Error> {
        options.validate()?;
        match options.max_bytes {
            None => convert_once(input, &options, options.quality),
            Some(max_bytes) => encode_within(input, &options, max_bytes),
        }
    }

    /// Converts packet-by-packet instead of into a single container,
    /// invoking `sink` with the open encoder's [`VideoEncoderParams`]
    /// once and then every produced [`VideoPacket`].
    ///
    /// This is the extension point consumed by external packaging
    /// pipelines (adaptive HLS/DASH segmentation) that mux packets into
    /// their own `AVFormatContext`. `global_headers` should be `true`
    /// when the target container stores codec extradata out-of-band
    /// (e.g. fragmented MP4).
    ///
    /// Video only — the input's audio track is not surfaced in this
    /// mode. [`ConversionOptions::max_bytes`] is rejected with
    /// [`Error::InvalidInput`] here, because the sink emits packets as
    /// a side effect and cannot be re-run for a size search.
    pub fn convert_to<F>(
        &self,
        input: &[u8],
        options: ConversionOptions,
        global_headers: bool,
        sink: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(&VideoEncoderParams, &VideoPacket),
    {
        options.validate()?;
        if options.max_bytes.is_some() {
            return Err(Error::InvalidInput);
        }
        let mut stream = VideoStream::open(input)?;
        let info = stream.info().clone();
        let (width, height) = resize::target_dimensions(info.width, info.height, &options)?;
        let (width, height) = resize::round_to_even(width, height, true);

        let mut encoder = VideoEncoder::packetized(
            options.format,
            options.quality,
            options.effort,
            options.fps,
            options.keyframe_interval,
            width,
            height,
            &info,
            global_headers,
        )?;
        let params = encoder.params().clone();
        let mut emitted = 0usize;
        let mut scheduler = FrameScheduler::new(options.fps, &info);

        while let Some(event) = stream.next_event()? {
            if let StreamEvent::Video(frame) = event {
                for mut out in scheduler.push(frame)? {
                    encoder.encode_frame(&mut out)?;
                }
            }
            for packet in encoder.take_pending() {
                emitted += 1;
                sink(&params, &packet);
            }
        }
        encoder.finish_drain()?;
        for packet in encoder.take_pending() {
            emitted += 1;
            sink(&params, &packet);
        }
        if emitted == 0 {
            return Err(Error::InvalidInput);
        }
        Ok(())
    }
}

/// Runs one full demux → decode → encode pass at a fixed `quality`.
fn convert_once(input: &[u8], options: &ConversionOptions, quality: u8) -> Result<Vec<u8>, Error> {
    let mut stream = VideoStream::open(input)?;
    let info = stream.info().clone();
    let (width, height) = resize::target_dimensions(info.width, info.height, options)?;
    let (width, height) = resize::round_to_even(width, height, true);

    let copy_audio = if options.drop_audio {
        None
    } else {
        stream.audio_source_for(options.format)
    };
    let has_audio = copy_audio.is_some();
    let mut encoder = VideoEncoder::muxed(
        options.format,
        quality,
        options.effort,
        options.fps,
        options.keyframe_interval,
        width,
        height,
        &info,
        copy_audio,
    )?;

    let mut scheduler = FrameScheduler::new(options.fps, &info);
    while let Some(event) = stream.next_event()? {
        match event {
            StreamEvent::Video(frame) => {
                for mut out in scheduler.push(frame)? {
                    encoder.encode_frame(&mut out)?;
                }
            }
            StreamEvent::Audio(packet) => {
                if has_audio {
                    encoder.write_audio_packet(&packet)?;
                }
            }
        }
    }
    encoder.finish()
}

/// Binary search over `quality` for the highest quality whose encoded
/// output fits `max_bytes` (at most eight full encodes of the input).
fn encode_within(
    input: &[u8],
    options: &ConversionOptions,
    max_bytes: u64,
) -> Result<Vec<u8>, Error> {
    let top = options.quality.min(100);
    let first = convert_once(input, options, top)?;
    if first.len() as u64 <= max_bytes {
        return Ok(first);
    }
    if top == 0 {
        return Err(Error::TargetSizeUnreachable);
    }

    let mut low = 0u8;
    let mut high = top - 1;
    let mut best: Option<Vec<u8>> = None;
    while low <= high {
        let mid = low + (high - low) / 2;
        let out = convert_once(input, options, mid)?;
        if out.len() as u64 <= max_bytes {
            best = Some(out);
            low = mid + 1;
        } else if mid == 0 {
            break;
        } else {
            high = mid - 1;
        }
    }
    best.ok_or(Error::TargetSizeUnreachable)
}

/// Resamples decoded frames onto a constant frame-rate timeline when
/// `fps` is `Some`: frames landing on an already-used slot are dropped
/// and gaps up to five seconds are filled with a refcounted copy of
/// the previous frame (larger jumps are treated as discontinuities and
/// left as a timestamp gap). With `fps = None` every frame passes
/// through untouched.
struct FrameScheduler {
    fps: Option<u32>,
    time_base_num: i32,
    time_base_den: i32,
    input_frame_rate_num: i32,
    input_frame_rate_den: i32,
    index: i64,
    last_slot: Option<i64>,
    previous: Option<VideoFrame>,
}

impl FrameScheduler {
    fn new(fps: Option<u32>, info: &VideoStreamInfo) -> FrameScheduler {
        FrameScheduler {
            fps,
            time_base_num: info.time_base_num.max(1),
            time_base_den: info.time_base_den.max(1),
            input_frame_rate_num: if info.frame_rate_num > 0 {
                info.frame_rate_num
            } else {
                25
            },
            input_frame_rate_den: if info.frame_rate_den > 0 {
                info.frame_rate_den
            } else {
                1
            },
            index: 0,
            last_slot: None,
            previous: None,
        }
    }

    fn push(&mut self, mut frame: VideoFrame) -> Result<Vec<VideoFrame>, Error> {
        let Some(fps) = self.fps else {
            return Ok(vec![frame]);
        };
        let slot = self.slot_of(frame.pts(), fps);
        self.index += 1;
        if let Some(last) = self.last_slot
            && slot <= last
        {
            return Ok(Vec::new());
        }
        let mut out = Vec::new();
        if let Some(last) = self.last_slot
            && slot - last <= i64::from(fps) * 5
        {
            for gap in (last + 1)..slot {
                let Some(previous) = &mut self.previous else {
                    break;
                };
                previous.set_pts(gap);
                out.push(previous.try_clone()?);
            }
        }
        frame.set_pts(slot);
        self.previous = Some(frame.try_clone()?);
        self.last_slot = Some(slot);
        out.push(frame);
        Ok(out)
    }

    fn slot_of(&self, pts: i64, fps: u32) -> i64 {
        let seconds = if pts == crate::ffi::AV_NOPTS_VALUE {
            self.index as f64 * f64::from(self.input_frame_rate_den)
                / f64::from(self.input_frame_rate_num)
        } else {
            pts as f64 * f64::from(self.time_base_num) / f64::from(self.time_base_den)
        };
        ((seconds * f64::from(fps)).round() as i64).max(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::video::crf_for_quality;
    use crate::video::{VideoFormat, VideoStream};

    fn fixture(name: &str) -> Vec<u8> {
        std::fs::read(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .unwrap()
    }

    #[test]
    fn crf_mapping_covers_all_formats() {
        for format in [
            VideoFormat::Mp4,
            VideoFormat::Mov,
            VideoFormat::Mkv,
            VideoFormat::Avi,
            VideoFormat::Mp4Hevc,
            VideoFormat::MovHevc,
        ] {
            assert_eq!(crf_for_quality(format, 0), 51, "{format:?}");
            assert_eq!(crf_for_quality(format, 100), 18, "{format:?}");
        }
        for format in [
            VideoFormat::WebM,
            VideoFormat::Mp4Av1,
            VideoFormat::WebMAv1,
            VideoFormat::MkvAv1,
        ] {
            assert_eq!(crf_for_quality(format, 0), 63, "{format:?}");
            assert_eq!(crf_for_quality(format, 100), 24, "{format:?}");
        }
        assert_eq!(crf_for_quality(VideoFormat::Mp4, 200), 18);
        assert_eq!(crf_for_quality(VideoFormat::Mp4Av1, 200), 24);
    }

    #[test]
    fn mp4_roundtrip_preserves_dimensions() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(&input, ConversionOptions::default())
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert_eq!(stream.info().width, 320);
        assert_eq!(stream.info().height, 180);
        let name = stream.info().format_name.clone().unwrap();
        assert!(name.contains("mp4") || name.contains("mov,mp4"), "{name}");
    }

    #[test]
    fn resize_targets_even_dimensions() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    width: Some(101),
                    height: Some(57),
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert_eq!(stream.info().width, 102);
        assert_eq!(stream.info().height, 58);
    }

    #[test]
    fn webm_output_drops_incompatible_audio() {
        let input = fixture("input_audio.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: VideoFormat::WebM,
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(!stream.info().has_audio);
        assert_eq!(stream.info().width, 320);
    }

    #[test]
    fn mp4_copies_compatible_audio() {
        let input = fixture("input_audio.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(&input, ConversionOptions::default())
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(stream.info().has_audio);
    }

    #[test]
    fn mkv_copies_compatible_audio() {
        let input = fixture("input_audio.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: VideoFormat::Mkv,
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(stream.info().has_audio);
    }

    #[test]
    fn avi_copies_compatible_audio() {
        let input = fixture("input_audio.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    format: VideoFormat::Avi,
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(stream.info().has_audio);
    }

    #[test]
    fn convert_to_emits_params_then_packets() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let mut saw_params = false;
        let mut packets = 0usize;
        let mut saw_keyframe = false;
        converter
            .convert_to(
                &input,
                ConversionOptions::default(),
                false,
                &mut |params, packet| {
                    saw_params = true;
                    assert_eq!(params.codec_name, "libx264");
                    assert_eq!(params.width, 320);
                    assert!(!packet.data.is_empty());
                    packets += 1;
                    saw_keyframe |= packet.is_keyframe;
                },
            )
            .unwrap();
        assert!(saw_params);
        assert!(packets > 1);
        assert!(saw_keyframe);
    }

    fn count_video_frames(output: &[u8]) -> usize {
        let mut stream = VideoStream::open(output).unwrap();
        let mut frames = 0usize;
        while let Some(event) = stream.next_event().unwrap() {
            if matches!(event, StreamEvent::Video(_)) {
                frames += 1;
            }
        }
        frames
    }

    #[test]
    fn scale_resizes_when_dimensions_omitted() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    scale: Some(0.5),
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert_eq!(stream.info().width, 160);
        assert_eq!(stream.info().height, 90);
    }

    #[test]
    fn fps_resamples_frame_count() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();

        let down = converter
            .convert(
                &input,
                ConversionOptions {
                    fps: Some(10),
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let down_frames = count_video_frames(&down);
        assert!(
            (19..=23).contains(&down_frames),
            "fps 10: {down_frames} frames"
        );

        let up = converter
            .convert(
                &input,
                ConversionOptions {
                    fps: Some(60),
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let up_frames = count_video_frames(&up);
        assert!(
            (113..=125).contains(&up_frames),
            "fps 60: {up_frames} frames"
        );
    }

    #[test]
    fn drop_audio_removes_audio_track() {
        let input = fixture("input_audio.mp4");
        let converter = VideoConverter::new().unwrap();
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    drop_audio: true,
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(!stream.info().has_audio);
    }

    #[test]
    fn max_bytes_finds_output_within_budget() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let plain = converter
            .convert(&input, ConversionOptions::default())
            .unwrap();
        let budget = plain.len() as u64 / 2;
        let output = converter
            .convert(
                &input,
                ConversionOptions {
                    max_bytes: Some(budget),
                    ..ConversionOptions::default()
                },
            )
            .unwrap();
        assert!(output.len() as u64 <= budget, "{} > {budget}", output.len());
    }

    #[test]
    fn impossible_budget_is_unreachable() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let err = converter
            .convert(
                &input,
                ConversionOptions {
                    quality: 0,
                    max_bytes: Some(10),
                    ..ConversionOptions::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::TargetSizeUnreachable);
    }

    #[test]
    fn keyframe_interval_caps_keyframe_gaps() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let mut positions = Vec::new();
        converter
            .convert_to(
                &input,
                ConversionOptions {
                    keyframe_interval: Some(10),
                    ..ConversionOptions::default()
                },
                false,
                &mut |_params, packet| positions.push(packet.is_keyframe),
            )
            .unwrap();
        assert!(positions.first().is_some_and(|k| *k), "first not keyframe");
        let keyframes: Vec<usize> = positions
            .iter()
            .enumerate()
            .filter(|(_, k)| **k)
            .map(|(i, _)| i)
            .collect();
        assert!(keyframes.len() >= 4, "{} keyframes", keyframes.len());
        for gap in keyframes.windows(2) {
            assert!(gap[1] - gap[0] <= 10, "gap {}", gap[1] - gap[0]);
        }
    }

    #[test]
    fn convert_to_rejects_size_budget() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let err = converter
            .convert_to(
                &input,
                ConversionOptions {
                    max_bytes: Some(1_000_000),
                    ..ConversionOptions::default()
                },
                false,
                &mut |_params, _packet| {},
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn convert_to_applies_fps() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let mut packets = 0usize;
        converter
            .convert_to(
                &input,
                ConversionOptions {
                    fps: Some(10),
                    ..ConversionOptions::default()
                },
                false,
                &mut |params, _packet| {
                    assert_eq!(params.time_base_num, 1);
                    assert_eq!(params.time_base_den, 10);
                    assert_eq!(params.frame_rate_num, 10);
                    assert_eq!(params.frame_rate_den, 1);
                    packets += 1;
                },
            )
            .unwrap();
        assert!((19..=23).contains(&packets), "{packets} packets");
    }

    #[test]
    fn empty_input_is_invalid() {
        let converter = VideoConverter::new().unwrap();
        let err = converter
            .convert(&[], ConversionOptions::default())
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn zero_dimensions_are_invalid() {
        let input = fixture("input.mp4");
        let converter = VideoConverter::new().unwrap();
        let err = converter
            .convert(
                &input,
                ConversionOptions {
                    width: Some(0),
                    ..ConversionOptions::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn audio_only_input_is_unsupported() {
        let input = wav_fixture();
        let converter = VideoConverter::new().unwrap();
        let err = converter
            .convert(&input, ConversionOptions::default())
            .unwrap_err();
        assert_eq!(err, Error::UnsupportedFormat);
    }

    fn wav_fixture() -> Vec<u8> {
        let sample_rate: u32 = 8000;
        let samples: u32 = 800;
        let data_len = samples * 2;
        let mut wav = Vec::with_capacity(44 + data_len as usize);
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36 + data_len).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&sample_rate.to_le_bytes());
        wav.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&data_len.to_le_bytes());
        wav.extend(std::iter::repeat_n(0u8, data_len as usize));
        wav
    }
}
