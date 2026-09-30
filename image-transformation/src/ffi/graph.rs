use super::*;
use image_conversion::Error;
use std::ffi::CString;
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
/// `buffer → <filter chain> → buffersink`.
pub struct FilterGraph {
    graph: *mut AVFilterGraph,
    src: *mut AVFilterContext,
    sink: *mut AVFilterContext,
}

impl FilterGraph {
    /// Builds the graph for one still image of the given dimensions and
    /// pixel format, with `chain` as the filter description between the
    /// source and the sink.
    ///
    /// # Safety
    ///
    /// `frame` must point to a live `AVFrame` (its `width`, `height` and
    /// `format` fields are read to configure the buffer source; the
    /// frame itself is not referenced).
    pub unsafe fn build(frame: *mut AVFrame, chain: &str) -> Result<FilterGraph, Error> {
        if frame.is_null() {
            return Err(Error::InvalidInput);
        }
        if chain.contains('\0') {
            return Err(Error::InvalidInput);
        }
        let chain = CString::new(chain).map_err(|_| Error::InvalidInput)?;
        unsafe { Self::build_inner(frame, &chain) }
    }

    unsafe fn build_inner(frame: *mut AVFrame, chain: &CString) -> Result<FilterGraph, Error> {
        unsafe {
            let graph = avfilter_graph_alloc();
            if graph.is_null() {
                return Err(Error::FfmpegError(
                    "avfilter_graph_alloc returned null".to_string(),
                ));
            }
            let guard = GraphGuard(graph);

            let buffer = avfilter_get_by_name(c"buffer".as_ptr());
            let buffersink = avfilter_get_by_name(c"buffersink".as_ptr());
            if buffer.is_null() || buffersink.is_null() {
                return Err(Error::FfmpegError(
                    "libavfilter buffer/buffersink filters unavailable".to_string(),
                ));
            }

            let (width, height, pix_fmt) = ((*frame).width, (*frame).height, (*frame).format);
            if width <= 0 || height <= 0 || pix_fmt < 0 {
                return Err(Error::InvalidInput);
            }
            let args = format!(
                "video_size={}x{}:pix_fmt={}:time_base=1/1:pixel_aspect=1/1",
                width, height, pix_fmt
            );
            let args = CString::new(args).map_err(|_| Error::InvalidInput)?;

            let mut src: *mut AVFilterContext = ptr::null_mut();
            let ret = avfilter_graph_create_filter(
                &mut src,
                buffer,
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
                buffersink,
                c"out".as_ptr(),
                ptr::null(),
                ptr::null_mut(),
                guard.0,
            );
            if ret < 0 {
                return Err(err_from_code(ret));
            }

            // `outputs` = where the chain's open inputs connect (our src,
            // labeled "in"); `inputs` = where the chain's open outputs
            // connect (our sink, labeled "out"). Leftover open entries
            // are returned in these lists after parsing.
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
    /// dimensions/pixel format the graph was built with.
    pub unsafe fn push(&self, frame: *mut AVFrame) -> Result<(), Error> {
        let ret = unsafe { av_buffersrc_write_frame(self.src, frame) };
        if ret < 0 {
            return Err(err_from_code(ret));
        }
        Ok(())
    }

    /// Pulls the single transformed frame out of the sink.
    pub fn pull(&self) -> Result<SinkFrame, Error> {
        let out = SinkFrame::new()?;
        let ret = unsafe { av_buffersink_get_frame(self.sink, out.as_ptr()) };
        if ret < 0 {
            return Err(err_from_code(ret));
        }
        Ok(out)
    }
}

impl Drop for FilterGraph {
    fn drop(&mut self) {
        if !self.graph.is_null() {
            unsafe { avfilter_graph_free(&mut self.graph) };
        }
    }
}
