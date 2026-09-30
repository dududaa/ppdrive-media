use super::io::AvioReader;
use super::wrappers::{CodecContext, Frame, Packet};
use super::*;
use crate::error::Error;
use std::ptr;

/// One demuxed event: either a decoded video frame or a raw audio
/// packet handed over for stream-copy (no decoding).
pub enum DemuxEvent {
    Video(Frame),
    Audio(Packet),
}

pub struct Demuxer<'a> {
    fmt: *mut AVFormatContext,
    #[allow(dead_code, reason = "owns the reader for the lifetime of fmt")]
    reader: AvioReader<'a>,
    video_codec: CodecContext,
    read_packet: Packet,
    video_idx: c_int,
    audio_idx: c_int,
    input_eof: bool,
    flush_sent: bool,
}

impl<'a> Demuxer<'a> {
    pub fn open(mut reader: AvioReader<'a>) -> Result<Demuxer<'a>, Error> {
        unsafe {
            let fmt = avformat_alloc_context();
            if fmt.is_null() {
                return Err(Error::FfmpegError(
                    "avformat_alloc_context returned null".to_string(),
                ));
            }

            (*fmt).pb = reader.as_ptr();
            (*fmt).flags |= AVFMT_FLAG_CUSTOM_IO as c_int;

            let mut fmt = fmt;
            let ret = avformat_open_input(&mut fmt, ptr::null(), ptr::null(), ptr::null_mut());
            if ret < 0 {
                return Err(Error::InvalidInput);
            }

            let ret = avformat_find_stream_info(fmt, ptr::null_mut());
            if ret < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::from_code(ret));
            }

            let mut codec_raw: *const AVCodec = ptr::null();
            let video_idx = av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, &mut codec_raw, 0);
            if video_idx < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::UnsupportedFormat);
            }
            if codec_raw.is_null() {
                avformat_close_input(&mut fmt);
                return Err(Error::DecoderNotFound);
            }

            let video_codec = CodecContext::alloc(codec_raw)?;
            let par = (*(*(*fmt).streams.add(video_idx as usize))).codecpar;
            let ret = avcodec_parameters_to_context(video_codec.as_ptr(), par);
            if ret < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::from_code(ret));
            }
            let ret = avcodec_open2(video_codec.as_ptr(), codec_raw, ptr::null_mut());
            if ret < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::from_code(ret));
            }

            let audio_idx =
                av_find_best_stream(fmt, AVMEDIA_TYPE_AUDIO, -1, -1, ptr::null_mut(), 0);
            let audio_idx = if audio_idx >= 0 { audio_idx } else { -1 };

            let read_packet = Packet::new()?;

            Ok(Demuxer {
                fmt,
                reader,
                video_codec,
                read_packet,
                video_idx,
                audio_idx,
                input_eof: false,
                flush_sent: false,
            })
        }
    }

    /// Name of the probed input demuxer (e.g. `"mov,mp4,m4a,3gp,3g2,mj2"`),
    /// as reported by FFmpeg's content probe.
    pub fn format_name(&self) -> Option<&str> {
        unsafe {
            let iformat = (*self.fmt).iformat;
            if iformat.is_null() || (*iformat).name.is_null() {
                return None;
            }
            std::ffi::CStr::from_ptr((*iformat).name).to_str().ok()
        }
    }

    pub fn video_stream(&self) -> *mut AVStream {
        unsafe { *(*self.fmt).streams.add(self.video_idx as usize) }
    }

    pub fn audio_stream(&self) -> Option<*mut AVStream> {
        if self.audio_idx < 0 {
            return None;
        }
        unsafe { Some(*(*self.fmt).streams.add(self.audio_idx as usize)) }
    }

    /// Container duration in seconds (`None` when the input does not
    /// report one).
    pub fn duration_secs(&self) -> Option<f64> {
        unsafe {
            if (*self.fmt).duration != AV_NOPTS_VALUE && (*self.fmt).duration > 0 {
                return Some((*self.fmt).duration as f64 / f64::from(AV_TIME_BASE));
            }
            let stream = self.video_stream();
            let duration = (*stream).duration;
            if duration != AV_NOPTS_VALUE && duration > 0 {
                let tb = (*stream).time_base;
                if tb.num > 0 {
                    return Some(duration as f64 * f64::from(tb.num) / f64::from(tb.den));
                }
            }
            None
        }
    }

    /// Reads the next event: decoded video frames and raw audio packets
    /// in demux order. Returns `Ok(None)` once the video stream is
    /// fully drained.
    pub fn next_event(&mut self) -> Result<Option<DemuxEvent>, Error> {
        loop {
            let frame = Frame::new()?;
            let r = unsafe { avcodec_receive_frame(self.video_codec.as_ptr(), frame.as_ptr()) };
            if r == 0 {
                return Ok(Some(DemuxEvent::Video(frame)));
            }
            if !is_eagain(r) && r != AVERROR_EOF_CODE {
                return Err(Error::from_code(r));
            }

            if self.input_eof {
                if self.flush_sent {
                    return Ok(None);
                }
                let s = unsafe { avcodec_send_packet(self.video_codec.as_ptr(), ptr::null()) };
                if is_eagain(s) {
                    continue;
                }
                if s < 0 && s != AVERROR_EOF_CODE {
                    return Err(Error::from_code(s));
                }
                self.flush_sent = true;
                continue;
            }

            let pr = unsafe { av_read_frame(self.fmt, self.read_packet.as_ptr()) };
            if pr < 0 {
                self.input_eof = true;
                continue;
            }

            let idx = unsafe { (*self.read_packet.as_ptr()).stream_index };
            if idx == self.video_idx {
                let s = unsafe {
                    avcodec_send_packet(self.video_codec.as_ptr(), self.read_packet.as_ptr())
                };
                self.read_packet.unref();
                if s < 0 && !is_eagain(s) && s != AVERROR_EOF_CODE {
                    return Err(Error::from_code(s));
                }
                continue;
            }
            if idx == self.audio_idx {
                let mut owned = Packet::new()?;
                owned.ref_from(self.read_packet.as_ptr())?;
                self.read_packet.unref();
                return Ok(Some(DemuxEvent::Audio(owned)));
            }
            self.read_packet.unref();
        }
    }
}

impl<'a> Drop for Demuxer<'a> {
    fn drop(&mut self) {
        unsafe { avformat_close_input(&mut self.fmt) };
    }
}
