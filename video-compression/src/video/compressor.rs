use crate::error::Error;
use crate::ffi::encode::{VideoEncoderParams, VideoPacket};
use crate::video::encoder::VideoEncoder;
use crate::video::options::CompressionOptions;
use crate::video::resize;
use crate::video::stream::{StreamEvent, VideoStream};

/// Entry point for probing and compressing videos through FFmpeg.
///
/// The struct itself is a zero-sized handle; creating it verifies that
/// both encoders (libx264 for MP4, libvpx-vp9 for WebM) are available
/// in the linked FFmpeg libraries and lowers FFmpeg's log verbosity to
/// errors only.
///
/// Instances may be reused for any number of
/// [`compress`](VideoCompressor::compress) calls.
pub struct VideoCompressor;

impl VideoCompressor {
    /// Creates a compressor after checking that the libx264 and
    /// libvpx-vp9 encoders are present in the linked FFmpeg build.
    ///
    /// Returns [`crate::Error::EncoderNotFound`] if either is missing,
    /// which can happen when FFmpeg was configured without
    /// `--enable-libx264` or `--enable-libvpx`.
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
        Ok(VideoCompressor)
    }

    /// Compresses a video into the format described by `options`.
    ///
    /// `input` may be any container/codec combination FFmpeg can decode
    /// (MP4, WebM, MKV, MOV, …), probed from content — file extensions
    /// are irrelevant. An input's audio track is stream-copied when the
    /// target container accepts its codec (AAC/MP3 into MP4,
    /// Vorbis/Opus into WebM) and dropped otherwise — audio is never
    /// transcoded.
    ///
    /// Pipeline: demux → decode → optional resize/conversion in the
    /// encoder → encode + mux. Frame timestamps of the source are
    /// preserved in the source time base.
    ///
    /// Returns [`Error::InvalidInput`] for empty input or a `Some(0)`
    /// width/height, [`Error::UnsupportedFormat`] when the input holds
    /// no video stream.
    pub fn compress(&self, input: &[u8], options: CompressionOptions) -> Result<Vec<u8>, Error> {
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

    /// Compresses packet-by-packet instead of into a single container,
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
    pub fn compress_to<F>(
        &self,
        input: &[u8],
        options: CompressionOptions,
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
    fn crf_mapping_covers_both_formats() {
        assert_eq!(crf_for_quality(VideoFormat::Mp4, 0), 51);
        assert_eq!(crf_for_quality(VideoFormat::Mp4, 100), 18);
        assert_eq!(crf_for_quality(VideoFormat::WebM, 0), 63);
        assert_eq!(crf_for_quality(VideoFormat::WebM, 100), 24);
        assert_eq!(crf_for_quality(VideoFormat::Mp4, 200), 18);
    }

    #[test]
    fn mp4_roundtrip_preserves_dimensions() {
        let input = fixture("input.mp4");
        let compressor = VideoCompressor::new().unwrap();
        let output = compressor
            .compress(&input, CompressionOptions::default())
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
        let compressor = VideoCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    width: Some(101),
                    height: Some(57),
                    ..CompressionOptions::default()
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
        let compressor = VideoCompressor::new().unwrap();
        let output = compressor
            .compress(
                &input,
                CompressionOptions {
                    format: VideoFormat::WebM,
                    ..CompressionOptions::default()
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
        let compressor = VideoCompressor::new().unwrap();
        let output = compressor
            .compress(&input, CompressionOptions::default())
            .unwrap();
        let stream = VideoStream::open(&output).unwrap();
        assert!(stream.info().has_audio);
    }

    #[test]
    fn compress_to_emits_params_then_packets() {
        let input = fixture("input.mp4");
        let compressor = VideoCompressor::new().unwrap();
        let mut saw_params = false;
        let mut packets = 0usize;
        let mut saw_keyframe = false;
        compressor
            .compress_to(
                &input,
                CompressionOptions::default(),
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
        let compressor = VideoCompressor::new().unwrap();
        let err = compressor
            .compress(&[], CompressionOptions::default())
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn zero_dimensions_are_invalid() {
        let input = fixture("input.mp4");
        let compressor = VideoCompressor::new().unwrap();
        let err = compressor
            .compress(
                &input,
                CompressionOptions {
                    width: Some(0),
                    ..CompressionOptions::default()
                },
            )
            .unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }

    #[test]
    fn audio_only_input_is_unsupported() {
        let input = wav_fixture();
        let compressor = VideoCompressor::new().unwrap();
        let err = compressor
            .compress(&input, CompressionOptions::default())
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
