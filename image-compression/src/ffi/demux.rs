use super::io::AvioReader;
use super::wrappers::{CodecContext, Frame, Packet};
use super::*;
use crate::error::Error;
use std::ptr;

pub struct Demuxer<'a> {
    fmt: *mut AVFormatContext,
    #[allow(dead_code, reason = "owns the reader for the lifetime of fmt")]
    reader: AvioReader<'a>,
    codec: CodecContext,
    stream_idx: c_int,
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
            let stream_idx =
                av_find_best_stream(fmt, AVMEDIA_TYPE_VIDEO, -1, -1, &mut codec_raw, 0);
            if stream_idx < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::InvalidInput);
            }
            if codec_raw.is_null() {
                avformat_close_input(&mut fmt);
                return Err(Error::DecoderNotFound);
            }

            let codec = CodecContext::alloc(codec_raw)?;
            let par = (*(*(*fmt).streams.add(stream_idx as usize))).codecpar;
            let ret = avcodec_parameters_to_context(codec.as_ptr(), par);
            if ret < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::from_code(ret));
            }
            let ret = avcodec_open2(codec.as_ptr(), codec_raw, ptr::null_mut());
            if ret < 0 {
                avformat_close_input(&mut fmt);
                return Err(Error::from_code(ret));
            }

            Ok(Demuxer {
                fmt,
                reader,
                codec,
                stream_idx,
            })
        }
    }

    /// Name of the probed input demuxer (e.g. `"jpeg_pipe"`), as reported
    /// by FFmpeg's content probe.
    pub fn format_name(&self) -> Option<&str> {
        unsafe {
            let iformat = (*self.fmt).iformat;
            if iformat.is_null() || (*iformat).name.is_null() {
                return None;
            }
            std::ffi::CStr::from_ptr((*iformat).name).to_str().ok()
        }
    }

    pub fn read_video_frame(&mut self) -> Result<Frame, Error> {
        let frame = Frame::new()?;
        let mut packet = Packet::new()?;
        let mut input_drained = false;
        let mut flush_sent = false;

        loop {
            let r = unsafe { avcodec_receive_frame(self.codec.as_ptr(), frame.as_ptr()) };
            if r == 0 {
                unsafe { (*frame.as_ptr()).pts = 0 };
                return Ok(frame);
            }
            if !is_eagain(r) && r != AVERROR_EOF_CODE {
                return Err(Error::from_code(r));
            }

            if input_drained {
                if !flush_sent {
                    let s = unsafe { avcodec_send_packet(self.codec.as_ptr(), ptr::null()) };
                    if is_eagain(s) {
                        continue;
                    }
                    if s < 0 && s != AVERROR_EOF_CODE {
                        return Err(Error::from_code(s));
                    }
                    flush_sent = true;
                    continue;
                }
                return Err(Error::InvalidInput);
            }

            let pr = unsafe { av_read_frame(self.fmt, packet.as_ptr()) };
            if pr >= 0 {
                if unsafe { (*packet.as_ptr()).stream_index } == self.stream_idx {
                    let s = unsafe { avcodec_send_packet(self.codec.as_ptr(), packet.as_ptr()) };
                    packet.unref();
                    if s < 0 && !is_eagain(s) && s != AVERROR_EOF_CODE {
                        return Err(Error::from_code(s));
                    }
                } else {
                    packet.unref();
                }
                continue;
            }
            input_drained = true;
        }
    }
}

impl<'a> Drop for Demuxer<'a> {
    fn drop(&mut self) {
        unsafe { avformat_close_input(&mut self.fmt) };
    }
}
