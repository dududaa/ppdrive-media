use super::*;
use audio_conversion::{AudioStreamParams, Error};
use std::ffi::{CStr, CString};
use std::ptr;

/// RAII wrapper for a filter-graph output frame (`av_buffersink_get_frame`).
///
/// Frees the underlying `AVFrame` on drop unless ownership was moved out
/// with [`SinkFrame::into_raw`].
pub struct SinkFrame(*mut AVFrame);

impl SinkFrame {
    fn new() -> Result<SinkFrame, Error> {
        unsafe {
            let frame = av_frame_alloc();
            if frame.is_null() {
                return Err(Error::FfmpegError(
                    "av_frame_alloc returned null".to_string(),
                ));
            }
            Ok(SinkFrame(frame))
        }
    }

    pub fn as_ptr(&self) -> *mut AVFrame {
        self.0
    }

    /// Transfers ownership of the raw `AVFrame *` out of the wrapper.
    pub fn into_raw(mut self) -> *mut AVFrame {
        let ptr = self.0;
        self.0 = ptr::null_mut();
        ptr
    }
}

impl Drop for SinkFrame {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { av_frame_free(&mut self.0) };
        }
    }
}

struct GraphGuard(*mut AVFilterGraph);

impl GraphGuard {
    fn disarm(mut self) -> *mut AVFilterGraph {
        let graph = self.0;
        self.0 = ptr::null_mut();
        graph
    }
}

impl Drop for GraphGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe { avfilter_graph_free(&mut self.0) };
        }
    }
}

/// RAII wrapper for an `AVFilterGraph` laid out as
/// `abuffer → <filter chain> → abuffersink`.
pub struct FilterGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
}

impl FilterGraph {
    /// Builds the graph for one decoded audio stream, with `chain` as
    /// the filter description between the source and the sink.
    pub unsafe fn build(params: &AudioStreamParams, chain: &str) -> Result<FilterGraph, Error> {
        if chain.contains('\0') {
            return Err(Error::InvalidInput);
        }
        let chain = CString::new(chain).map_err(|_| Error::InvalidInput)?;
        unsafe { Self::build_inner(params, &chain) }
    }

    unsafe fn build_inner(
        params: &AudioStreamParams,
        chain: &CString,
    ) -> Result<FilterGraph, Error> {
        unsafe {
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err(Error::FfmpegError(
                    "avfilter_graph_alloc returned null".to_string(),
                ));
            }
            let guard = GraphGuard(graph);

            let abuffer = avfilter_get_by_name(c"abuffer".as_ptr());
            let abuffersink = avfilter_get_by_name(c"abuffersink".as_ptr());
            if abuffer.is_null() || abuffersink.is_null() {
                return Err(Error::FfmpegError(
                    "libavfilter abuffer/abuffersink filters unavailable".to_string(),
                ));
            }

            if params.sample_rate == 0 || params.channels == 0 || params.sample_fmt < 0 {
                return Err(Error::InvalidInput);
            }
            let fmt_name = av_get_sample_fmt_name(params.sample_fmt as AVSampleFormat);
            if fmt_name.is_null() {
                return Err(Error::InvalidInput);
            }
            let fmt_name = CStr::from_ptr(fmt_name).to_string_lossy().into_owned();
            if params.channel_layout.contains('\0') {
                return Err(Error::InvalidInput);
            }
            let args = format!(
                "time_base=1/{}:sample_rate={}:sample_fmt={}:channel_layout={}",
                params.sample_rate, params.sample_rate, fmt_name, params.channel_layout
            );
            let args = CString::new(args).map_err(|_| Error::InvalidInput)?;

            let mut src: *mut AVFilterContext = ptr::null_mut();
            let ret = avfilter_graph_create_filter(
                &mut src,
                abuffer,
                c"in".as_ptr(),
                args.as_ptr(),
                ptr::null_mut(),
                guard.0,
            );
            if ret < 0 {
                return Err(err_from_code(ret));
            }

            let mut sink: *mut AVFilterContext = ptr::null_mut();
            let ret = avfilter_graph_create_filter(
                &mut sink,
                abuffersink,
                c"out".as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                guard.0,
            );
            if ret < 0 {
                return Err(err_from_code(ret));
            }

            let mut outputs: *mut AVFilterInOut = avfilter_inout_alloc();
            let mut inputs: *mut AVFilterInOut = avfilter_inout_alloc();
            if outputs.is_null() || inputs.is_null() {
                if !outputs.is_null() {
                    avfilter_inout_free(&mut outputs);
                }
                if !inputs.is_null() {
                    avfilter_inout_free(&mut inputs);
                }
                return Err(Error::FfmpegError(
                    "avfilter_inout_alloc returned null".to_string(),
                ));
            }

            (*outputs).name = av_strdup(c"in".as_ptr());
            (*outputs).filter_ctx = src;
            (*outputs).pad_idx = 0;
            (*outputs).next = ptr::null_mut();

            (*inputs).name = av_strdup(c"out".as_ptr());
            (*inputs).filter_ctx = sink;
            (*inputs).pad_idx = 0;
            (*inputs).next = ptr::null_mut();

            let ret = avfilter_graph_parse_ptr(
                guard.0,
                chain.as_ptr(),
                &mut inputs,
                &mut outputs,
                ptr::null_mut(),
            );
            avfilter_inout_free(&mut inputs);
            avfilter_inout_free(&mut outputs);
            if ret < 0 {
                return Err(err_from_code(ret));
            }

            let ret = avfilter_graph_config(guard.0, ptr::null_mut());
            if ret < 0 {
                return Err(err_from_code(ret));
            }

            Ok(FilterGraph {
                graph: guard.disarm(),
                src,
                sink,
            })
        }
    }

    /// Pushes one frame into the graph (`av_buffersrc_write_frame`
    /// creates its own reference — the caller keeps ownership).
    ///
    /// # Safety
    ///
    /// `frame` must be a valid `AVFrame *` compatible with the
    /// rate/format/layout the graph was built with.
    pub unsafe fn push(&self, frame: *mut AVFrame) -> Result<(), Error> {
        let ret = unsafe { av_buffersrc_write_frame(self.src, frame) };
        if ret < 0 {
            return Err(err_from_code(ret));
        }
        Ok(())
    }

    /// Signals end-of-stream so buffered filters (`areverse`,
    /// `afade` out, …) can flush their output.
    pub unsafe fn close(&self) -> Result<(), Error> {
        let ret = unsafe { av_buffersrc_close(self.src, AV_NOPTS_VALUE, 0) };
        if ret < 0 {
            return Err(err_from_code(ret));
        }
        Ok(())
    }

    /// Pulls the next transformed frame out of the sink; `Ok(None)`
    /// once the graph is drained (`EAGAIN` or `EOF`).
    pub fn pull(&self) -> Result<Option<SinkFrame>, Error> {
        let out = SinkFrame::new()?;
        let ret = unsafe { av_buffersink_get_frame(self.sink, out.as_ptr()) };
        if ret == 0 {
            return Ok(Some(out));
        }
        if is_eagain(ret) || ret == AVERROR_EOF_CODE {
            return Ok(None);
        }
        Err(err_from_code(ret))
    }
}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        if !self.graph.is_null() {
            unsafe { avfilter_graph_free(&mut self.graph) };
        }
    }
}
