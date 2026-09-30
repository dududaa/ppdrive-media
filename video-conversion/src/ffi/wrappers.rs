use super::*;
use crate::error::Error;
use std::ffi::{CStr, CString};
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

    pub(crate) fn from_ptr(ptr: *mut AVFrame) -> Frame {
        Frame(ptr)
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

    pub fn pts(&self) -> i64 {
        unsafe { (*self.0).pts }
    }

    pub fn set_pts(&mut self, pts: i64) {
        unsafe { (*self.0).pts = pts };
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

    pub fn ref_from(&mut self, src: *const AVPacket) -> Result<(), Error> {
        let ret = unsafe { av_packet_ref(self.0, src) };
        if ret < 0 {
            return Err(Error::from_code(ret));
        }
        Ok(())
    }

    pub fn pts(&self) -> i64 {
        unsafe { (*self.0).pts }
    }

    pub fn set_pts(&mut self, pts: i64) {
        unsafe { (*self.0).pts = pts };
    }

    pub fn dts(&self) -> i64 {
        unsafe { (*self.0).dts }
    }

    pub fn set_dts(&mut self, dts: i64) {
        unsafe { (*self.0).dts = dts };
    }

    pub fn duration(&self) -> i64 {
        unsafe { (*self.0).duration }
    }

    pub fn size(&self) -> usize {
        unsafe { (*self.0).size as usize }
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

    pub fn set_frame_rate(&mut self, num: c_int, den: c_int) {
        unsafe {
            (*self.0).framerate = AVRational { num, den };
        }
    }

    pub fn set_global_header(&mut self) {
        unsafe { (*self.0).flags |= AV_CODEC_FLAG_GLOBAL_HEADER as c_int };
    }

    pub fn extradata(&self) -> Vec<u8> {
        unsafe {
            let ptr = (*self.0).extradata;
            if ptr.is_null() {
                return Vec::new();
            }
            let size = (*self.0).extradata_size as usize;
            std::slice::from_raw_parts(ptr, size).to_vec()
        }
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

/// Owned copy of an `AVCodecParameters`, used to clone a demuxed audio
/// stream into a new muxed output stream without decoding it.
pub struct CodecParameters(*mut AVCodecParameters);

impl CodecParameters {
    pub fn new() -> Result<CodecParameters, Error> {
        let ptr = unsafe { avcodec_parameters_alloc() };
        if ptr.is_null() {
            return Err(Error::FfmpegError(
                "avcodec_parameters_alloc returned null".to_string(),
            ));
        }
        Ok(CodecParameters(ptr))
    }

    pub fn as_ptr(&self) -> *const AVCodecParameters {
        self.0
    }

    pub fn copy_from(&mut self, src: *const AVCodecParameters) -> Result<(), Error> {
        if src.is_null() {
            return Err(Error::InvalidInput);
        }
        let ret = unsafe { avcodec_parameters_copy(self.0, src) };
        if ret < 0 {
            return Err(Error::from_code(ret));
        }
        Ok(())
    }

    pub fn codec_id(&self) -> u32 {
        unsafe { (*self.0).codec_id }
    }
}

impl Drop for CodecParameters {
    fn drop(&mut self) {
        unsafe { avcodec_parameters_free(&mut self.0) };
    }
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

    pub fn name(&self) -> String {
        unsafe {
            if (*self.0).name.is_null() {
                return String::new();
            }
            CStr::from_ptr((*self.0).name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

/// Cached `libswscale` context for repeated frame conversion — video
/// pipelines convert every frame, so the context is created once per
/// source/destination shape instead of per frame.
pub struct Scaler {
    ctx: *mut SwsContext,
    src_width: u32,
    src_height: u32,
    src_fmt: AVPixelFormat,
    dst_width: u32,
    dst_height: u32,
    dst_fmt: AVPixelFormat,
}

impl Scaler {
    pub fn new(
        src_width: u32,
        src_height: u32,
        src_fmt: AVPixelFormat,
        dst_width: u32,
        dst_height: u32,
        dst_fmt: AVPixelFormat,
    ) -> Result<Scaler, Error> {
        if src_width == 0 || src_height == 0 || dst_width == 0 || dst_height == 0 {
            return Err(Error::InvalidInput);
        }
        let ctx = unsafe {
            sws_getContext(
                src_width as c_int,
                src_height as c_int,
                src_fmt,
                dst_width as c_int,
                dst_height as c_int,
                dst_fmt,
                SWS_BILINEAR as c_int,
                ptr::null_mut(),
                ptr::null_mut(),
                ptr::null_mut(),
            )
        };
        if ctx.is_null() {
            return Err(Error::FfmpegError(
                "sws_getContext returned null".to_string(),
            ));
        }
        Ok(Scaler {
            ctx,
            src_width,
            src_height,
            src_fmt,
            dst_width,
            dst_height,
            dst_fmt,
        })
    }

    pub fn matches(&self, src_width: u32, src_height: u32, src_fmt: AVPixelFormat) -> bool {
        self.src_width == src_width && self.src_height == src_height && self.src_fmt == src_fmt
    }

    pub fn convert(&self, src: &Frame) -> Result<Frame, Error> {
        let src_height = self.src_height;
        let dst = Frame::new()?;
        unsafe {
            (*dst.0).width = self.dst_width as c_int;
            (*dst.0).height = self.dst_height as c_int;
            (*dst.0).format = self.dst_fmt as c_int;
            let ret = av_frame_get_buffer(dst.0, 32);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            let ret = av_frame_make_writable(dst.0);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }

            let src_ptr = src.as_ptr();
            let dst_ptr = dst.0;
            let scaled = sws_scale(
                self.ctx,
                (*src_ptr).data.as_ptr() as *const *const u8,
                (*src_ptr).linesize.as_ptr(),
                0,
                src_height as c_int,
                (*dst_ptr).data.as_ptr(),
                (*dst_ptr).linesize.as_ptr(),
            );
            if scaled <= 0 {
                return Err(Error::FfmpegError("sws_scale failed".to_string()));
            }
            (*dst_ptr).pts = (*src_ptr).pts;
        }
        Ok(dst)
    }
}

impl Drop for Scaler {
    fn drop(&mut self) {
        if !self.ctx.is_null() {
            unsafe { sws_freeContext(self.ctx) };
            self.ctx = ptr::null_mut();
        }
    }
}

pub fn to_cstr(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::InvalidInput)
}
