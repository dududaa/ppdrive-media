use crate::error::Error;
use crate::ffi;
use crate::ffi::wrappers::CodecParameters;
use crate::video::options::VideoFormat;
use std::os::raw::c_void;

/// A decoded video frame: the safe hand-off point between demuxing and
/// encoding.
///
/// Produced by [`VideoStream::next_event`]. Frames carry the presentation
/// timestamp of the source stream in the stream's time base (or
/// `AV_NOPTS_VALUE` when the source omits it — encoders then synthesize
/// a timestamp from the frame index).
pub struct VideoFrame {
    inner: ffi::wrappers::Frame,
}

impl VideoFrame {
    pub(crate) fn new(inner: ffi::wrappers::Frame) -> VideoFrame {
        VideoFrame { inner }
    }

    /// Frame width in pixels.
    pub fn width(&self) -> u32 {
        self.inner.width()
    }

    /// Frame height in pixels.
    pub fn height(&self) -> u32 {
        self.inner.height()
    }

    /// Presentation timestamp in the source stream's time base, or
    /// `AV_NOPTS_VALUE` (i64::MIN) when the source omits it.
    pub fn pts(&self) -> i64 {
        self.inner.pts()
    }

    /// Overwrites the presentation timestamp (same time base as
    /// [`VideoFrame::pts`]) — lets callers re-base a clip's timeline
    /// without touching pixel data.
    pub fn set_pts(&mut self, pts: i64) {
        self.inner.set_pts(pts);
    }

    /// Raw `AVFrame *` for direct FFmpeg interop (filter graphs).
    ///
    /// # Safety contract
    ///
    /// The pointer is valid only while this [`VideoFrame`] is alive; the
    /// caller must not free it or outlive the owning value.
    pub fn as_raw_frame(&self) -> *mut c_void {
        self.inner.as_ptr().cast()
    }

    /// Takes ownership of a raw `AVFrame *` produced by FFmpeg (e.g. a
    /// filter-graph sink frame) and wraps it as a [`VideoFrame`].
    ///
    /// # Safety
    ///
    /// - `raw` must be a valid pointer from `av_frame_alloc` /
    ///   `av_buffersink_get_frame` (or equivalent).
    /// - Ownership is transferred: the pointer must not be freed or
    ///   reused by the caller afterwards.
    /// - `raw` must not be aliased by any other owner for the lifetime
    ///   of the returned [`VideoFrame`].
    pub unsafe fn from_raw(raw: *mut c_void) -> Result<VideoFrame, Error> {
        if raw.is_null() {
            return Err(Error::InvalidInput);
        }
        Ok(VideoFrame::new(ffi::wrappers::Frame::from_ptr(raw.cast())))
    }

    pub(crate) fn raw_mut(&mut self) -> &mut ffi::wrappers::Frame {
        &mut self.inner
    }
}

/// A demuxed audio packet held for stream-copy — never decoded.
pub struct AudioPacket {
    inner: ffi::wrappers::Packet,
}

impl AudioPacket {
    pub(crate) fn new(inner: ffi::wrappers::Packet) -> AudioPacket {
        AudioPacket { inner }
    }

    /// Presentation timestamp in the audio stream's time base.
    pub fn pts(&self) -> i64 {
        self.inner.pts()
    }

    /// Overwrites the presentation timestamp (same time base as
    /// [`AudioPacket::pts`]) — lets callers re-base a clip's timeline
    /// without touching packet data.
    pub fn set_pts(&mut self, pts: i64) {
        self.inner.set_pts(pts);
    }

    /// Decode timestamp (same time base as [`AudioPacket::pts`]).
    pub fn dts(&self) -> i64 {
        self.inner.dts()
    }

    /// Overwrites the decode timestamp (same time base as
    /// [`AudioPacket::pts`]) — must move together with
    /// [`AudioPacket::set_pts`] or the muxer rejects the packet.
    pub fn set_dts(&mut self, dts: i64) {
        self.inner.set_dts(dts);
    }

    /// Duration in the audio stream's time base.
    pub fn duration(&self) -> i64 {
        self.inner.duration()
    }

    /// Encoded payload size in bytes.
    pub fn size(&self) -> usize {
        self.inner.size()
    }

    pub(crate) fn raw(&self) -> &ffi::wrappers::Packet {
        &self.inner
    }
}

/// One demuxed event from [`VideoStream::next_event`].
pub enum StreamEvent {
    /// A decoded video frame ready for encoding.
    Video(VideoFrame),
    /// A raw audio packet for stream-copy (only emitted when the input
    /// carries an audio track).
    Audio(AudioPacket),
}

/// Probed description of an opened input video.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoStreamInfo {
    /// Coded width in pixels.
    pub width: u32,
    /// Coded height in pixels.
    pub height: u32,
    /// Coded pixel format as an `AVPixelFormat` value (`-1` when the
    /// container does not say).
    pub pix_fmt: i32,
    /// Average frame rate numerator (`fps = num / den`).
    pub frame_rate_num: i32,
    /// Average frame rate denominator.
    pub frame_rate_den: i32,
    /// Stream time base numerator (timestamps are in this base).
    pub time_base_num: i32,
    /// Stream time base denominator.
    pub time_base_den: i32,
    /// Container duration in seconds; `None` when the input does not
    /// report one.
    pub duration: Option<f64>,
    /// FFmpeg demuxer name the input was probed as (content-based).
    pub format_name: Option<String>,
    /// Whether the input carries an audio track alongside the video.
    pub has_audio: bool,
}

impl VideoStreamInfo {
    /// Frame rate in frames per second, defaulting to 25.0 when the
    /// input does not report one.
    pub fn frame_rate(&self) -> f64 {
        if self.frame_rate_num > 0 && self.frame_rate_den > 0 {
            f64::from(self.frame_rate_num) / f64::from(self.frame_rate_den)
        } else {
            25.0
        }
    }
}

/// Demuxed input video: iterated event-by-event, feeding both the
/// decoder's frames and the raw audio packets of a stream copy.
///
/// The input bytes are borrowed for the lifetime of the stream — the
/// returned value owns all FFmpeg state and releases it on drop.
pub struct VideoStream<'a> {
    demux: ffi::Demuxer<'a>,
    info: VideoStreamInfo,
}

impl<'a> VideoStream<'a> {
    /// Opens and probes `input` (any container FFmpeg can probe from
    /// content) and decodes its video stream.
    ///
    /// Returns [`Error::InvalidInput`] for empty or unprobeable input
    /// and [`Error::UnsupportedFormat`] when the input holds no video
    /// stream.
    pub fn open(input: &'a [u8]) -> Result<VideoStream<'a>, Error> {
        if input.is_empty() {
            return Err(Error::InvalidInput);
        }
        let reader = ffi::AvioReader::new(input)?;
        let demux = ffi::Demuxer::open(reader)?;
        let info = probe_info(&demux)?;
        Ok(VideoStream { demux, info })
    }

    /// Probed description of the input video.
    pub fn info(&self) -> &VideoStreamInfo {
        &self.info
    }

    /// Builds an [`AudioSource`] for stream-copying the input's audio
    /// track into `format`, or `None` when the input has no audio track
    /// or its codec is not accepted by the target container (the audio
    /// is then dropped, never transcoded).
    pub fn audio_source_for(&self, format: VideoFormat) -> Option<ffi::encode::AudioSource> {
        let stream = self.demux.audio_stream()?;
        let mut params = CodecParameters::new().ok()?;
        unsafe {
            params.copy_from((*stream).codecpar).ok()?;
            if !format.accepts_audio_codec(params.codec_id()) {
                return None;
            }
            let tb = (*stream).time_base;
            Some(ffi::encode::AudioSource::new(
                params,
                ffi::AVRational {
                    num: tb.num,
                    den: tb.den,
                },
            ))
        }
    }

    /// Reads the next event: decoded video frames and raw audio packets
    /// in demux order. Returns `Ok(None)` once the video stream is
    /// fully drained.
    pub fn next_event(&mut self) -> Result<Option<StreamEvent>, Error> {
        Ok(match self.demux.next_event()? {
            None => None,
            Some(ffi::demux::DemuxEvent::Video(frame)) => {
                Some(StreamEvent::Video(VideoFrame::new(frame)))
            }
            Some(ffi::demux::DemuxEvent::Audio(packet)) => {
                Some(StreamEvent::Audio(AudioPacket::new(packet)))
            }
        })
    }
}

fn probe_info(demux: &ffi::Demuxer<'_>) -> Result<VideoStreamInfo, Error> {
    unsafe {
        let stream = demux.video_stream();
        let par = (*stream).codecpar;
        let width = (*par).width.max(0) as u32;
        let height = (*par).height.max(0) as u32;
        if width == 0 || height == 0 {
            return Err(Error::InvalidInput);
        }

        let avg = (*stream).avg_frame_rate;
        let raw = (*stream).r_frame_rate;
        let (frame_rate_num, frame_rate_den) = if avg.num > 0 && avg.den > 0 {
            (avg.num, avg.den)
        } else if raw.num > 0 && raw.den > 0 {
            (raw.num, raw.den)
        } else {
            (25, 1)
        };

        let tb = (*stream).time_base;
        let format_name = demux.format_name().map(|name| name.to_string());

        Ok(VideoStreamInfo {
            width,
            height,
            pix_fmt: (*par).format,
            frame_rate_num,
            frame_rate_den,
            time_base_num: tb.num,
            time_base_den: tb.den,
            duration: demux.duration_secs(),
            format_name,
            has_audio: demux.audio_stream().is_some(),
        })
    }
}
