use super::*;
use crate::error::Error;
use std::os::raw::{c_int, c_void};
use std::ptr;

const AVIO_BUFFER_SIZE: usize = 32 * 1024;

struct ReadState<'a> {
    data: &'a [u8],
    pos: usize,
}

extern "C" fn read_callback(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int {
    let state = unsafe { &mut *(opaque as *mut ReadState<'_>) };
    if state.pos >= state.data.len() {
        return AVERROR_EOF_CODE;
    }
    let remaining = state.data.len() - state.pos;
    let to_copy = remaining.min(buf_size as usize);
    unsafe {
        ptr::copy_nonoverlapping(state.data.as_ptr().add(state.pos), buf, to_copy);
    }
    state.pos += to_copy;
    to_copy as c_int
}

pub struct AvioReader<'a> {
    avio: *mut AVIOContext,
    state: *mut ReadState<'a>,
}

impl<'a> AvioReader<'a> {
    pub fn new(data: &'a [u8]) -> Result<AvioReader<'a>, Error> {
        unsafe {
            let buffer = av_malloc(AVIO_BUFFER_SIZE) as *mut u8;
            if buffer.is_null() {
                return Err(Error::FfmpegError("av_malloc failed".to_string()));
            }

            let state = Box::into_raw(Box::new(ReadState { data, pos: 0 }));

            let avio = avio_alloc_context(
                buffer,
                AVIO_BUFFER_SIZE as c_int,
                0,
                state as *mut c_void,
                Some(read_callback),
                None,
                None,
            );

            if avio.is_null() {
                av_free(buffer as *mut c_void);
                drop(Box::from_raw(state));
                return Err(Error::FfmpegError(
                    "avio_alloc_context returned null".to_string(),
                ));
            }

            Ok(AvioReader { avio, state })
        }
    }

    pub fn as_ptr(&mut self) -> *mut AVIOContext {
        self.avio
    }
}

impl<'a> Drop for AvioReader<'a> {
    fn drop(&mut self) {
        unsafe {
            if !self.avio.is_null() {
                av_freep(&mut (*self.avio).buffer as *mut *mut u8 as *mut c_void);
                avio_context_free(&mut self.avio);
            }
            if !self.state.is_null() {
                drop(Box::from_raw(self.state));
                self.state = ptr::null_mut();
            }
        }
    }
}

struct WriteState {
    buf: Vec<u8>,
    pos: usize,
}

impl WriteState {
    fn ensure_capacity(&mut self, end: usize) {
        if end > self.buf.len() {
            self.buf.resize(end, 0);
        }
    }

    fn seek_to(&mut self, offset: i64, whence: c_int) -> i64 {
        if whence & AVSEEK_SIZE as c_int != 0 {
            return self.buf.len() as i64;
        }
        let base = match whence & !(AVSEEK_SIZE as c_int | AVSEEK_FORCE as c_int) {
            0 => 0i64,
            1 => self.pos as i64,
            2 => self.buf.len() as i64,
            _ => return -1,
        };
        let target = base + offset;
        if target < 0 {
            return -1;
        }
        self.ensure_capacity(target as usize);
        self.pos = target as usize;
        target
    }
}

#[cfg(ffmpeg_avio_write_nonconst)]
extern "C" fn write_callback(opaque: *mut c_void, buf: *mut u8, buf_size: c_int) -> c_int {
    write_bytes(opaque, buf, buf_size)
}

#[cfg(not(ffmpeg_avio_write_nonconst))]
extern "C" fn write_callback(opaque: *mut c_void, buf: *const u8, buf_size: c_int) -> c_int {
    write_bytes(opaque, buf, buf_size)
}

fn write_bytes(opaque: *mut c_void, buf: *const u8, buf_size: c_int) -> c_int {
    let state = unsafe { &mut *(opaque as *mut WriteState) };
    let slice = unsafe { std::slice::from_raw_parts(buf, buf_size as usize) };
    state.ensure_capacity(state.pos + slice.len());
    state.buf[state.pos..state.pos + slice.len()].copy_from_slice(slice);
    state.pos += slice.len();
    buf_size
}

extern "C" fn seek_callback(opaque: *mut c_void, offset: i64, whence: c_int) -> i64 {
    let state = unsafe { &mut *(opaque as *mut WriteState) };
    state.seek_to(offset, whence)
}

pub struct AvioWriter {
    avio: *mut AVIOContext,
    state: *mut WriteState,
}

impl AvioWriter {
    pub fn new() -> Result<AvioWriter, Error> {
        unsafe {
            let buffer = av_malloc(AVIO_BUFFER_SIZE) as *mut u8;
            if buffer.is_null() {
                return Err(Error::FfmpegError("av_malloc failed".to_string()));
            }

            let state = Box::into_raw(Box::new(WriteState {
                buf: Vec::new(),
                pos: 0,
            }));

            let avio = avio_alloc_context(
                buffer,
                AVIO_BUFFER_SIZE as c_int,
                1,
                state as *mut c_void,
                None,
                Some(write_callback),
                Some(seek_callback),
            );
            if avio.is_null() {
                av_free(buffer as *mut c_void);
                drop(Box::from_raw(state));
                return Err(Error::FfmpegError(
                    "avio_alloc_context returned null".to_string(),
                ));
            }

            Ok(AvioWriter { avio, state })
        }
    }

    pub fn as_ptr(&mut self) -> *mut AVIOContext {
        self.avio
    }

    pub fn flush(&mut self) {
        unsafe {
            if !self.avio.is_null() {
                avio_flush(self.avio);
            }
        }
    }

    pub fn take_bytes(&mut self) -> Vec<u8> {
        self.flush();
        if self.state.is_null() {
            return Vec::new();
        }
        unsafe {
            let state = Box::from_raw(self.state);
            self.state = ptr::null_mut();
            state.buf
        }
    }
}

impl Drop for AvioWriter {
    fn drop(&mut self) {
        unsafe {
            if !self.avio.is_null() {
                av_freep(&mut (*self.avio).buffer as *mut *mut u8 as *mut c_void);
                avio_context_free(&mut self.avio);
            }
            if !self.state.is_null() {
                drop(Box::from_raw(self.state));
                self.state = ptr::null_mut();
            }
        }
    }
}
