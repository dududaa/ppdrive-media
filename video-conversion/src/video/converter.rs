use crate::error::Error;
use crate::ffi::encode::{VideoEncoderParams, VideoPacket};
use crate::video::encoder::VideoEncoder;
use crate::video::options::ConversionOptions;
use crate::video::resize;
use crate::video::stream::{StreamEvent, VideoStream};

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
    /// dropped otherwise — audio is never transcoded.
    ///
    /// Pipeline: demux → decode → optional resize/conversion in the
    /// encoder → encode + mux. Frame timestamps of the source are
    /// preserved in the source time base.
    ///
    /// Returns [`Error::InvalidInput`] for empty input or a `Some(0)`
    /// width/height, [`Error::UnsupportedFormat`] when the input holds
    /// no video stream, and [`Error::EncoderNotFound`] when `options`
    /// requests a format whose encoder is missing from the linked
    /// FFmpeg build.
    pub fn convert(&self, input: &[u8], options: ConversionOptions) -> Result<Vec<u8>, Error> {
        options.validate()?;
        let mut stream = VideoStream::open(input)?;
        let info = stream.info().clone();
        let (width, height) = resize::target_dimensions(info.width, info.height, &options)?;
        let (width, height) = resize::round_to_even(width, height, true);

        let copy_audio = stream.audio_source_for(options.format);
        let has_audio = copy_audio.is_some();
        let mut encoder = VideoEncoder::muxed(
            options.format,
            options.quality,
            width,
            height,
            &info,
            copy_audio,
        )?;

        while let Some(event) = stream.next_event()? {
            match event {
                StreamEvent::Video(mut frame) => encoder.encode_frame(&mut frame)?,
                StreamEvent::Audio(packet) => {
                    if has_audio {
                        encoder.write_audio_packet(&packet)?;
                    }
                }
            }
        }
        encoder.finish()
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
    /// mode.
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
        let mut stream = VideoStream::open(input)?;
        let info = stream.info().clone();
        let (width, height) = resize::target_dimensions(info.width, info.height, &options)?;
        let (width, height) = resize::round_to_even(width, height, true);

        let mut encoder = VideoEncoder::packetized(
            options.format,
            options.quality,
            width,
            height,
            &info,
            global_headers,
        )?;
        let params = encoder.params().clone();
        let mut emitted = 0usize;

        while let Some(event) = stream.next_event()? {
            if let StreamEvent::Video(mut frame) = event {
                encoder.encode_frame(&mut frame)?;
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
