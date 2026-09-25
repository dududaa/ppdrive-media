use super::*;
use crate::error::Error;
use std::ffi::CString;
use std::os::raw::c_int;
use std::ptr;

pub struct Frame(*mut AVFrame);

impl Frame {
    pub fn new() -> Result<Frame, Error> {
        let ptr = unsafe { av_frame_alloc() };
        if ptr.is_null() {
            return Err(Error::FfmpegError(
                "av_frame_alloc returned null".to_string(),
            ));
        }
        Ok(Frame(ptr))
    }

    pub fn as_ptr(&self) -> *mut AVFrame {
        self.0
    }

    pub fn width(&self) -> u32 {
        unsafe { (*self.0).width as u32 }
    }

    pub fn height(&self) -> u32 {
        unsafe { (*self.0).height as u32 }
    }

    pub fn format(&self) -> i32 {
        unsafe { (*self.0).format }
    }

    pub fn has_alpha(&self) -> bool {
        let fmt = self.format() as AVPixelFormat;
        unsafe {
            let desc = av_pix_fmt_desc_get(fmt);
            !desc.is_null() && ((*desc).flags & AV_PIX_FMT_FLAG_ALPHA as u64) != 0
        }
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        unsafe { av_frame_free(&mut self.0) };
    }
}

pub struct Packet(*mut AVPacket);

impl Packet {
    pub fn new() -> Result<Packet, Error> {
        let ptr = unsafe { av_packet_alloc() };
        if ptr.is_null() {
            return Err(Error::FfmpegError(
                "av_packet_alloc returned null".to_string(),
            ));
        }
        Ok(Packet(ptr))
    }

    pub fn as_ptr(&self) -> *mut AVPacket {
        self.0
    }

    pub fn unref(&mut self) {
        unsafe { av_packet_unref(self.0) };
    }
}

impl Drop for Packet {
    fn drop(&mut self) {
        unsafe { av_packet_free(&mut self.0) };
    }
}

pub struct CodecContext(*mut AVCodecContext);

impl CodecContext {
    pub fn alloc(codec: *const AVCodec) -> Result<CodecContext, Error> {
        let ptr = unsafe { avcodec_alloc_context3(codec) };
        if ptr.is_null() {
            return Err(Error::FfmpegError(
                "avcodec_alloc_context3 returned null".to_string(),
            ));
        }
        Ok(CodecContext(ptr))
    }

    pub fn as_ptr(&self) -> *mut AVCodecContext {
        self.0
    }

    pub fn set_dimensions(&mut self, width: u32, height: u32) {
        unsafe {
            (*self.0).width = width as c_int;
            (*self.0).height = height as c_int;
        }
    }

    pub fn set_pix_fmt(&mut self, fmt: AVPixelFormat) {
        unsafe { (*self.0).pix_fmt = fmt };
    }

    pub fn set_time_base(&mut self, num: c_int, den: c_int) {
        unsafe {
            (*self.0).time_base = AVRational { num, den };
        }
    }

    pub fn set_qscale(&mut self, qscale: i32) {
        unsafe {
            (*self.0).flags |= AV_CODEC_FLAG_QSCALE as c_int;
            (*self.0).global_quality = (qscale * FF_QP2LAMBDA as i32) as c_int;
        }
    }

    pub fn set_global_header(&mut self) {
        unsafe { (*self.0).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as c_int };
    }

    pub fn open(
        &mut self,
        codec: *const AVCodec,
        options: &[(String, String)],
    ) -> Result<(), Error> {
        let mut dict: *mut AVDictionary = ptr::null_mut();
        for (key, value) in options {
            let key = CString::new(key.as_str()).map_err(|_| Error::InvalidInput)?;
            let value = CString::new(value.as_str()).map_err(|_| Error::InvalidInput)?;
            unsafe {
                av_dict_set(&mut dict, key.as_ptr(), value.as_ptr(), 0);
            }
        }
        let ret = unsafe { avcodec_open2(self.0, codec, &mut dict) };
        unsafe { av_dict_free(&mut dict) };
        if ret < 0 {
            return Err(Error::from_code(ret));
        }
        Ok(())
    }

    pub fn send_frame(&mut self, frame: *const AVFrame) -> Result<(), Error> {
        let ret = unsafe { avcodec_send_frame(self.0, frame) };
        if ret < 0 {
            return Err(Error::from_code(ret));
        }
        Ok(())
    }
}

impl Drop for CodecContext {
    fn drop(&mut self) {
        unsafe { avcodec_free_context(&mut self.0) };
    }
}

pub fn find_encoder_by_id(id: AVCodecID) -> Result<EncoderHandle, Error> {
    let codec = unsafe { avcodec_find_encoder(id) };
    if codec.is_null() {
        return Err(Error::EncoderNotFound);
    }
    Ok(EncoderHandle(codec))
}

pub fn find_encoder_by_name(name: &str) -> Result<EncoderHandle, Error> {
    let name = CString::new(name).map_err(|_| Error::InvalidInput)?;
    let codec = unsafe { avcodec_find_encoder_by_name(name.as_ptr()) };
    if codec.is_null() {
        return Err(Error::EncoderNotFound);
    }
    Ok(EncoderHandle(codec))
}

pub struct EncoderHandle(*const AVCodec);

impl EncoderHandle {
    pub fn as_ptr(&self) -> *const AVCodec {
        self.0
    }
}

pub fn scale_frame(
    src: &Frame,
    width: u32,
    height: u32,
    dst_fmt: AVPixelFormat,
) -> Result<Frame, Error> {
    let src_width = src.width();
    let src_height = src.height();
    if src_width == 0 || src_height == 0 || width == 0 || height == 0 {
        return Err(Error::InvalidInput);
    }
    let src_fmt = src.format() as AVPixelFormat;

    let dst = Frame::new()?;
    unsafe {
        (*dst.0).width = width as c_int;
        (*dst.0).height = height as c_int;
        (*dst.0).format = dst_fmt as c_int;
        let ret = av_frame_get_buffer(dst.0, 32);
        if ret < 0 {
            return Err(Error::from_code(ret));
        }
        let ret = av_frame_make_writable(dst.0);
        if ret < 0 {
            return Err(Error::from_code(ret));
        }

        let sws = sws_getContext(
            src_width as c_int,
            src_height as c_int,
            src_fmt,
            width as c_int,
            height as c_int,
            dst_fmt,
            SWS_BILINEAR as c_int,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
        );
        if sws.is_null() {
            return Err(Error::FfmpegError(
                "sws_getContext returned null".to_string(),
            ));
        }

        let src_ptr = src.as_ptr();
        let dst_ptr = dst.0;
        let scaled = sws_scale(
            sws,
            (*src_ptr).data.as_ptr() as *const *const u8,
            (*src_ptr).linesize.as_ptr(),
            0,
            src_height as c_int,
            (*dst_ptr).data.as_ptr(),
            (*dst_ptr).linesize.as_ptr(),
        );
        sws_freeContext(sws);
        if scaled <= 0 {
            return Err(Error::FfmpegError("sws_scale failed".to_string()));
        }
    }

    Ok(dst)
}

pub fn to_cstr(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::InvalidInput)
}
