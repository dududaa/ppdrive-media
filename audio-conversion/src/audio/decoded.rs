use crate::audio::encode;
use crate::audio::options::AudioFormat;
use crate::error::Error;
use crate::ffi;
use crate::ffi::encode::{EncodedPacket, EncoderParams, InputParams};
use std::ffi::c_void;

/// Highest sample rate the crate will accept from a decoded stream.
///
/// Corrupt container headers can report absurd rates (gigahertz-range
/// values seen in the wild); feeding those into the resampler makes it
/// size multi-gigabyte buffers. Every legitimate codec tops out well
/// below this (FFmpeg's own decoders cap at 768 kHz).
const MAX_SAMPLE_RATE: u32 = 768_000;

/// Highest channel count the crate will accept from a decoded stream.
///
/// Legitimate content is mono/stereo with rare up to 8–16 channel
/// layouts; this bound keeps corrupt headers from inflating resampler
/// and encoder allocations.
const MAX_CHANNELS: u8 = 64;

/// Stream-level properties of decoded audio, read from the first frame.
///
/// Produced by [`DecodedAudio::params`]; used by downstream consumers
/// (e.g. a filter-graph source description) to describe the stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AudioStreamParams {
    /// Samples per second (Hz).
    pub sample_rate: u32,
    /// Channel count (mono/stereo for this crate's pipelines).
    pub channels: u8,
    /// Raw `ffi::AVSampleFormat` value of the decoded frames.
    pub sample_fmt: i32,
    /// FFmpeg channel-layout description, e.g. `"stereo"` or `"mono"`.
    pub channel_layout: String,
}

/// A decoded audio stream: the safe hand-off point between decoding,
/// filtering and encoding.
///
/// Produced by [`DecodedAudio::decode`] (or by a downstream filter
/// pipeline via [`DecodedAudio::from_raw_frames`]) and re-encodable with
/// [`DecodedAudio::encode`] / [`DecodedAudio::encode_to`]. The
/// underlying `AVFrame` pointers are exposed to same-workspace crates
/// through [`DecodedAudio::as_raw_frames`] so filter graphs can run on
/// them directly.
///
/// Frames carry a normalized `pts` timeline: sample offsets from the
/// start of the stream in units of `1 / sample_rate`, which is what
/// time-based filters (`atrim`, `afade`, …) expect.
///
/// # Example
///
/// ```no_run
/// use ppff_audio_conversion::{AudioFormat, DecodedAudio};
///
/// # let bytes: Vec<u8> = Vec::new();
/// let decoded = DecodedAudio::decode(&bytes)?;
/// println!("{} Hz, {} ch, {:.2}s", decoded.sample_rate(), decoded.channels(), decoded.duration_secs());
/// let mp3 = decoded.encode(AudioFormat::Mp3, 85)?;
/// # Ok::<(), ppff_audio_conversion::Error>(())
/// ```
#[derive(Debug)]
pub struct DecodedAudio {
    frames: Vec<ffi::wrappers::Frame>,
    sample_rate: u32,
    channels: u8,
}

impl DecodedAudio {
    pub(crate) fn from_frames(frames: Vec<ffi::wrappers::Frame>) -> Result<DecodedAudio, Error> {
        let first = frames.first().ok_or(Error::InvalidInput)?;
        let sample_rate = first.sample_rate();
        let channels = first.channels();
        if sample_rate == 0
            || sample_rate > MAX_SAMPLE_RATE
            || channels == 0
            || channels > MAX_CHANNELS
        {
            return Err(Error::InvalidInput);
        }
        for frame in &frames {
            unsafe {
                let layout = &mut (*frame.as_ptr()).ch_layout;
                if layout.order == ffi::AV_CHANNEL_ORDER_UNSPEC {
                    ffi::av_channel_layout_uninit(layout);
                    ffi::av_channel_layout_default(layout, i32::from(channels));
                }
            }
        }
        Ok(DecodedAudio {
            frames,
            sample_rate,
            channels,
        })
    }

    /// Decodes an encoded audio stream (any container/codec FFmpeg can
    /// probe from content) into a [`DecodedAudio`].
    ///
    /// Input file extensions are irrelevant. Inputs without an audio
    /// stream are rejected with [`Error::UnsupportedFormat`].
    pub fn decode(input: &[u8]) -> Result<DecodedAudio, Error> {
        crate::audio::decode::decode(input)
    }

    /// Samples per second of the decoded stream.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Channel count of the decoded stream.
    pub fn channels(&self) -> u8 {
        self.channels
    }

    /// Number of decoded frames in the stream.
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Total duration in seconds, measured as decoded samples over the
    /// sample rate.
    pub fn duration_secs(&self) -> f64 {
        let samples: i64 = self.frames.iter().map(|f| i64::from(f.nb_samples())).sum();
        samples as f64 / f64::from(self.sample_rate)
    }

    /// Stream properties read from the first decoded frame.
    ///
    /// Returns [`Error::InvalidInput`] when the stream has no frames.
    pub fn params(&self) -> Result<AudioStreamParams, Error> {
        let first = self.frames.first().ok_or(Error::InvalidInput)?;
        let channel_layout = first.channel_layout_desc().ok_or(Error::InvalidInput)?;
        Ok(AudioStreamParams {
            sample_rate: first.sample_rate(),
            channels: first.channels(),
            sample_fmt: first.sample_fmt(),
            channel_layout,
        })
    }

    pub(crate) fn input_params(&self) -> Result<InputParams, Error> {
        let first = self.frames.first().ok_or(Error::InvalidInput)?;
        Ok(InputParams {
            sample_fmt: first.sample_fmt() as ffi::AVSampleFormat,
            sample_rate: first.sample_rate(),
            channels: first.channels(),
        })
    }

    /// Borrowed `AVFrame *` pointers for every decoded frame, valid
    /// while this [`DecodedAudio`] is alive.
    ///
    /// # Safety contract
    ///
    /// The caller must not free the pointers or outlive the owning
    /// value.
    pub fn as_raw_frames(&self) -> Vec<*mut c_void> {
        self.frames.iter().map(|f| f.as_ptr().cast()).collect()
    }

    /// Takes ownership of raw `AVFrame *` pointers produced by FFmpeg
    /// (e.g. a filter-graph sink) and wraps them as a [`DecodedAudio`].
    ///
    /// Sample rate, channels and duration are derived from the frames
    /// themselves. A uniform `pts` timeline is assigned in sample units.
    ///
    /// # Safety
    ///
    /// - Every pointer must be valid and from `av_frame_alloc` /
    ///   `av_buffersink_get_frame` (or equivalent).
    /// - Ownership is transferred: the pointers must not be freed or
    ///   reused by the caller afterwards.
    /// - The pointers must not be aliased by any other owner for the
    ///   lifetime of the returned [`DecodedAudio`].
    pub unsafe fn from_raw_frames(frames: Vec<*mut c_void>) -> Result<DecodedAudio, Error> {
        if frames.is_empty() {
            return Err(Error::InvalidInput);
        }
        let mut owned = Vec::with_capacity(frames.len());
        for ptr in frames {
            if ptr.is_null() {
                return Err(Error::InvalidInput);
            }
            owned.push(ffi::wrappers::Frame::from_ptr(ptr.cast()));
        }
        let mut samples: i64 = 0;
        for frame in &mut owned {
            unsafe { (*frame.as_ptr()).pts = samples };
            samples += i64::from(frame.nb_samples());
        }
        DecodedAudio::from_frames(owned)
    }

    /// Encodes the stream into `format` at `quality` (0–100).
    ///
    /// Resamples and downmixes as required by the target codec (see
    /// [`crate::ConversionOptions`] for the rate/channel rules).
    pub fn encode(&self, format: AudioFormat, quality: u8) -> Result<Vec<u8>, Error> {
        self.encode_inner(format, quality, None, None)
    }

    pub(crate) fn encode_inner(
        &self,
        format: AudioFormat,
        quality: u8,
        sample_rate: Option<u32>,
        channels: Option<u8>,
    ) -> Result<Vec<u8>, Error> {
        let input = self.input_params()?;
        encode::encode_muxed(&self.frames, &input, format, quality, sample_rate, channels)
    }

    /// Encodes the stream packet-by-packet instead of into a single
    /// container, invoking `sink` with the open encoder's
    /// [`EncoderParams`] once and then every produced [`EncodedPacket`].
    ///
    /// This is the extension point consumed by external packaging
    /// pipelines (adaptive HLS/DASH segmentation) that mux packets into
    /// their own `AVFormatContext`. `global_headers` should be `true`
    /// when the target container stores codec extradata out-of-band
    /// (e.g. fragmented MP4).
    pub fn encode_to<F>(
        &self,
        format: AudioFormat,
        quality: u8,
        global_headers: bool,
        sink: &mut F,
    ) -> Result<(), Error>
    where
        F: FnMut(&EncoderParams, &EncodedPacket),
    {
        let input = self.input_params()?;
        encode::encode_packetized(&self.frames, &input, format, quality, global_headers, sink)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_raw_frames_rejects_empty_and_null() {
        let empty: Vec<*mut c_void> = Vec::new();
        assert_eq!(
            unsafe { DecodedAudio::from_raw_frames(empty) }.unwrap_err(),
            Error::InvalidInput
        );
        let null = vec![std::ptr::null_mut()];
        assert_eq!(
            unsafe { DecodedAudio::from_raw_frames(null) }.unwrap_err(),
            Error::InvalidInput
        );
    }

    #[test]
    fn corrupt_header_with_absurd_sample_rate_is_invalid() {
        let payload: &[u8] = &[
            b'C', b'R', b'Y', b'O', b'_', b'A', b'P', b'C', 0x98, 0x98, 0x98, 0x98, 0x98, 0x98,
            0x2a, 0x98, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54, 0x54,
            0x54, 0x98, 0x98, 0x21, 0x21, 0x21, 0x21, 0x21, 0x21, 0x21, 0x0a,
        ];
        let err = DecodedAudio::decode(payload).unwrap_err();
        assert_eq!(err, Error::InvalidInput);
    }
}
