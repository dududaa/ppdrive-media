use super::io::AvioWriter;
use super::wrappers::{CodecContext, EncoderHandle, Frame, Packet, to_cstr};
use super::*;
use crate::error::Error;
use std::ptr;

pub struct EncodeConfig {
    pub codec: EncoderHandle,
    pub muxer: String,
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: AVPixelFormat,
    pub options: Vec<(String, String)>,
    pub qscale: Option<i32>,
}

struct FmtGuard(*mut AVFormatContext);

impl FmtGuard {
    fn disarm(&mut self) -> *mut AVFormatContext {
        let fmt = self.0;
        self.0 = ptr::null_mut();
        fmt
    }
}

impl Drop for FmtGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { avformat_free_context(self.0) };
        }
    }
}

pub struct Encoder {
    codec_ctx: CodecContext,
    fmt: *mut AVFormatContext,
    writer: AvioWriter,
}

impl Encoder {
    pub fn new(config: EncodeConfig) -> Result<Encoder, Error> {
        let mut writer = AvioWriter::new()?;

        let mut guard = unsafe {
            let muxer = to_cstr(&config.muxer)?;
            let filename = to_cstr(&config.filename)?;
            let mut fmt: *mut AVFormatContext = ptr::null_mut();
            let ret = avformat_alloc_output_context2(
                &mut fmt,
                ptr::null_mut(),
                muxer.as_ptr(),
                filename.as_ptr(),
            );
            if ret < 0 || fmt.is_null() {
                return Err(Error::from_code(ret));
            }
            (*fmt).pb = writer.as_ptr();
            (*fmt).flags |= AVFMT_FLAG_CUSTOM_IO as c_int;
            FmtGuard(fmt)
        };

        let mut codec_ctx = CodecContext::alloc(config.codec.as_ptr())?;
        codec_ctx.set_dimensions(config.width, config.height);
        codec_ctx.set_pix_fmt(config.pix_fmt);
        codec_ctx.set_time_base(1, 25);

        unsafe {
            if (*guard.0).flags & AVFMT_GLOBALHEADER as c_int != 0 {
                codec_ctx.set_global_header();
            }
        }

        if let Some(qscale) = config.qscale {
            codec_ctx.set_qscale(qscale);
        }

        codec_ctx.open(config.codec.as_ptr(), &config.options)?;

        unsafe {
            let stream = avformat_new_stream(guard.0, ptr::null());
            if stream.is_null() {
                return Err(Error::FfmpegError(
                    "avformat_new_stream returned null".to_string(),
                ));
            }
            (*stream).time_base = (*codec_ctx.as_ptr()).time_base;
            let ret = avcodec_parameters_from_context((*stream).codecpar, codec_ctx.as_ptr());
            if ret < 0 {
                return Err(Error::from_code(ret));
            }

            let mut header_opts: *mut AVDictionary = ptr::null_mut();
            let ret = avformat_write_header(guard.0, &mut header_opts);
            av_dict_free(&mut header_opts);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
        }

        let fmt = guard.disarm();
        Ok(Encoder {
            codec_ctx,
            fmt,
            writer,
        })
    }

    pub fn encode_frame(&mut self, frame: &Frame) -> Result<(), Error> {
        unsafe {
            (*frame.as_ptr()).quality = (*self.codec_ctx.as_ptr()).global_quality;
        }
        self.codec_ctx.send_frame(frame.as_ptr())?;
        self.drain_packets()
    }

    fn drain_packets(&mut self) -> Result<(), Error> {
        let mut packet = Packet::new()?;
        loop {
            let r = unsafe { avcodec_receive_packet(self.codec_ctx.as_ptr(), packet.as_ptr()) };
            if r == 0 {
                unsafe {
                    let pkt = packet.as_ptr();
                    if (*pkt).pts == AV_NOPTS_VALUE {
                        (*pkt).pts = 0;
                    }
                    if (*pkt).dts == AV_NOPTS_VALUE {
                        (*pkt).dts = (*pkt).pts;
                    }
                }
                let ret = unsafe { av_interleaved_write_frame(self.fmt, packet.as_ptr()) };
                if ret < 0 {
                    return Err(Error::from_code(ret));
                }
                packet.unref();
                continue;
            }
            if is_eagain(r) || r == AVERROR_EOF_CODE {
                return Ok(());
            }
            return Err(Error::from_code(r));
        }
    }

    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        self.codec_ctx.send_frame(ptr::null())?;
        self.drain_packets()?;

        unsafe {
            let ret = av_write_trailer(self.fmt);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            if !(*self.fmt).pb.is_null() {
                avio_flush((*self.fmt).pb);
            }
        }

        let bytes = self.writer.take_bytes();
        Ok(bytes)
    }
}

impl Drop for Encoder {
    fn drop(&mut self) {
        unsafe {
            if !self.fmt.is_null() {
                avformat_free_context(self.fmt);
                self.fmt = ptr::null_mut();
            }
        }
    }
}
