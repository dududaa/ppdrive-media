use super::io::AvioWriter;
use super::wrappers::{
    CodecContext, CodecParameters, EncoderHandle, Frame, Packet, Scaler, to_cstr,
};
use super::*;
use crate::error::Error;
use std::ptr;

pub enum EncodeOutput {
    Muxed,
    Packets { global_headers: bool },
}

pub struct EncodeConfig {
    pub codec: EncoderHandle,
    pub muxer: String,
    pub filename: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: AVPixelFormat,
    pub time_base: AVRational,
    pub frame_rate: AVRational,
    pub options: Vec<(String, String)>,
    pub output: EncodeOutput,
    pub audio: Option<AudioSource>,
}

/// A demuxed audio stream held for stream-copy: its codec parameters
/// (cloned, never decoded) and the input time base its packet
/// timestamps are expressed in.
pub struct AudioSource {
    params: CodecParameters,
    time_base: AVRational,
}

impl AudioSource {
    pub fn new(params: CodecParameters, time_base: AVRational) -> AudioSource {
        AudioSource { params, time_base }
    }

    /// Raw `AVCodecID` of the audio codec to be copied.
    pub fn codec_id(&self) -> u32 {
        self.params.codec_id()
    }

    /// Input time base of the audio packets, as `(num, den)`.
    pub fn time_base(&self) -> (i32, i32) {
        (self.time_base.num, self.time_base.den)
    }
}

/// Everything a muxer needs to open a video rendition stream, sent to
/// the packetized sink once before the first [`VideoPacket`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoEncoderParams {
    pub codec_name: String,
    pub width: u32,
    pub height: u32,
    pub pix_fmt: String,
    pub time_base_num: i32,
    pub time_base_den: i32,
    pub frame_rate_num: i32,
    pub frame_rate_den: i32,
    pub extradata: Vec<u8>,
}

/// One encoded video packet surfaced by the packetized encoder sink.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPacket {
    pub data: Vec<u8>,
    pub pts: i64,
    pub dts: i64,
    pub duration: i64,
    pub tb_num: i32,
    pub tb_den: i32,
    pub is_keyframe: bool,
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

struct AudioState {
    in_tb: AVRational,
    out_idx: c_int,
}

pub struct Encoder {
    codec_ctx: CodecContext,
    fmt: *mut AVFormatContext,
    writer: Option<AvioWriter>,
    pending: Vec<VideoPacket>,
    params: VideoEncoderParams,
    audio: Option<AudioState>,
    video_out_idx: c_int,
    scaler: Option<Scaler>,
    dst_width: u32,
    dst_height: u32,
    dst_fmt: AVPixelFormat,
    time_base: AVRational,
    in_tb: AVRational,
    frame_ticks: i64,
    next_index: i64,
}

impl Encoder {
    pub fn new(config: EncodeConfig) -> Result<Encoder, Error> {
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
        codec_ctx.set_dimensions(config.width, config.height);
        codec_ctx.set_pix_fmt(config.pix_fmt);
        codec_ctx.set_time_base(config.time_base.num, config.time_base.den);
        codec_ctx.set_frame_rate(config.frame_rate.num, config.frame_rate.den);

        unsafe {
            if !guard.0.is_null() && (*(*guard.0).oformat).flags & AVFMT_GLOBALHEADER as c_int != 0
            {
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

        let mut audio_out_idx: c_int = -1;
        let mut video_out_idx: c_int = 0;
        if muxed {
            unsafe {
                let stream = avformat_new_stream(guard.0, ptr::null());
                if stream.is_null() {
                    return Err(Error::FfmpegError(
                        "avformat_new_stream returned null".to_string(),
                    ));
                }
                (*stream).time_base = config.time_base;
                video_out_idx = (*stream).index;
                let ret = avcodec_parameters_from_context((*stream).codecpar, codec_ctx.as_ptr());
                if ret < 0 {
                    return Err(Error::from_code(ret));
                }

                if let Some(audio) = &config.audio {
                    let audio_stream = avformat_new_stream(guard.0, ptr::null());
                    if audio_stream.is_null() {
                        return Err(Error::FfmpegError(
                            "avformat_new_stream returned null".to_string(),
                        ));
                    }
                    (*audio_stream).time_base = audio.time_base;
                    let ret =
                        avcodec_parameters_copy((*audio_stream).codecpar, audio.params.as_ptr());
                    if ret < 0 {
                        return Err(Error::from_code(ret));
                    }
                    // The demuxed input carries its source container's tag
                    // (e.g. `mp4a`), which the Matroska and AVI muxers
                    // reject as incompatible at `avformat_write_header`.
                    // Clear it so the muxer picks a compatible tag; MP4 and
                    // MOV overwrite it either way.
                    (*(*audio_stream).codecpar).codec_tag = 0;
                    audio_out_idx = (*audio_stream).index;
                }

                let mut header_opts: *mut AVDictionary = ptr::null_mut();
                let ret = avformat_write_header(guard.0, &mut header_opts);
                av_dict_free(&mut header_opts);
                if ret < 0 {
                    return Err(Error::from_code(ret));
                }
            }
        }

        let audio = config.audio.as_ref().map(|audio| AudioState {
            in_tb: audio.time_base,
            out_idx: audio_out_idx,
        });

        let fmt = guard.disarm();
        let time_base = config.time_base;
        let frame_rate = normalize_frame_rate(config.frame_rate);
        let frame_ticks = unsafe {
            let ticks = av_rescale_q(
                1,
                AVRational {
                    num: frame_rate.den,
                    den: frame_rate.num,
                },
                time_base,
            );
            ticks.max(1)
        };
        let pix_fmt_name = unsafe {
            let name = av_get_pix_fmt_name(config.pix_fmt);
            if name.is_null() {
                String::new()
            } else {
                std::ffi::CStr::from_ptr(name)
                    .to_string_lossy()
                    .into_owned()
            }
        };
        let params = VideoEncoderParams {
            codec_name: config.codec.name(),
            width: config.width,
            height: config.height,
            pix_fmt: pix_fmt_name,
            time_base_num: time_base.num,
            time_base_den: time_base.den,
            frame_rate_num: frame_rate.num,
            frame_rate_den: frame_rate.den,
            extradata: codec_ctx.extradata(),
        };

        Ok(Encoder {
            codec_ctx,
            fmt,
            writer,
            pending: Vec::new(),
            params,
            audio,
            video_out_idx,
            scaler: None,
            dst_width: config.width,
            dst_height: config.height,
            dst_fmt: config.pix_fmt,
            time_base,
            in_tb: config.time_base,
            frame_ticks,
            next_index: 0,
        })
    }

    pub fn params(&self) -> &VideoEncoderParams {
        &self.params
    }

    pub fn take_pending(&mut self) -> Vec<VideoPacket> {
        std::mem::take(&mut self.pending)
    }

    pub fn encode_frame(&mut self, frame: &mut Frame) -> Result<(), Error> {
        let raw_pts = frame.pts();
        let out_pts = if raw_pts == AV_NOPTS_VALUE {
            self.next_index * self.frame_ticks
        } else {
            unsafe { av_rescale_q(raw_pts, self.in_tb, self.time_base) }
        };
        self.next_index += 1;

        let needs_convert = frame.width() != self.dst_width
            || frame.height() != self.dst_height
            || frame.format() as AVPixelFormat != self.dst_fmt;

        if needs_convert {
            let (src_width, src_height, src_fmt) = (
                frame.width(),
                frame.height(),
                frame.format() as AVPixelFormat,
            );
            let reuse = self
                .scaler
                .as_ref()
                .is_some_and(|s| s.matches(src_width, src_height, src_fmt));
            if !reuse {
                self.scaler = Some(Scaler::new(
                    src_width,
                    src_height,
                    src_fmt,
                    self.dst_width,
                    self.dst_height,
                    self.dst_fmt,
                )?);
            }
            let scaler = self
                .scaler
                .as_ref()
                .ok_or_else(|| Error::FfmpegError("scaler missing".to_string()))?;
            let mut converted = scaler.convert(frame)?;
            converted.set_pts(out_pts);
            self.codec_ctx.send_frame(converted.as_ptr())?;
        } else {
            frame.set_pts(out_pts);
            self.codec_ctx.send_frame(frame.as_ptr())?;
        }
        self.drain_packets()
    }

    pub fn write_audio_packet(&mut self, packet: &Packet) -> Result<(), Error> {
        let Some(state) = &self.audio else {
            return Err(Error::InvalidInput);
        };
        if self.fmt.is_null() {
            return Err(Error::InvalidInput);
        }
        unsafe {
            let out_tb = (*(*(*self.fmt).streams.add(state.out_idx as usize))).time_base;
            av_packet_rescale_ts(packet.as_ptr(), state.in_tb, out_tb);
            (*packet.as_ptr()).stream_index = state.out_idx;
            let ret = av_interleaved_write_frame(self.fmt, packet.as_ptr());
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
        }
        Ok(())
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
                    let (data, pts, dts, duration, keyframe) = unsafe {
                        let slice = std::slice::from_raw_parts((*pkt).data, (*pkt).size as usize);
                        (
                            slice.to_vec(),
                            (*pkt).pts,
                            (*pkt).dts,
                            (*pkt).duration,
                            (*pkt).flags & AV_PKT_FLAG_KEY as c_int != 0,
                        )
                    };
                    self.pending.push(VideoPacket {
                        data,
                        pts,
                        dts,
                        duration,
                        tb_num: self.params.time_base_num,
                        tb_den: self.params.time_base_den,
                        is_keyframe: keyframe,
                    });
                    packet.unref();
                    continue;
                }
                unsafe {
                    let out_tb =
                        (*(*(*self.fmt).streams.add(self.video_out_idx as usize))).time_base;
                    av_packet_rescale_ts(packet.as_ptr(), self.time_base, out_tb);
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

    pub fn finish_drain(&mut self) -> Result<(), Error> {
        self.codec_ctx.send_frame(ptr::null())?;
        self.drain_packets()
    }

    pub fn finish(mut self) -> Result<Vec<u8>, Error> {
        if self.next_index == 0 {
            return Err(Error::InvalidInput);
        }
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

pub(crate) fn normalize_frame_rate(mut frame_rate: AVRational) -> AVRational {
    if frame_rate.num <= 0 || frame_rate.den <= 0 {
        frame_rate = AVRational { num: 25, den: 1 };
    }
    frame_rate
}
