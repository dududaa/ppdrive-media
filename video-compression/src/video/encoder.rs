use crate::error::Error;
use crate::ffi;
use crate::ffi::encode::{EncodeConfig, EncodeOutput};
use crate::video::options::VideoFormat;
use crate::video::stream::{AudioPacket, VideoFrame, VideoStreamInfo};

use crate::ffi::encode::Encoder;
pub use crate::ffi::encode::{VideoEncoderParams, VideoPacket};

/// Maps the 0–100 quality slider to the target format's rate-control
/// value (CRF): `51` at 0 → `18` at 100 for [`VideoFormat::Mp4`]
/// (libx264 range 0–51), `63` at 0 → `24` at 100 for
/// [`VideoFormat::WebM`] (libvpx-vp9 range 0–63). Values above 100 are
/// clamped to 100.
pub fn crf_for_quality(format: VideoFormat, quality: u8) -> u8 {
    let q = u32::from(quality.min(100));
    match format {
        VideoFormat::Mp4 => (51 - q * 33 / 100) as u8,
        VideoFormat::WebM => (63 - q * 39 / 100) as u8,
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
    }
}

fn encoder_options(format: VideoFormat, quality: u8) -> Vec<(String, String)> {
    let crf = crf_for_quality(format, quality).to_string();
    match format {
        VideoFormat::Mp4 => vec![
            ("crf".to_string(), crf),
            ("preset".to_string(), "veryfast".to_string()),
        ],
        VideoFormat::WebM => vec![
            ("crf".to_string(), crf),
            ("deadline".to_string(), "good".to_string()),
            ("cpu-used".to_string(), "4".to_string()),
        ],
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
    fn build(
        format: VideoFormat,
        quality: u8,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        output: EncodeOutput,
        audio: Option<crate::ffi::encode::AudioSource>,
    ) -> Result<VideoEncoder, Error> {
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput);
        }
        let spec = spec_for(format);
        let codec = ffi::wrappers::find_encoder_by_name(spec.encoder_name)?;
        let config = EncodeConfig {
            codec,
            muxer: spec.muxer.to_string(),
            filename: spec.filename.to_string(),
            width,
            height,
            pix_fmt: ffi::AV_PIX_FMT_YUV420P,
            time_base: valid_time_base(info.time_base_num, info.time_base_den),
            frame_rate: ffi::AVRational {
                num: info.frame_rate_num,
                den: info.frame_rate_den,
            },
            options: encoder_options(format, quality),
            output,
            audio,
        };
        Ok(VideoEncoder {
            inner: Encoder::new(config)?,
        })
    }

    /// Opens a muxed encoder writing `format` at `quality` with output
    /// size `width`×`height`, timestamps in the input stream's time
    /// base, and an optional stream-copied audio track.
    pub fn muxed(
        format: VideoFormat,
        quality: u8,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        audio: Option<crate::ffi::encode::AudioSource>,
    ) -> Result<VideoEncoder, Error> {
        Self::build(
            format,
            quality,
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
    pub fn packetized(
        format: VideoFormat,
        quality: u8,
        width: u32,
        height: u32,
        info: &VideoStreamInfo,
        global_headers: bool,
    ) -> Result<VideoEncoder, Error> {
        Self::build(
            format,
            quality,
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
