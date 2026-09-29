use super::io::AvioWriter;
use super::resample::Swr;
use super::wrappers::{CodecContext, EncoderHandle, Frame, Packet, to_cstr};
use super::*;
use crate::error::Error;
use std::ptr;

pub struct InputParams {
    pub sample_fmt: AVSampleFormat,
    pub sample_rate: u32,
    pub channels: u8,
}

pub enum EncodeOutput {
    Muxed,
    Packets { global_headers: bool },
}

pub struct EncodeConfig {
    pub codec: EncoderHandle,
    pub muxer: String,
    pub filename: String,
    pub sample_fmt: AVSampleFormat,
    pub sample_rate: u32,
    pub channels: u8,
    pub bit_rate: Option<u64>,
    pub options: Vec<(String, String)>,
    pub output: EncodeOutput,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncoderParams {
    pub codec_name: String,
    pub sample_rate: u32,
    pub channels: u8,
    pub sample_fmt: String,
    pub bitrate_bps: u64,
    pub extradata: Vec<u8>,
    pub time_base_num: i32,
    pub time_base_den: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EncodedPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub tb_num: i32,
    pub tb_den: i32,
    pub duration: i64,
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
    swr: Swr,
    fmt: *mut AVFormatContext,
    writer: Option<AvioWriter>,
    pending: Vec<EncodedPacket>,
    params: EncoderParams,
    next_pts: i64,
    frame_size: c_int,
    out_fmt: AVSampleFormat,
    out_channels: u8,
    staged: Option<Frame>,
    staged_len: c_int,
}

impl Encoder {
    pub fn new(config: EncodeConfig, input: &InputParams) -> Result<Encoder, Error> {
        let muxed = matches!(config.output, EncodeOutput::Muxed);
        let (writer, mut guard) = if muxed {
            let mut writer = AvioWriter::new()?;
            let guard = unsafe {
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
            (Some(writer), guard)
        } else {
            (None, FmtGuard(ptr::null_mut()))
        };

        let mut codec_ctx = CodecContext::alloc(config.codec.as_ptr())?;
        codec_ctx.set_sample_fmt(config.sample_fmt);
        codec_ctx.set_sample_rate(config.sample_rate);
        codec_ctx.set_channels(config.channels);
        codec_ctx.set_time_base(1, config.sample_rate as c_int);
        if let Some(bit_rate) = config.bit_rate {
            codec_ctx.set_bit_rate(bit_rate);
        }

        unsafe {
            if !guard.0.is_null() && (*guard.0).flags & AVFMT_GLOBALHEADER as c_int != 0 {
                codec_ctx.set_global_header();
            }
        }
        if let EncodeOutput::Packets {
            global_headers: true,
        } = config.output
        {
            codec_ctx.set_global_header();
        }

        codec_ctx.open(config.codec.as_ptr(), &config.options)?;

        let frame_size = unsafe { (*codec_ctx.as_ptr()).frame_size };
        let staged = if frame_size > 0 {
            Some(unsafe { Self::alloc_staged(codec_ctx.as_ptr(), frame_size)? })
        } else {
            None
        };

        let swr = Swr::new(
            input.sample_fmt,
            input.sample_rate,
            input.channels,
            config.sample_fmt,
            config.sample_rate,
            config.channels,
        )?;

        if muxed {
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
        }

        let fmt = guard.disarm();
        let sample_fmt_name = unsafe {
            let name = av_get_sample_fmt_name(config.sample_fmt);
            if name.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(name)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        let params = EncoderParams {
            codec_name: config.codec.name(),
            sample_rate: config.sample_rate,
            channels: config.channels,
            sample_fmt: sample_fmt_name,
            bitrate_bps: config.bit_rate.unwrap_or(0),
            extradata: codec_ctx.extradata(),
            time_base_num: 1,
            time_base_den: config.sample_rate as i32,
        };

        Ok(Encoder {
            codec_ctx,
            swr,
            fmt,
            writer,
            pending: Vec::new(),
            params,
            next_pts: 0,
            frame_size,
            out_fmt: config.sample_fmt,
            out_channels: config.channels,
            staged,
            staged_len: 0,
        })
    }

    unsafe fn alloc_staged(ctx: *mut AVCodecContext, frame_size: c_int) -> Result<Frame, Error> {
        unsafe {
            let frame = Frame::new()?;
            (*frame.as_ptr()).format = (*ctx).sample_fmt;
            (*frame.as_ptr()).sample_rate = (*ctx).sample_rate;
            av_channel_layout_default(
                &mut (*frame.as_ptr()).ch_layout,
                (*ctx).ch_layout.nb_channels,
            );
            (*frame.as_ptr()).nb_samples = frame_size;
            let ret = av_frame_get_buffer(frame.as_ptr(), 0);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            Ok(frame)
        }
    }

    pub fn params(&self) -> &EncoderParams {
        &self.params
    }

    pub fn take_pending(&mut self) -> Vec<EncodedPacket> {
        std::mem::take(&mut self.pending)
    }

    pub fn encode_frame(&mut self, frame: &Frame) -> Result<(), Error> {
        let Some(out) = self.swr.convert(Some(frame))? else {
            return Ok(());
        };
        self.append(out)
    }

    fn append(&mut self, out: Frame) -> Result<(), Error> {
        if self.frame_size <= 0 {
            return self.submit(out);
        }
        let src = out.as_ptr();
        unsafe {
            let remaining_total = (*src).nb_samples;
            let bytes = av_get_bytes_per_sample(self.out_fmt) as usize;
            let planar = av_sample_fmt_is_planar(self.out_fmt) != 0;
            let planes: usize = if planar {
                self.out_channels as usize
            } else {
                1
            };
            let stride = if planar {
                bytes
            } else {
                bytes * self.out_channels as usize
            };

            let mut off: c_int = 0;
            let mut remaining = remaining_total;
            while remaining > 0 {
                let take = remaining.min(self.frame_size - self.staged_len);
                let dst = self
                    .staged
                    .as_ref()
                    .ok_or_else(|| Error::FfmpegError("staging frame missing".to_string()))?
                    .as_ptr();
                for plane in 0..planes {
                    std::ptr::copy_nonoverlapping(
                        (*src).data[plane].add(off as usize * stride),
                        (*dst).data[plane].add(self.staged_len as usize * stride),
                        take as usize * stride,
                    );
                }
                self.staged_len += take;
                off += take;
                remaining -= take;
                if self.staged_len == self.frame_size {
                    self.submit_staged()?;
                }
            }
        }
        Ok(())
    }

    fn submit_staged(&mut self) -> Result<(), Error> {
        let nb = self.staged_len;
        let staged = self
            .staged
            .take()
            .ok_or_else(|| Error::FfmpegError("staging frame missing".to_string()))?;
        unsafe { (*staged.as_ptr()).nb_samples = nb };
        self.staged_len = 0;
        let result = self.submit(staged);
        self.staged =
            Some(unsafe { Self::alloc_staged(self.codec_ctx.as_ptr(), self.frame_size)? });
        result
    }

    fn submit(&mut self, out: Frame) -> Result<(), Error> {
        unsafe {
            (*out.as_ptr()).pts = self.next_pts;
            self.next_pts += (*out.as_ptr()).nb_samples as i64;
        }
        self.codec_ctx.send_frame(out.as_ptr())?;
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
                if self.fmt.is_null() {
                    let pkt = packet.as_ptr();
                    let (data, pts, duration) = unsafe {
                        let slice = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                        (slice.to_vec(), (*pkt).pts, (*pkt).duration)
                    };
                    self.pending.push(EncodedPacket {
                        data,
                        pts,
                        tb_num: self.params.time_base_num,
                        tb_den: self.params.time_base_den,
                        duration,
                    });
                    packet.unref();
                    continue;
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

    pub fn flush(&mut self) -> Result<(), Error> {
        while let Some(tail) = self.swr.convert(None)? {
            self.append(tail)?;
        }
        if self.frame_size > 0 && self.staged_len > 0 {
            return self.submit_staged();
        }
        Ok(())
    }

    pub fn finish_drain(&mut self) -> Result<(), Error> {
        self.flush()?;
        self.codec_ctx.send_frame(ptr::null())?;
        self.drain_packets()
    }

    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        self.finish_drain()?;

        if let Some(mut writer) = self.writer.take() {
            unsafe {
                if !self.fmt.is_null() {
                    let ret = av_write_trailer(self.fmt);
                    if ret < 0 {
                        return Err(Error::from_code(ret));
                    }
                    if !(*self.fmt).pb.is_null() {
                        avio_flush((*self.fmt).pb);
                    }
                }
            }
            Ok(writer.take_bytes())
        } else {
            Ok(Vec::new())
        }
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
