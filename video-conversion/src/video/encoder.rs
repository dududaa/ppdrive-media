use crate::error::Error;
use crate::ffi;
use crate::ffi::encode::{EncodeConfig, EncodeOutput};
use crate::video::options::VideoFormat;
use crate::video::stream::{AudioPacket, VideoFrame, VideoStreamInfo};

use crate::ffi::encode::Encoder;
pub use crate::ffi::encode::{VideoEncoderParams, VideoPacket};

/// Maps the 0–100 quality slider to the target format's rate-control
/// value (CRF): `51` at 0 → `18` at 100 for the x264/x265 formats
/// ([`VideoFormat::Mp4`], [`VideoFormat::Mov`], [`VideoFormat::Mkv`],
/// [`VideoFormat::Avi`], [`VideoFormat::Mp4Hevc`],
/// [`VideoFormat::MovHevc`]; codec range 0–51), `63` at 0 → `24` at
/// 100 for VP9/AV1 formats ([`VideoFormat::WebM`],
/// [`VideoFormat::Mp4Av1`], [`VideoFormat::WebMAv1`],
/// [`VideoFormat::MkvAv1`]; codec range 0–63). Values above 100 are
/// clamped to 100.
pub fn crf_for_quality(format: VideoFormat, quality: u8) -> u8 {
    let q = u32::from(quality.min(100));
    match format {
        VideoFormat::Mp4
        | VideoFormat::Mov
        | VideoFormat::Mkv
        | VideoFormat::Avi
        | VideoFormat::Mp4Hevc
        | VideoFormat::MovHevc => (51 - q * 33 / 100) as u8,
        VideoFormat::WebM | VideoFormat::Mp4Av1 | VideoFormat::WebMAv1 | VideoFormat::MkvAv1 => {
            (63 - q * 39 / 100) as u8
        }
    }
}

struct EncodeSpec {
    encoder_name: &'static str,
    muxer: &'static str,
    filename: &'static str,
}

fn spec_for(format: VideoFormat) -> EncodeSpec {
    match format {
        VideoFormat::Mp4 => EncodeSpec {
            encoder_name: "libx264",
            muxer: "mp4",
            filename: "out.mp4",
        },
        VideoFormat::WebM => EncodeSpec {
            encoder_name: "libvpx-vp9",
            muxer: "webm",
            filename: "out.webm",
        },
        VideoFormat::Mov => EncodeSpec {
            encoder_name: "libx264",
            muxer: "mov",
            filename: "out.mov",
        },
        VideoFormat::Mkv => EncodeSpec {
            encoder_name: "libx264",
            muxer: "matroska",
            filename: "out.mkv",
        },
        VideoFormat::Avi => EncodeSpec {
            encoder_name: "libx264",
            muxer: "avi",
            filename: "out.avi",
        },
        VideoFormat::Mp4Av1 => EncodeSpec {
            encoder_name: "libaom-av1",
            muxer: "mp4",
            filename: "out.mp4",
        },
        VideoFormat::WebMAv1 => EncodeSpec {
            encoder_name: "libaom-av1",
            muxer: "webm",
            filename: "out.webm",
        },
        VideoFormat::MkvAv1 => EncodeSpec {
            encoder_name: "libaom-av1",
            muxer: "matroska",
            filename: "out.mkv",
        },
        VideoFormat::Mp4Hevc => EncodeSpec {
            encoder_name: "libx265",
            muxer: "mp4",
            filename: "out.mp4",
        },
        VideoFormat::MovHevc => EncodeSpec {
            encoder_name: "libx265",
            muxer: "mov",
            filename: "out.mov",
        },
    }
}

/// Maps `quality` plus the optional `effort` knob to encoder options.
/// `effort` is clamped to 100; `None` keeps the per-family defaults
/// (`preset=veryfast`, `cpu-used=4`).
fn encoder_options(format: VideoFormat, quality: u8, effort: Option<u8>) -> Vec<(String, String)> {
    let crf = crf_for_quality(format, quality).to_string();
    match format {
        VideoFormat::Mp4
        | VideoFormat::Mov
        | VideoFormat::Mkv
        | VideoFormat::Avi
        | VideoFormat::Mp4Hevc
        | VideoFormat::MovHevc => {
            const PRESETS: [&str; 9] = [
                "ultrafast",
                "superfast",
                "veryfast",
                "faster",
                "fast",
                "medium",
                "slow",
                "slower",
                "veryslow",
            ];
            let preset = match effort {
                None => "veryfast".to_string(),
                Some(e) => PRESETS[(u32::from(e.min(100)) * 8 / 100) as usize].to_string(),
            };
            vec![("crf".to_string(), crf), ("preset".to_string(), preset)]
        }
        VideoFormat::WebM => {
            let cpu_used = match effort {
                None => "4".to_string(),
                Some(e) => (8 - u32::from(e.min(100)) * 8 / 100).to_string(),
            };
            vec![
                ("crf".to_string(), crf),
                ("deadline".to_string(), "good".to_string()),
                ("cpu-used".to_string(), cpu_used),
            ]
        }
        VideoFormat::Mp4Av1 | VideoFormat::WebMAv1 | VideoFormat::MkvAv1 => {
            let cpu_used = match effort {
                None => "4".to_string(),
                Some(e) => (8 - u32::from(e.min(100)) * 8 / 100).to_string(),
            };
            vec![
                ("crf".to_string(), crf),
                ("row-mt".to_string(), "1".to_string()),
                ("cpu-used".to_string(), cpu_used),
            ]
        }
    }
}

fn valid_time_base(num: i32, den: i32) -> ffi::AVRational {
    if num > 0 && den > 0 {
        ffi::AVRational { num, den }
    } else {
        ffi::AVRational {
            num: 1,
            den: 1_000_000,
        }
    }
}

/// An open FFmpeg video encoder: feeds decoded [`VideoFrame`]s in,
/// writes muxed bytes (or surfaces packets) out.
///
/// Created in either mode:
/// - [`VideoEncoder::muxed`] — full container written to an in-memory
///   buffer, with an optional stream-copied audio track; returned by
///   [`VideoEncoder::finish`].
/// - [`VideoEncoder::packetized`] — encoded packets handed back through
///   [`VideoEncoder::take_pending`] for an external muxer (adaptive
///   packaging). Audio is not written in this mode.
pub struct VideoEncoder {
    inner: Encoder,
}

impl VideoEncoder {
    #[allow(clippy::too_many_arguments)]
    fn build(
        format: VideoFormat,
        quality: u8,
        effort: Option<u8>,
        fps: Option<u32>,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        output: EncodeOutput,
        audio: Option<crate::ffi::encode::AudioSource>,
    ) -> Result<VideoEncoder, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput);
        }
        if fps == Some(0) {
            return Err(Error::InvalidInput);
        }
        let spec = spec_for(format);
        let codec = ffi::wrappers::find_encoder_by_name(spec.encoder_name)?;
        let (time_base, frame_rate) = match fps {
            Some(fps) => (
                ffi::AVRational {
                    num: 1,
                    den: fps as i32,
                },
                ffi::AVRational {
                    num: fps as i32,
                    den: 1,
                },
            ),
            None => (
                valid_time_base(info.time_base_num, info.time_base_den),
                ffi::AVRational {
                    num: info.frame_rate_num,
                    den: info.frame_rate_den,
                },
            ),
        };
        let config = EncodeConfig {
            codec,
            muxer: spec.muxer.to_string(),
            filename: spec.filename.to_string(),
            width,
            height,
            pix_fmt: ffi::AV_PIX_FMT_YUV420P,
            time_base,
            frame_rate,
            options: encoder_options(format, quality, effort),
            output,
            audio,
        };
        Ok(VideoEncoder {
            inner: Encoder::new(config)?,
        })
    }

    /// Opens a muxed encoder writing `format` at `quality` with output
    /// size `width`×`height`, timestamps in the input stream's time
    /// base (or `1/fps` when `fps` is `Some`), `effort` mapped to the
    /// encoder's speed knob (see [`crate::ConversionOptions`]), and an
    /// optional stream-copied audio track.
    #[allow(clippy::too_many_arguments)]
    pub fn muxed(
        format: VideoFormat,
        quality: u8,
        effort: Option<u8>,
        fps: Option<u32>,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        audio: Option<crate::ffi::encode::AudioSource>,
    ) -> Result<VideoEncoder, Error> {
        Self::build(
            format,
            quality,
            effort,
            fps,
            width,
            height,
            info,
            EncodeOutput::Muxed,
            audio,
        )
    }

    /// Opens a packetized encoder that surfaces encoded packets one at
    /// a time instead of a container. `global_headers` should be `true`
    /// when the target container stores codec extradata out-of-band
    /// (e.g. fragmented MP4).
    #[allow(clippy::too_many_arguments)]
    pub fn packetized(
        format: VideoFormat,
        quality: u8,
        effort: Option<u8>,
        fps: Option<u32>,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        global_headers: bool,
    ) -> Result<VideoEncoder, Error> {
        Self::build(
            format,
            quality,
            effort,
            fps,
            width,
            height,
            info,
            EncodeOutput::Packets { global_headers },
            None,
        )
    }

    /// Stream parameters of the open encoder, sent once to a
    /// packetized sink before the first [`VideoPacket`].
    pub fn params(&self) -> &VideoEncoderParams {
        self.inner.params()
    }

    /// Encodes one frame. The frame's `pts` (in the input stream's
    /// time base) is preserved; frames without a timestamp get
    /// `index × frame_ticks`.
    pub fn encode_frame(&mut self, frame: &mut VideoFrame) -> Result<(), Error> {
        self.inner.encode_frame(frame.raw_mut())
    }

    /// Writes one audio packet to the muxed container (stream copy —
    /// timestamps rescaled to the output stream's time base, never
    /// re-encoded). Fails with [`Error::InvalidInput`] when the encoder
    /// was opened without an audio track or in packetized mode.
    pub fn write_audio_packet(&mut self, packet: &AudioPacket) -> Result<(), Error> {
        self.inner.write_audio_packet(packet.raw())
    }

    /// Takes the packets drained so far in packetized mode (empty for
    /// muxed encoders).
    pub fn take_pending(&mut self) -> Vec<VideoPacket> {
        self.inner.take_pending()
    }

    /// Flushes the codec (sends EOF) and drains the remaining packets.
    /// Call before [`VideoEncoder::finish`] when using a packetized
    /// sink; muxed encoders call it inside `finish`.
    pub fn finish_drain(&mut self) -> Result<(), Error> {
        self.inner.finish_drain()
    }

    /// Flushes, writes the container trailer and returns the encoded
    /// bytes (empty for packetized encoders).
    ///
    /// Fails with [`Error::InvalidInput`] when no frame was ever
    /// encoded — such a container would be empty and unplayable.
    pub fn finish(self) -> Result<Vec<u8>, Error> {
        self.inner.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL: [(VideoFormat, &str, &str, &str); 10] = [
        (VideoFormat::Mp4, "libx264", "mp4", "out.mp4"),
        (VideoFormat::WebM, "libvpx-vp9", "webm", "out.webm"),
        (VideoFormat::Mov, "libx264", "mov", "out.mov"),
        (VideoFormat::Mkv, "libx264", "matroska", "out.mkv"),
        (VideoFormat::Avi, "libx264", "avi", "out.avi"),
        (VideoFormat::Mp4Av1, "libaom-av1", "mp4", "out.mp4"),
        (VideoFormat::WebMAv1, "libaom-av1", "webm", "out.webm"),
        (VideoFormat::MkvAv1, "libaom-av1", "matroska", "out.mkv"),
        (VideoFormat::Mp4Hevc, "libx265", "mp4", "out.mp4"),
        (VideoFormat::MovHevc, "libx265", "mov", "out.mov"),
    ];

    #[test]
    fn spec_pairs_encoder_muxer_and_filename() {
        for (format, encoder, muxer, filename) in ALL {
            let spec = spec_for(format);
            assert_eq!(spec.encoder_name, encoder, "{format:?}");
            assert_eq!(spec.muxer, muxer, "{format:?}");
            assert_eq!(spec.filename, filename, "{format:?}");
        }
    }

    #[test]
    fn encoder_options_carry_crf_and_codec_knobs() {
        for (format, encoder, _, _) in ALL {
            let options = encoder_options(format, 80, None);
            assert!(
                options.iter().any(|(k, _)| k == "crf"),
                "{format:?}: missing crf"
            );
            let knobs: Vec<&str> = options.iter().map(|(k, _)| k.as_str()).collect();
            match encoder {
                "libx264" | "libx265" => assert!(knobs.contains(&"preset"), "{format:?}"),
                "libvpx-vp9" => {
                    assert!(
                        knobs.contains(&"deadline") && knobs.contains(&"cpu-used"),
                        "{format:?}"
                    );
                }
                "libaom-av1" => {
                    assert!(
                        knobs.contains(&"row-mt") && knobs.contains(&"cpu-used"),
                        "{format:?}"
                    );
                }
                other => panic!("unexpected encoder {other} for {format:?}"),
            }
        }
    }

    fn option<'a>(options: &'a [(String, String)], key: &str) -> &'a str {
        &options
            .iter()
            .find(|(k, _)| k == key)
            .unwrap_or_else(|| panic!("missing {key}"))
            .1
    }

    #[test]
    fn effort_none_keeps_defaults() {
        for format in [
            VideoFormat::Mp4,
            VideoFormat::Mov,
            VideoFormat::Mkv,
            VideoFormat::Avi,
            VideoFormat::Mp4Hevc,
            VideoFormat::MovHevc,
        ] {
            let options = encoder_options(format, 80, None);
            assert_eq!(option(&options, "preset"), "veryfast", "{format:?}");
        }
        for format in [VideoFormat::WebM, VideoFormat::Mp4Av1, VideoFormat::MkvAv1] {
            let options = encoder_options(format, 80, None);
            assert_eq!(option(&options, "cpu-used"), "4", "{format:?}");
        }
    }

    #[test]
    fn effort_zero_is_fastest_and_hundred_is_slowest() {
        let fast = encoder_options(VideoFormat::Mp4, 80, Some(0));
        assert_eq!(option(&fast, "preset"), "ultrafast");
        let slow = encoder_options(VideoFormat::Mp4, 80, Some(100));
        assert_eq!(option(&slow, "preset"), "veryslow");
        let clamped = encoder_options(VideoFormat::Mp4, 80, Some(255));
        assert_eq!(option(&clamped, "preset"), "veryslow");

        let fast = encoder_options(VideoFormat::WebM, 80, Some(0));
        assert_eq!(option(&fast, "cpu-used"), "8");
        let slow = encoder_options(VideoFormat::WebM, 80, Some(100));
        assert_eq!(option(&slow, "cpu-used"), "0");

        let fast = encoder_options(VideoFormat::MkvAv1, 80, Some(0));
        assert_eq!(option(&fast, "cpu-used"), "8");
        let slow = encoder_options(VideoFormat::MkvAv1, 80, Some(100));
        assert_eq!(option(&slow, "cpu-used"), "0");
    }

    #[test]
    fn aom_crf_rejects_zero_quality_at_lossless() {
        let low = encoder_options(VideoFormat::Mp4Av1, 0, None);
        let high = encoder_options(VideoFormat::Mp4Av1, 100, None);
        assert!(low.iter().any(|(k, v)| k == "crf" && v == "63"));
        assert!(high.iter().any(|(k, v)| k == "crf" && v == "24"));
    }
}
