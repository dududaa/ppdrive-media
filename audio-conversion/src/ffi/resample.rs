use super::wrappers::Frame;
use super::*;
use crate::error::Error;
use std::ptr;

pub struct Swr {
    ctx: *mut SwrContext,
    in_rate: u32,
    out_rate: u32,
    out_channels: u8,
    out_fmt: AVSampleFormat,
}

impl Swr {
    pub fn new(
        in_fmt: AVSampleFormat,
        in_rate: u32,
        in_channels: u8,
        out_fmt: AVSampleFormat,
        out_rate: u32,
        out_channels: u8,
    ) -> Result<Swr, Error> {
        if in_rate == 0 || out_rate == 0 || in_channels == 0 || out_channels == 0 {
            return Err(Error::InvalidInput);
        }
        unsafe {
            let mut in_layout: AVChannelLayout = std::mem::zeroed();
            let mut out_layout: AVChannelLayout = std::mem::zeroed();
            av_channel_layout_default(&mut in_layout, in_channels as c_int);
            av_channel_layout_default(&mut out_layout, out_channels as c_int);

            let mut ctx: *mut SwrContext = ptr::null_mut();
            let ret = swr_alloc_set_opts2(
                &mut ctx,
                &out_layout,
                out_fmt,
                out_rate as c_int,
                &in_layout,
                in_fmt,
                in_rate as c_int,
                0,
                ptr::null_mut(),
            );
            av_channel_layout_uninit(&mut in_layout);
            av_channel_layout_uninit(&mut out_layout);
            if ret < 0 || ctx.is_null() {
                swr_free(&mut ctx);
                return Err(Error::from_code(ret));
            }

            let ret = swr_init(ctx);
            if ret < 0 {
                swr_free(&mut ctx);
                return Err(Error::from_code(ret));
            }

            Ok(Swr {
                ctx,
                in_rate,
                out_rate,
                out_channels,
                out_fmt,
            })
        }
    }

    pub fn convert(&mut self, input: Option<&Frame>) -> Result<Option<Frame>, Error> {
        unsafe {
            let (in_rate, out_rate) = (self.in_rate, self.out_rate);
            let pending = swr_get_delay(self.ctx, in_rate as i64);
            let in_samples = input.map(|f| f.nb_samples() as i64).unwrap_or(0);
            let out_samples =
                av_rescale_rnd(pending + in_samples, out_rate as i64, in_rate as i64, 3);
            if out_samples <= 0 {
                return Ok(None);
            }

            let out = Frame::new()?;
            (*out.as_ptr()).format = self.out_fmt as c_int;
            (*out.as_ptr()).sample_rate = out_rate as c_int;
            av_channel_layout_default(&mut (*out.as_ptr()).ch_layout, self.out_channels as c_int);
            (*out.as_ptr()).nb_samples = out_samples as c_int;
            let ret = av_frame_get_buffer(out.as_ptr(), 0);
            if ret < 0 {
                return Err(Error::from_code(ret));
            }
            let ret = av_frame_make_writable(out.as_ptr());
            if ret < 0 {
                return Err(Error::from_code(ret));
            }

            let (in_ptr, in_count): (*mut *const u8, c_int) = match input {
                Some(frame) => (
                    (*frame.as_ptr()).data.as_ptr() as *mut *const u8,
                    frame.nb_samples(),
                ),
                None => (ptr::null_mut(), 0),
            };
            let converted = swr_convert(
                self.ctx,
                (*out.as_ptr()).data.as_mut_ptr(),
                out_samples as c_int,
                in_ptr,
                in_count,
            );
            if converted < 0 {
                return Err(Error::from_code(converted));
            }
            if converted == 0 {
                return Ok(None);
            }
            (*out.as_ptr()).nb_samples = converted;

            match input {
                Some(frame) if (*frame.as_ptr()).pts != AV_NOPTS_VALUE => {
                    let from = AVRational {
                        num: 1,
                        den: in_rate as c_int,
                    };
                    let to = AVRational {
                        num: 1,
                        den: out_rate as c_int,
                    };
                    (*out.as_ptr()).pts = av_rescale_q((*frame.as_ptr()).pts, from, to);
                }
                _ => (*out.as_ptr()).pts = 0,
            }

            Ok(Some(out))
        }
    }
}

impl Drop for Swr {
    fn drop(&mut self) {
        unsafe { swr_free(&mut self.ctx) };
    }
}
