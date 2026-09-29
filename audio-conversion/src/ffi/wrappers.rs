use super::*;
use crate::error::Error;
use std::ffi::CString;
use std::os::raw::c_int;
use std::ptr;

#[derive(Debug)]
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

    pub fn nb_samples(&self) -> i32 {
        unsafe { (*self.0).nb_samples }
    }

    pub fn sample_fmt(&self) -> i32 {
        unsafe { (*self.0).format }
    }

    pub fn sample_rate(&self) -> u32 {
        unsafe { (*self.0).sample_rate as u32 }
    }

    pub fn channels(&self) -> u8 {
        unsafe { (*self.0).ch_layout.nb_channels as u8 }
    }

    pub fn channel_layout_desc(&self) -> Option<String> {
        unsafe {
            let mut buf = [0 as std::os::raw::c_char; 128];
            let ret = av_channel_layout_describe(&(*self.0).ch_layout, buf.as_mut_ptr(), buf.len());
            if ret < 0 {
                return None;
            }
            Some(
                std::ffi::CStr::from_ptr(buf.as_ptr())
                    .to_string_lossy()
                    .into_owned(),
            )
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

    pub fn set_sample_fmt(&mut self, fmt: AVSampleFormat) {
        unsafe { (*self.0).sample_fmt = fmt };
    }

    pub fn set_sample_rate(&mut self, rate: u32) {
        unsafe { (*self.0).sample_rate = rate as c_int };
    }

    pub fn set_channels(&mut self, channels: u8) {
        unsafe { av_channel_layout_default(&mut (*self.0).ch_layout, channels as c_int) };
    }

    pub fn set_bit_rate(&mut self, bits_per_second: u64) {
        unsafe { (*self.0).bit_rate = bits_per_second as i64 };
    }

    pub fn set_time_base(&mut self, num: c_int, den: c_int) {
        unsafe {
            (*self.0).time_base = AVRational { num, den };
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

    pub fn extradata(&self) -> Vec<u8> {
        unsafe {
            let ctx = self.0;
            if (*ctx).extradata.is_null() || (*ctx).extradata_size <= 0 {
                return Vec::new();
            }
            std::slice::from_raw_parts((*ctx).extradata, (*ctx).extradata_size as usize).to_vec()
        }
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

    pub fn name(&self) -> String {
        unsafe {
            if (*self.0).name.is_null() {
                return String::new();
            }
            std::ffi::CStr::from_ptr((*self.0).name)
                .to_string_lossy()
                .into_owned()
        }
    }
}

pub fn to_cstr(s: &str) -> Result<CString, Error> {
    CString::new(s).map_err(|_| Error::InvalidInput)
}
